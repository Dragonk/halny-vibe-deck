//! Halny Vibe Deck — desktop GUI for the Mistral Vibe CLI.
//!
//! Unofficial community project. Not affiliated with Mistral AI.
//! `halny-vibe-deck-core` holds the shared domain types (messages, sessions,
//! tool calls) and the event/command protocol used between the UI
//! layer and the agent worker layer.

pub mod proto;
pub mod snapshot;
pub mod types;
pub mod util;

pub use proto::{AgentCommand, AgentEvent};
pub use types::*;
