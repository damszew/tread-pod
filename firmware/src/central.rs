//! The treadmill side: scans for FTMS treadmills and holds the one connection the session asks
//! for.
//!
//! Runs on its own thread because esp32-nimble's scan and connect calls are async and block for
//! their duration. Scanning runs in short windows; between them the thread picks up commands, so a
//! connect waits at most one window. NimBLE can't start a connection while discovering, which
//! rules out one endless scan.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use embassy_futures::select::{Either, select};
use engine::config::Address;
use esp_idf_svc::{
    hal::{delay::FreeRtos, task::block_on},
    timer::{EspAsyncTimer, EspTaskTimerService},
};
use esp32_nimble::{
    BLEAddress, BLEAddressType, BLEAdvertisedData, BLEClient, BLEDevice, BLEError, BLEScan,
    utilities::BleUuid,
};

use crate::input::{Input, Sender, send};

const FTMS_SERVICE: BleUuid = BleUuid::from_uuid16(0x1826);
const TREADMILL_DATA: BleUuid = BleUuid::from_uuid16(0x2acd);

/// Length of one scan window; also the longest a command waits.
const SCAN_WINDOW_MS: i32 = 500;
/// Scan duty cycle: listen 30 ms of every 100 ms, leaving the radio to the two connections.
const SCAN_INTERVAL_MS: u16 = 100;
const SCAN_LISTEN_MS: u16 = 30;
/// How often to log what scanning saw.
const SCAN_SUMMARY: Duration = Duration::from_secs(10);
/// Longest a connect + discover + subscribe may take. esp32-nimble's `connect` waits for the MTU
/// exchange, and a link lost before it completes never wakes it, so it needs an outer bound.
/// Shorter than the session's connect timeout, so this thread is free again when the session
/// retries.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

enum Command {
    Connect(Address),
    Disconnect(Address),
}

/// Handle used by the main loop.
pub struct Central {
    commands: mpsc::Sender<Command>,
    /// The address a connection attempt is in flight to, so it can be cancelled.
    connecting: Arc<Mutex<Option<Address>>>,
}

impl Central {
    pub fn spawn(tx: Sender) -> anyhow::Result<Self> {
        let (commands, rx) = mpsc::channel();
        let connecting = Arc::new(Mutex::new(None));
        let state = connecting.clone();
        thread::Builder::new()
            .name("central".into())
            .stack_size(12 * 1024)
            .spawn(move || block_on(run(&rx, &tx, &state)))?;
        Ok(Self {
            commands,
            connecting,
        })
    }

    pub fn connect(&self, address: Address) {
        let _ = self.commands.send(Command::Connect(address));
    }

    pub fn disconnect(&self, address: Address) {
        // The central thread is stuck in the connect call until it completes, so an attempt the
        // session gave up on is cancelled from here.
        if *self.connecting.lock().unwrap() == Some(address) {
            // SAFETY: plain NimBLE host call; it fails harmlessly if no attempt is pending.
            unsafe {
                esp_idf_svc::sys::ble_gap_conn_cancel();
            }
        }
        let _ = self.commands.send(Command::Disconnect(address));
    }
}

async fn run(commands: &Receiver<Command>, tx: &Sender, connecting: &Mutex<Option<Address>>) {
    let device = BLEDevice::take();
    let timers = EspTaskTimerService::new().expect("timer service");
    let mut timer = timers.timer_async().expect("async timer");
    let mut scan = BLEScan::new();
    scan.active_scan(true)
        .interval(SCAN_INTERVAL_MS)
        .window(SCAN_LISTEN_MS);
    // Treadmills seen, with their address type: many use random static addresses.
    let mut known: HashMap<Address, BLEAddress> = HashMap::new();
    // Treadmills whose name has been logged.
    let mut named: HashSet<Address> = HashSet::new();
    // Every advertiser seen since the last summary, to tell "no treadmill" from "scan sees nothing".
    let mut heard: HashSet<Address> = HashSet::new();
    let mut summary_at = Instant::now() + SCAN_SUMMARY;
    let mut wanted: Option<Address> = None;
    // Boxed: NimBLE keeps a pointer to the client, so it must not move once connected.
    let mut link: Option<(Address, Box<BLEClient>)> = None;

    loop {
        // Commands are folded into the wanted state, so a connect cancelled before it started
        // never happens.
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Connect(address) => wanted = Some(address),
                Command::Disconnect(address) => {
                    if wanted == Some(address) {
                        wanted = None;
                    }
                }
            }
        }

        if let Some((address, client)) = &mut link {
            if !client.connected() {
                // Lost; the disconnect callback already told the session.
                link = None;
            } else if wanted != Some(*address) {
                log::info!("disconnecting {}", fmt(*address));
                if let Err(err) = client.disconnect() {
                    log::warn!("disconnect: {err:?}");
                }
                // Dropping a connected client detaches its callbacks: the session asked for this
                // and expects no event.
                link = None;
            }
        }

        if link.is_none()
            && let Some(address) = wanted
        {
            let peer = known
                .get(&address)
                .copied()
                .unwrap_or_else(|| BLEAddress::from_be_bytes(address, BLEAddressType::Public));
            *connecting.lock().unwrap() = Some(address);
            let result = connect(device, peer, tx, &mut timer).await;
            *connecting.lock().unwrap() = None;
            match result {
                Ok(client) => {
                    log::info!("treadmill {} connected", fmt(address));
                    send(tx, Input::Connected(address));
                    link = Some((address, client));
                }
                Err(err) => {
                    log::warn!("treadmill {} connect failed: {err:?}", fmt(address));
                    send(tx, Input::Disconnected(address));
                    // Don't retry until the session asks again.
                    wanted = None;
                }
            }
            continue;
        }

        let result = scan
            .start(device, SCAN_WINDOW_MS, |dev, data| {
                let address = dev.addr().as_be_bytes();
                heard.insert(address);
                // Scan responses carry the name but usually not the service list.
                if is_ftms(&data) || known.contains_key(&address) {
                    if known.insert(address, dev.addr()).is_none() {
                        log::info!("treadmill seen: {} ({} dBm)", fmt(address), dev.rssi());
                    }
                    let name = data.name().map(|n| String::from_utf8_lossy(n).into_owned());
                    if let Some(name) = &name
                        && named.insert(address)
                    {
                        log::info!("treadmill {} is {name:?}", fmt(address));
                    }
                    send(
                        tx,
                        Input::Adv {
                            address,
                            rssi: dev.rssi(),
                            name,
                        },
                    );
                }
                None::<()>
            })
            .await;
        if let Err(err) = result {
            log::warn!("scan: {err:?}");
            FreeRtos::delay_ms(100);
        }
        if Instant::now() >= summary_at {
            summary_at += SCAN_SUMMARY;
            log::info!(
                "scan: {} advertisers in the last {} s, {} treadmills known",
                heard.len(),
                SCAN_SUMMARY.as_secs(),
                known.len()
            );
            heard.clear();
        }
    }
}

/// Connects and subscribes to Treadmill Data, within [`CONNECT_TIMEOUT`].
async fn connect(
    device: &BLEDevice,
    peer: BLEAddress,
    tx: &Sender,
    timer: &mut EspAsyncTimer,
) -> anyhow::Result<Box<BLEClient>> {
    let address = peer.as_be_bytes();
    // Boxed: NimBLE keeps a pointer to the client, so it must not move once connecting.
    let mut client = Box::new(device.new_client());
    {
        let tx = tx.clone();
        client.on_disconnect(move |reason| {
            log::info!("treadmill {} disconnected ({reason})", fmt(address));
            send(&tx, Input::Disconnected(address));
        });
    }
    match select(set_up(&mut client, peer, tx), timer.after(CONNECT_TIMEOUT)).await {
        Either::First(Ok(())) => Ok(client),
        Either::First(Err(err)) => {
            // Dropping a connected client detaches its callbacks; a failed connect left none.
            let _ = client.disconnect();
            Err(anyhow::anyhow!("{err:?}"))
        }
        Either::Second(_) => {
            // SAFETY: plain NimBLE host call; it fails harmlessly if no attempt is pending.
            unsafe {
                esp_idf_svc::sys::ble_gap_conn_cancel();
            }
            let _ = client.disconnect();
            // NimBLE may still deliver this attempt's events to the client, so it must outlive
            // them: leak it. A few hundred bytes, only when a connect hangs.
            Box::leak(client);
            Err(anyhow::anyhow!(
                "timed out after {} s",
                CONNECT_TIMEOUT.as_secs()
            ))
        }
    }
}

async fn set_up(client: &mut BLEClient, peer: BLEAddress, tx: &Sender) -> Result<(), BLEError> {
    client.connect(&peer).await?;
    let tx = tx.clone();
    client
        .get_service(FTMS_SERVICE)
        .await?
        .get_characteristic(TREADMILL_DATA)
        .await?
        .on_notify(move |data| send(&tx, Input::TreadmillData(data.to_vec())))
        // With response: the CCCD must be written with a Write Request, and a Write Command
        // (no response) can be silently ignored: the FS-4C02A8 sent no data after one.
        .subscribe_notify(true)
        .await
}

fn is_ftms(data: &BLEAdvertisedData<&[u8]>) -> bool {
    data.is_advertising_service(&FTMS_SERVICE)
        || data.service_data().is_some_and(|d| d.uuid == FTMS_SERVICE)
}

/// `AA:BB:CC:DD:EE:FF`, most significant byte first.
pub fn fmt(address: Address) -> String {
    address
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
