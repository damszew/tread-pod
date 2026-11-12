//! tread-pod bridge firmware.
//!
//! For now this only boots and loads the persisted config. BLE (FTMS client + RSC footpod),
//! the button, the LED and config mode come next; see docs/architecture.md.

use engine::config::Config;
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

const NVS_NAMESPACE: &str = "tread-pod";
const NVS_CONFIG_KEY: &str = "config";

fn main() -> anyhow::Result<()> {
    // Required once at startup so patches to the runtime implemented by esp-idf-sys get linked.
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let nvs = EspNvs::new(EspDefaultNvsPartition::take()?, NVS_NAMESPACE, true)?;
    let config = load_config(&nvs);
    log::info!("tread-pod starting with {config:?}");

    Ok(())
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
