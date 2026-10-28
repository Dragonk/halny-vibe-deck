//! Client transport for the `vibe-app-server` protocol (the engine's
//! only agent transport; replaces the legacy `mistraldeck-acp`).

pub mod conn;
pub mod patch;
pub mod proto;
pub mod reducer;
pub mod worker;

pub use worker::ApsHandle;
