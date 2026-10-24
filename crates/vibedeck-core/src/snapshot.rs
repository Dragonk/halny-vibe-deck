//! Shared engine<->UI types: status, bus events, snapshots.
//! Wasm-safe (serde only) so the web client can reuse them.

use serde::{Deserialize, Serialize};
use crate::proto::{AcpEvent, AcpSessionInfo, AvailableCommand, ConfigOption, ModeInfo};
use crate::types::ChatSession;

/// Live usage counters for the current turn (ported from the egui app).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageStats {
    pub prompt_tokens: f64,
    pub completion_tokens: f64,
    pub cached_tokens: f64,
    pub total_tokens: f64,
    pub tokens_per_second: f64,
    pub last_turn_duration_ms: f64,
    pub context_used: f64,
    pub context_size: f64,
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

    /// Consume a usage_update event; returns true when handled.
    pub fn apply(&mut self, e: &AcpEvent) -> bool {
        if let AcpEvent::UsageUpdate {
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            total_tokens,
            tokens_per_second,
            last_turn_duration_ms,
            context_used,
            context_size,
            ..
        } = e
        {
            self.prompt_tokens = *prompt_tokens;
            self.completion_tokens = *completion_tokens;
            self.cached_tokens = *cached_tokens;
            self.total_tokens = *total_tokens;
            self.tokens_per_second = *tokens_per_second;
            self.last_turn_duration_ms = *last_turn_duration_ms;
            self.context_used = *context_used;
            self.context_size = *context_size;
            self.updates += 1;
            true
        } else {
            false
        }
    }
}

/// Connection state of the vibe-acp worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcpStatus {
    Connecting,
    Connected,
    Disconnected,
    Error,
}

impl AcpStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AcpStatus::Connecting => "connecting",
            AcpStatus::Connected => "connected",
            AcpStatus::Disconnected => "disconnected",
            AcpStatus::Error => "error",
        }
    }
}

/// A pending tool-approval request from the agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingApproval {
    pub request_id: String,
    pub method: String,
    pub params_json: String,
    pub tool_name: Option<String>,
}

/// A pending free-form question from the agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub request_id: String,
    pub question: String,
    pub choices: Vec<String>,
}

/// Everything published on the engine bus. UI clients keep a local
/// projection: stream events are applied in place, structural changes
/// are easiest to handle by re-fetching a snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BusEvent {
    /// A raw ACP worker event (stream + structural).
    Acp(AcpEvent),
    /// A user message entered locally in one client, mirrored to all.
    UserMessageAdded {
        local_session_id: String,
        message: crate::types::Message,
    },
    /// Sessions/current/settings changed - refetch the snapshot.
    StructureChanged,
}

/// Sidebar-friendly summary of one session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub name: String,
    pub acp_session_id: Option<String>,
    pub working_directory: String,
    pub created_at_ms: i64,
    pub last_active_ms: i64,
    pub message_count: usize,
}

impl From<&ChatSession> for SessionMeta {
    fn from(s: &ChatSession) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            acp_session_id: s.acp_session_id.clone(),
            working_directory: s.working_directory.clone(),
            created_at_ms: s.created_at_ms,
            last_active_ms: s.last_active_ms,
            message_count: s.messages.len(),
        }
    }
}

/// UI preferences mirrored from the engine config (subset shown to clients).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub acp_path: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub remote_enabled: bool,
    #[serde(default = "default_remote_port")]
    pub remote_port: u16,
    #[serde(default)]
    pub auth_configured: bool,
    #[serde(default)]
    pub dock_open: bool,
    #[serde(default)]
    pub sidebar_sessions_limit: Option<u32>,
    #[serde(default)]
    pub hidden_sessions: Vec<String>,
    #[serde(default)]
    pub hidden_projects: Vec<String>,
}

fn default_remote_port() -> u16 {
    8080
}

/// Full state snapshot served to freshly-connected UI clients.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub status: String,
    pub acp_error: String,
    pub busy: bool,
    pub current: Option<usize>,
    pub sessions: Vec<SessionMeta>,
    pub current_messages: Vec<crate::types::Message>,
    pub modes: Vec<ModeInfo>,
    pub config_options: Vec<ConfigOption>,
    pub commands: Vec<AvailableCommand>,
    pub backend_sessions: Vec<AcpSessionInfo>,
    pub usage: UsageStats,
    pub approval: Option<PendingApproval>,
    pub question: Option<PendingQuestion>,
    pub working_directory: String,
    pub preferred_model: Option<String>,
    pub config: UiConfig,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: "disconnected".to_string(),
            acp_error: String::new(),
            busy: false,
            current: None,
            sessions: Vec::new(),
            current_messages: Vec::new(),
            modes: Vec::new(),
            config_options: Vec::new(),
            commands: Vec::new(),
            backend_sessions: Vec::new(),
            usage: UsageStats::default(),
            approval: None,
            question: None,
            working_directory: String::new(),
            preferred_model: None,
            config: UiConfig::default(),
        }
    }
}
