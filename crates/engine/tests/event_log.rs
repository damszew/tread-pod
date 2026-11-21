//! The event log against RAM storage that behaves like NOR flash, including power cuts.

use engine::event_log::{EventLog, MAX_TEXT, MemStorage, SECTOR_SIZE, SLOT_SIZE};

const SECTORS: u32 = 4;
const SLOTS: u32 = SECTORS * SECTOR_SIZE / SLOT_SIZE;

fn erased() -> Vec<u8> {
    vec![0xff; (SECTORS * SECTOR_SIZE) as usize]
}

fn seqs(flash: &mut [u8]) -> Vec<u32> {
    let mut out = Vec::new();
    EventLog::open(MemStorage(flash))
        .unwrap()
        .for_each(|e| out.push(e.seq))
        .unwrap();
    out
}

/// Appends `n` entries in one boot.
fn append(flash: &mut [u8], n: u32) {
    let mut log = EventLog::open(MemStorage(flash)).unwrap();
    for i in 0..n {
        log.append(u64::from(i), "watch connected").unwrap();
    }
}

#[test]
fn an_erased_partition_is_an_empty_log() {
    assert_eq!(seqs(&mut erased()), Vec::<u32>::new());
}

#[test]
fn entries_survive_a_reboot_in_order() {
    let mut flash = erased();
    for (uptime, text) in [(0, "boot"), (1_500, "watch connected")] {
        EventLog::open(MemStorage(&mut flash))
            .unwrap()
            .append(uptime, text)
            .unwrap();
    }
    let mut log = EventLog::open(MemStorage(&mut flash)).unwrap();
    log.append(42_000, "treadmill data gap: 3500 ms").unwrap();
    let mut lines = Vec::new();
    log.for_each(|e| lines.push((e.seq, e.uptime_ms, e.text.to_owned())))
        .unwrap();
    assert_eq!(
        lines,
        [
            (0, 0, "boot".to_owned()),
            (1, 1_500, "watch connected".to_owned()),
            (2, 42_000, "treadmill data gap: 3500 ms".to_owned()),
        ]
    );
}

#[test]
fn long_lines_are_cut_on_a_char_boundary() {
    let mut flash = erased();
    let mut log = EventLog::open(MemStorage(&mut flash)).unwrap();
    // A two-byte char straddles the limit.
    let line = format!("{}é tail", "x".repeat(MAX_TEXT - 1));
    log.append(0, &line).unwrap();
    let mut text = String::new();
    log.for_each(|e| text = e.text.to_owned()).unwrap();
    assert_eq!(text, "x".repeat(MAX_TEXT - 1));
}

#[test]
fn wrapping_drops_the_oldest_sector() {
    let mut flash = erased();
    let per_sector = SECTOR_SIZE / SLOT_SIZE;
    // Fill everything, then one more: the first sector is erased to make room.
    append(&mut flash, SLOTS + 1);
    let expected: Vec<u32> = (per_sector..=SLOTS).collect();
    assert_eq!(seqs(&mut flash), expected);
}

#[test]
fn wrapping_across_reboots_keeps_counting() {
    let mut flash = erased();
    for _ in 0..10 {
        append(&mut flash, SLOTS / 3);
    }
    let seqs = seqs(&mut flash);
    let last = 10 * (SLOTS / 3) - 1;
    assert_eq!(seqs.last(), Some(&last));
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");
    assert!(seqs.len() > (SLOTS - SECTOR_SIZE / SLOT_SIZE) as usize);
}

#[test]
fn a_half_written_slot_is_skipped() {
    let mut flash = erased();
    append(&mut flash, 3);
    // A power cut during the fourth write left part of a slot programmed.
    let torn = 3 * SLOT_SIZE as usize;
    flash[torn..torn + 5].copy_from_slice(&[0x02, 0x05, 0x03, 0x00, 0x00]);
    append(&mut flash, 2);
    assert_eq!(seqs(&mut flash), [0, 1, 2, 3, 4]);
    // The new entries went after the torn slot.
    assert_eq!(&flash[torn..torn + 5], &[0x02, 0x05, 0x03, 0x00, 0x00]);
}

#[test]
fn leftover_garbage_is_ignored() {
    let mut flash: Vec<u8> = (0..(SECTORS * SECTOR_SIZE) as usize)
        .map(|i| u8::try_from(i * 7 % 251).unwrap())
        .collect();
    assert_eq!(seqs(&mut flash), Vec::<u32>::new());
    append(&mut flash, 3);
    assert_eq!(seqs(&mut flash), [0, 1, 2]);
}
