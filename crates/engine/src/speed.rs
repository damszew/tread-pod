//! Speed unit conversions between FTMS and RSC, and speed derived from total distance.

use heapless::Deque;

/// FTMS Treadmill Data instantaneous speed: 0.01 km/h per unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct FtmsSpeed(pub u16);

/// RSC Measurement instantaneous speed: 1/256 m/s per unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct RscSpeed(pub u16);

impl From<FtmsSpeed> for RscSpeed {
    /// Converts 0.01 km/h to 1/256 m/s, rounding **up**.
    ///
    /// 0.01 km/h = 1/360 m/s, so `rsc = ftms * 256 / 360`. Rounding up means the watch never shows a
    /// slower pace than the treadmill does.
    fn from(FtmsSpeed(centi_kmh): FtmsSpeed) -> Self {
        let rsc = (u32::from(centi_kmh) * 256).div_ceil(360);
        // u16::MAX * 256 / 360 < u16::MAX, so this never saturates.
        RscSpeed(u16::try_from(rsc).unwrap_or(u16::MAX))
    }
}

/// Window over which [`DistanceSpeed`] averages.
pub const DISTANCE_WINDOW_MS: u64 = 10_000;

/// Shortest span [`DistanceSpeed`] derives a speed from: distance has 1 m resolution, so shorter
/// spans are too coarse.
const MIN_SPAN_MS: u64 = 2_000;

/// Speed derived from total distance, for treadmills that don't report instantaneous speed.
///
/// Averages over the last [`DISTANCE_WINDOW_MS`]; at 1 m resolution that is 0.36 km/h steps.
#[derive(Debug, Default)]
pub struct DistanceSpeed {
    /// `(ms, metres)` samples, oldest first.
    samples: Deque<(u64, u32), 16>,
}

impl DistanceSpeed {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a total-distance sample and returns the speed over the window, once it spans at least
    /// two seconds. A distance going backwards (a new workout) restarts the window.
    pub fn push(&mut self, now_ms: u64, metres: u32) -> Option<FtmsSpeed> {
        if self.samples.back().is_some_and(|&(_, d)| d > metres) {
            self.samples.clear();
        }
        if self.samples.is_full() {
            self.samples.pop_front();
        }
        // Not full after the pop above.
        let _ = self.samples.push_back((now_ms, metres));
        while self
            .samples
            .front()
            .is_some_and(|&(t, _)| t + DISTANCE_WINDOW_MS < now_ms)
        {
            self.samples.pop_front();
        }
        let &(t0, d0) = self.samples.front()?;
        let span = now_ms - t0;
        if span < MIN_SPAN_MS {
            return None;
        }
        // metres per ms -> 0.01 km/h: * 1000 (per s) * 3.6 (km/h) * 100.
        let centi_kmh = u64::from(metres - d0) * 360_000 / span;
        Some(FtmsSpeed(u16::try_from(centi_kmh).unwrap_or(u16::MAX)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_speed_needs_two_seconds() {
        let mut s = DistanceSpeed::new();
        assert_eq!(s.push(0, 0), None);
        assert_eq!(s.push(1_000, 3), None);
        // 6 m in 2 s = 3 m/s = 10.8 km/h
        assert_eq!(s.push(2_000, 6), Some(FtmsSpeed(1080)));
    }

    #[test]
    fn distance_speed_averages_over_the_window() {
        let mut s = DistanceSpeed::new();
        // 2.5 m/s, sampled every second with 1 m resolution
        let mut last = None;
        for i in 0..30u32 {
            last = s.push(u64::from(i) * 1_000, i * 5 / 2);
        }
        // 9 km/h, within one metre per window (0.36 km/h)
        let FtmsSpeed(v) = last.unwrap();
        assert!((864..=936).contains(&v), "{v}");
    }

    #[test]
    fn distance_reset_restarts_the_window() {
        let mut s = DistanceSpeed::new();
        s.push(0, 100);
        s.push(2_000, 106);
        assert_eq!(s.push(3_000, 0), None);
        assert_eq!(s.push(5_000, 4), Some(FtmsSpeed(720)));
    }
    use proptest::prelude::*;

    #[test]
    fn zero_stays_zero() {
        assert_eq!(RscSpeed::from(FtmsSpeed(0)), RscSpeed(0));
    }

    #[test]
    fn exact_values_are_not_rounded() {
        // 3.6 km/h == 1 m/s == 256/256 m/s
        assert_eq!(RscSpeed::from(FtmsSpeed(360)), RscSpeed(256));
        // 18.0 km/h == 5 m/s
        assert_eq!(RscSpeed::from(FtmsSpeed(1800)), RscSpeed(1280));
    }

    #[test]
    fn inexact_values_round_up() {
        // 10.00 km/h == 2.7777… m/s == 711.11…/256 m/s
        assert_eq!(RscSpeed::from(FtmsSpeed(1000)), RscSpeed(712));
    }

    proptest! {
        #[test]
        fn never_slower_than_treadmill(centi_kmh: u16) {
            let RscSpeed(rsc) = RscSpeed::from(FtmsSpeed(centi_kmh));
            // rsc/256 m/s >= centi_kmh/360 m/s, and less than one RSC unit above it.
            let lhs = u64::from(rsc) * 360;
            let rhs = u64::from(centi_kmh) * 256;
            prop_assert!(lhs >= rhs);
            prop_assert!(lhs - rhs < 360);
        }
    }
}
