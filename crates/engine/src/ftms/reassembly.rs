//! Merges Treadmill Data notifications that were split with the "More Data" flag into records.
//!
//! A treadmill whose data doesn't fit one notification splits it; every part except the one
//! carrying Instantaneous Speed has "More Data" set. Treadmills differ in the order of the parts
//! (the FS-4C02A8 sends the speed part *first*), so a record is completed by:
//!
//! - the part carrying speed: everything pending plus this part is one record;
//! - a field arriving that is already pending: a new record has started, so the pending one is
//!   complete. This also keeps treadmills that never send speed (every part has "More Data")
//!   producing records, one per cycle of fields.
//!
//! With speed-first ordering, the trailing parts are merged into the *next* record. That makes
//! their fields up to one update old, which is fine for what the bridge needs (speed, distance).

use super::TreadmillData;

#[derive(Debug, Default)]
pub struct Reassembler {
    pending: TreadmillData,
}

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one parsed notification and returns the records it completes (zero, one or two).
    pub fn push(&mut self, part: TreadmillData) -> impl Iterator<Item = TreadmillData> {
        let flushed = if overlaps(&self.pending, &part) {
            Some(self.take())
        } else {
            None
        };
        merge(&mut self.pending, &part);
        let completed = part.speed.is_some().then(|| self.take());
        flushed.into_iter().chain(completed)
    }

    fn take(&mut self) -> TreadmillData {
        let mut record = core::mem::take(&mut self.pending);
        record.more_data = false;
        record
    }
}

/// Applies `$f!(field)` to every optional field of [`TreadmillData`].
macro_rules! for_each_field {
    ($f:ident) => {
        $f!(speed);
        $f!(average_speed);
        $f!(total_distance);
        $f!(incline);
        $f!(elevation_gain);
        $f!(pace);
        $f!(average_pace);
        $f!(energy);
        $f!(heart_rate);
        $f!(metabolic_equivalent);
        $f!(elapsed_time);
        $f!(remaining_time);
        $f!(force_and_power);
    };
}

fn overlaps(a: &TreadmillData, b: &TreadmillData) -> bool {
    macro_rules! check {
        ($field:ident) => {
            if a.$field.is_some() && b.$field.is_some() {
                return true;
            }
        };
    }
    for_each_field!(check);
    false
}

fn merge(into: &mut TreadmillData, from: &TreadmillData) {
    macro_rules! copy {
        ($field:ident) => {
            if from.$field.is_some() {
                into.$field = from.$field;
            }
        };
    }
    for_each_field!(copy);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speed::FtmsSpeed;

    fn speed(s: u16) -> TreadmillData {
        TreadmillData {
            speed: Some(FtmsSpeed(s)),
            ..TreadmillData::default()
        }
    }

    fn more(distance: Option<u32>, elapsed: Option<u16>) -> TreadmillData {
        TreadmillData {
            more_data: true,
            total_distance: distance,
            elapsed_time: elapsed,
            ..TreadmillData::default()
        }
    }

    fn push(r: &mut Reassembler, part: TreadmillData) -> std::vec::Vec<TreadmillData> {
        r.push(part).collect()
    }

    #[test]
    fn single_part_records_pass_through() {
        let mut r = Reassembler::new();
        assert_eq!(push(&mut r, speed(100)), [speed(100)]);
        assert_eq!(push(&mut r, speed(200)), [speed(200)]);
    }

    #[test]
    fn speed_last_merges_preceding_parts() {
        let mut r = Reassembler::new();
        assert_eq!(push(&mut r, more(Some(5), None)), []);
        assert_eq!(push(&mut r, more(None, Some(9))), []);
        assert_eq!(
            push(&mut r, speed(100)),
            [TreadmillData {
                total_distance: Some(5),
                elapsed_time: Some(9),
                ..speed(100)
            }]
        );
    }

    #[test]
    fn speed_first_carries_trailing_parts_into_next_record() {
        let mut r = Reassembler::new();
        assert_eq!(push(&mut r, speed(100)), [speed(100)]);
        assert_eq!(push(&mut r, more(Some(5), None)), []);
        assert_eq!(
            push(&mut r, speed(110)),
            [TreadmillData {
                total_distance: Some(5),
                ..speed(110)
            }]
        );
    }

    #[test]
    fn treadmills_without_speed_still_produce_records() {
        let mut r = Reassembler::new();
        assert_eq!(push(&mut r, more(Some(5), None)), []);
        assert_eq!(push(&mut r, more(None, Some(9))), []);
        // distance repeats: the previous record is complete
        assert_eq!(
            push(&mut r, more(Some(7), None)),
            [TreadmillData {
                total_distance: Some(5),
                elapsed_time: Some(9),
                ..TreadmillData::default()
            }]
        );
    }

    #[test]
    fn repeated_field_and_speed_complete_two_records() {
        let mut r = Reassembler::new();
        assert_eq!(push(&mut r, more(Some(5), None)), []);
        let part = TreadmillData {
            total_distance: Some(7),
            ..speed(100)
        };
        assert_eq!(
            push(&mut r, part),
            [
                TreadmillData {
                    total_distance: Some(5),
                    ..TreadmillData::default()
                },
                part
            ]
        );
    }
}
