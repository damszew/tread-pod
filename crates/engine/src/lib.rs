//! Sans-IO core of tread-pod.
//!
//! No BLE, no async, no clock, no allocation. Backends feed timestamped events in and carry out
//! the actions that come back, which keeps this crate independent of any executor or BLE stack.
//! See `docs/architecture.md`.

#![cfg_attr(not(test), no_std)]

pub mod config;
pub mod speed;
