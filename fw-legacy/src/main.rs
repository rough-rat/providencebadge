
#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use cortex_m::peripheral::SCB;
use cortex_m_rt::{exception, ExceptionFrame};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Timer};

use py32_hal::gpio::{AnyPin, Flex, Input, Pin, Pull, Speed};

use py32_hal::rcc::Sysclk;
use py32_hal::time::Hertz;
use defmt::*;
use py32_hal::Peripheral;
use {defmt_rtt as _, panic_halt as _};

mod bat;
mod sleep;

const AWAKE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const BUTTON_DEBOUNCE: Duration = Duration::from_millis(40);
const FLASH_RESCUE_WINDOW: Duration = Duration::from_secs(5);
const LED_SCAN_HOLD_US: u64 = 1_200;
const CROSS_STEP: Duration = Duration::from_millis(2_000 / 12);
const WHITE_HOLD: Duration = Duration::from_secs(2);
const DARK_HOLD: Duration = Duration::from_secs(6);

const PINK_LAYOUT_TO_OFFSET: [usize; 12] = [3, 6, 9, 0, 7, 10, 1, 4, 11, 2, 5, 8];
const WHITE_LAYOUT_TO_OFFSET: [usize; 12] = PINK_LAYOUT_TO_OFFSET;
const OFFSET_TO_ROW_COL: [(usize, usize); 12] = [
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

const WHITE_TEST_LEDS: [usize; 4] = [14, 15, 20, 21];

struct DirectCharlieBank<'a, const PIN_COUNT: usize> {
    pin_list: [Flex<'a>; PIN_COUNT],
}

impl<'a, const PIN_COUNT: usize> DirectCharlieBank<'a, PIN_COUNT> {
    fn new(pin_list: [AnyPin; PIN_COUNT]) -> Self {
        let gpio_list = core::array::from_fn(|i| unsafe {
            Flex::new(pin_list[i].clone_unchecked())
        });

        let mut bank = Self {
            pin_list: gpio_list,
        };
        bank.all_tri_state();
        bank
    }

    fn all_tri_state(&mut self) {
        for pin in self.pin_list.iter_mut() {
            pin.set_as_input(Pull::None);
        }
    }

    fn all_analog(&mut self) {
        for pin in self.pin_list.iter_mut() {
            pin.set_as_analog();
        }
    }

    fn set_masks(&mut self, high_mask: u8, low_mask: u8) {
        self.all_tri_state();

        for i in 0..PIN_COUNT {
            let mask = 1 << i;
            if high_mask & mask != 0 {
                self.pin_list[i].set_high();
            } else if low_mask & mask != 0 {
                self.pin_list[i].set_low();
            }
        }

        for i in 0..PIN_COUNT {
            let mask = 1 << i;
            if (high_mask | low_mask) & mask != 0 {
                self.pin_list[i].set_as_output(Speed::VeryHigh);
            }
        }
    }

    fn set_led_by_offset(&mut self, offset: usize) {
        let (row_low, col_high) = OFFSET_TO_ROW_COL[offset];
        self.set_masks(1 << col_high, 1 << row_low);
    }
}

async fn wait_for_button_release(button: &Input<'_>) {
    while button.is_low() {
        Timer::after_millis(10).await;
    }
    Timer::after(BUTTON_DEBOUNCE).await;
}

async fn pressed_after_debounce(button: &Input<'_>) -> bool {
    if button.is_high() {
        return false;
    }

    Timer::after(BUTTON_DEBOUNCE).await;
    button.is_low()
}

async fn flash_rescue_window(button: &Input<'_>) {
    let deadline = Instant::now() + FLASH_RESCUE_WINDOW;

    while Instant::now() < deadline {
        if button.is_low() {
            info!("button held during rescue window; staying awake");
            wait_for_button_release(button).await;
            return;
        }

        Timer::after_millis(25).await;
    }
}

fn pink_offset(layout_index: usize) -> usize {
    PINK_LAYOUT_TO_OFFSET[layout_index]
}

fn white_offset(layout_index: usize) -> usize {
    WHITE_LAYOUT_TO_OFFSET[layout_index - 12]
}

async fn render_led_sets_for(
    red: &mut DirectCharlieBank<'_, 4>,
    white: &mut DirectCharlieBank<'_, 4>,
    button: &Input<'_>,
    red_offsets: &[usize],
    white_offsets: &[usize],
    duration: Duration,
) -> bool {
    let deadline = Instant::now() + duration;
    let scan_count = usize::max(red_offsets.len(), white_offsets.len());

    while Instant::now() < deadline {
        if pressed_after_debounce(button).await {
            return true;
        }

        if scan_count == 0 {
            Timer::after_millis(10).await;
            continue;
        }

        for scan_index in 0..scan_count {
            if let Some(offset) = red_offsets.get(scan_index % red_offsets.len().max(1)) {
                red.set_led_by_offset(*offset);
            } else {
                red.all_tri_state();
            }

            if let Some(offset) = white_offsets.get(scan_index % white_offsets.len().max(1)) {
                white.set_led_by_offset(*offset);
            } else {
                white.all_tri_state();
            }

            Timer::after_micros(LED_SCAN_HOLD_US).await;
            red.all_tri_state();
            white.all_tri_state();
        }
    }

    false
}

async fn render_dark_for(button: &Input<'_>, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;

    while Instant::now() < deadline {
        if pressed_after_debounce(button).await {
            return true;
        }

        Timer::after_millis(20).await;
    }

    false
}

async fn run_target_animation_once(
    red: &mut DirectCharlieBank<'_, 4>,
    white: &mut DirectCharlieBank<'_, 4>,
    button: &Input<'_>,
) -> bool {
    for step in 0..12 {
        let cross = [
            pink_offset(step),
            pink_offset((step + 3) % 12),
            pink_offset((step + 6) % 12),
            pink_offset((step + 9) % 12),
        ];

        if render_led_sets_for(red, white, button, &cross, &[], CROSS_STEP).await {
            return true;
        }
    }

    let white_leds = [
        white_offset(WHITE_TEST_LEDS[0]),
        white_offset(WHITE_TEST_LEDS[1]),
        white_offset(WHITE_TEST_LEDS[2]),
        white_offset(WHITE_TEST_LEDS[3]),
    ];

    if render_led_sets_for(red, white, button, &[], &white_leds, WHITE_HOLD).await {
        return true;
    }

    red.all_tri_state();
    white.all_tri_state();
    render_dark_for(button, DARK_HOLD).await
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("Hello World!");

    let mut cfg: py32_hal::Config = Default::default();
    cfg.rcc.hsi = Some(Hertz::mhz(24));
    cfg.rcc.sys = Sysclk::HSI;
    let p = py32_hal::init(cfg);

    // _spawner.spawn(bat::run_bat_monitor(p.ADC)).unwrap();

    let button = Input::new(p.PB0, Pull::None);

    let red_pins = [
        p.PA0.degrade(),
        p.PA1.degrade(),
        p.PA2.degrade(),
        p.PA3.degrade(),
    ];

    let white_pins = [
        p.PA4.degrade(),
        p.PA5.degrade(),
        p.PA6.degrade(),
        p.PA8.degrade(),
    ];
    
    let mut red = DirectCharlieBank::new(red_pins);
    let mut white = DirectCharlieBank::new(white_pins);

    red.all_analog();
    white.all_analog();
    info!("flash rescue window");
    flash_rescue_window(&button).await;

    info!("entering initial stop mode");
    sleep::enter_stop_mode();
    wait_for_button_release(&button).await;
    info!("awake");

    loop {
        let wake_deadline = Instant::now() + AWAKE_TIMEOUT;

        while Instant::now() < wake_deadline {
            if run_target_animation_once(&mut red, &mut white, &button).await {
                info!("button requested stop");
                wait_for_button_release(&button).await;
                break;
            }
        }

        info!("entering stop mode");
        red.all_analog();
        white.all_analog();
        sleep::enter_stop_mode();
        wait_for_button_release(&button).await;
        info!("awake");
    }
}

//conserve flash space with the non-handler
#[exception]
unsafe fn HardFault(_frame: &ExceptionFrame) -> ! {
    SCB::sys_reset()
}
