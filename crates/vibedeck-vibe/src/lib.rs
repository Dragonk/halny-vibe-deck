//! VibeDeck — desktop GUI for the Mistral Vibe CLI.
//!
//! `vibedeck-vibe` reads and writes the Mistral Vibe CLI user
//! configuration layer: `~/.vibe/config.toml` (latest schema,
//! round-trip safe), `agents/`, `prompts/`, `skills/` and the
//! session logs used for statistics.

pub mod config;
pub mod files;
pub mod stats;

pub use config::{estimate_cost, model_prices, McpServer, ModelConfig, ModelPrice, Provider, VibeConfigFile};
pub use files::{AgentFile, PromptFile, SkillFile};
pub use stats::{
    load_full_session_preview, load_session_last_config, load_session_preview, AggregateStats,
    PreviewMessage, PreviewPlanEntry, PreviewToolCall, PreviewUsage, SessionLastConfig,
    SessionPreviewData, VibeMessage, VibeSession, VibeSessionStats,
};

use std::path::PathBuf;

/// Resolve the Vibe home directory (`VIBE_HOME` or `~/.vibe`).
pub fn vibe_home() -> PathBuf {
    if let Some(custom) = std::env::var_os("VIBE_HOME") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".vibe")
}

/// Find the closest project-level `.vibe/config.toml` walking up from
/// `cwd`, without going past the home directory. Project settings
/// override user settings in the CLI — the editor surfaces them read-only.
pub fn find_project_config(cwd: &std::path::Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let mut dir = Some(cwd.to_path_buf());
    while let Some(d) = dir {
        let candidate = d.join(".vibe").join("config.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if d == home {
            return None;
        }
        dir = d.parent().map(|p| p.to_path_buf());
    }
    None
}
