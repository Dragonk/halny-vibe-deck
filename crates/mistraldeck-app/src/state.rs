//! UI-local application state (settings persistence, view enum,
//! pending agent interactions).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// App settings persisted in the platform config dir
/// (`~/.config/mistraldeck/config.json` on Linux).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub language: String, // "system" | "en" | "pl"
    pub theme: String,    // "system" | "dark" | "light"
    pub acp_path: String,
    pub working_directory: String,
    pub recent_folders: Vec<String>,
    pub last_session_id: Option<String>,
    pub preferred_model: Option<String>,
    pub sidebar_sessions_limit: Option<usize>,
    /// Sidebar collapsed to the icon rail (Nexus-like toggle).
    pub sidebar_collapsed: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "system".into(),
            theme: "system".into(),
            acp_path: String::new(),
            working_directory: String::new(),
            recent_folders: Vec::new(),
            last_session_id: None,
            preferred_model: None,
            sidebar_sessions_limit: Some(30),
            sidebar_collapsed: false,
        }
    }
}

impl AppSettings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("mistraldeck").join("config.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(path, json);
            }
        }
    }

    pub fn remember_folder(&mut self, folder: &str) {
        if folder.is_empty() {
            return;
        }
        self.recent_folders.retain(|f| f != folder);
        self.recent_folders.insert(0, folder.to_string());
        self.recent_folders.truncate(8);
    }
}

/// Top-level view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Chat,
    Settings,
    Stats,
    Setup,
}

/// ACP connection status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcpStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// A tool approval waiting for the user.
#[derive(Clone, Debug)]
pub struct PendingApproval {
    pub request_id: String,
    pub method: String,
    pub params_json: String,
    pub tool_name: Option<String>,
}

/// A question from the agent waiting for the user.
#[derive(Clone, Debug)]
pub struct PendingQuestion {
    pub request_id: String,
    pub question: String,
    pub choices: Vec<String>,
    pub answer: String,
}

/// State of the in-app folder picker.
#[derive(Clone, Debug, Default)]
pub struct FolderPicker {
    pub open: bool,
    /// Directory currently listed.
    pub dir: PathBuf,
    pub entry: String, // chosen path being built
}

/// @-mention autocomplete state.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct MentionState {
    /// The query after the last `@` (file or command).
    pub query: String,
    pub kind: MentionKind,
    pub matches: Vec<(String, bool)>, // (relative path, is_dir)
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MentionKind {
    File,
    Command,
}

/// Live usage counters for the active session, fed by `usage_update`
/// notifications (prompt / completion / cached tokens, tok/s, context).
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageStats {
    pub prompt_tokens: f64,
    pub completion_tokens: f64,
    pub cached_tokens: f64,
    pub total_tokens: f64,
    pub tokens_per_second: f64,
    pub last_turn_duration_ms: f64,
    pub context_used: f64,
    pub context_size: f64,
    /// Number of `usage_update` notifications seen (for "live" badge).
    pub updates: u32,
}

impl UsageStats {
    /// Cache hit rate: cached tokens / prompt tokens (0 when unknown).
    pub fn cache_hit_rate(&self) -> f64 {
        if self.prompt_tokens > 0.0 {
            (self.cached_tokens / self.prompt_tokens).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn apply(&mut self, u: &mistraldeck_core::proto::AcpEvent) -> bool {
        if let mistraldeck_core::proto::AcpEvent::UsageUpdate {
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            total_tokens,
            tokens_per_second,
            last_turn_duration_ms,
            context_used,
            context_size,
            ..
        } = u
        {
            self.prompt_tokens = *prompt_tokens;
            self.completion_tokens = *completion_tokens;
            self.cached_tokens = *cached_tokens;
            self.total_tokens = *total_tokens;
            self.tokens_per_second = *tokens_per_second;
            self.last_turn_duration_ms = *last_turn_duration_ms;
            self.context_used = *context_used;
            self.context_size = *context_size;
            self.updates = self.updates.saturating_add(1);
            true
        } else {
            false
        }
    }
}
