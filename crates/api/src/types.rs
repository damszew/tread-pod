use serde::{Deserialize, Serialize};

pub use engine::config::Address;

/// Max number of treadmills reported by `GET /api/treadmills`.
pub const MAX_TREADMILLS: usize = 16;

/// Max length of a treadmill's advertised name, in bytes.
pub const MAX_NAME_LEN: usize = 32;

/// A treadmill seen while scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Treadmill {
    pub address: Address,
    pub name: heapless::String<MAX_NAME_LEN>,
    pub rssi: i8,
    /// Last instantaneous speed in 0.01 km/h, if the bridge has seen any data from it.
    pub speed: Option<u16>,
    /// Whether this is the treadmill the bridge currently forwards.
    pub selected: bool,
}

/// Response of `GET /api/treadmills`, nearest first.
pub type Treadmills = heapless::Vec<Treadmill, MAX_TREADMILLS>;

/// Body of `POST /api/select`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Select {
    pub address: Address,
}

/// Body and response of `GET|PUT /api/settings`: the user-editable part of
/// [`engine::config::Config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub pinned: Option<Address>,
    pub cadence: bool,
}

impl From<&engine::config::Config> for Settings {
    fn from(config: &engine::config::Config) -> Self {
        Self {
            pinned: config.pinned,
            cadence: config.cadence,
        }
    }
}

impl Settings {
    /// Applies these settings on top of `config`, keeping fields the API doesn't expose.
    pub fn apply_to(self, config: &mut engine::config::Config) {
        config.pinned = self.pinned;
        config.cadence = self.cadence;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn settings_round_trip_through_config() {
        let mut config = engine::config::Config::default();
        let settings = Settings {
            pinned: Some([1, 2, 3, 4, 5, 6]),
            cadence: true,
        };
        settings.apply_to(&mut config);
        assert_eq!(Settings::from(&config), settings);
    }

    #[test]
    fn treadmill_json_shape() {
        let treadmill = Treadmill {
            address: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            name: "Gym Treadmill 3".try_into().unwrap(),
            rssi: -61,
            speed: Some(1050),
            selected: true,
        };
        let json = serde_json::to_string(&treadmill).unwrap();
        assert_eq!(
            json,
            r#"{"address":[170,187,204,221,238,255],"name":"Gym Treadmill 3","rssi":-61,"speed":1050,"selected":true}"#
        );
        assert_eq!(serde_json::from_str::<Treadmill>(&json).unwrap(), treadmill);
    }
}
