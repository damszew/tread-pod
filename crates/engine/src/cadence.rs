//! Cadence and stride length estimated from belt speed.
//!
//! RSC cadence is mandatory, and Garmin records a footpod's 0 as 0 spm instead of falling back to
//! wrist cadence, which also zeroes running power and running dynamics. Until cadence is measured,
//! one generic curve stands in for everyone. It aims at the middle of adult heights (about
//! 1.5–1.9 m), so a tall runner reads a few steps per minute high and a short one a few low.
//!
//! - **Walking** (up to [`WALK_MAX`]): `64 + 11 × km/h`, about 108 spm at 4 km/h and 130 at 6 km/h.
//! - **Running** (from [`RUN_MIN`]): `162 + 2.1 × (km/h − 10.8)`, about 157 spm at 8.5 km/h and
//!   177 at 18 km/h. Faster running comes mostly from longer steps, so cadence rises slowly.
//! - **In between**, the two blend linearly, so a speed hovering around the switch doesn't make
//!   cadence jump.
//!
//! **RSC carries strides, not steps.** A stride is one left-plus-right cycle, two steps. Garmin
//! reads the cadence field as strides per minute and doubles it for display: sending steps showed
//! exactly twice the estimate in two test runs (2026-11-20). So the curve works in steps, and
//! [`Estimate`] also gives the halved value and the length of a full stride for the RSC fields.
//!
//! Garmin seems to ignore the stride length field and shows speed ÷ its own cadence as stride
//! length; it is sent anyway, consistent with the cadence, for receivers that read it.

use crate::speed::FtmsSpeed;

/// Below this the belt is starting or stopping, and no estimate is sent.
pub const MIN_SPEED: FtmsSpeed = FtmsSpeed(100);
/// Pure walking up to here.
pub const WALK_MAX: FtmsSpeed = FtmsSpeed(650);
/// Pure running from here.
pub const RUN_MIN: FtmsSpeed = FtmsSpeed(750);

/// Running cadence limits, in spm.
const RUN_CADENCE: (i32, i32) = (150, 200);

/// An estimate for one speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    /// Steps per minute, both feet: what the watch displays.
    pub steps_per_minute: u8,
    /// Strides (two steps) per minute, rounded: the RSC cadence field.
    pub strides_per_minute: u8,
    /// One stride (two steps), in 1/100 m: speed ÷ [`Self::strides_per_minute`], so the two RSC
    /// fields agree.
    pub stride_length: u16,
}

/// Cadence and stride length for a belt speed, or `None` below [`MIN_SPEED`].
pub fn estimate(speed: FtmsSpeed) -> Option<Estimate> {
    if speed < MIN_SPEED {
        return None;
    }
    let v = i32::from(speed.0);
    // Tenths of a step per minute.
    let walk = 640 + 11 * v / 10;
    let run = (1620 + 21 * (v - 1080) / 100).clamp(RUN_CADENCE.0 * 10, RUN_CADENCE.1 * 10);
    let (lo, hi) = (i32::from(WALK_MAX.0), i32::from(RUN_MIN.0));
    let tenths = if v <= lo {
        walk
    } else if v >= hi {
        run
    } else {
        (walk * (hi - v) + run * (v - lo)) / (hi - lo)
    };
    let steps = (tenths + 5) / 10;
    let strides = (steps + 1) / 2;
    // speed / strides: (v / 360 m/s) / (strides / 60 s) * 100 cm = v * 50 / (3 * strides), rounded.
    let stride = (v * 50 + 3 * strides / 2) / (3 * strides);
    Some(Estimate {
        // Walking peaks at WALK_MAX (136 spm) and running is clamped to 200, so both fit.
        steps_per_minute: u8::try_from(steps).unwrap_or(u8::MAX),
        strides_per_minute: u8::try_from(strides).unwrap_or(u8::MAX),
        // At most 65535 * 50 / (3 * 100) < 11000 cm.
        stride_length: u16::try_from(stride).unwrap_or(u16::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn at(centi_kmh: u16) -> Estimate {
        estimate(FtmsSpeed(centi_kmh)).unwrap()
    }

    #[test]
    fn nothing_while_the_belt_starts_or_stops() {
        assert_eq!(estimate(FtmsSpeed(0)), None);
        assert_eq!(estimate(FtmsSpeed(99)), None);
        assert!(estimate(FtmsSpeed(100)).is_some());
    }

    #[test]
    fn reference_points() {
        // Walking at 4 km/h: 108 spm, 54 strides of 1.23 m (62 cm steps).
        assert_eq!(
            at(400),
            Estimate {
                steps_per_minute: 108,
                strides_per_minute: 54,
                stride_length: 123
            }
        );
        // The first test run, 8.5 km/h: 157 spm, sent as 79 strides of 1.79 m.
        assert_eq!(
            at(850),
            Estimate {
                steps_per_minute: 157,
                strides_per_minute: 79,
                stride_length: 179
            }
        );
        // 3 m/s, the anchor of the running curve.
        assert_eq!(at(1080).steps_per_minute, 162);
    }

    #[test]
    fn running_cadence_is_clamped() {
        assert_eq!(at(750).steps_per_minute, 155);
        assert_eq!(at(3000).steps_per_minute, 200);
        assert_eq!(at(u16::MAX).steps_per_minute, 200);
        assert_eq!(at(u16::MAX).strides_per_minute, 100);
    }

    proptest! {
        #[test]
        fn cadence_never_drops_as_speed_rises(a in 100u16..4000, b in 100u16..4000) {
            let (slow, fast) = (a.min(b), a.max(b));
            prop_assert!(at(slow).steps_per_minute <= at(fast).steps_per_minute);
            prop_assert!(at(slow).strides_per_minute <= at(fast).strides_per_minute);
        }

        #[test]
        fn strides_are_half_the_steps(v in 100u16..=u16::MAX) {
            let e = at(v);
            prop_assert_eq!(e.strides_per_minute, e.steps_per_minute.div_ceil(2));
        }

        #[test]
        fn stride_times_cadence_is_the_speed(v in 100u16..=u16::MAX) {
            let e = at(v);
            // cm per minute from the RSC fields, against the belt's: within half a cm per stride.
            let strides = u64::from(e.strides_per_minute);
            let estimated = u64::from(e.stride_length) * strides;
            let belt = u64::from(v) * 100_000 / 60 / 100; // 0.01 km/h -> cm/min
            prop_assert!(estimated.abs_diff(belt) <= strides / 2 + 1, "{estimated} vs {belt}");
        }
    }
}
