# Roadmap

What's done and what's next. Tick items off as they land, and add new ones where they fit.
Design details live in [architecture.md](architecture.md); user-facing behaviour in
[user-guide.md](user-guide.md).

## Done

### Foundations
- [x] devenv environment, scripts and pre-commit hooks
- [x] Cargo workspace with the `engine`, `api`, `web` and `firmware` crates
- [x] `ftms-sniff` tool and a first treadmill capture (FS-4C02A8)

### Engine (tested on the host)
- [x] FTMS Treadmill Data parser, including split records
- [x] RSC Measurement and Feature encoder
- [x] Speed from total distance, for treadmills that don't report speed
- [x] Session state machine: watch-gated, sticky target, auto-pick by RSSI, timeouts and retries

### Firmware
- [x] Treadmill → watch bridge over BLE, verified end to end on a Garmin (pace and distance correct)
- [x] Status LED for the treadmill link
- [x] Device Information (maker, model, firmware version, serial) shown on the watch
- [x] Logs: treadmills seen, scan summaries, session state

### Config UI (not yet on the device)
- [x] Config API types and router, with a dev server
- [x] Leptos web UI built with Trunk

### CI
- [x] `devenv test` (all git hooks, then `check`) on every push and PR

### Project
- [x] Architecture overview, user guide, README
- [x] MIT license
- [x] Hosted on GitHub, with a description and topics

## Next

### Button
- [ ] Short press: switch to the next treadmill
- [ ] Long press: allow config changes over Bluetooth for a few minutes
- [ ] Very long press: factory reset

### Config over Bluetooth
Replaces the Wi-Fi soft AP and captive portal. It needs no Wi-Fi, and the same GATT service carries
over if the firmware moves to the nRF52840. Web Bluetooth works in Chrome and Edge on Windows,
macOS, ChromeOS and Android (Linux behind a flag), but not in Safari, Firefox or anything on iOS.
- [ ] Check the footpod is still discoverable while a watch is connected (advertising restarts)
- [ ] Config GATT service (custom 128-bit UUID): treadmills seen, select, settings
- [ ] Replace the HTTP router in `api` with characteristic handling; keep the shared types
- [ ] Switch `web` from HTTP (`gloo-net`) to Web Bluetooth and host it on GitHub Pages
- [ ] Pin a treadmill from the web UI
- [ ] Only accept writes after a button long press, so nobody nearby can change the settings

### Cadence
The first test run (2026-11-18) showed that a Garmin records the footpod's cadence of 0 as 0 spm
instead of falling back to wrist cadence, which also zeroes running power and running dynamics.
Cadence is a mandatory RSC field, so it cannot be left out.

1. Estimate from speed, kept as simple as possible: one fixed curve for everyone, no settings.
   It only has to beat 0 spm until the accelerometer works.
   - [ ] Fixed speed-to-cadence curve, with a separate walking range below about 6–7 km/h
   - [ ] Send stride length (speed ÷ cadence)
   - [ ] Fix the docs that say the watch falls back to wrist cadence
2. Measure with an accelerometer (the main focus once the estimate lands)
   - [ ] Pick a sensor: Grove LSM6DS3 and the Grove Shield for XIAO on the current board, or the
     onboard IMU if the board changes (see Platform)
   - [ ] Record vibration logs on the treadmill and add them as test fixtures
   - [ ] Footstrike detection in `engine`: band-pass, period over a few seconds, the speed estimate
     to reject double or half cadence, falling back to the estimate when the signal is weak
   - [ ] Mount the sensor rigidly in the enclosure, fixed to the treadmill frame

### Release
- [ ] CI: firmware build and merged image
- [ ] CI: web build and bundle size
- [ ] Browser flasher on GitHub Pages
- [ ] Make the GitHub repository public

### Features
- [ ] Send total distance in RSC
- [ ] Real battery level (once a battery is fitted)

### Hardware and testing
- [ ] Battery and enclosure
- [ ] Field test in a real gym: is auto-pick reliable between adjacent treadmills?
- [ ] Try more treadmills and watches

## Maybe later

- [ ] Per-watch cadence profiles (height, or a measured cadence at a known pace), keyed by the
  watch's address, for people who use the speed estimate without an accelerometer
- [ ] Switch to the XIAO nRF52840 Sense Plus (optional): onboard IMU, far longer battery life,
  drag-and-drop UF2 flashing. Only the `firmware` crate changes (Embassy with `nrf-softdevice` or
  `trouble`), since `engine` is already `no_std`. First check the onboard antenna's range, and get
  a debug probe for development.
- [ ] Replace espup with a Nix flake for the Xtensa toolchain (moot if the board changes)
- [ ] no_std port (esp-hal, trouble, embassy-net) (moot if the board changes)
- [ ] Incline
- [ ] Walking/running status bit in RSC. Not needed for now: the watch already tells walking from
  running correctly from speed alone (first test run, 2026-11-18).
