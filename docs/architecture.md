# tread-pod architecture

tread-pod is a BLE bridge that lets a Garmin watch use an FTMS treadmill as its speed source.
It connects to the treadmill as a GATT client and appears to the watch as a standard Running
Speed & Cadence (RSC) footpod. The watch pairs with it **once**. The treadmill behind it can change.

The only runtime target is a **Seeed XIAO ESP32-S3**. Host-side code exists for tests, UI development
and early spikes.

**One language:** everything is Rust, including the web UI (compiled to WASM). The only non-Rust
files are Nix, TOML, CI YAML and a static HTML flashing page.

## Behaviour

- **Upstream (GATT client):** FTMS treadmill: service `0x1826`, Treadmill Data `0x2ACD`.
- **Downstream (GATT server / peripheral):** RSC footpod: service `0x1814`, RSC Measurement `0x2A53`,
  RSC Feature `0x2A54`, plus Battery Service `0x180F`.
- **Runner-owned identity:** the footpod's address and name stay stable forever, including across
  reboots. Only the treadmill behind it changes.
- **Treadmill selection:** auto-pick the nearest by RSSI, a button cycles to the next candidate,
  and the config can pin one treadmill.
- **Connect only when needed:** the bridge connects to the treadmill only while a watch is
  subscribed to RSC notifications. Treadmills often allow a single BLE connection.

## Repository layout

```
tread-pod/
├── devenv.nix / devenv.yaml / devenv.lock
├── Cargo.toml              # workspace: engine, api, web, tools/*
├── crates/
│   ├── engine/             # no_std sans-IO logic + captured-packet tests
│   ├── api/                # API types; picoserve router behind the `server` feature
│   └── web/                # Leptos CSR config UI (wasm32), built with Trunk
├── firmware/               # ESP32-S3 bridge; NOT a workspace member
├── tools/                  # spike tools, can be deleted later
│   ├── ftms-sniff/
│   ├── rsc-fixed/
│   └── rssi-survey/
├── flasher/                # static ESP Web Tools page (index.html + manifest template)
├── docs/
└── .github/workflows/
```

Naming: crates are named by role, with no shared prefix. The logic crate is `engine`, not `core`,
because `core` clashes with Rust's built-in `core` library.

`firmware` is outside the workspace because it needs the Xtensa `esp` toolchain, `build-std` and
the ESP-IDF build environment. It has its own `rust-toolchain.toml`, `.cargo/config.toml` and
`Cargo.lock`, and depends on `engine` and `api` by path. A plain `cargo test` at the root works
on any machine.

## `engine`: sans-IO core

`no_std`, no alloc, no async, no BLE, no clock. Backends feed it timestamped events and carry out
the actions it returns. This keeps it executor-agnostic: it works the same under Tokio on ESP-IDF
or Embassy, and a later esp-hal/trouble port only replaces the adapter code.

```rust
session.handle(now_ms: u64, event: Event) -> Actions   // Actions: heapless::Vec<Action, 4>
```

- **Events:** `AdvSeen { address, rssi, name }`, `WatchSubscribed`, `WatchUnsubscribed`,
  `Connected(addr)`, `Disconnected(addr)`, `TreadmillData(&[u8])`, `Button`, `Select(addr)`,
  `SetConfig(Config)`, `Tick` (at least every 250 ms).
- **Actions:** `Connect(addr)`, `Disconnect(addr)`, `NotifyRsc(Encoded)`, `Persist(Config)`.
- **State for the LED and API** is read from the session (`link()`, `target()`, `candidates()`,
  `speed(now)`) rather than pushed as actions.
- The backend **scans continuously** (low duty cycle) and reports every FTMS advertisement, so
  the candidate list stays fresh for the button and the web UI.

Session rules:
- Nothing connects until the watch subscribes; the treadmill is released when it unsubscribes.
- The *target* sticks for the whole workout: a button or API choice, else the pinned treadmill,
  else the strongest RSSI after a 2 s scan window. A lost link waits for the same treadmill
  instead of jumping to a neighbour; the button cycles through candidates in discovery order.
- Timeouts: connect 10 s, no data 5 s, retry after 1 s.
- RSC is notified for every treadmill record, and as a heartbeat after 2 s without one; a speed
  older than 3 s is sent as 0. Nothing is sent in the first 1 s after the watch subscribes.

Modules:

| module | responsibility |
|---|---|
| `ftms::treadmill_data` | Flag-driven parser covering every optional field |
| `ftms::reassembly` | Merges notifications split with the "More Data" flag |
| `rsc::measurement` | RSC Measurement (`0x2A53`) encoder |
| `rsc::feature` | RSC Feature (`0x2A54`) bits (cadence has none: it is always sent, 0 when off) |
| `speed` | 0.01 km/h → 1/256 m/s, rounded **up**; speed derived from total distance (10 s window) |
| `selection` | Candidate list (discovery order, smoothed RSSI, 10 s expiry), strongest / next pick |
| `session` | Top-level machine: link state (Idle → Scanning → Connecting → Connected), target, watch gating, notify delay/heartbeat, button, API |
| `config` | `Config { version, pinned, cadence, … }`, serde + postcard |

Tests:
- Parser tests against real packets captured with `tools/ftms-sniff` (`tests/fixtures/*.hex`).
- Property tests: every FTMS flag combination round-trips through a test-only encoder.
- Scripted end-to-end session tests: watch subscribes, treadmill found, split packets, link lost,
  cycle to the next treadmill. Debugging on the device is slow, so as much as possible is tested here.
- CI also builds `engine` for `thumbv7em-none-eabihf` to prove it really is `no_std`.

## `api`: config HTTP API

The crate has two layers:

- **API types** (always built, `no_std` + serde): request/response and status types, separate from
  the stored `Config` format. Both the firmware and the `web` UI use these same types, so the
  client and server can't drift apart.
- **Router** (`server` feature): a single picoserve router, generic over an `AppHandle` trait that
  the firmware implements. `web` depends on `api` without this feature, so picoserve never ends up
  in the WASM build.

Routes live under `/api` so that a dev proxy and static file serving can tell them apart:

- `GET /api/treadmills`: candidates with name, RSSI and live speed
- `POST /api/select`
- `GET|PUT /api/settings`
- `WS /api/status`

`examples/dev_server.rs` serves the real router on the host with a fake `AppHandle` that reports
made-up treadmills, RSSI and speeds. `trunk serve` proxies `/api` to it, so all UI work happens on
the PC without flashing the board.

## `firmware`: ESP32-S3

- **Stack (std path):** ESP-IDF v5.5.6 + `esp-idf-svc` + `esp32-nimble` (NimBLE, dual-role).
  No async runtime: plain threads and an `mpsc` channel are enough for the bridge.
- **Main loop** (`main.rs`): BLE callbacks and the central thread send owned `Input`s over a
  channel; the loop timestamps each one, passes it to `session.handle`, ticks the session every
  100 ms, and carries out the returned actions.
- **Footpod** (`footpod.rs`): RSC service (Measurement notify, Feature read), Battery Service
  (level 100) and Device Information (manufacturer, model, hardware, serial = the footpod address,
  firmware = crate version + commit embedded by `build.rs`). Advertises name `tread-pod`, appearance Running Walking Sensor In-Shoe (`0x0441`),
  and the chip's public address, which comes from the eFuse base MAC and so never changes.
  Watch subscription is tracked per connection: the first subscriber sends `WatchSubscribed`, the
  last unsubscribe or disconnect sends `WatchUnsubscribed`.
- **Central** (`central.rs`): its own thread, since esp32-nimble scan and connect calls block.
  Scans in 500 ms windows at a 30 % duty cycle (NimBLE can't start a connection while scanning),
  reports FTMS advertisements and their scan responses, and between windows reconciles the one
  connection the session wants: connect, discover FTMS, subscribe to Treadmill Data. A connect
  the session gives up on is cancelled from the main thread (`ble_gap_conn_cancel`).
- **Config:** postcard blob in NVS.
- **Button:** short press cycles treadmills, long press enters config mode, very long press does
  a factory reset.
- **LED** (`led.rs`, XIAO user LED on GPIO21, active low): the treadmill link only, since the
  watch shows its own connection. Off: no watch listening; slow blink: looking for a treadmill;
  fast blink: connecting or waiting for first data; on: forwarding.
- **Config mode only:** Wi-Fi soft AP + captive DNS, serving the `api` router and the web UI.
  Wi-Fi is off during runs.
- **Web assets:** `build.rs` gzips `crates/web/dist` (the Trunk output: `index.html`, JS glue and
  `.wasm`) and embeds it with `include_bytes!`. If `dist/` is missing, it embeds a placeholder page.
- **Later:** a possible no_std port (esp-hal + esp-radio + trouble + embassy-net).

## `web`: config UI

A Leptos client-side-rendered app compiled to `wasm32-unknown-unknown` and built with Trunk into
`crates/web/dist`. The firmware embeds the result gzipped. The treadmill picker shows name, RSSI and
live speed (via the `/api/status` WebSocket). It uses the `api` crate's types directly.

**Size budget:** the release build uses `opt-level = "z"`, LTO, `panic = "abort"` and `wasm-opt`;
the gzipped bundle should stay in the low hundreds of KB. The XIAO ESP32-S3 has 8 MB of flash,
so this fits, but CI should report the size so growth is visible.

Why Leptos (decided 2026-11-12). Only client-side rendering is used, because the server is picoserve
on the ESP32, so no framework's full-stack features apply. With that, Leptos wins on bundle size
(fine-grained signals, no virtual DOM) and tooling (plain Trunk).

Alternatives considered:
- **Dioxus:** larger bundle (virtual DOM), and its `dx` CLI has to match the library version.
  Its main strength, one codebase for web, desktop and mobile, doesn't apply here.
- **Yew:** virtual DOM, and development has slowed.
- **Sycamore:** similar model to Leptos, but a smaller ecosystem.
- **Server-rendered maud + htmx:** smallest option with no WASM toolchain, but needs one vendored JS
  file and offers less interactivity.

## Development environment: devenv.sh

All tooling comes from devenv.sh. **No direnv:** enter the environment with `devenv shell`. CI runs
the same scripts through `devenv shell -- <script>`, so local and CI builds use the same tools.

`devenv.nix` provides:
- Rust via `languages.rust` (fenix): stable, clippy, rustfmt, `thumbv7em-none-eabihf` and
  `wasm32-unknown-unknown` targets
- `trunk`, `wasm-bindgen-cli` (pinned to the `wasm-bindgen` version in `Cargo.lock`), `binaryen` (`wasm-opt`)
- `espflash`, `cargo-espflash`, `ldproxy`, Python for the ESP-IDF build scripts
- `pkg-config`, `dbus`, `bluez` for the host spike tools
- git hooks: rustfmt, clippy, nixfmt

No Node.js.

Scripts:

| script | does |
|---|---|
| `check` | fmt check, clippy, `cargo test`, `engine` built for thumbv7em |
| `web-build` | `trunk build --release` into `crates/web/dist` |
| `web-dev` | `api` dev server + `trunk serve` with `/api` proxied (also available via `devenv up`) |
| `sniff` | runs `tools/ftms-sniff`, writes captures to `crates/engine/tests/fixtures/` |
| `esp-setup` | `espup install` into `.devenv/state/esp` (separate `RUSTUP_HOME`/`CARGO_HOME`) |
| `fw-build` / `fw-flash` | build / flash `firmware` |
| `fw-image` | merged image via `espflash save-image --merge` |

The Xtensa Rust toolchain is not in nixpkgs, so `esp-setup` uses espup and keeps the result inside
the project. esp-idf-sys downloads ESP-IDF on its own into `firmware/.embuild`. Only the `fw-*`
scripts use that toolchain; `check` stays fully reproducible by Nix. Replacing espup with a Nix
flake is an open question (see below).

## CI and flashing

- `ci.yml`: `check` and `web-build`, and reports the gzipped web bundle size.
- `firmware.yml`: `web-build`, `esp-setup`, `fw-build`, `fw-image`; uploads the merged image.
- `pages.yml`: publishes `flasher/` (a static HTML page loading ESP Web Tools), `manifest.json`
  and the merged image to GitHub Pages, for flashing from the browser. ESP Web Tools is used as a
  ready-made component; we don't write any JavaScript for it.

## Known quirks

| quirk | handled in |
|---|---|
| Garmin may drop data if RSC notifications start before the connect event is processed | `engine::session` (notify delay checked against `now_ms`) |
| Some Garmins need a Battery Service to pair | `firmware` GATT setup |
| Some treadmills don't report instantaneous speed | `engine::speed` (derive from total distance) |
| Pace must never read slower than the treadmill | `engine::speed` (round up to the next 1/256 m/s) |
| Estimated cadence can skew running dynamics | `engine::config` (cadence optional, off by default) |
| Footpod address must stay the same across reboots | `firmware` (fixed address from the eFuse base MAC; survives factory reset) |

## Clean-room policy

Everything is written from the Bluetooth SIG specifications (FTMS, RSC, BAS).
- QZ (GPL-3.0) and RunBridge (closed source): no code is read or copied.
- TreadLink (MIT): used only as a behavioral reference.

## Roadmap

0. **Spikes:** capture the treadmill's FTMS data (`ftms-sniff`). Run a minimal RSC peripheral
   sending a fixed speed to the watch (`rsc-fixed`). Measure RSSI separation between adjacent
   treadmills (`rssi-survey`). Decide between Tokio and Embassy on ESP-IDF.
1. **`engine` + `firmware` working end to end.**
2. **Button and LED:** cycle / config mode / factory reset, LED states.
3. **Web config UI + browser flashing.**
4. **Battery, enclosure, field testing in a real gym.**

## Open questions

- Which executor serves picoserve in config mode (Tokio `current_thread` or Embassy on ESP-IDF); the
  BLE side needs none.
- Whether a community Nix flake for the Xtensa Rust toolchain is good enough to replace espup.
- RSSI separation between adjacent gym treadmills: is auto-pick reliable, or should pinning be the default?
- Actual gzipped size of the Leptos bundle, and how long it takes to load over the soft AP.
