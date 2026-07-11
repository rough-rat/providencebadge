# Providence Badge Firmware

Rust firmware for the PY32F002A-based Providence Badge.

## Current Features

- Drives two 4-pin charlieplex LED banks:
  - pink hour ring on `PA0..PA3`
  - white half-step ring on `PA4`, `PA5`, `PA6`, `PA8`
- Implements Stop-mode sleep with PB0 wake:
  - PB0 is expected to have an external pullup and button-to-ground
  - firmware wakes for about 1 hour
  - pressing PB0 while awake returns to sleep
- Uses low-power Stop configuration:
  - low-power regulator enabled
  - 1.0 V Stop-mode voltage scaling
  - reduced SRAM retention voltage
- Provides a 5 second flash rescue window after reset.
  - Hold PB0 during reset to keep the firmware awake for flashing/debug.
- Includes a layout/SVG-based LED index workflow:
  - `../layout.json` is the source layout
  - `../led-layout.svg` marks LED positions, colors, and indexes
- Current animation test:
  - rotating 4-pink-LED cross for about 2 seconds
  - white LEDs with layout indexes `14`, `15`, `20`, `21` for 2 seconds
  - 6 seconds dark

## Build

```sh
cargo +nightly build --release
```

## Flash

```sh
./flash.sh
```

If the target is hard to attach to, wake it with PB0 or hold PB0 during reset. As a last resort:

```sh
CONNECT_UNDER_RESET=1 ./flash.sh
```

## Logs

```sh
probe-rs attach --chip PY32F030x8 target/thumbv6m-none-eabi/release/blobbadge-fw
```