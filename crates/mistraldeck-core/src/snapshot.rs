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
    /// Session cost in USD (from the wire `usage_update.cost.amount`).
    #[serde(default)]
    pub cost: f64,
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
            cost,
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
            self.cost = *cost;
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

/// Replay progress of a waking `session/load` (Nexus
/// `sessionLoadProgress`): shown by the chat as a "Resuming session…
/// (N)" overlay while the lazy-resumed session replays its history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionLoadProgress {
    pub session_id: String,
    pub count: usize,
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
    /// This session's agent is streaming (per-slot busy flag).
    #[serde(default)]
    pub busy: bool,
    /// Per-slot connection status ("connecting"/"connected"/...).
    #[serde(default)]
    pub status: String,
    /// SSH remote connection this session runs on (None = local).
    #[serde(default)]
    pub remote_id: Option<String>,
    /// A `session/cancel` is in flight for this session (Nexus v0.10.5
    /// `stoppingSessions`) — the stop button turns into a disabled
    /// spinner and the busy row shows the stopping label until the turn
    /// really ends.
    #[serde(default)]
    pub stopping: bool,
}

impl SessionMeta {
    pub fn from_session(s: &ChatSession, busy: bool, status: &str) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            acp_session_id: s.acp_session_id.clone(),
            working_directory: s.working_directory.clone(),
            created_at_ms: s.created_at_ms,
            last_active_ms: s.last_active_ms,
            message_count: s.messages.len(),
            busy,
            status: status.to_string(),
            remote_id: s.remote_id.clone(),
            stopping: false,
        }
    }
}

impl From<&ChatSession> for SessionMeta {
    fn from(s: &ChatSession) -> Self {
        Self::from_session(s, false, "")
    }
}

/// One SSH remote where vibe-acp runs (Codex-style remote projects;
/// mirrors Nexus `RemoteConnection`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteConnection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    /// e.g. `ssh user@host -p 2222` (no remote command part).
    #[serde(default)]
    pub ssh_cmd: String,
    /// Remote working directories shown as sub-projects.
    #[serde(default)]
    pub locations: Vec<String>,
    /// Optional custom install command for vibe-acp.
    #[serde(default)]
    pub install_command: String,
}

/// Result of testing an (unsaved) connection draft.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteTestResult {
    pub reachable: bool,
    pub acp_installed: bool,
    #[serde(default)]
    pub acp_path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One CLI session discovered on a remote host.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteSessionInfo {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub updated_at: String,
}

/// Quick liveness check for the sidebar dot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteStatus {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub checked_at_ms: i64,
    #[serde(default)]
    pub latency_ms: Option<u64>,
}

/// Result of installing vibe-acp on a remote host.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteInstallResult {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub log: String,
}

/// CLI update check (Nexus `CliUpdateInfo`): current + latest PyPI
/// version of the mistral-vibe CLI, local or remote.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CliUpdateInfo {
    #[serde(default)]
    pub current_version: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub update_available: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Result of a CLI upgrade (Nexus `CliUpgradeResult`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CliUpgradeResult {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// App self-update check (Nexus `AppUpdateInfo`): current flavor/version,
/// latest GitHub release and the matching platform asset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateInfo {
    #[serde(default)]
    pub current_version: String,
    #[serde(default)]
    pub flavor: String,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub update_available: bool,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Result of an app self-update attempt (Nexus `AppUpdateResult`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateResult {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// App self-update download/install progress (Nexus `UpdateProgress`):
/// phase download/install/relaunch, bytes received/total, percent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateProgress {
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub received: u64,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub percent: Option<i64>,
}

/// A `(tool, permission)` pair from a `[tools.<name>]` block of a remote
/// agent profile (Nexus `VibeAgentFileConfig.tools`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolPermissionEntry {
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub permission: String,
}

/// Remote agent profile (Nexus `VibeAgentFileConfig`), parsed from
/// `<dir>/agents/<name>.toml` on the remote host.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteAgentFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub active_model: Option<String>,
    #[serde(default)]
    pub system_prompt_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default)]
    pub tool_permissions: Vec<ToolPermissionEntry>,
    /// Raw TOML of the agent file (Settings raw editor over SSH).
    #[serde(default)]
    pub content: String,
}

/// Remote prompt (`<dir>/prompts/<name>.md` or `.txt`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemotePromptFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content: String,
}

/// One file of the remote raw-config tree (Nexus `listConfigFiles` entry).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteConfigFileEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub full: String,
}

/// One file/dir inside a remote skill (Nexus `VibeSkill.files` entry).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteSkillFileEntry {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub full: String,
    #[serde(default)]
    pub is_dir: bool,
}

/// Remote skill (Nexus `VibeSkill`): `<dir>/skills/<name>/SKILL.md` plus
/// the file tree of the skill dir.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RemoteSkill {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub files: Vec<RemoteSkillFileEntry>,
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
    pub auto_close_idle_minutes: Option<u32>,
    #[serde(default)]
    pub recent_folders: Vec<String>,
    #[serde(default)]
    pub hidden_sessions: Vec<String>,
    #[serde(default)]
    pub hidden_projects: Vec<String>,
    #[serde(default)]
    pub remote_connections: Vec<RemoteConnection>,
}

fn default_remote_port() -> u16 {
    8080
}

// ---------------------------------------------------------------------------
// L1: konto / logowanie (setup/status, account/read, identity/read)
// ---------------------------------------------------------------------------

/// `provider` z setup/status (raport §3.0; models/_setup.py).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SetupProviderView {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub api_key_env_var: String,
    #[serde(default)]
    pub browser_auth_base_url: String,
    #[serde(default)]
    pub browser_auth_api_base_url: String,
    #[serde(default)]
    pub browser_auth_allow_origin_rewrite: bool,
}

/// Seed wizarda: `setup/status`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SetupStatusView {
    #[serde(default)]
    pub provider: Option<SetupProviderView>,
    #[serde(default)]
    pub console_base_url: String,
    #[serde(default)]
    pub vibe_base_url: String,
    #[serde(default)]
    pub supports_browser_sign_in: bool,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default)]
    pub pinned_console_url: Option<String>,
    /// true gdy serwer nie zna metody (method_not_found) — UI degraduje się.
    #[serde(default)]
    pub unsupported: bool,
}

/// `AccountPlanView` (models.py:101): kind api|chat|mistral_code.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountPlanView {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: String,
}

/// `AccountActionView` (models.py:96): kind switch_api_key|upgrade_to_pro + url.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountActionView {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub url: String,
}

/// `AccountApiKeyView` (models.py:112); accessScope: None na starszych backendach.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountApiKeyView {
    #[serde(default)]
    pub preview: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub access_scope: Option<String>,
}

/// `AccountView` (models.py:119-126) — JEDYNE źródło danych konta.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountView {
    /// ready|missing_key|unauthorized|unavailable
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub plan: Option<AccountPlanView>,
    #[serde(default)]
    pub plan_offer: Option<AccountActionView>,
    #[serde(default)]
    pub rate_limit_action: Option<AccountActionView>,
    #[serde(default)]
    pub teleport_eligible: bool,
    #[serde(default)]
    pub api_key: Option<AccountApiKeyView>,
    /// method_not_found na starym CLI
    #[serde(default)]
    pub unsupported: bool,
}

/// identity/read → /whoami (commands/simple.rs:232-280); pola tolerancyjne.
/// UI czyta name/email/org z surowego JSONa.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct IdentityView {
    #[serde(default)]
    pub raw_json: String,
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
    /// Current session's agent plan / todo checklist (last update wins).
    #[serde(default)]
    pub current_plan: Vec<crate::types::PlanEntry>,
    pub modes: Vec<ModeInfo>,
    pub config_options: Vec<ConfigOption>,
    pub commands: Vec<AvailableCommand>,
    pub backend_sessions: Vec<AcpSessionInfo>,
    pub usage: UsageStats,
    /// `session/load` replay progress for the current session's agent
    /// (None when nothing is resuming) — lazy-resume overlay.
    #[serde(default)]
    pub session_load_progress: Option<SessionLoadProgress>,
    /// v0.13.7: messageId of the text streaming into the current session
    /// (client-side mirror of `ChatSession.stream_msg_id` - the segment
    /// split state, kept in sync by the engine snapshot).
    #[serde(default)]
    pub ai_stream_msg_id: Option<String>,
    pub approval: Option<PendingApproval>,
    pub question: Option<PendingQuestion>,
    pub working_directory: String,
    pub preferred_model: Option<String>,
    pub config: UiConfig,
    /// L1: konto (account/read) — None = jeszcze nie odpytane.
    #[serde(default)]
    pub account: Option<AccountView>,
    /// L1: seed logowania (setup/status) — None = nie odpytane / stary CLI.
    #[serde(default)]
    pub setup: Option<SetupStatusView>,
    /// L1: whoami (identity/read) — opcjonalne, do ustawień.
    #[serde(default)]
    pub identity: Option<IdentityView>,
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
            current_plan: Vec::new(),
            modes: Vec::new(),
            config_options: Vec::new(),
            commands: Vec::new(),
            backend_sessions: Vec::new(),
            usage: UsageStats::default(),
            session_load_progress: None,
            ai_stream_msg_id: None,
            approval: None,
            question: None,
            working_directory: String::new(),
            preferred_model: None,
            config: UiConfig::default(),
            account: None,
            setup: None,
            identity: None,
        }
    }
}
