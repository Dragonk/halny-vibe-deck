//! Mistral Deck ACP client — spawns and drives the `vibe-acp` CLI
//! (Mistral Vibe Agent Client Protocol server) over JSON-RPC 2.0
//! on stdin/stdout, without blocking the UI thread.

mod client;
mod convert;

pub use client::{AcpHandle, CLIENT_INFO_NAME};
