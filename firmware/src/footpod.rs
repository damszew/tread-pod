//! The footpod the watch pairs with: an RSC sensor (`0x1814`), a Battery Service (`0x180F`),
//! which some Garmins need before they pair, and Device Information (`0x180A`) for the watch's
//! sensor details page and the activity's FIT `device_info` record.
//!
//! The identity never changes: the name is fixed and NimBLE advertises the chip's public address,
//! derived from the eFuse base MAC, so it is the same after every reboot and factory reset.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex as StdMutex},
};

use engine::session::Session;
use esp32_nimble::{
    BLEAdvertisementData, BLECharacteristic, BLEDevice, NimbleProperties, NimbleSub,
    utilities::{BleUuid, mutex::Mutex},
};

use crate::input::{Input, Sender, send};

pub const NAME: &str = "tread-pod";

const RSC_SERVICE: BleUuid = BleUuid::from_uuid16(0x1814);
const RSC_MEASUREMENT: BleUuid = BleUuid::from_uuid16(0x2a53);
const RSC_FEATURE: BleUuid = BleUuid::from_uuid16(0x2a54);
const BATTERY_SERVICE: BleUuid = BleUuid::from_uuid16(0x180f);
const BATTERY_LEVEL: BleUuid = BleUuid::from_uuid16(0x2a19);
const DEVICE_INFORMATION: BleUuid = BleUuid::from_uuid16(0x180a);
const MODEL_NUMBER: BleUuid = BleUuid::from_uuid16(0x2a24);
const SERIAL_NUMBER: BleUuid = BleUuid::from_uuid16(0x2a25);
const FIRMWARE_REVISION: BleUuid = BleUuid::from_uuid16(0x2a26);
const HARDWARE_REVISION: BleUuid = BleUuid::from_uuid16(0x2a27);
const SOFTWARE_REVISION: BleUuid = BleUuid::from_uuid16(0x2a28);
const MANUFACTURER_NAME: BleUuid = BleUuid::from_uuid16(0x2a29);
const PNP_ID: BleUuid = BleUuid::from_uuid16(0x2a50);

/// Shown by the watch under the sensor's "about" page.
const MANUFACTURER: &str = "tread-pod";
const MODEL: &str = "tread-pod FTMS bridge";
const HARDWARE: &str = "Seeed XIAO ESP32-S3";
/// Firmware Revision as a plain `major.minor` number. A Garmin parses this into the FIT
/// `software_version` and records nothing for text like `0.1.0 (318e493)`.
const FIRMWARE: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR")
);
/// Software Revision: the full crate version and commit, set by `build.rs`.
const SOFTWARE: &str = env!("TREAD_POD_FIRMWARE_REVISION");

/// PnP ID vendor: pid.codes, the USB vendor ID shared by open-source hardware (`0x02` = USB-IF
/// vendor ID source). The product ID is one of pid.codes' test IDs until we register our own.
const PNP_VENDOR_SOURCE_USB: u8 = 0x02;
const PNP_VENDOR: u16 = 0x1209;
const PNP_PRODUCT: u16 = 0x0001;

/// GAP appearance: Running Walking Sensor, In-Shoe.
const APPEARANCE: u16 = 0x0441;

pub struct Footpod {
    measurement: Arc<Mutex<BLECharacteristic>>,
}

impl Footpod {
    /// Registers the GATT services and starts advertising.
    pub fn start(device: &BLEDevice, tx: Sender) -> anyhow::Result<Self> {
        BLEDevice::set_device_name(NAME)?;
        let server = device.get_server();

        // Connections with notifications on. The session only needs "any watch listening", so
        // inputs are sent on the first subscribe and the last unsubscribe. Disconnects are tracked
        // here too: NimBLE's subscribe event for a closed link is dropped by esp32-nimble.
        let subscribed = Arc::new(StdMutex::new(HashSet::<u16>::new()));

        let rsc = server.create_service(RSC_SERVICE);
        let measurement = rsc
            .lock()
            .create_characteristic(RSC_MEASUREMENT, NimbleProperties::NOTIFY);
        {
            let subscribed = subscribed.clone();
            let tx = tx.clone();
            measurement.lock().on_subscribe(move |_, desc, sub| {
                let mut set = subscribed.lock().unwrap();
                let was_empty = set.is_empty();
                if sub.contains(NimbleSub::NOTIFY) {
                    set.insert(desc.conn_handle());
                } else {
                    set.remove(&desc.conn_handle());
                }
                log::info!("watch {} notifications: {sub:?}", desc.address());
                notify_change(&tx, was_empty, set.is_empty());
            });
        }
        rsc.lock()
            .create_characteristic(RSC_FEATURE, NimbleProperties::READ)
            .lock()
            .set_value(&Session::rsc_feature().encode());

        let battery = server.create_service(BATTERY_SERVICE);
        battery
            .lock()
            .create_characteristic(
                BATTERY_LEVEL,
                NimbleProperties::READ | NimbleProperties::NOTIFY,
            )
            .lock()
            // Mains powered.
            .set_value(&[100]);

        // Device Information. The serial number comes from the footpod's address, which is
        // unique to the chip and never changes.
        let serial = serial_number(device.get_addr()?.as_be_bytes()).to_string();
        let info = server.create_service(DEVICE_INFORMATION);
        for (uuid, value) in [
            (MANUFACTURER_NAME, MANUFACTURER.as_bytes()),
            (MODEL_NUMBER, MODEL.as_bytes()),
            (SERIAL_NUMBER, serial.as_bytes()),
            (HARDWARE_REVISION, HARDWARE.as_bytes()),
            (FIRMWARE_REVISION, FIRMWARE.as_bytes()),
            (SOFTWARE_REVISION, SOFTWARE.as_bytes()),
            (PNP_ID, &pnp_id()),
        ] {
            info.lock()
                .create_characteristic(uuid, NimbleProperties::READ)
                .lock()
                .set_value(value);
        }
        log::info!("device info: {MODEL}, firmware {FIRMWARE} ({SOFTWARE}), serial {serial}");

        // One central at a time: NimBLE stops advertising when a central connects, and
        // esp32-nimble restarts it only on disconnect (`advertise_on_disconnect`). Don't restart
        // it here: a second watch nearby could then take the footpod mid-run and record another
        // runner's treadmill. sdkconfig caps connections at 2 (treadmill + this one) as a backstop.
        server.on_connect(|_, desc| {
            log::info!(
                "watch connected: {}; not advertising until it leaves",
                desc.address()
            );
        });
        server.on_disconnect(move |desc, reason| {
            log::info!("watch disconnected: {} ({reason:?})", desc.address());
            let mut set = subscribed.lock().unwrap();
            let was_empty = set.is_empty();
            set.remove(&desc.conn_handle());
            notify_change(&tx, was_empty, set.is_empty());
        });

        let mut advertising = device.get_advertising().lock();
        advertising.set_data(
            BLEAdvertisementData::new()
                .name(NAME)
                .appearance(APPEARANCE)
                .add_service_uuid(RSC_SERVICE),
        )?;
        advertising.start()?;
        log::info!("footpod advertising as {NAME} ({:?})", device.get_addr()?);

        Ok(Self { measurement })
    }

    pub fn notify(&self, measurement: &[u8]) {
        self.measurement.lock().set_value(measurement).notify();
    }
}

/// The serial number as a decimal: a Garmin reads it as a number into the FIT `serial_number`
/// (a non-zero `u32`), stopping at the first non-digit, so hex text would be cut short. The low
/// four bytes of the address, which differ between chips.
fn serial_number(address_be: [u8; 6]) -> u32 {
    let [_, _, a, b, c, d] = address_be;
    u32::from_be_bytes([a, b, c, d]).max(1)
}

/// PnP ID: vendor ID source, vendor ID, product ID and product version (`0xJJMN` for `JJ.M.N`),
/// little-endian.
fn pnp_id() -> [u8; 7] {
    let part = |v: &str| v.parse::<u16>().unwrap_or(0);
    let version = (part(env!("CARGO_PKG_VERSION_MAJOR")) << 8)
        | (part(env!("CARGO_PKG_VERSION_MINOR")).min(0xf) << 4)
        | part(env!("CARGO_PKG_VERSION_PATCH")).min(0xf);
    let [v0, v1] = PNP_VENDOR.to_le_bytes();
    let [p0, p1] = PNP_PRODUCT.to_le_bytes();
    let [r0, r1] = version.to_le_bytes();
    [PNP_VENDOR_SOURCE_USB, v0, v1, p0, p1, r0, r1]
}

fn notify_change(tx: &Sender, was_empty: bool, is_empty: bool) {
    match (was_empty, is_empty) {
        (true, false) => send(tx, Input::WatchSubscribed),
        (false, true) => send(tx, Input::WatchUnsubscribed),
        _ => {}
    }
}
