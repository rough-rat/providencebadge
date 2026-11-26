//! Providence badge firmware on py32-badgekit: two 12-LED charlieplex rings
//! (pink hours, white half-steps between them) and one button on PB0.
//!
//! The legacy firmware in ../fw is kept as it is; this is its port.

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cortex_m::peripheral::SCB;
use cortex_m_rt::{exception, ExceptionFrame};
use defmt::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant};

use py32_badgekit::button::Button;
use py32_badgekit::charlieplex::{self, Buffers, Led};
use py32_badgekit::power::{self, WakePin};
use py32_badgekit::Port;
use py32_hal::gpio::{Input, Pull};
use py32_hal::rcc::Sysclk;
use py32_hal::time::Hertz;
use {defmt_rtt as _, panic_halt as _};

// ---- Board -----------------------------------------------------------------

/// Highest HSI setting that needs no flash wait state.
const SYSCLK_HZ: u32 = 24_000_000;

/// 12 slots of 10000 cycles each: ~13 bits of brightness.
const REFRESH_HZ: u32 = 200;

/// Pink hour ring on PA0..PA3, white half-step ring on PA4, PA5, PA6, PA8.
/// Separate matrices, so one LED of each is lit at the same time.
const PINK_PINS: [u8; 4] = [0, 1, 2, 3];
const WHITE_PINS: [u8; 4] = [4, 5, 6, 8];
const RING_LEDS: usize = 12;
const LED_COUNT: usize = 2 * RING_LEDS;

/// Both rings are wired the same way. Position along the ring (clockwise,
/// from 12 o'clock) -> matrix offset, and offset -> (cathode, anode) as
/// indexes into the ring's pins. Taken over from the legacy firmware.
const RING_TO_OFFSET: [usize; RING_LEDS] = [3, 6, 9, 0, 7, 10, 1, 4, 11, 2, 5, 8];
const OFFSET_TO_CATHODE_ANODE: [(usize, usize); RING_LEDS] = [
    (0, 1),
    (0, 2),
    (0, 3),
    (1, 0),
    (1, 2),
    (1, 3),
    (2, 0),
    (2, 1),
    (2, 3),
    (3, 0),
    (3, 1),
    (3, 2),
];

const fn ring(pins: [u8; 4]) -> [Led; RING_LEDS] {
    let mut leds = [Led::new(Port::A, 0, 1); RING_LEDS];
    let mut i = 0;
    while i < RING_LEDS {
        let (cathode, anode) = OFFSET_TO_CATHODE_ANODE[RING_TO_OFFSET[i]];
        leds[i] = Led::new(Port::A, pins[anode], pins[cathode]);
        i += 1;
    }
    leds
}

/// LED indexes: bank A 0..=11 (H12, H1, ...), bank B 12..=23 (H12+, H1+,
/// ...), each bank B LED halfway to the next hour (../led-layout.svg calls
/// them pink and white; for now pink LEDs are fitted in both).
static PINK: [Led; RING_LEDS] = ring(PINK_PINS);
static WHITE: [Led; RING_LEDS] = ring(WHITE_PINS);
static LED_GROUPS: [&[Led]; 2] = [&PINK, &WHITE];
static LED_BUFFERS: Buffers<LED_COUNT, RING_LEDS> = Buffers::new();

/// Button to ground on PB0, with an external pull-up.
const WAKE: WakePin = WakePin::new(Port::B, 0);

/// LED index of position `pos` on the combined 24-LED ring, clockwise from
/// 12 o'clock: even positions are bank A, odd ones bank B.
fn ring_led(pos: usize) -> usize {
    let pos = pos % LED_COUNT;
    if pos % 2 == 0 {
        pos / 2
    } else {
        RING_LEDS + pos / 2
    }
}

// ---- Behaviour -------------------------------------------------------------

/// The badge turns itself off after this long without a button press.
const AWAKE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// After reset the badge stays awake this long (LEDs off) so a probe can
/// attach, then sleeps. A press in that time skips the sleep.
const FLASH_RESCUE_WINDOW: Duration = Duration::from_secs(5);

const FRAME: Duration = Duration::from_millis(20);

/// An attack: the cross closes in on the player for this long, then cuts
/// out at once.
const ATTACK_MS: u32 = 3_000;
/// Perceived level of the cross at the start, and of the whole ring at the
/// end.
const CROSS_LEVEL: i32 = 255;
const RING_LEVEL: i32 = 60;
/// The spinning cross turns half a revolution (12 of 24 positions) in this
/// long, then holds still for the rest of the attack.
const SPIN_STEPS: i32 = 12;
const SPIN_MS: u32 = 1_500;
/// Plain crosses between two spinning ones.
const CROSSES_MIN: u32 = 0;
const CROSSES_MAX: u32 = 2;
/// Random pause between attacks.
const GAP_MIN_MS: u32 = 500;
const GAP_MAX_MS: u32 = 2_000;

/// Fixed point: one ring position.
const STEP: i32 = 256;
const TURN: i32 = LED_COUNT as i32 * STEP;
/// The four beams are this far apart.
const ARM_GAP: i32 = TURN / 4;

#[derive(Clone, Copy, PartialEq, Eq, Format)]
enum Attack {
    /// Cross of beams heading straight at the player.
    Cross,
    /// The same cross, spinning as it closes in.
    Spin,
}

/// xorshift32 pseudorandom generator. Seeded from the time of the wake-up
/// press, so the sequence differs from one wake-up to the next.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform-ish in `lo..=hi`.
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + self.next() % (hi - lo + 1)
    }
}

/// Attack order: a spinning cross, then CROSSES_MIN..=CROSSES_MAX plain
/// ones (picked anew each time), then the next spinning one.
struct Sequence {
    crosses_left: u32,
}

impl Sequence {
    fn new() -> Self {
        Self { crosses_left: 0 }
    }

    fn next(&mut self, rng: &mut Rng) -> Attack {
        if self.crosses_left > 0 {
            self.crosses_left -= 1;
            Attack::Cross
        } else {
            self.crosses_left = rng.range(CROSSES_MIN, CROSSES_MAX);
            Attack::Spin
        }
    }
}

/// One attack frame at `t` ms. Each beam widens from one LED until the
/// beams meet and cover the whole ring, slowly at first and faster towards
/// the end (with the square of the time), while its level falls from
/// CROSS_LEVEL to RING_LEVEL. `rotation` is the cross's angle (fixed point
/// ring position).
fn draw_attack(t: u32, rotation: i32) {
    let t = t.min(ATTACK_MS) as i32;
    let total = ATTACK_MS as i32;
    // Half width of a beam: 0 at the start, half the gap between beams at
    // the end.
    let half_width = (ARM_GAP / 2) * t / total * t / total;
    let level = CROSS_LEVEL + (RING_LEVEL - CROSS_LEVEL) * t / total;

    for pos in 0..LED_COUNT {
        // Distance to the nearest beam, 0..=ARM_GAP/2.
        let d = (pos as i32 * STEP - rotation).rem_euclid(ARM_GAP);
        let d = d.min(ARM_GAP - d);
        // Soft edge one position wide, so a rotating beam moves smoothly
        // from LED to LED.
        let cover = (half_width + STEP - d).clamp(0, STEP);
        charlieplex::set_level(ring_led(pos), (level * cover / STEP) as u8);
    }
}

/// Plays one attack. True if the button was pressed.
async fn attack(button: &mut Button<'_>, kind: Attack, start_pos: i32) -> bool {
    let start = Instant::now();
    loop {
        let t = (Instant::now() - start).as_millis() as u32;
        if t >= ATTACK_MS {
            break;
        }

        let spin = match kind {
            Attack::Cross => 0,
            Attack::Spin => SPIN_STEPS * STEP * t.min(SPIN_MS) as i32 / SPIN_MS as i32,
        };
        draw_attack(t, start_pos + spin);
        charlieplex::present().await;

        if button.pressed_within(FRAME).await {
            return true;
        }
    }

    // The hit: everything off at once.
    charlieplex::clear();
    charlieplex::present().await;
    false
}

/// Runs until the button is pressed or the badge has been on long enough.
async fn animate(button: &mut Button<'_>, rng: &mut Rng) {
    let deadline = Instant::now() + AWAKE_TIMEOUT;
    let mut sequence = Sequence::new();

    while Instant::now() < deadline {
        let kind = sequence.next(rng);
        // Either orientation of the cross: on the hours or between them.
        let start_pos = (rng.next() % 2) as i32 * STEP;
        debug!("attack {}", kind);
        if attack(button, kind, start_pos).await {
            return;
        }

        let gap = rng.range(GAP_MIN_MS, GAP_MAX_MS);
        if button.pressed_within(Duration::from_millis(gap as u64)).await {
            return;
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    // Recovery window: stay awake with every pin in its reset state for a
    // second or two (HSI reset default, before any clock setup), so a probe
    // can always attach right after power-up even if the code below ever
    // breaks SWD or sleeps too deep. Keep this first.
    cortex_m::asm::delay(8_000_000);

    let mut cfg: py32_hal::Config = Default::default();
    cfg.rcc.hsi = Some(Hertz::mhz(SYSCLK_HZ / 1_000_000));
    cfg.rcc.sys = Sysclk::HSI;
    let p = py32_hal::init(cfg);

    info!("Hello World!");

    // PA13/PA14 (SWD) and PF2 (nRST) are left alone on purpose.
    let syst = cortex_m::Peripherals::take().unwrap().SYST;
    charlieplex::init(syst, SYSCLK_HZ, REFRESH_HZ, &LED_GROUPS, &LED_BUFFERS);
    // Few LEDs lit (the cross) -> those get a bigger share of the time and
    // shine brighter; the whole ring lit -> each gets 1/12.
    charlieplex::set_skip_dark(true);
    let timing = charlieplex::timing();
    info!(
        "charlieplex: window {} cycles, max on-time {} cycles",
        timing.window_cycles, timing.max_on_cycles
    );
    charlieplex::stop();

    let mut button = Button::new(Input::new(p.PB0, Pull::None));

    info!("flash rescue window");
    if button.pressed_within(FLASH_RESCUE_WINDOW).await {
        info!("button pressed during rescue window; staying awake");
    } else {
        info!("off");
        power::stop(WAKE);
    }
    button.wait_release().await;

    let mut rng = Rng::new(0);
    loop {
        info!("on");
        // Mix in when the button was let go.
        rng = Rng::new(rng.next() ^ Instant::now().as_ticks() as u32);
        charlieplex::start();
        animate(&mut button, &mut rng).await;

        info!("off");
        charlieplex::clear();
        charlieplex::present().await;
        charlieplex::stop();
        button.wait_release().await;
        power::stop(WAKE);
        button.wait_release().await;
    }
}

//conserve flash space with the non-handler
#[exception]
unsafe fn HardFault(_frame: &ExceptionFrame) -> ! {
    SCB::sys_reset()
}
