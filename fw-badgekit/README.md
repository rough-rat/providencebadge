# Providence Badge Firmware (py32-badgekit)

Port of the firmware in `../fw` (kept there as the legacy target) to
[py32-badgekit](https://github.com/rough-rat/py32-badgekit).

- Two charlieplex banks, `PA0..PA3` (LED indexes 0-11, on the hours) and
  `PA4`, `PA5`, `PA6`, `PA8` (indexes 12-23, halfway between), used as one
  24-LED ring. Indexes match `../led-layout.svg`, which calls them pink and
  white; for now pink LEDs are fitted in both.
- Animation: see below.
- PB0 (external pull-up, button to ground) turns the badge on/off; it turns
  itself off after 1 hour.
- After reset: ~1 s recovery delay, then a 5 s window with LEDs off for
  flashing. A press in that window keeps it awake, otherwise it sleeps.

Per-LED brightness is only on badgekit's `charlieplex-pwm` branch so far, so
`Cargo.toml` points at the local checkout. Switch to `git` + `tag` once that is
released.

## Animation

The badge plays "attacks" with a pause of 0.5-2 s (pseudorandom) between them.

Each attack lasts 3 s:

- It starts as four bright LEDs in a cross, either on the hour LEDs or on the
  ones between them (pseudorandom).
- The four beams widen until they meet and the whole ring is lit. They widen
  with the square of the time, so the cross stays narrow for most of the
  attack and spreads quickly at the end (at 1.5 s, a quarter of the full
  width).
- The level falls linearly from 255 at the start to 60 at the end.
- After 3 s everything goes off at once (the hit).

There are two kinds of attack:

- **Spin:** the cross turns half a revolution (180°) at constant speed in the
  first 1.5 s, then stops and keeps widening in place.
- **Cross:** the same without turning.

Order: a spin, then 0-2 crosses (picked anew after each spin), then the next
spin, and so on.

The pseudorandom numbers come from xorshift32, seeded from the time of the
wake-up press, so every wake-up gives a different sequence.

The charlieplex scan skips dark slots (`charlieplex::set_skip_dark(true)`):
the fewer LEDs are lit, the bigger share of the time each one gets. So the
narrow cross is brighter than its level alone would give (up to 3x), and the
brightness drops further as the beams widen.

Timing and levels are constants at the top of the "Behaviour" section in
`src/main.rs`.

## Build / flash

```sh
cargo +nightly build --release
./flash.sh
```

## Chip quirk: hidden RAM and flash

The PY32F002AW15U6TR is sold as 20 KB flash / 3 KB RAM, but it is the same
die as the PY32F030x6: it has **32 KB flash and 4 KB RAM**. Checked on one
badge on 2026-11-25 with probe-rs (target `PY32F030x6`, badge awake):

- RAM `0x20000C00..=0x20000FFF`: writes read back correctly; it is separate
  memory, not a mirror of the lower 3 KB. `0x20001000` gives a bus error.
- Flash `0x08005000..=0x08007FFF`: blank, and a test pattern programmed at
  `0x08007C00` (verified) read back correctly. `0x08008000` gives a bus error.
  The test pattern may still be in `0x08007C00..=0x08007CFF`; nothing uses that
  area.

The firmware does not use the extra memory (it needs ~11.6 KB flash, ~2.2 KB
RAM), so `memory.x` stays at the official 20K/3K. To use it: set `memory.x`
to `FLASH 32K`, `RAM 4K`, and the chip in `flash.sh` and `.cargo/config.toml`
to `PY32F030x6` (probe-rs's `PY32F002Ax5` target refuses addresses above
20 KB). This is undocumented and was checked on one chip only; other batches
may differ.
