//! Client transport for the `vibe-app-server` protocol (the engine's
//! only agent transport; the legacy ACP transport was removed.

pub mod conn;
pub mod patch;
pub mod proto;
pub mod reducer;
pub mod worker;

pub use worker::ApsHandle;
