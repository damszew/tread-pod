//! The footpod the watch pairs with: an RSC sensor (`0x1814`), a Battery Service (`0x180F`),
//! which some Garmins need before they pair, and Device Information (`0x180A`) for the watch's
//! sensor details page.
//!
//! The identity never changes: the name is fixed and NimBLE advertises the chip's public address,
//! derived from the eFuse base MAC, so it is the same after every reboot and factory reset.

use std::{
    collections::HashSet,
    ffi::c_void,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicU8, Ordering},
    },
};

use engine::config::Address;
use engine::session::Session;
use esp32_nimble::{
    BLEAddress, BLEAdvertisementData, BLECharacteristic, BLEDevice, BLEError, NimbleProperties,
    NimbleSub,
    enums::AdvFilterPolicy,
    utilities::{BleUuid, mutex::Mutex},
};

use esp_idf_svc::sys;

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
const MANUFACTURER_NAME: BleUuid = BleUuid::from_uuid16(0x2a29);

/// Shown by the watch under the sensor's "about" page.
const MANUFACTURER: &str = "tread-pod";
const MODEL: &str = "tread-pod FTMS bridge";
const HARDWARE: &str = "Seeed XIAO ESP32-S3";
/// Crate version and commit, set by `build.rs`.
const FIRMWARE: &str = env!("TREAD_POD_FIRMWARE_REVISION");

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
                notify_change(&tx, address(desc.address()), was_empty, set.is_empty());
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

        // Device Information. The serial number is the footpod's address, which is unique to the
        // chip and never changes.
        let serial: String = device
            .get_addr()?
            .as_be_bytes()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect();
        let info = server.create_service(DEVICE_INFORMATION);
        for (uuid, value) in [
            (MANUFACTURER_NAME, MANUFACTURER),
            (MODEL_NUMBER, MODEL),
            (SERIAL_NUMBER, serial.as_str()),
            (HARDWARE_REVISION, HARDWARE),
            (FIRMWARE_REVISION, FIRMWARE),
        ] {
            info.lock()
                .create_characteristic(uuid, NimbleProperties::READ)
                .lock()
                .set_value(value.as_bytes());
        }
        log::info!("device info: {MODEL}, firmware {FIRMWARE}, serial {serial}");

        // One central at a time: NimBLE stops advertising when a central connects, and only the
        // main loop starts it again (see `advertise`), once it knows who may connect next. Never
        // restart it here: a second watch nearby could take the footpod mid-run and record
        // another runner's treadmill. sdkconfig caps connections at 2 (treadmill + this one) as a
        // backstop.
        server.advertise_on_disconnect(false);
        // Connects come from a raw GAP listener, not `on_connect`: see `watch_links`.
        watch_links(tx.clone());
        server.on_disconnect(move |desc, reason| {
            log::info!("watch disconnected: {} ({reason:?})", desc.address());
            let watch = address(desc.address());
            let mut set = subscribed.lock().unwrap();
            let was_empty = set.is_empty();
            set.remove(&desc.conn_handle());
            notify_change(&tx, watch, was_empty, set.is_empty());
            send(
                &tx,
                Input::WatchDisconnected {
                    address: watch,
                    reason: hci_reason(reason),
                },
            );
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

    /// (Re)starts advertising. With `only`, just that watch may connect (it still shows to
    /// everyone, but the controller refuses other connections); with `None`, anyone may.
    pub fn advertise(only: Option<BLEAddress>) -> Result<(), BLEError> {
        let device = BLEDevice::take();
        let mut advertising = device.get_advertising().lock();
        if advertising.is_advertising() {
            advertising.stop()?;
        }
        // The allowlist can only change while nothing uses it.
        match only {
            Some(watch) => {
                device.set_white_list(&[watch])?;
                advertising.filter_policy(AdvFilterPolicy::Connect);
            }
            None => {
                device.set_white_list(&[])?;
                advertising.filter_policy(AdvFilterPolicy::None);
            }
        }
        advertising.start()
    }

    /// Whether a central holds the footpod, as NimBLE sees it. Set before `WatchConnected`
    /// reaches the main loop.
    pub fn has_central() -> bool {
        CENTRALS.load(Ordering::Relaxed) > 0
    }

    /// Drops a central that must not hold the footpod.
    pub fn drop_central(conn: u16) {
        if let Err(err) = BLEDevice::take().get_server().disconnect(conn) {
            log::warn!("dropping central {conn}: {err:?}");
        }
    }
}

/// Centrals connected to the footpod (links where it is the peripheral).
static CENTRALS: AtomicU8 = AtomicU8::new(0);

/// Reports every central that connects, as `Input::WatchConnected`, and keeps `CENTRALS`.
///
/// esp32-nimble's `on_connect` misses some: as a peripheral, NimBLE delivers the connect event
/// only after reading the central's version and features, and with the feature read's error as
/// its status if that fails, though the link stays up. esp32-nimble ignores a connect with an
/// error status, so such a watch (seen with `C3:A4:A4:0B:3E:75`) could hold the footpod unseen,
/// and once it left, advertising never came back. A GAP listener gets every event first, before
/// esp32-nimble's handler.
fn watch_links(tx: Sender) {
    // Both live as long as NimBLE: the listener is linked into its list.
    let tx: &'static Sender = Box::leak(Box::new(tx));
    let listener: &'static mut sys::ble_gap_event_listener =
        Box::leak(Box::new(unsafe { core::mem::zeroed() }));
    let rc = unsafe {
        sys::ble_gap_event_listener_register(
            listener,
            Some(on_gap_event),
            core::ptr::from_ref(tx).cast_mut().cast(),
        )
    };
    if rc != 0 {
        log::error!("can't listen for watch connections: {rc}");
    }
}

extern "C" fn on_gap_event(event: *mut sys::ble_gap_event, arg: *mut c_void) -> i32 {
    // SAFETY: NimBLE passes a valid event, and `arg` is the leaked `Sender` from `watch_links`.
    let (event, tx) = unsafe { (&*event, &*arg.cast::<Sender>()) };
    match event.type_ as u32 {
        sys::BLE_GAP_EVENT_CONNECT => {
            let connect = unsafe { &event.__bindgen_anon_1.connect };
            // `EAGAIN`: the link closed before it was reported, and no disconnect follows.
            if connect.status == sys::BLE_HS_EAGAIN as i32 {
                return 0;
            }
            let mut desc: sys::ble_gap_conn_desc = unsafe { core::mem::zeroed() };
            let found = unsafe { sys::ble_gap_conn_find(connect.conn_handle, &mut desc) } == 0;
            if !found || u32::from(desc.role) != sys::BLE_GAP_ROLE_SLAVE {
                return 0;
            }
            CENTRALS.fetch_add(1, Ordering::Relaxed);
            let address = BLEAddress::from(desc.peer_ota_addr);
            match connect.status {
                0 => log::info!("watch connected: {address}; not advertising until it leaves"),
                status => log::info!(
                    "watch connected: {address} (status {status}); not advertising until it leaves"
                ),
            }
            send(
                tx,
                Input::WatchConnected {
                    conn: connect.conn_handle,
                    address,
                },
            );
        }
        sys::BLE_GAP_EVENT_DISCONNECT => {
            let disconnect = unsafe { &event.__bindgen_anon_1.disconnect };
            if u32::from(disconnect.conn.role) == sys::BLE_GAP_ROLE_SLAVE {
                let _ = CENTRALS
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
            }
        }
        _ => {}
    }
    0
}

/// The address as the engine stores it, most significant byte first.
pub fn address(address: BLEAddress) -> Address {
    address.as_be_bytes()
}

/// The HCI reason behind a disconnect; 0 (unknown, treated as a lost link) if it isn't one.
fn hci_reason(reason: Result<(), BLEError>) -> u8 {
    let hci_base = sys::BLE_HS_ERR_HCI_BASE;
    match reason {
        Err(err) if (hci_base..hci_base + 0x100).contains(&err.code()) => {
            u8::try_from(err.code() - hci_base).unwrap_or(0)
        }
        _ => 0,
    }
}

fn notify_change(tx: &Sender, watch: Address, was_empty: bool, is_empty: bool) {
    match (was_empty, is_empty) {
        (true, false) => send(tx, Input::WatchSubscribed(watch)),
        (false, true) => send(tx, Input::WatchUnsubscribed),
        _ => {}
    }
}
