//! Client transport for the `vibe-app-server` protocol — the
//! drop-in alternative to `mistraldeck-acp` (docs/migration-appserver.md).

pub mod conn;
pub mod patch;
pub mod proto;
pub mod reducer;
pub mod worker;

pub use worker::ApsHandle;
