//! tread-pod bridge firmware.
//!
//! BLE callbacks and the central thread send [`Input`]s over a channel; the main loop hands them
//! to the [`Session`] with a timestamp, ticks it every 100 ms, and carries out the actions it
//! returns. All decisions live in `engine`; this crate only moves bytes. The button and config
//! mode come next; see docs/architecture.md.

mod central;
mod footpod;
mod input;
mod led;

use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};

use engine::{
    config::Config,
    session::{Action, Event, Link, Session},
    watch::{Admission, Gate, Leave},
};
use esp_idf_svc::{
    hal::peripherals::Peripherals,
    nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault},
};
use esp32_nimble::{BLEAddress, BLEDevice};

use crate::{central::Central, footpod::Footpod, input::Input, led::Led};

const NVS_NAMESPACE: &str = "tread-pod";
const NVS_CONFIG_KEY: &str = "config";

/// The session needs a tick at least every 250 ms.
const TICK: Duration = Duration::from_millis(100);

/// How often to retry starting advertising after it failed.
const ADVERTISE_RETRY_MS: u64 = 1_000;

fn main() -> anyhow::Result<()> {
    // Required once at startup so patches to the runtime implemented by esp-idf-sys get linked.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut nvs = EspNvs::new(EspDefaultNvsPartition::take()?, NVS_NAMESPACE, true)?;
    let config = load_config(&nvs);
    log::info!("tread-pod starting with {config:?}");

    let peripherals = Peripherals::take()?;
    // XIAO ESP32-S3 user LED.
    let mut led = Led::new(peripherals.pins.gpio21)?;

    let (tx, rx) = mpsc::channel::<Input>();
    let device = BLEDevice::take();
    let footpod = Footpod::start(device, tx.clone())?;
    let central = Central::spawn(tx)?;

    let mut session = Session::new(config);
    let start = Instant::now();
    let mut last_tick = start;
    let mut link = session.link();
    let mut advertising = Advertising::default();
    loop {
        let input = match rx.recv_timeout(TICK) {
            Ok(input) => Some(input),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => anyhow::bail!("all BLE senders dropped"),
        };
        let now = Instant::now();
        let now_ms = u64::try_from((now - start).as_millis()).unwrap_or(u64::MAX);
        let mut actions = Vec::new();
        if let Some(input) = &input {
            let forward = advertising.handle(now_ms, input);
            if forward && let Some(event) = input.as_event() {
                actions.extend(session.handle(now_ms, event));
            }
        }
        advertising.poll(now_ms);
        // A busy channel must not starve the tick.
        if now - last_tick >= TICK {
            last_tick = now;
            actions.extend(session.handle(now_ms, Event::Tick));
        }
        log_link_change(&mut link, session.link());
        led.show(session.link(), now_ms);
        for action in actions {
            match action {
                Action::Connect(address) => {
                    log::info!("connect {}", central::fmt(address));
                    central.connect(address);
                }
                Action::Disconnect(address) => {
                    log::info!("disconnect {}", central::fmt(address));
                    central.disconnect(address);
                }
                Action::NotifyRsc(measurement) => {
                    log::debug!("rsc {:02x?}", &*measurement);
                    footpod.notify(&measurement);
                }
                Action::Persist(config) => save_config(&mut nvs, &config),
            }
        }
    }
}

/// Who may connect to the footpod, and keeping the advertising in line with it. NimBLE stops
/// advertising when a central connects; only this restarts it, once that central is gone.
#[derive(Default)]
struct Advertising {
    gate: Gate,
    /// The central on the footpod, if any.
    central: Option<BLEAddress>,
    /// The central was refused and is being dropped: it must not become the owner or reach the
    /// session, even if it subscribes before it's gone.
    rejected: bool,
    /// The owner's address with its type, as the allowlist needs it.
    owner: Option<BLEAddress>,
    /// What advertising currently allows, if it is running.
    applied: Option<Admission>,
    /// Don't retry a failed start before this.
    retry_at: u64,
}

impl Advertising {
    /// Returns whether the input should also go to the session.
    fn handle(&mut self, now_ms: u64, input: &Input) -> bool {
        match input {
            Input::WatchConnected { conn, address } => {
                // NimBLE stopped advertising.
                self.applied = None;
                self.central = Some(*address);
                self.rejected = !self.gate.connected(now_ms, footpod::address(*address));
                if self.rejected {
                    log::warn!("dropping {address}: another watch's grace period is running");
                    Footpod::drop_central(*conn);
                }
            }
            Input::WatchSubscribed(_) | Input::WatchUnsubscribed if self.rejected => return false,
            Input::WatchSubscribed(address) => {
                self.gate.subscribed(*address);
                self.owner = self.central.filter(|c| footpod::address(*c) == *address);
            }
            Input::WatchDisconnected { address, reason } => {
                // A central was on, so NimBLE stopped advertising, even if its connect was never
                // reported.
                self.applied = None;
                self.central = None;
                self.rejected = false;
                let leave = Leave::from_hci(*reason);
                self.gate.disconnected(now_ms, *address, leave);
                if let Admission::Only(_) = self.gate.admission(now_ms) {
                    log::info!(
                        "watch link lost (reason 0x{reason:02x}): only it may reconnect for {} s",
                        engine::watch::GRACE_MS / 1000
                    );
                }
            }
            _ => {}
        }
        true
    }

    /// Restarts advertising when nobody is connected and it isn't running with the right
    /// allowlist, including when a grace period has just run out.
    ///
    /// Never decide from "is it advertising?": NimBLE stops advertising the moment a central
    /// connects, before `WatchConnected` reaches this loop, and restarting it then would let a
    /// second watch in. So only our own state and NimBLE's connection table count.
    fn poll(&mut self, now_ms: u64) {
        if self.central.is_some() || now_ms < self.retry_at {
            return;
        }
        let wanted = self.gate.admission(now_ms);
        if self.applied == Some(wanted) || Footpod::has_central() {
            return;
        }
        let only = match wanted {
            Admission::Anyone => None,
            Admission::Only(_) => self.owner,
        };
        match Footpod::advertise(only) {
            Ok(()) => {
                match only {
                    Some(watch) => log::info!("advertising for {watch} only"),
                    None => log::info!("advertising for any watch"),
                }
                self.applied = Some(wanted);
            }
            Err(err) => {
                log::warn!("can't start advertising: {err:?}");
                self.applied = None;
                self.retry_at = now_ms + ADVERTISE_RETRY_MS;
            }
        }
    }
}

/// Logs session link changes, so the log shows where a run is stuck: scanning without a
/// treadmill, connecting, or connected without data.
fn log_link_change(last: &mut Link, now: Link) {
    let state = |l: &Link| match l {
        Link::Idle => "idle (no watch)".to_string(),
        Link::Scanning { .. } => "looking for a treadmill".to_string(),
        Link::Connecting { address, .. } => format!("connecting to {}", central::fmt(*address)),
        Link::Connected {
            address,
            last_data: None,
            ..
        } => format!("connected to {}, waiting for data", central::fmt(*address)),
        Link::Connected { address, .. } => {
            format!("forwarding from {}", central::fmt(*address))
        }
    };
    let (before, after) = (state(last), state(&now));
    if before != after {
        log::info!("session: {after}");
    }
    *last = now;
}

/// Loads the config, falling back to defaults if it's missing, corrupt or from another version.
fn load_config(nvs: &EspNvs<NvsDefault>) -> Config {
    let mut buf = [0; Config::MAX_ENCODED_LEN];
    match nvs.get_blob(NVS_CONFIG_KEY, &mut buf) {
        Ok(Some(bytes)) => Config::decode_or_default(bytes),
        Ok(None) => Config::default(),
        Err(err) => {
            log::warn!("reading config from NVS failed, using defaults: {err}");
            Config::default()
        }
    }
}

fn save_config(nvs: &mut EspNvs<NvsDefault>, config: &Config) {
    let mut buf = [0; Config::MAX_ENCODED_LEN];
    let result = config
        .encode(&mut buf)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| Ok(nvs.set_blob(NVS_CONFIG_KEY, bytes)?));
    match result {
        Ok(()) => log::info!("config saved: {config:?}"),
        Err(err) => log::error!("saving config failed: {err}"),
    }
}
