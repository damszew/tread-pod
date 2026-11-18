//! Persistent bridge configuration, stored on the device as postcard bytes.

use serde::{Deserialize, Serialize};

/// A Bluetooth device address, most significant byte first.
pub type Address = [u8; 6];

/// Bridge configuration persisted across reboots.
///
/// `version` must be bumped on any breaking layout change. A blob that fails to decode, or has an
/// unknown version, falls back to [`Config::default`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub version: u8,
    /// Always use this treadmill instead of auto-picking the nearest one.
    pub pinned: Option<Address>,
    /// Send cadence and stride length estimated from speed (see [`crate::cadence`]). On by
    /// default: with it off the footpod sends 0, and Garmin records 0 spm rather than wrist cadence.
    pub cadence: bool,
}

impl Config {
    pub const VERSION: u8 = 1;

    /// Upper bound on the encoded size, for sizing storage buffers.
    pub const MAX_ENCODED_LEN: usize = 16;

    /// Encodes into `buf`, returning the used prefix.
    ///
    /// # Errors
    /// Fails if `buf` is too small; [`Self::MAX_ENCODED_LEN`] bytes are always enough.
    pub fn encode<'a>(&self, buf: &'a mut [u8]) -> Result<&'a mut [u8], postcard::Error> {
        postcard::to_slice(self, buf)
    }

    /// Decodes a stored blob, falling back to defaults if it is corrupt or from another version.
    pub fn decode_or_default(bytes: &[u8]) -> Self {
        postcard::from_bytes::<Self>(bytes)
            .ok()
            .filter(|c| c.version == Self::VERSION)
            .unwrap_or_default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            pinned: None,
            cadence: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let config = Config {
            pinned: Some([0xc0, 0xff, 0xee, 0x00, 0x11, 0x22]),
            cadence: true,
            ..Config::default()
        };
        let mut buf = [0; Config::MAX_ENCODED_LEN];
        let bytes = config.encode(&mut buf).unwrap();
        assert_eq!(Config::decode_or_default(bytes), config);
    }

    #[test]
    fn garbage_and_other_versions_fall_back_to_default() {
        assert_eq!(Config::decode_or_default(&[]), Config::default());
        assert_eq!(Config::decode_or_default(&[0xff; 4]), Config::default());

        let future = Config {
            version: Config::VERSION + 1,
            ..Config::default()
        };
        let mut buf = [0; Config::MAX_ENCODED_LEN];
        let bytes = future.encode(&mut buf).unwrap();
        assert_eq!(Config::decode_or_default(bytes), Config::default());
    }
}
