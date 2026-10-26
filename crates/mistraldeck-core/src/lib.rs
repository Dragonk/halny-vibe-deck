//! Mistral Deck — desktop GUI for the Mistral Vibe CLI.
//!
//! Unofficial community project. Not affiliated with Mistral AI.
//! `mistraldeck-core` holds the shared domain types (messages, sessions,
//! tool calls) and the event/command protocol used between the UI
//! layer and the ACP worker layer.

pub mod proto;
pub mod snapshot;
pub mod types;

pub use proto::{AcpCommand, AcpEvent};
pub use types::*;
