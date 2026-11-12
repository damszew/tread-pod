//! RSC Feature (`0x2A54`): a `u16` bit field the watch reads once to learn which optional
//! RSC Measurement fields to expect.
//!
//! Cadence has no bit: it is mandatory in every measurement. With cadence off the footpod sends
//! 0, and the watch falls back to its own wrist cadence.

/// Supported RSC features. Calibration (bit 3) and multiple sensor locations (bit 4) are never
/// offered: the footpod has nothing to calibrate and a single location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Feature {
    pub stride_length: bool,
    pub total_distance: bool,
    pub running_status: bool,
}

mod bit {
    pub const STRIDE_LENGTH: u16 = 1 << 0;
    pub const TOTAL_DISTANCE: u16 = 1 << 1;
    pub const RUNNING_STATUS: u16 = 1 << 2;
}

impl Feature {
    /// The characteristic value, little-endian.
    pub fn encode(self) -> [u8; 2] {
        let mut bits = 0;
        for (on, b) in [
            (self.stride_length, bit::STRIDE_LENGTH),
            (self.total_distance, bit::TOTAL_DISTANCE),
            (self.running_status, bit::RUNNING_STATUS),
        ] {
            if on {
                bits |= b;
            }
        }
        u16::to_le_bytes(bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_zero() {
        assert_eq!(Feature::default().encode(), [0, 0]);
    }

    #[test]
    fn each_bit() {
        let d = Feature::default();
        let stride = Feature {
            stride_length: true,
            ..d
        };
        let distance = Feature {
            total_distance: true,
            ..d
        };
        let status = Feature {
            running_status: true,
            ..d
        };
        assert_eq!(stride.encode(), [0x01, 0]);
        assert_eq!(distance.encode(), [0x02, 0]);
        assert_eq!(status.encode(), [0x04, 0]);
    }
}
