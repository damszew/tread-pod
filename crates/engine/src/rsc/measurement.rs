//! RSC Measurement (`0x2A53`) encoder.
//!
//! Layout (little-endian):
//!
//! | field | type | unit | present |
//! |---|---|---|---|
//! | flags | `u8` | | always |
//! | instantaneous speed | `u16` | 1/256 m/s | always |
//! | instantaneous cadence | `u8` | strides/min (Garmin doubles it to steps) | always |
//! | instantaneous stride length | `u16` | 1/100 m | flag bit 0 |
//! | total distance | `u32` | 1/10 m | flag bit 1 |
//!
//! Flag bit 2 is the walking (0) / running (1) status.

use core::ops::Deref;

use crate::speed::RscSpeed;

/// One RSC Measurement notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Measurement {
    pub speed: RscSpeed,
    /// Strides (two steps) per minute; Garmin shows twice this as steps. 0 when cadence is off.
    pub cadence: u8,
    /// 1/100 m.
    pub stride_length: Option<u16>,
    /// 1/10 m.
    pub total_distance: Option<u32>,
    pub running: bool,
}

mod flag {
    pub const STRIDE_LENGTH: u8 = 1 << 0;
    pub const TOTAL_DISTANCE: u8 = 1 << 1;
    pub const RUNNING: u8 = 1 << 2;
}

impl Measurement {
    /// Flags + speed + cadence + stride length + total distance.
    pub const MAX_LEN: usize = 1 + 2 + 1 + 2 + 4;

    pub fn encode(&self) -> Encoded {
        let mut out = Encoded {
            buf: [0; Self::MAX_LEN],
            len: 0,
        };
        let mut flags = 0;
        if self.stride_length.is_some() {
            flags |= flag::STRIDE_LENGTH;
        }
        if self.total_distance.is_some() {
            flags |= flag::TOTAL_DISTANCE;
        }
        if self.running {
            flags |= flag::RUNNING;
        }
        out.push(&[flags]);
        out.push(&self.speed.0.to_le_bytes());
        out.push(&[self.cadence]);
        if let Some(stride) = self.stride_length {
            out.push(&stride.to_le_bytes());
        }
        if let Some(distance) = self.total_distance {
            out.push(&distance.to_le_bytes());
        }
        out
    }
}

/// An encoded measurement: a fixed buffer plus its length, so it can be stored in an action
/// without allocating. Derefs to the bytes to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoded {
    buf: [u8; Measurement::MAX_LEN],
    len: u8,
}

impl Encoded {
    fn push(&mut self, bytes: &[u8]) {
        let start = usize::from(self.len);
        self.buf[start..start + bytes.len()].copy_from_slice(bytes);
        // Never exceeds MAX_LEN (10).
        self.len += u8::try_from(bytes.len()).unwrap_or(u8::MAX);
    }
}

impl Deref for Encoded {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.buf[..usize::from(self.len)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Test-only decoder, written from the layout table independently of `encode`.
    fn decode(b: &[u8]) -> Measurement {
        let flags = b[0];
        let mut rest = &b[4..];
        let mut take = |n: usize| {
            let (head, tail) = rest.split_at(n);
            rest = tail;
            head
        };
        let stride_length =
            (flags & 1 != 0).then(|| u16::from_le_bytes(take(2).try_into().unwrap()));
        let total_distance =
            (flags & 2 != 0).then(|| u32::from_le_bytes(take(4).try_into().unwrap()));
        assert!(rest.is_empty(), "trailing bytes");
        Measurement {
            speed: RscSpeed(u16::from_le_bytes([b[1], b[2]])),
            cadence: b[3],
            stride_length,
            total_distance,
            running: flags & 4 != 0,
        }
    }

    #[test]
    fn minimal() {
        let m = Measurement {
            speed: RscSpeed(712), // 10 km/h
            ..Measurement::default()
        };
        assert_eq!(&*m.encode(), [0x00, 0xc8, 0x02, 0x00]);
    }

    #[test]
    fn every_field() {
        let m = Measurement {
            speed: RscSpeed(0x0102),
            cadence: 170,
            stride_length: Some(0x0304),
            total_distance: Some(0x0506_0708),
            running: true,
        };
        assert_eq!(
            &*m.encode(),
            [0x07, 0x02, 0x01, 170, 0x04, 0x03, 0x08, 0x07, 0x06, 0x05]
        );
    }

    #[test]
    fn distance_without_stride() {
        let m = Measurement {
            total_distance: Some(1240),
            ..Measurement::default()
        };
        assert_eq!(&*m.encode(), [0x02, 0, 0, 0, 0xd8, 0x04, 0, 0]);
    }

    proptest! {
        #[test]
        fn round_trips(
            speed: u16,
            cadence: u8,
            stride_length: Option<u16>,
            total_distance: Option<u32>,
            running: bool,
        ) {
            let m = Measurement {
                speed: RscSpeed(speed),
                cadence,
                stride_length,
                total_distance,
                running,
            };
            let encoded = m.encode();
            prop_assert!(encoded.len() <= Measurement::MAX_LEN);
            prop_assert_eq!(decode(&encoded), m);
        }
    }
}
