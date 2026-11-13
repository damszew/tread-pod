//! tread-pod bridge firmware.
//!
//! BLE callbacks and the central thread send [`Input`]s over a channel; the main loop hands them
//! to the [`Session`] with a timestamp, ticks it every 100 ms, and carries out the actions it
//! returns. All decisions live in `engine`; this crate only moves bytes. The button, LED and
//! config mode come next; see docs/architecture.md.

mod central;
mod footpod;
mod input;

use std::{
    sync::mpsc::{self, RecvTimeoutError},
    time::{Duration, Instant},
};

use engine::{
    config::Config,
    session::{Action, Event, Session},
};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp32_nimble::BLEDevice;

use crate::{central::Central, footpod::Footpod, input::Input};

const NVS_NAMESPACE: &str = "tread-pod";
const NVS_CONFIG_KEY: &str = "config";

/// The session needs a tick at least every 250 ms.
const TICK: Duration = Duration::from_millis(100);

fn main() -> anyhow::Result<()> {
    // Required once at startup so patches to the runtime implemented by esp-idf-sys get linked.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let mut nvs = EspNvs::new(EspDefaultNvsPartition::take()?, NVS_NAMESPACE, true)?;
    let config = load_config(&nvs);
    log::info!("tread-pod starting with {config:?}");

    let (tx, rx) = mpsc::channel::<Input>();
    let device = BLEDevice::take();
    let footpod = Footpod::start(device, tx.clone())?;
    let central = Central::spawn(tx)?;

    let mut session = Session::new(config);
    let start = Instant::now();
    let mut last_tick = start;
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
            actions.extend(session.handle(now_ms, input.as_event()));
        }
        // A busy channel must not starve the tick.
        if now - last_tick >= TICK {
            last_tick = now;
            actions.extend(session.handle(now_ms, Event::Tick));
        }
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
