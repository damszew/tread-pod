//! Owned events sent from BLE callbacks to the main loop, which hands them to the session and
//! the watch gate.

use engine::{config::Address, session::Event};
use esp32_nimble::BLEAddress;

#[derive(Debug)]
pub enum Input {
    Adv {
        address: Address,
        rssi: i8,
        name: Option<String>,
    },
    /// A central (watch or phone) connected to the footpod.
    WatchConnected {
        conn: u16,
        address: BLEAddress,
    },
    /// The first subscriber to RSC Measurement.
    WatchSubscribed(Address),
    /// The last subscriber unsubscribed or disconnected.
    WatchUnsubscribed,
    /// A central left the footpod, with the HCI disconnect reason.
    WatchDisconnected {
        address: Address,
        reason: u8,
    },
    Connected(Address),
    Disconnected(Address),
    TreadmillData(Vec<u8>),
}

impl Input {
    /// The session's event for this input, if it has one; the rest only concern the watch gate.
    pub fn as_event(&self) -> Option<Event<'_>> {
        Some(match self {
            Self::Adv {
                address,
                rssi,
                name,
            } => Event::AdvSeen {
                address: *address,
                rssi: *rssi,
                name: name.as_deref(),
            },
            Self::WatchSubscribed(_) => Event::WatchSubscribed,
            Self::WatchUnsubscribed => Event::WatchUnsubscribed,
            Self::Connected(address) => Event::Connected(*address),
            Self::Disconnected(address) => Event::Disconnected(*address),
            Self::TreadmillData(bytes) => Event::TreadmillData(bytes),
            Self::WatchConnected { .. } | Self::WatchDisconnected { .. } => return None,
        })
    }
}

/// Where callbacks send their inputs.
pub type Sender = std::sync::mpsc::Sender<Input>;

/// Sends, ignoring a closed channel: that only happens while the main loop is shutting down.
pub fn send(tx: &Sender, input: Input) {
    let _ = tx.send(input);
}
