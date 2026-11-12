//! Running Speed and Cadence (RSC, Bluetooth SIG service `0x1814`), the footpod the watch sees.

pub mod feature;
pub mod measurement;

pub use feature::Feature;
pub use measurement::{Encoded, Measurement};
