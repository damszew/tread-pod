# tread-pod

[![CI](https://github.com/damszew/tread-pod/actions/workflows/ci.yml/badge.svg)](https://github.com/damszew/tread-pod/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Make any Bluetooth treadmill look like a running foot pod to your watch, so indoor runs record the
treadmill's real pace and distance instead of a wrist-based guess.

```
treadmill  ──FTMS──▶  tread-pod (ESP32-S3)  ──RSC foot pod──▶  watch
```

tread-pod is firmware for a small ESP32-S3 board. It connects to the treadmill as a Bluetooth
**FTMS** client and advertises itself as a standard **RSC** (Running Speed and Cadence) foot pod.
You pair your watch with it once. After that it works with any FTMS treadmill nearby, with no app
and no pairing per treadmill.

## Status

Early, but working end to end. Pace and distance are correct on a Garmin watch with an FS-4C02A8
treadmill. Planned next: a button for switching treadmills and a Wi-Fi settings page. See the
[roadmap](docs/roadmap.md).

## Features

- Picks the nearest treadmill automatically and stays with it for the whole run, reconnecting if
  the link drops
- Works with treadmills that report only distance, by deriving speed from it
- Only uses the treadmill while your watch is listening, so the treadmill's own app still works
  after your run
- A status LED shows the treadmill connection
- Shows its maker, model, firmware version and serial number on the watch

## Hardware

- [Seeed Studio XIAO ESP32-S3](https://wiki.seeedstudio.com/xiao_esp32s3_getting_started/) with
  its external antenna attached (it has no onboard antenna)
- A treadmill that supports Bluetooth FTMS (most that work with Zwift or Kinomap)
- A watch that supports Bluetooth foot pods (tested with Garmin)

## Using it

The [user guide](docs/user-guide.md) covers setup, what the light means, everyday use and
troubleshooting, written for non-technical users.

## Building

All tooling comes from [devenv](https://devenv.sh), so you only need Nix and devenv installed.

```bash
devenv shell -- check       # fmt, clippy, tests, no_std build of the engine
devenv shell -- esp-setup   # once per toolchain version: install the Xtensa Rust toolchain
devenv shell -- fw-flash    # build, flash the board over USB, open the serial monitor
```

`devenv test` runs every git hook and `check`, as CI does. [architecture.md](docs/architecture.md)
describes the crates, the session state machine and the firmware design.

## Layout

| Path | What it is |
|---|---|
| `crates/engine` | Sans-IO core: FTMS parsing, RSC encoding, treadmill selection, config. `no_std`, tested on the host |
| `crates/api` | Config API types and the HTTP router |
| `crates/web` | Config web UI (Leptos), not yet served by the device |
| `firmware` | ESP-IDF firmware for the XIAO ESP32-S3 |
| `tools/ftms-sniff` | Records a treadmill's FTMS traffic as test fixtures |
| `tools/fit-inspect` | Prints the device records of a Garmin FIT activity |

## Clean room

Written from the Bluetooth SIG specifications (FTMS, RSC, BAS, DIS). No code from other treadmill
bridge projects is read or copied.

## License

[MIT](LICENSE)
