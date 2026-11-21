//! The top-level state machine: which treadmill to use, when to talk to it, and what to send the
//! watch.
//!
//! The backend calls [`Session::handle`] with a millisecond timestamp for every BLE, button, API
//! and timer event, and carries out the returned [`Action`]s in order. It must send
//! [`Event::Tick`] at least every 250 ms; timeouts and heartbeat notifications run off it.
//!
//! # Rules
//!
//! - **Only while a watch listens.** Nothing connects until the watch subscribes to RSC
//!   Measurement, and the treadmill is released when it unsubscribes: many treadmills accept a
//!   single connection, and their own app may want it.
//! - **One treadmill per workout.** Once picked, the *target* sticks until the watch leaves: if
//!   the link drops, the session waits for the same treadmill rather than jumping to a neighbour.
//!   The target is, in order of precedence, a button or API choice, the pinned treadmill, or the
//!   strongest signal after a [`SCAN_WINDOW_MS`] scan.
//! - **Steady notifications.** The watch gets a measurement for every treadmill record, and at
//!   least every [`NOTIFY_INTERVAL_MS`] otherwise; with no fresh speed it is 0, never silence,
//!   so the watch keeps the footpod. Nothing is sent for [`NOTIFY_DELAY_MS`] after subscribing:
//!   some Garmins drop data that arrives before they have processed the connection.
//!
//! # Logs
//!
//! The session logs, through `log`, what changes the watch's recording without changing the link
//! state, such as a treadmill going quiet for a few seconds. The backend picks the logger.

use heapless::Vec;

use crate::{
    cadence,
    config::{Address, Config},
    ftms::{Reassembler, TreadmillData},
    rsc::{Encoded, Feature, Measurement},
    selection::Candidates,
    speed::{DistanceSpeed, FtmsSpeed},
};

/// How long to collect advertisements before auto-picking the strongest treadmill.
pub const SCAN_WINDOW_MS: u64 = 2_000;
/// Give up on a connection attempt after this long.
pub const CONNECT_TIMEOUT_MS: u64 = 10_000;
/// Drop a connected treadmill that sends no data for this long.
pub const DATA_TIMEOUT_MS: u64 = 5_000;
/// Wait this long after a failed or lost connection before trying again.
pub const RETRY_DELAY_MS: u64 = 1_000;
/// No notifications for this long after the watch subscribes.
pub const NOTIFY_DELAY_MS: u64 = 1_000;
/// Longest gap between notifications. Treadmills send about once a second with jitter, so a
/// heartbeat at 1 s would often land just before a late record and double the rate.
pub const NOTIFY_INTERVAL_MS: u64 = 2_000;
/// A speed older than this is reported as 0.
pub const SPEED_STALE_MS: u64 = 3_000;
/// A gap between treadmill records at least this long is reported. Records normally come about
/// once a second.
pub const GAP_REPORT_MS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    /// An FTMS advertisement. `name` is `None` if the packet had no name.
    AdvSeen {
        address: Address,
        rssi: i8,
        name: Option<&'a str>,
    },
    /// The watch enabled RSC Measurement notifications.
    WatchSubscribed,
    /// The watch disabled notifications or disconnected.
    WatchUnsubscribed,
    /// Connected to the treadmill and subscribed to Treadmill Data.
    Connected(Address),
    /// The link to a treadmill closed, or a connection attempt failed.
    Disconnected(Address),
    /// A Treadmill Data (`0x2ACD`) notification from the connected treadmill.
    TreadmillData(&'a [u8]),
    /// Short button press: cycle to the next treadmill.
    Button,
    /// Use this treadmill (from the config API).
    Select(Address),
    /// New settings (from the config API).
    SetConfig(Config),
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Connect to a treadmill and subscribe to Treadmill Data; answer with
    /// [`Event::Connected`] or [`Event::Disconnected`].
    Connect(Address),
    /// Close the link or cancel the attempt; [`Event::Disconnected`] for it is expected and ignored.
    Disconnect(Address),
    /// Notify RSC Measurement to the watch.
    NotifyRsc(Encoded),
    /// Store the config.
    Persist(Config),
}

/// The most actions one event produces: disconnect, connect, notify, persist.
pub type Actions = Vec<Action, 4>;

/// The link to the treadmill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// No watch is subscribed.
    Idle,
    /// Waiting for the target to show up, or for the scan window to pick one.
    Scanning {
        since: u64,
        retry_at: u64,
    },
    Connecting {
        address: Address,
        since: u64,
    },
    /// `last_data` is `None` until the first record: connected but not forwarding yet.
    Connected {
        address: Address,
        since: u64,
        last_data: Option<u64>,
    },
}

#[derive(Debug)]
pub struct Session {
    config: Config,
    candidates: Candidates,
    target: Option<Address>,
    link: Link,
    /// When the watch subscribed.
    watch: Option<u64>,
    reassembler: Reassembler,
    distance_speed: DistanceSpeed,
    /// Latest speed and when it arrived.
    speed: Option<(u64, FtmsSpeed)>,
    last_notify: Option<u64>,
    /// The stale speed was logged for the current speed.
    stale_reported: bool,
}

impl Session {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            candidates: Candidates::new(),
            target: None,
            link: Link::Idle,
            watch: None,
            reassembler: Reassembler::new(),
            distance_speed: DistanceSpeed::new(),
            speed: None,
            last_notify: None,
            stale_reported: false,
        }
    }

    /// The RSC Feature value: stride length is the only optional field. It is read once at
    /// pairing, so it is offered even while [`Config::cadence`] is off; the measurements then
    /// simply leave it out.
    pub fn rsc_feature() -> Feature {
        Feature {
            stride_length: true,
            ..Feature::default()
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn candidates(&self) -> &Candidates {
        &self.candidates
    }

    /// The treadmill this workout uses, once chosen.
    pub fn target(&self) -> Option<Address> {
        self.target
    }

    pub fn link(&self) -> Link {
        self.link
    }

    /// The speed the watch gets now.
    pub fn speed(&self, now_ms: u64) -> FtmsSpeed {
        match self.speed {
            Some((t, s)) if now_ms < t + SPEED_STALE_MS => s,
            _ => FtmsSpeed::ZERO,
        }
    }

    pub fn handle(&mut self, now_ms: u64, event: Event<'_>) -> Actions {
        let mut out = Actions::new();
        match event {
            Event::AdvSeen {
                address,
                rssi,
                name,
            } => self.candidates.seen(now_ms, address, rssi, name),
            Event::WatchSubscribed => self.watch_subscribed(now_ms),
            Event::WatchUnsubscribed => self.watch_unsubscribed(&mut out),
            Event::Connected(address) => self.connected(now_ms, address, &mut out),
            Event::Disconnected(address) => {
                if self.link_address() == Some(address) {
                    self.rescan(now_ms, now_ms + RETRY_DELAY_MS);
                }
            }
            Event::TreadmillData(bytes) => self.treadmill_data(now_ms, bytes, &mut out),
            Event::Button => {
                if self.watch.is_some()
                    && let Some(next) = self.candidates.after(self.target)
                {
                    self.select(now_ms, next, &mut out);
                }
            }
            Event::Select(address) => self.select(now_ms, address, &mut out),
            Event::SetConfig(config) => {
                let new_pin = config
                    .pinned
                    .filter(|_| config.pinned != self.config.pinned);
                self.config = config.clone();
                emit(&mut out, Action::Persist(config));
                if let Some(pin) = new_pin {
                    self.select(now_ms, pin, &mut out);
                }
            }
            Event::Tick => self.candidates.expire(now_ms),
        }
        self.poll(now_ms, &mut out);
        out
    }

    fn watch_subscribed(&mut self, now_ms: u64) {
        self.watch = Some(now_ms);
        self.last_notify = None;
        if self.link == Link::Idle {
            self.target = self.target.or(self.config.pinned);
            self.rescan(now_ms, now_ms);
        }
    }

    fn watch_unsubscribed(&mut self, out: &mut Actions) {
        if let Some(address) = self.link_address() {
            emit(out, Action::Disconnect(address));
        }
        self.watch = None;
        self.target = None;
        self.link = Link::Idle;
        self.reset_stream();
    }

    fn connected(&mut self, now_ms: u64, address: Address, out: &mut Actions) {
        match self.link {
            Link::Connecting { address: a, .. } if a == address => {
                self.link = Link::Connected {
                    address,
                    since: now_ms,
                    last_data: None,
                };
            }
            // Late: the session gave up on it or moved on.
            _ => emit(out, Action::Disconnect(address)),
        }
    }

    fn treadmill_data(&mut self, now_ms: u64, bytes: &[u8], out: &mut Actions) {
        let Link::Connected {
            address,
            last_data,
            since,
        } = self.link
        else {
            return;
        };
        // A malformed packet is dropped; the data timeout catches a treadmill that only sends those.
        let Ok(part) = TreadmillData::parse(bytes) else {
            return;
        };
        let mut any = false;
        for record in self.reassembler.push(part) {
            any = true;
            if let Some(speed) = record_speed(&mut self.distance_speed, now_ms, &record) {
                if speed.is_zero() && self.speed.is_some_and(|(_, s)| !s.is_zero()) {
                    log::info!("treadmill reports speed 0");
                }
                self.speed = Some((now_ms, speed));
                self.stale_reported = false;
            }
        }
        if any {
            if let Some(previous) = last_data
                && now_ms >= previous + GAP_REPORT_MS
            {
                log::warn!("treadmill data gap: {} ms", now_ms - previous);
            }
            self.link = Link::Connected {
                address,
                since,
                last_data: Some(now_ms),
            };
            self.candidates.touch(now_ms, address);
            self.notify(now_ms, out);
        }
    }

    /// Switches the target, dropping the current link if it is to another treadmill.
    fn select(&mut self, now_ms: u64, address: Address, out: &mut Actions) {
        self.target = Some(address);
        match self.link_address() {
            Some(current) if current == address => {}
            Some(current) => {
                emit(out, Action::Disconnect(current));
                self.rescan(now_ms, now_ms);
            }
            None => {
                if let Link::Scanning { since, .. } = self.link {
                    self.link = Link::Scanning {
                        since,
                        retry_at: now_ms,
                    };
                }
            }
        }
    }

    /// Timeouts, connecting and the heartbeat notification; runs after every event.
    fn poll(&mut self, now_ms: u64, out: &mut Actions) {
        match self.link {
            Link::Idle => return,
            Link::Scanning { since, retry_at } => {
                if now_ms >= retry_at {
                    let pick = match self.target {
                        Some(target) => self.candidates.get(target).map(|c| c.address),
                        None if now_ms >= since + SCAN_WINDOW_MS => self.candidates.strongest(),
                        None => None,
                    };
                    if let Some(address) = pick {
                        self.target = Some(address);
                        self.link = Link::Connecting {
                            address,
                            since: now_ms,
                        };
                        emit(out, Action::Connect(address));
                    }
                }
            }
            Link::Connecting { address, since } => {
                if now_ms >= since + CONNECT_TIMEOUT_MS {
                    emit(out, Action::Disconnect(address));
                    self.rescan(now_ms, now_ms + RETRY_DELAY_MS);
                }
            }
            Link::Connected {
                address,
                since,
                last_data,
            } => {
                if now_ms >= last_data.unwrap_or(since) + DATA_TIMEOUT_MS {
                    log::warn!("treadmill silent too long, reconnecting");
                    emit(out, Action::Disconnect(address));
                    self.rescan(now_ms, now_ms + RETRY_DELAY_MS);
                } else if let Some((t, _)) = self.speed
                    && now_ms >= t + SPEED_STALE_MS
                    && !self.stale_reported
                {
                    self.stale_reported = true;
                    log::warn!("no treadmill data, sending speed 0");
                }
            }
        }
        if self
            .last_notify
            .is_none_or(|t| now_ms >= t + NOTIFY_INTERVAL_MS)
        {
            self.notify(now_ms, out);
        }
    }

    fn notify(&mut self, now_ms: u64, out: &mut Actions) {
        let Some(subscribed) = self.watch else {
            return;
        };
        if now_ms < subscribed + NOTIFY_DELAY_MS {
            return;
        }
        let speed = self.speed(now_ms);
        let estimate = self
            .config
            .cadence
            .then(|| cadence::estimate(speed))
            .flatten();
        let measurement = Measurement {
            speed: speed.into(),
            cadence: estimate.map_or(0, |e| e.strides_per_minute),
            stride_length: estimate.map(|e| e.stride_length),
            ..Measurement::default()
        };
        emit(out, Action::NotifyRsc(measurement.encode()));
        self.last_notify = Some(now_ms);
    }

    fn rescan(&mut self, now_ms: u64, retry_at: u64) {
        self.link = Link::Scanning {
            since: now_ms,
            retry_at,
        };
        self.reset_stream();
    }

    /// Forgets data from the previous link, so nothing carries over to another treadmill.
    fn reset_stream(&mut self) {
        self.reassembler = Reassembler::new();
        self.distance_speed = DistanceSpeed::new();
        self.speed = None;
        self.stale_reported = false;
    }

    fn link_address(&self) -> Option<Address> {
        match self.link {
            Link::Connecting { address, .. } | Link::Connected { address, .. } => Some(address),
            Link::Idle | Link::Scanning { .. } => None,
        }
    }
}

/// The record's speed, or one derived from its total distance if it has none.
fn record_speed(
    distance_speed: &mut DistanceSpeed,
    now_ms: u64,
    record: &TreadmillData,
) -> Option<FtmsSpeed> {
    match (record.speed, record.total_distance) {
        (Some(speed), _) => Some(speed),
        (None, Some(metres)) => distance_speed.push(now_ms, metres),
        (None, None) => None,
    }
}

/// A free function so it can run while the reassembler is borrowed.
fn emit(out: &mut Actions, action: Action) {
    let pushed = out.push(action);
    debug_assert!(pushed.is_ok(), "Actions capacity too small");
}
