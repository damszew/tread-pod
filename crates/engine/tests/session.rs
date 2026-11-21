//! Scripted end-to-end runs of [`Session`]: the backend's side of the conversation, event by event.

mod common;

use engine::{
    config::{Address, Config},
    session::{
        Action, DATA_TIMEOUT_MS, Event, GAP_REPORT_MS, Link, NOTIFY_DELAY_MS, SCAN_WINDOW_MS,
        SPEED_STALE_MS, Session,
    },
    speed::{FtmsSpeed, RscSpeed},
};

/// The session's log lines on this test's thread since the last call.
fn logs() -> Vec<String> {
    struct Capture;
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            LINES.with(|l| l.borrow_mut().push(record.args().to_string()));
        }
        fn flush(&self) {}
    }
    thread_local!(static LINES: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) });
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
    LINES.take()
}

const A: Address = [0xa; 6];
const B: Address = [0xb; 6];
const C: Address = [0xc; 6];

/// Drives a session with a clock and a 100 ms tick, like the firmware's main loop.
struct Sim {
    session: Session,
    now: u64,
}

impl Sim {
    fn new() -> Self {
        Self::with(Config::default())
    }

    fn with(config: Config) -> Self {
        // Installs the capture logger and clears this thread's lines.
        logs();
        Self {
            session: Session::new(config),
            now: 0,
        }
    }

    fn ev(&mut self, event: Event<'_>) -> Vec<Action> {
        self.session.handle(self.now, event).into_iter().collect()
    }

    fn adv(&mut self, address: Address, rssi: i8) -> Vec<Action> {
        self.ev(Event::AdvSeen {
            address,
            rssi,
            name: None,
        })
    }

    /// Ticks until `ms` later, returning every action with its time.
    fn run(&mut self, ms: u64) -> Vec<(u64, Action)> {
        let end = self.now + ms;
        let mut out = Vec::new();
        while self.now < end {
            self.now = (self.now + 100).min(end);
            out.extend(self.ev(Event::Tick).into_iter().map(|a| (self.now, a)));
        }
        out
    }

    /// Subscribes the watch and connects to `address`, the only treadmill around.
    fn connect(&mut self, address: Address) {
        self.ev(Event::WatchSubscribed);
        self.adv(address, -60);
        assert_eq!(connects(&self.run(SCAN_WINDOW_MS)), [address]);
        self.ev(Event::Connected(address));
    }
}

fn speed_only(centi_kmh: u16) -> Vec<u8> {
    let [lo, hi] = centi_kmh.to_le_bytes();
    vec![0x00, 0x00, lo, hi]
}

/// More Data + Total Distance only, as a treadmill without instantaneous speed sends.
fn distance_only(metres: u32) -> Vec<u8> {
    let [a, b, c, _] = metres.to_le_bytes();
    vec![0x05, 0x00, a, b, c]
}

/// RSC speeds notified, in order.
fn notified(actions: &[(u64, Action)]) -> Vec<u16> {
    actions.iter().filter_map(|(_, a)| rsc_speed(a)).collect()
}

fn rsc_speed(action: &Action) -> Option<u16> {
    match action {
        Action::NotifyRsc(m) => Some(u16::from_le_bytes([m[1], m[2]])),
        _ => None,
    }
}

fn connects(actions: &[(u64, Action)]) -> Vec<Address> {
    actions
        .iter()
        .filter_map(|(_, a)| match a {
            Action::Connect(address) => Some(*address),
            _ => None,
        })
        .collect()
}

fn rsc(centi_kmh: u16) -> u16 {
    RscSpeed::from(FtmsSpeed(centi_kmh)).0
}

#[test]
fn nothing_happens_without_a_watch() {
    let mut sim = Sim::new();
    sim.adv(A, -50);
    assert_eq!(sim.run(10_000), []);
    assert_eq!(sim.session.link(), Link::Idle);
}

#[test]
fn auto_picks_the_strongest_after_the_scan_window() {
    let mut sim = Sim::new();
    sim.ev(Event::WatchSubscribed);
    sim.adv(A, -70);
    sim.adv(B, -50);
    let actions = sim.run(SCAN_WINDOW_MS);
    assert_eq!(connects(&actions), [B]);
    assert_eq!(
        actions
            .iter()
            .find(|(_, a)| *a == Action::Connect(B))
            .unwrap()
            .0,
        SCAN_WINDOW_MS
    );
}

#[test]
fn pinned_treadmill_is_connected_on_sight_and_others_ignored() {
    let mut sim = Sim::with(Config {
        pinned: Some(A),
        ..Config::default()
    });
    sim.ev(Event::WatchSubscribed);
    sim.adv(B, -40);
    assert_eq!(connects(&sim.run(5_000)), Vec::<Address>::new());
    assert_eq!(sim.adv(A, -90), [Action::Connect(A)]);
}

#[test]
fn notifications_wait_for_the_delay_then_follow_the_treadmill() {
    let mut sim = Sim::with(Config {
        pinned: Some(A),
        ..Config::default()
    });
    sim.ev(Event::WatchSubscribed);
    sim.adv(A, -60);
    sim.ev(Event::Connected(A));
    sim.now = 500;
    assert_eq!(
        sim.ev(Event::TreadmillData(&speed_only(1000))),
        [],
        "within the notify delay"
    );

    let actions = sim.run(NOTIFY_DELAY_MS - 500);
    assert_eq!(notified(&actions), [rsc(1000)]);
    assert_eq!(actions[0].0, NOTIFY_DELAY_MS);

    // every record is notified immediately
    let actions = sim.ev(Event::TreadmillData(&speed_only(1010)));
    assert_eq!(
        actions.iter().filter_map(rsc_speed).collect::<Vec<_>>(),
        [rsc(1010)]
    );
}

#[test]
fn silence_becomes_zero_speed_then_the_treadmill_is_dropped_and_retried() {
    let mut sim = Sim::new();
    sim.connect(A);
    sim.run(NOTIFY_DELAY_MS);
    sim.ev(Event::TreadmillData(&speed_only(900)));
    let last_data = sim.now;

    let actions = sim.run(DATA_TIMEOUT_MS + 1_500);
    // heartbeat every two seconds: last speed while fresh, then 0
    assert_eq!(notified(&actions), [rsc(900), 0, 0]);
    let disconnect = actions
        .iter()
        .find(|(_, a)| *a == Action::Disconnect(A))
        .unwrap();
    assert_eq!(disconnect.0, last_data + DATA_TIMEOUT_MS);
    // still the target, still a known candidate: retried after the retry delay
    assert_eq!(connects(&actions), [A]);
}

#[test]
fn a_lost_link_waits_for_the_same_treadmill() {
    let mut sim = Sim::new();
    sim.connect(A);
    sim.ev(Event::Disconnected(A));
    sim.adv(C, -30); // a stronger neighbour
    let actions = sim.run(3_000);
    assert_eq!(connects(&actions), [A]);
    assert_eq!(sim.session.target(), Some(A));
}

#[test]
fn button_cycles_to_the_next_treadmill() {
    let mut sim = Sim::new();
    sim.ev(Event::WatchSubscribed);
    sim.adv(A, -50);
    sim.adv(B, -70);
    sim.adv(C, -80);
    sim.run(SCAN_WINDOW_MS);
    sim.ev(Event::Connected(A));

    assert_eq!(
        sim.ev(Event::Button),
        [Action::Disconnect(A), Action::Connect(B)]
    );
    // the old link's close arrives late and changes nothing
    assert_eq!(sim.ev(Event::Disconnected(A)), []);
    assert_eq!(sim.ev(Event::Connected(B)), []);
    assert!(matches!(
        sim.session.link(),
        Link::Connected { address: B, .. }
    ));

    sim.ev(Event::Button);
    sim.ev(Event::Connected(C));
    assert_eq!(
        sim.ev(Event::Button),
        [Action::Disconnect(C), Action::Connect(A)],
        "wraps around"
    );
}

#[test]
fn button_without_alternatives_does_nothing() {
    let mut sim = Sim::new();
    sim.connect(A);
    assert_eq!(sim.ev(Event::Button), []);
}

#[test]
fn watch_leaving_releases_the_treadmill_and_the_next_workout_picks_again() {
    let mut sim = Sim::new();
    sim.connect(A);
    sim.adv(B, -30);
    assert_eq!(sim.ev(Event::WatchUnsubscribed), [Action::Disconnect(A)]);
    assert_eq!(sim.run(5_000), [], "no notifications, no reconnects");

    sim.adv(B, -30);
    sim.ev(Event::WatchSubscribed);
    assert_eq!(connects(&sim.run(SCAN_WINDOW_MS)), [B]);
}

#[test]
fn connect_timeout_gives_up_and_retries() {
    let mut sim = Sim::new();
    sim.ev(Event::WatchSubscribed);
    sim.adv(A, -60);
    sim.run(SCAN_WINDOW_MS);
    let actions = sim.run(10_000);
    assert!(actions.contains(&(12_000, Action::Disconnect(A))));
    assert_eq!(
        connects(&actions),
        Vec::<Address>::new(),
        "A expired from the list: wait for it"
    );
    sim.adv(A, -60);
    assert_eq!(connects(&sim.run(1_000)), [A], "after the retry delay");
    // a connection the session is not waiting for is closed
    assert_eq!(sim.ev(Event::Connected(B)), [Action::Disconnect(B)]);
}

#[test]
fn api_select_before_the_workout_is_used_at_subscribe() {
    let mut sim = Sim::new();
    sim.adv(A, -40);
    sim.adv(B, -80);
    sim.ev(Event::Select(B));
    assert_eq!(sim.ev(Event::WatchSubscribed), [Action::Connect(B)]);
}

#[test]
fn new_pin_is_persisted_and_switches_treadmill() {
    let mut sim = Sim::new();
    sim.connect(A);
    sim.adv(B, -80);
    let config = Config {
        pinned: Some(B),
        ..Config::default()
    };
    assert_eq!(
        sim.ev(Event::SetConfig(config.clone())),
        [
            Action::Persist(config.clone()),
            Action::Disconnect(A),
            Action::Connect(B)
        ]
    );
    // the same pin again only persists
    assert_eq!(
        sim.ev(Event::SetConfig(config.clone())),
        [Action::Persist(config)]
    );
}

#[test]
fn speed_is_derived_from_distance_when_the_treadmill_has_none() {
    let mut sim = Sim::new();
    sim.connect(A);
    let mut speeds = Vec::new();
    // 2.5 m/s = 9 km/h, one record per second
    for i in 0..15u32 {
        speeds.extend(
            sim.ev(Event::TreadmillData(&distance_only(i * 5 / 2)))
                .iter()
                .filter_map(rsc_speed),
        );
        speeds.extend(notified(&sim.run(1_000)));
    }
    let last = *speeds.last().unwrap();
    assert!(
        (rsc(864)..=rsc(936)).contains(&last),
        "{last} not about {}",
        rsc(900)
    );
}

/// Replays the FS-4C02A8 capture through a whole session.
#[test]
fn fs_4c02a8_replay() {
    let lines = common::load("fs-4c02a8-1794504381.txt");
    let mut sim = Sim::new();
    sim.connect(A);
    let start = sim.now;
    let mut notifications = Vec::new();
    for line in lines.iter().filter(|l| l.uuid == 0x2acd) {
        let t = start + u64::from(line.ms);
        notifications.extend(sim.run(t - sim.now));
        let actions = sim.ev(Event::TreadmillData(&line.bytes));
        notifications.extend(actions.into_iter().map(|a| (t, a)));
    }
    assert!(
        notifications
            .iter()
            .all(|(_, a)| matches!(a, Action::NotifyRsc(_))),
        "no reconnects during the session"
    );
    let speeds = notified(&notifications);
    // one notification per treadmill record (about 1 Hz), no heartbeats in between
    assert_eq!(speeds.len(), 132);
    assert_eq!(speeds.iter().max(), Some(&rsc(900)));
    assert!(speeds.contains(&rsc(80)));
    assert_eq!(speeds.last(), Some(&0));
}

/// The cadence byte and stride length (1/100 m) of a notification.
fn cadence_and_stride(action: &Action) -> Option<(u8, Option<u16>)> {
    match action {
        Action::NotifyRsc(m) => {
            let stride = (m[0] & 0x01 != 0).then(|| u16::from_le_bytes([m[4], m[5]]));
            Some((m[3], stride))
        }
        _ => None,
    }
}

#[test]
fn estimated_cadence_and_stride_follow_the_speed() {
    let mut sim = Sim::new();
    sim.connect(A);
    sim.run(NOTIFY_DELAY_MS);
    let actions = sim.ev(Event::TreadmillData(&speed_only(850)));
    let expected = engine::cadence::estimate(FtmsSpeed(850)).unwrap();
    assert_eq!(
        actions
            .iter()
            .filter_map(cadence_and_stride)
            .collect::<Vec<_>>(),
        [(expected.strides_per_minute, Some(expected.stride_length))]
    );

    // standing still: cadence 0, no stride length
    let actions = sim.ev(Event::TreadmillData(&speed_only(0)));
    assert_eq!(
        actions
            .iter()
            .filter_map(cadence_and_stride)
            .collect::<Vec<_>>(),
        [(0, None)]
    );
}

#[test]
fn cadence_off_sends_zero_and_no_stride() {
    let mut sim = Sim::with(Config {
        cadence: false,
        ..Config::default()
    });
    sim.connect(A);
    sim.run(NOTIFY_DELAY_MS);
    let actions = sim.ev(Event::TreadmillData(&speed_only(850)));
    assert_eq!(
        actions
            .iter()
            .filter_map(cadence_and_stride)
            .collect::<Vec<_>>(),
        [(0, None)]
    );
    // the feature still offers stride length: it is read once, at pairing
    assert_eq!(Session::rsc_feature().encode(), [0x01, 0x00]);
}

/// Sends a record every second for `secs` seconds.
fn steady(sim: &mut Sim, centi_kmh: u16, secs: u64) {
    for _ in 0..secs {
        sim.ev(Event::TreadmillData(&speed_only(centi_kmh)));
        sim.run(1_000);
    }
}

#[test]
fn steady_records_report_nothing() {
    let mut sim = Sim::new();
    sim.connect(A);
    steady(&mut sim, 1000, 30);
    assert_eq!(logs(), Vec::<String>::new());
}

#[test]
fn a_short_silence_is_reported_as_stale_speed_then_a_gap() {
    let mut sim = Sim::new();
    sim.connect(A);
    steady(&mut sim, 1000, 5);
    logs();

    // The loop above ended a second after the last record.
    sim.run(SPEED_STALE_MS - 1_000 - 100);
    assert_eq!(logs(), Vec::<String>::new());
    sim.run(100);
    assert_eq!(logs(), ["no treadmill data, sending speed 0"]);
    // Reported once, not on every tick.
    sim.run(500);
    assert_eq!(logs(), Vec::<String>::new());

    sim.ev(Event::TreadmillData(&speed_only(1000)));
    assert_eq!(logs(), ["treadmill data gap: 3500 ms"]);
    assert_eq!(
        sim.session.link(),
        Link::Connected {
            address: A,
            since: SCAN_WINDOW_MS,
            last_data: Some(sim.now)
        }
    );
}

#[test]
fn a_gap_below_the_stale_limit_is_reported_alone() {
    let mut sim = Sim::new();
    sim.connect(A);
    steady(&mut sim, 1000, 3);
    sim.run(GAP_REPORT_MS - 1_000);
    sim.ev(Event::TreadmillData(&speed_only(1000)));
    assert_eq!(logs(), [format!("treadmill data gap: {GAP_REPORT_MS} ms")]);
}

#[test]
fn a_long_silence_ends_in_a_data_timeout() {
    let mut sim = Sim::new();
    sim.connect(A);
    steady(&mut sim, 1000, 3);
    sim.run(DATA_TIMEOUT_MS);
    assert_eq!(
        logs(),
        [
            "no treadmill data, sending speed 0",
            "treadmill silent too long, reconnecting"
        ]
    );
    // The next link starts clean: no gap from the old one.
    sim.adv(A, -60);
    sim.run(1_000);
    sim.ev(Event::Connected(A));
    sim.ev(Event::TreadmillData(&speed_only(1000)));
    assert_eq!(logs(), Vec::<String>::new());
}

#[test]
fn the_treadmill_reporting_zero_is_reported_once() {
    let mut sim = Sim::new();
    sim.connect(A);
    steady(&mut sim, 1000, 3);
    steady(&mut sim, 0, 3);
    assert_eq!(logs(), ["treadmill reports speed 0"]);
    // Starting from 0 is not a stop.
    steady(&mut sim, 1000, 3);
    assert_eq!(logs(), Vec::<String>::new());
}
