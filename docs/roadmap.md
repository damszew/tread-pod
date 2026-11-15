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
- [ ] Long press: config mode
- [ ] Very long press: factory reset

### Config mode
- [ ] Wi-Fi soft AP with a captive portal, serving the web UI from the device
- [ ] Choose the executor for picoserve (Tokio or Embassy)
- [ ] Pin a treadmill from the web UI

### Release
- [ ] CI: firmware build and merged image
- [ ] CI: web build and bundle size
- [ ] Browser flasher on GitHub Pages
- [ ] Make the GitHub repository public

### Features
- [ ] Send total distance in RSC
- [ ] Optional cadence estimate
- [ ] Real battery level (once a battery is fitted)

### Hardware and testing
- [ ] Battery and enclosure
- [ ] Field test in a real gym: is auto-pick reliable between adjacent treadmills?
- [ ] Try more treadmills and watches

## Maybe later

- [ ] Replace espup with a Nix flake for the Xtensa toolchain
- [ ] no_std port (esp-hal, trouble, embassy-net)
- [ ] Incline
