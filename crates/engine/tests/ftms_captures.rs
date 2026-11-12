//! Parses real treadmill captures from `tests/fixtures/ftms` (recorded with `sniff`).

use engine::{
    ftms::{Reassembler, TreadmillData, treadmill_data::Incline},
    speed::FtmsSpeed,
};

/// A capture line: `<ms> <uuid16> <hex>`.
struct Line {
    ms: u32,
    uuid: u16,
    bytes: Vec<u8>,
}

fn load(name: &str) -> Vec<Line> {
    let path = format!("{}/tests/fixtures/ftms/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.split_whitespace();
            let ms = parts.next().unwrap().parse().unwrap();
            let uuid = u16::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let hex = parts.next().unwrap();
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            Line { ms, uuid, bytes }
        })
        .collect()
}

/// Parses every Treadmill Data notification and reassembles records, with the time each
/// record completed.
fn records(lines: &[Line]) -> Vec<(u32, TreadmillData)> {
    let mut reassembler = Reassembler::new();
    lines
        .iter()
        .filter(|l| l.uuid == 0x2acd)
        .flat_map(|l| {
            let part = TreadmillData::parse(&l.bytes)
                .unwrap_or_else(|e| panic!("{}ms {:02x?}: {e:?}", l.ms, l.bytes));
            reassembler
                .push(part)
                .map(|r| (l.ms, r))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn at(records: &[(u32, TreadmillData)], ms: u32) -> TreadmillData {
    records
        .iter()
        .find(|(t, _)| *t >= ms)
        .map(|(_, r)| *r)
        .unwrap()
}

/// See the capture's `# note:` for the session.
#[test]
fn fs_4c02a8_session() {
    let records = records(&load("fs-4c02a8-1794504381.txt"));

    // Two notifications per second (data + a More Data part), one record each.
    assert_eq!(records.len(), 132);
    for (_, r) in &records {
        assert!(r.speed.is_some(), "every record has speed");
        // No incline on this treadmill, but the fields are always sent.
        assert_eq!(
            r.incline,
            Some(Incline {
                inclination: 0,
                ramp_angle: 0
            })
        );
        assert_eq!(r.heart_rate, Some(0));
        let energy = r.energy.unwrap();
        assert_eq!((energy.per_hour, energy.per_minute), (None, None));
    }

    let speed = |ms| at(&records, ms).speed.unwrap();
    assert_eq!(speed(0), FtmsSpeed(0), "idle");
    assert_eq!(
        speed(9_000),
        FtmsSpeed(80),
        "starts at the 0.8 km/h minimum"
    );
    assert_eq!(speed(23_000), FtmsSpeed(300));
    assert_eq!(speed(50_000), FtmsSpeed(800));
    assert_eq!(speed(72_000), FtmsSpeed(900));
    let peak = records.iter().map(|(_, r)| r.speed.unwrap()).max().unwrap();
    assert_eq!(peak, FtmsSpeed(900));

    // After stop (~79 s) the belt slows down for ~17 s while distance and elapsed time stay
    // frozen, then everything resets to 0.
    let slowing: Vec<_> = records
        .iter()
        .filter(|(t, _)| (79_000..96_000).contains(t))
        .map(|(_, r)| r)
        .collect();
    assert!(slowing.len() >= 15);
    assert!(slowing.windows(2).all(|w| w[1].speed < w[0].speed));
    assert!(slowing.iter().all(|r| r.total_distance == Some(124)));
    assert!(slowing.iter().all(|r| r.elapsed_time == Some(70)));

    let end = records.last().unwrap().1;
    assert_eq!(
        (end.speed, end.total_distance, end.elapsed_time),
        (Some(FtmsSpeed(0)), Some(0), Some(0))
    );

    // Distance never goes backwards during the workout.
    let workout: Vec<_> = records
        .iter()
        .filter(|(t, _)| *t < 96_000)
        .map(|(_, r)| r.total_distance.unwrap())
        .collect();
    assert!(workout.windows(2).all(|w| w[0] <= w[1]));
}
