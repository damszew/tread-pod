//! Fitness Machine Service (FTMS, Bluetooth SIG service `0x1826`), treadmill side.

pub mod reassembly;
pub mod treadmill_data;

pub use reassembly::Reassembler;
pub use treadmill_data::{ParseError, TreadmillData};
