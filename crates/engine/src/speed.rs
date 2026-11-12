//! Speed unit conversions between FTMS and RSC.

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

#[cfg(test)]
mod tests {
    use super::*;
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
