//! Owned events sent from BLE callbacks to the main loop, which hands them to the session.

use engine::{config::Address, session::Event};

#[derive(Debug)]
pub enum Input {
    Adv {
        address: Address,
        rssi: i8,
        name: Option<String>,
    },
    WatchSubscribed,
    WatchUnsubscribed,
    Connected(Address),
    Disconnected(Address),
    TreadmillData(Vec<u8>),
}

impl Input {
    pub fn as_event(&self) -> Event<'_> {
        match self {
            Self::Adv {
                address,
                rssi,
                name,
            } => Event::AdvSeen {
                address: *address,
                rssi: *rssi,
                name: name.as_deref(),
            },
            Self::WatchSubscribed => Event::WatchSubscribed,
            Self::WatchUnsubscribed => Event::WatchUnsubscribed,
            Self::Connected(address) => Event::Connected(*address),
            Self::Disconnected(address) => Event::Disconnected(*address),
            Self::TreadmillData(bytes) => Event::TreadmillData(bytes),
        }
    }
}

/// Where callbacks send their inputs.
pub type Sender = std::sync::mpsc::Sender<Input>;

/// Sends, ignoring a closed channel: that only happens while the main loop is shutting down.
pub fn send(tx: &Sender, input: Input) {
    let _ = tx.send(input);
}
