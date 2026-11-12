//! tread-pod config API.
//!
//! The types are always available (`no_std`, serde) and are shared by the firmware and the web UI,
//! so the two can't disagree about message formats. The picoserve router is behind the `server`
//! feature so it stays out of the WASM build.
//!
//! All routes live under `/api`; everything else is static UI assets served by the firmware.

#![no_std]

#[cfg(feature = "server")]
mod router;
mod types;

#[cfg(feature = "server")]
pub use router::{AppHandle, router};
pub use types::*;
