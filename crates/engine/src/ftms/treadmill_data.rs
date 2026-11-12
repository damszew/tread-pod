//! Treadmill Data characteristic (`0x2ACD`), per the FTMS specification.
//!
//! A notification is a little-endian `u16` flags field followed by the fields the flags select,
//! in flag-bit order. Bit 0 ("More Data") is inverted: when it is **clear**, Instantaneous Speed
//! is present. Bits 13–15 are reserved; since fields appear in bit order, any data for them comes
//! after every field we know, so it is ignored instead of rejected.

use crate::speed::FtmsSpeed;

/// One Treadmill Data notification. Fields absent from the packet are `None`.
///
/// Units follow the spec; the field docs give the resolution of the raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TreadmillData {
    /// The "More Data" flag: more notifications follow for this record. Set exactly when
    /// `speed` is `None`.
    pub more_data: bool,
    /// Instantaneous speed, 0.01 km/h.
    pub speed: Option<FtmsSpeed>,
    /// Average speed, 0.01 km/h.
    pub average_speed: Option<FtmsSpeed>,
    /// Total distance, metres (`u24` on the wire).
    pub total_distance: Option<u32>,
    pub incline: Option<Incline>,
    pub elevation_gain: Option<ElevationGain>,
    /// Instantaneous pace, 0.1 km/min.
    pub pace: Option<u8>,
    /// Average pace, 0.1 km/min.
    pub average_pace: Option<u8>,
    pub energy: Option<Energy>,
    /// Heart rate, beats per minute.
    pub heart_rate: Option<u8>,
    /// Metabolic equivalent, 0.1 MET.
    pub metabolic_equivalent: Option<u8>,
    /// Elapsed time, seconds.
    pub elapsed_time: Option<u16>,
    /// Remaining time, seconds.
    pub remaining_time: Option<u16>,
    pub force_and_power: Option<ForceAndPower>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Incline {
    /// Inclination, 0.1 %.
    pub inclination: i16,
    /// Ramp angle, 0.1 degree.
    pub ramp_angle: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElevationGain {
    /// Positive elevation gain, 0.1 m.
    pub positive: u16,
    /// Negative elevation gain, 0.1 m.
    pub negative: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Energy {
    /// Total expended energy, kcal.
    pub total: u16,
    /// Energy per hour, kcal. `None` when the treadmill sends "not available" (`0xFFFF`).
    pub per_hour: Option<u16>,
    /// Energy per minute, kcal. `None` when the treadmill sends "not available" (`0xFF`).
    pub per_minute: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForceAndPower {
    /// Force on belt, newtons.
    pub force_on_belt: i16,
    /// Power output, watts.
    pub power: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    /// The packet ended before all fields selected by its flags.
    Truncated,
}

/// Flag bits, in wire order.
pub(crate) mod flag {
    pub const MORE_DATA: u16 = 1 << 0;
    pub const AVERAGE_SPEED: u16 = 1 << 1;
    pub const TOTAL_DISTANCE: u16 = 1 << 2;
    pub const INCLINE: u16 = 1 << 3;
    pub const ELEVATION_GAIN: u16 = 1 << 4;
    pub const PACE: u16 = 1 << 5;
    pub const AVERAGE_PACE: u16 = 1 << 6;
    pub const ENERGY: u16 = 1 << 7;
    pub const HEART_RATE: u16 = 1 << 8;
    pub const METABOLIC_EQUIVALENT: u16 = 1 << 9;
    pub const ELAPSED_TIME: u16 = 1 << 10;
    pub const REMAINING_TIME: u16 = 1 << 11;
    pub const FORCE_AND_POWER: u16 = 1 << 12;
}

impl TreadmillData {
    /// Parses one notification.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] if the packet is shorter than its flags require.
    pub fn parse(packet: &[u8]) -> Result<Self, ParseError> {
        let mut r = Reader(packet);
        let flags = r.u16()?;
        let has = |bit: u16| flags & bit != 0;

        let more_data = has(flag::MORE_DATA);
        Ok(Self {
            more_data,
            speed: (!more_data).then(|| r.u16().map(FtmsSpeed)).transpose()?,
            average_speed: r.field(has(flag::AVERAGE_SPEED), |r| r.u16().map(FtmsSpeed))?,
            total_distance: r.field(has(flag::TOTAL_DISTANCE), Reader::u24)?,
            incline: r.field(has(flag::INCLINE), |r| {
                Ok(Incline {
                    inclination: r.i16()?,
                    ramp_angle: r.i16()?,
                })
            })?,
            elevation_gain: r.field(has(flag::ELEVATION_GAIN), |r| {
                Ok(ElevationGain {
                    positive: r.u16()?,
                    negative: r.u16()?,
                })
            })?,
            pace: r.field(has(flag::PACE), Reader::u8)?,
            average_pace: r.field(has(flag::AVERAGE_PACE), Reader::u8)?,
            energy: r.field(has(flag::ENERGY), |r| {
                Ok(Energy {
                    total: r.u16()?,
                    per_hour: Some(r.u16()?).filter(|&v| v != u16::MAX),
                    per_minute: Some(r.u8()?).filter(|&v| v != u8::MAX),
                })
            })?,
            heart_rate: r.field(has(flag::HEART_RATE), Reader::u8)?,
            metabolic_equivalent: r.field(has(flag::METABOLIC_EQUIVALENT), Reader::u8)?,
            elapsed_time: r.field(has(flag::ELAPSED_TIME), Reader::u16)?,
            remaining_time: r.field(has(flag::REMAINING_TIME), Reader::u16)?,
            force_and_power: r.field(has(flag::FORCE_AND_POWER), |r| {
                Ok(ForceAndPower {
                    force_on_belt: r.i16()?,
                    power: r.i16()?,
                })
            })?,
        })
    }
}

/// Little-endian cursor over a packet.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        let (head, rest) = self.0.split_first_chunk().ok_or(ParseError::Truncated)?;
        self.0 = rest;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8, ParseError> {
        self.take::<1>().map(|[b]| b)
    }

    fn u16(&mut self) -> Result<u16, ParseError> {
        self.take().map(u16::from_le_bytes)
    }

    fn i16(&mut self) -> Result<i16, ParseError> {
        self.take().map(i16::from_le_bytes)
    }

    fn u24(&mut self) -> Result<u32, ParseError> {
        self.take::<3>()
            .map(|[a, b, c]| u32::from_le_bytes([a, b, c, 0]))
    }

    /// Reads a field with `read` if `present`.
    fn field<T>(
        &mut self,
        present: bool,
        read: impl FnOnce(&mut Self) -> Result<T, ParseError>,
    ) -> Result<Option<T>, ParseError> {
        present.then(|| read(self)).transpose()
    }
}

/// Test-only encoder: the inverse of [`TreadmillData::parse`], used to property-test every
/// flag combination.
#[cfg(test)]
pub(crate) fn encode(data: &TreadmillData) -> std::vec::Vec<u8> {
    let mut flags = 0u16;
    let mut body = std::vec::Vec::new();
    let mut put = |bit: u16, present: bool, bytes: &[u8]| {
        if present {
            flags |= bit;
            body.extend_from_slice(bytes);
        }
    };

    match data.speed {
        Some(FtmsSpeed(s)) => put(0, true, &s.to_le_bytes()),
        None => put(flag::MORE_DATA, true, &[]),
    }
    let avg = data.average_speed.map_or(0, |FtmsSpeed(s)| s);
    put(
        flag::AVERAGE_SPEED,
        data.average_speed.is_some(),
        &avg.to_le_bytes(),
    );
    let dist = data.total_distance.unwrap_or(0).to_le_bytes();
    put(
        flag::TOTAL_DISTANCE,
        data.total_distance.is_some(),
        &dist[..3],
    );
    if let Some(i) = data.incline {
        put(
            flag::INCLINE,
            true,
            &[i.inclination.to_le_bytes(), i.ramp_angle.to_le_bytes()].concat(),
        );
    }
    if let Some(e) = data.elevation_gain {
        put(
            flag::ELEVATION_GAIN,
            true,
            &[e.positive.to_le_bytes(), e.negative.to_le_bytes()].concat(),
        );
    }
    put(flag::PACE, data.pace.is_some(), &[data.pace.unwrap_or(0)]);
    put(
        flag::AVERAGE_PACE,
        data.average_pace.is_some(),
        &[data.average_pace.unwrap_or(0)],
    );
    if let Some(e) = data.energy {
        let mut bytes = std::vec::Vec::from(e.total.to_le_bytes());
        bytes.extend_from_slice(&e.per_hour.unwrap_or(u16::MAX).to_le_bytes());
        bytes.push(e.per_minute.unwrap_or(u8::MAX));
        put(flag::ENERGY, true, &bytes);
    }
    put(
        flag::HEART_RATE,
        data.heart_rate.is_some(),
        &[data.heart_rate.unwrap_or(0)],
    );
    let met = data.metabolic_equivalent;
    put(
        flag::METABOLIC_EQUIVALENT,
        met.is_some(),
        &[met.unwrap_or(0)],
    );
    let elapsed = data.elapsed_time.unwrap_or(0).to_le_bytes();
    put(flag::ELAPSED_TIME, data.elapsed_time.is_some(), &elapsed);
    let remaining = data.remaining_time.unwrap_or(0).to_le_bytes();
    put(
        flag::REMAINING_TIME,
        data.remaining_time.is_some(),
        &remaining,
    );
    if let Some(f) = data.force_and_power {
        put(
            flag::FORCE_AND_POWER,
            true,
            &[f.force_on_belt.to_le_bytes(), f.power.to_le_bytes()].concat(),
        );
    }

    let mut packet = std::vec::Vec::from(flags.to_le_bytes());
    packet.extend_from_slice(&body);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn speed_only() {
        // flags 0x0000, speed 10.50 km/h
        let data = TreadmillData::parse(&[0x00, 0x00, 0x1a, 0x04]).unwrap();
        assert_eq!(
            data,
            TreadmillData {
                speed: Some(FtmsSpeed(1050)),
                ..TreadmillData::default()
            }
        );
    }

    #[test]
    fn more_data_means_no_speed() {
        let data = TreadmillData::parse(&[0x01, 0x00]).unwrap();
        assert!(data.more_data);
        assert_eq!(data.speed, None);
    }

    #[test]
    fn energy_not_available_sentinels() {
        // More Data + Expended Energy: total 12 kcal, per hour/minute "not available".
        let data = TreadmillData::parse(&[0x81, 0x00, 0x0c, 0x00, 0xff, 0xff, 0xff]).unwrap();
        assert_eq!(
            data.energy,
            Some(Energy {
                total: 12,
                per_hour: None,
                per_minute: None
            })
        );
    }

    #[test]
    fn reserved_bits_and_trailing_bytes_are_ignored() {
        // FS-4C02A8 sends this after every record: More Data + reserved bit 13 + 3 zero bytes.
        let data = TreadmillData::parse(&[0x01, 0x20, 0x00, 0x00, 0x00]).unwrap();
        assert_eq!(
            data,
            TreadmillData {
                more_data: true,
                ..TreadmillData::default()
            }
        );
    }

    #[test]
    fn truncated_packets_are_rejected() {
        assert_eq!(TreadmillData::parse(&[]), Err(ParseError::Truncated));
        assert_eq!(TreadmillData::parse(&[0x00]), Err(ParseError::Truncated));
        // speed present but only one byte of it
        assert_eq!(
            TreadmillData::parse(&[0x00, 0x00, 0x1a]),
            Err(ParseError::Truncated)
        );
        // total distance flagged but missing its third byte
        assert_eq!(
            TreadmillData::parse(&[0x05, 0x00, 0x01, 0x02]),
            Err(ParseError::Truncated)
        );
    }

    fn treadmill_data() -> impl Strategy<Value = TreadmillData> {
        let speeds = (
            proptest::option::of(any::<u16>().prop_map(FtmsSpeed)),
            proptest::option::of(any::<u16>().prop_map(FtmsSpeed)),
            proptest::option::of(0u32..1 << 24),
        );
        let shapes = (
            proptest::option::of((any::<i16>(), any::<i16>()).prop_map(
                |(inclination, ramp_angle)| Incline {
                    inclination,
                    ramp_angle,
                },
            )),
            proptest::option::of(
                (any::<u16>(), any::<u16>())
                    .prop_map(|(positive, negative)| ElevationGain { positive, negative }),
            ),
            proptest::option::of(any::<u8>()),
            proptest::option::of(any::<u8>()),
            proptest::option::of(
                (
                    any::<u16>(),
                    proptest::option::of(0..u16::MAX),
                    proptest::option::of(0..u8::MAX),
                )
                    .prop_map(|(total, per_hour, per_minute)| Energy {
                        total,
                        per_hour,
                        per_minute,
                    }),
            ),
        );
        let rest = (
            proptest::option::of(any::<u8>()),
            proptest::option::of(any::<u8>()),
            proptest::option::of(any::<u16>()),
            proptest::option::of(any::<u16>()),
            proptest::option::of((any::<i16>(), any::<i16>()).prop_map(
                |(force_on_belt, power)| ForceAndPower {
                    force_on_belt,
                    power,
                },
            )),
        );
        (speeds, shapes, rest).prop_map(
            |(
                (speed, average_speed, total_distance),
                (incline, elevation_gain, pace, average_pace, energy),
                (heart_rate, metabolic_equivalent, elapsed_time, remaining_time, force_and_power),
            )| TreadmillData {
                more_data: speed.is_none(),
                speed,
                average_speed,
                total_distance,
                incline,
                elevation_gain,
                pace,
                average_pace,
                energy,
                heart_rate,
                metabolic_equivalent,
                elapsed_time,
                remaining_time,
                force_and_power,
            },
        )
    }

    proptest! {
        #[test]
        fn every_flag_combination_round_trips(data in treadmill_data()) {
            prop_assert_eq!(TreadmillData::parse(&encode(&data)), Ok(data));
        }

        #[test]
        fn any_proper_prefix_is_truncated(data in treadmill_data()) {
            let packet = encode(&data);
            for len in 0..packet.len() {
                prop_assert_eq!(TreadmillData::parse(&packet[..len]), Err(ParseError::Truncated));
            }
        }

        #[test]
        fn never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = TreadmillData::parse(&bytes);
        }
    }
}
