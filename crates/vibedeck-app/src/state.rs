//! UI-local application state (settings persistence, view enum,
//! pending agent interactions).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// App settings persisted in the platform config dir
/// (`~/.config/vibedeck/config.json` on Linux).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub language: String, // "en" | "pl"
    pub theme: String,    // "dark" | "light"
    pub acp_path: String,
    pub working_directory: String,
    pub recent_folders: Vec<String>,
    pub last_session_id: Option<String>,
    pub preferred_model: Option<String>,
    pub sidebar_sessions_limit: Option<usize>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: "en".into(),
            theme: "dark".into(),
            acp_path: String::new(),
            working_directory: String::new(),
            recent_folders: Vec::new(),
            last_session_id: None,
            preferred_model: None,
            sidebar_sessions_limit: Some(30),
        }
    }
}

impl AppSettings {
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("vibedeck").join("config.json"))
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
