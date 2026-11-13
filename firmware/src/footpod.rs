//! The footpod the watch pairs with: an RSC sensor (`0x1814`) plus a Battery Service (`0x180F`),
//! which some Garmins need before they pair.
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

        server.on_connect(|_, desc| log::info!("watch connected: {}", desc.address()));
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

fn notify_change(tx: &Sender, was_empty: bool, is_empty: bool) {
    match (was_empty, is_empty) {
        (true, false) => send(tx, Input::WatchSubscribed),
        (false, true) => send(tx, Input::WatchUnsubscribed),
        _ => {}
    }
}
