//! Event & command protocol between the UI layer and the agent worker.
//!
//! The UI never blocks: it sends [`AgentCommand`]s to the worker thread
//! and consumes [`AgentEvent`]s every frame (the same shape as the old
//! Electron IPC bridge, minus Electron).

use crate::types::{ToolCall, ToolStatus};

use serde::{Deserialize, Serialize};

/// A session previously created with the `vibe` CLI, reported by `session/list`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionInfo {
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub start_time: String,
    #[serde(default)]
    pub pinned_at: Option<i64>,
    #[serde(default)]
    pub archived_at: Option<i64>,
    #[serde(default)]
    pub is_unseen: bool,
    #[serde(default)]
    pub worktree: Option<String>,
}

/// One image attached to a prompt — sent as an agent-transport `image` content
/// block (v1.0.8: images must actually reach the LLM).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptImage {
    /// Base64-encoded image bytes.
    pub data_base64: String,
    /// MIME type, e.g. `image/png`.
    pub mime: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModeInfo {
    pub id: String,
    pub name: String,
}

/// One selectable value of a session config option
/// (`session/new` → `configOptions[].options[]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigOptionChoice {
    pub value: String,
    pub name: String,
    pub description: String,
}

/// A session config option advertised by the agent
/// (model / mode / thinking / …) with its current value and choices.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigOption {
    pub id: String,
    pub name: String,
    pub current_value: String,
    pub choices: Vec<ConfigOptionChoice>,
}

impl ConfigOption {
    /// Human-readable label of the current value (falls back to the raw value).
    pub fn current_label(&self) -> String {
        self.choices
            .iter()
            .find(|c| c.value == self.current_value)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| self.current_value.clone())
    }

    pub fn find(&self, id: &str) -> bool {
        self.id == id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
    /// Argument hint from the agent (`input.hint`), e.g. `status | login`.
    pub hint: String,
}

/// Events emitted by the agent worker towards the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentEvent {
    /// `initialize` handshake finished.
    Initialized { protocol_version: u32 },
    /// The agent subprocess terminated.
    Disconnected { code: Option<i32> },
    /// `session/list` result.
    SessionsList(Vec<AgentSessionInfo>),
    /// A session was created / loaded / resumed and is ready for prompts.
    SessionReady {
        session_id: String,
        modes: Vec<ModeInfo>,
        /// model / mode / thinking / … — the source of truth for the
        /// toolbar pickers (agent ≥ 3.0 reports these as
        /// `configOptions`; older servers fall back to `models`/`modes`).
        config_options: Vec<ConfigOption>,
        title: Option<String>,
        /// `modes.currentModeId` from the wire (Nexus `parseModes`: falls
        /// back to the first mode) — updates the mode chip display.
        #[serde(default)]
        current_mode_id: Option<String>,
        /// `PublicSessionState.childSessions` — raw wire JSON per child.
        #[serde(default)]
        child_sessions: Vec<String>,
    },
    /// Live token usage for a turn (`session/update` → `usage_update`).
    UsageUpdate {
        session_id: String,
        prompt_tokens: f64,
        completion_tokens: f64,
        cached_tokens: f64,
        total_tokens: f64,
        tokens_per_second: f64,
        last_turn_duration_ms: f64,
        context_used: f64,
        context_size: f64,
        /// Session cost from the wire (`update.cost.amount`, USD) —
        /// the agent reports it with every usage update.
        #[serde(default)]
        cost: f64,
    },
    /// A worker operation failed.
    SessionError { context: String, message: String },
    /// Streaming assistant text.
    MessageChunk {
        session_id: String,
        /// Backend `messageId` of the streamed message ("current" when the CLI
        /// does not send one) — stream dedup key (Nexus v0.10.2).
        #[serde(default)]
        message_id: String,
        text: String,
    },
    /// Replayed user text (`session/load` history, `user_message_chunk`).
    UserMessageChunk { session_id: String, text: String },
    /// Full conversation history replayed by `session/load` — the UI
    /// replaces the local mirror's messages with these. The plan is
    /// cumulative state: only the last replayed snapshot matters.
    SessionHistory {
        session_id: String,
        messages: Vec<crate::types::Message>,
        #[serde(default)]
        plan: Vec<crate::types::PlanEntry>,
    },
    /// Replay progress while `session/load` is streaming history back
    /// (Nexus `SessionLoadProgress`: emitted every 25 buffered updates).
    /// Drives the "Resuming session… (N)" overlay.
    SessionLoadProgress {
        session_id: String,
        count: usize,
    },
    /// Streaming assistant reasoning text.
    ThinkingChunk {
        session_id: String,
        /// Backend `messageId` of the thought block ("thinking" default) —
        /// stream dedup key (Nexus v0.10.2).
        #[serde(default)]
        block_id: String,
        text: String,
    },
    /// Reasoning stream ended.
    ThinkingComplete { session_id: String },
    /// A tool call started.
    ToolCallStarted {
        session_id: String,
        tool_call: ToolCall,
    },
    /// A tool call changed status (done / failed) or produced output.
    ToolCallUpdated {
        session_id: String,
        tool_call_id: String,
        status: ToolStatus,
        output: Option<String>,
        /// v0.13 subagent meta (`_meta.effect_kind == "subagent"`) — merged
        /// into the running call, carries `child_session_id` once spawned.
        #[serde(default)]
        subagent: Option<crate::types::SubagentInfo>,
    },
    /// The agent asks the user to approve a tool call / permission.
    ToolApprovalRequest {
        request_id: String,
        method: String,
        params_json: String,
        tool_name: Option<String>,
    },
    /// The agent asked a free-form question.
    AskUserQuestion {
        request_id: String,
        question: String,
        choices: Vec<String>,
    },
    /// Agent plan entries (raw JSON, shape varies).
    PlanUpdate {
        session_id: String,
        entries_json: String,
    },
    /// A child (subagent) session changed — `session/childSessionUpdated`
    /// or `PublicSessionState.childSessions` in a snapshot. Raw wire JSON.
    ChildSessionUpdated {
        session_id: String,
        child_json: String,
    },
    /// `session/rewind/read` answer — does rewinding to this entry touch
    /// files on disk (and which).
    RewindInfo {
        session_id: String,
        entry_id: String,
        has_file_changes: bool,
        paths: Vec<String>,
    },
    /// `session/lease/updated|ended` — another process holds this session.
    LeaseUpdated {
        session_id: String,
        held_elsewhere: bool,
    },
    /// `vibeCode/projects/open|select|create|loadMore` picker view (raw JSON).
    TeleportPicker {
        session_id: String,
        picker_id: String,
        view_json: String,
    },
    /// `vibeCode/teleport/event` notification (raw event JSON).
    TeleportProgress {
        session_id: String,
        event_json: String,
    },
    /// `loops/list` result (raw JSON array of ScheduledLoop).
    LoopsListResult {
        session_id: String,
        loops_json: String,
    },
    /// `session/log/read` result (raw SessionLogSummary JSON).
    SessionLog {
        session_id: String,
        log_json: String,
    },
    /// `diagnostics/logs/read` result (raw DebugLogPage JSON).
    DiagnosticsLogs {
        session_id: String,
        logs_json: String,
    },
    /// `workspace/git/worktrees/list` result (raw JSON).
    WorktreesResult {
        cwd: String,
        items_json: String,
    },
    /// `artifacts/list` result (raw JSON).
    ArtifactsResult {
        session_id: String,
        artifacts_json: String,
    },
    /// `artifacts/created|updated` — refetch.
    ArtifactsChanged {
        session_id: String,
    },
    /// Slash commands offered by the agent for this session.
    AvailableCommands {
        session_id: String,
        commands: Vec<AvailableCommand>,
    },
    /// `session/prompt` returned.
    PromptCompleted {
        session_id: String,
        stop_reason: String,
    },
    /// Worker stdout/stderr noise for the debug panel.
    DebugLog(String),

    // --- L1: setup / konto (metody globalne app-servera, nie sesyjne) ---
    /// L1: odpowiedź setup/status; `unsupported` = method_not_found (stare CLI).
    SetupStatusReady {
        status: crate::snapshot::SetupStatusView,
    },
    /// L1: wynik zapisu klucza: "completed" | "env_var_error" | "save_error" (+detail).
    SetupStoreOutcome {
        outcome: String,
        detail: Option<String>,
    },
    /// L1: wynik submit-choices: "completed" | "provider_config_error" (+failures).
    SetupChoicesOutcome {
        outcome: String,
        failures: Vec<String>,
    },
    /// L1: odpowiedź identity/read (tolerancyjnie: surowy JSON → string).
    IdentityReady {
        identity: crate::snapshot::IdentityView,
    },
    /// L1: odpowiedź account/read.
    AccountReady {
        account: crate::snapshot::AccountView,
    },
    /// L1: tura zakończona błędem (rate_limit i in.) — z wire `turn.error`.
    TurnFailed {
        session_id: String,
        code: String,
        message: String,
    },

    // --- Setup install (engine-side `AgentCommand::SetupInstall` flow) ---
    /// One progress line of the local dependency install (kind: info |
    /// success | error).
    SetupInstallLine { message: String, kind: String },
    /// The local dependency install finished (`ok` = vibe-app-server verified).
    SetupInstallDone { ok: bool },
}

/// Commands from the UI to the agent worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AgentCommand {
    /// Spawn the agent from `argv` and run the `initialize` handshake.
    /// For local sessions this is `[vibe-app-server]`; for SSH remotes it
    /// is the tokenized `ssh` command with `bash -lc "exec vibe-app-server"`
    /// appended - the JSON-RPC protocol is identical over the ssh pipe.
    Start {
        argv: Vec<String>,
    },
    /// Kill the subprocess and reset state.
    Stop,
    ListSessions,
    NewSession {
        cwd: String,
        model: Option<String>,
    },
    LoadSession {
        session_id: String,
        cwd: String,
        model: Option<String>,
    },
    ResumeSession {
        session_id: String,
        cwd: String,
        model: Option<String>,
    },
    Prompt {
        session_id: String,
        text: String,
        /// Image attachments turned into `image` content blocks.
        #[serde(default)]
        images: Vec<PromptImage>,
    },
    /// Handled with priority even while a prompt is streaming.
    Cancel {
        session_id: String,
    },
    SetConfigOption {
        session_id: String,
        key: String,
        value: String,
    },
    /// Answer a [`AgentEvent::ToolApprovalRequest`].
    Approve {
        request_id: String,
        option_id: String,
    },
    /// Answer a [`AgentEvent::AskUserQuestion`].
    AnswerQuestion {
        request_id: String,
        answer: String,
    },
    /// `session/compact` — TUI `/compact [instructions]`.
    SessionCompact {
        session_id: String,
        extra_instructions: String,
    },
    /// `session/history/clear` — TUI `/clear`, `/new`.
    HistoryClear {
        session_id: String,
    },
    /// `session/rename` — TUI `/rename <title>`.
    SessionRename {
        session_id: String,
        title: String,
    },
    /// `session/shellCommand` — TUI `!cmd` inline shell.
    ShellCommand {
        session_id: String,
        command: String,
    },
    /// `config/reload` — TUI `/reload`.
    ConfigReload,
    /// `session/fork` — TUI `/branch`: fork this session at `entry_id`
    /// (None = end of history); `attach` semantics handled engine-side.
    SessionFork {
        source_session_id: String,
        entry_id: Option<String>,
    },
    /// `session/rewind` — TUI `/rewind`: rewind the session to `entry_id`.
    SessionRewind {
        session_id: String,
        entry_id: String,
        restore_files: bool,
    },
    /// `session/rewind/read` — preview which files a rewind would restore.
    RewindRead {
        session_id: String,
        entry_id: String,
    },
    /// `session/pin` — pin/unpin a session on the shelf (GUI extra).
    PinSession {
        session_id: String,
        pinned: bool,
    },
    /// `session/archive` — archive/unarchive (GUI extra).
    ArchiveSession {
        session_id: String,
        archived: bool,
    },
    /// `session/markAsSeen` — clear the unread dot.
    MarkSessionSeen {
        session_id: String,
    },
    /// `session/delete` — delete a backend session from disk.
    DeleteBackendSession {
        session_id: String,
    },
    /// `session/lease/subscribe` — learn when another process opens this
    /// session ("open elsewhere" badge).
    LeaseSubscribe {
        session_id: String,
    },
    // --- teleport to Vibe Code Web (TUI /teleport) ---
    /// `vibeCode/projects/open` (purpose "teleport") → picker view.
    TeleportOpen {
        session_id: String,
        prompt: Option<String>,
    },
    /// `vibeCode/projects/select` + `vibeCode/teleport/start`.
    TeleportSelect {
        session_id: String,
        picker_id: String,
        project_id: String,
        prompt: Option<String>,
    },
    /// `vibeCode/projects/create`.
    TeleportCreate {
        session_id: String,
        picker_id: String,
        name: String,
        default_branch: String,
    },
    /// `vibeCode/projects/loadMore`.
    TeleportLoadMore {
        session_id: String,
        picker_id: String,
    },
    /// `vibeCode/projects/cancel`.
    TeleportPickerCancel {
        session_id: String,
        picker_id: String,
    },
    /// `vibeCode/teleport/push/respond` — answer a push_required event.
    TeleportPushRespond {
        session_id: String,
        operation_id: String,
        approved: bool,
    },
    /// `vibeCode/teleport/cancel`.
    TeleportAbort {
        session_id: String,
        operation_id: String,
    },
    // --- loops admin (loops/* are Desktop/GUI-only per the TUI report) ---
    LoopsList { session_id: String },
    LoopsDelete { session_id: String, loop_id: String },
    LoopsClear { session_id: String },
    // --- logs / debug console ---
    SessionLogRead { session_id: String },
    DiagnosticsLogsRead { session_id: String, limit: i64, offset: i64 },
    // --- worktrees + artifacts ---
    WorktreesList { cwd: String },
    ArtifactsList { session_id: String },

    // --- L1: setup / konto (metody globalne app-servera, nie sesyjne) ---
    /// setup/status — seed wizarda (supportsBrowserSignIn, hasApiKey, browserAuth*).
    SetupStatus,
    /// setup/store-credential — zapis klucza (keyring / $VIBE_HOME/.env).
    SetupStoreCredential {
        provider: Option<String>,
        api_key: String,
        custom_domain: Option<String>,
    },
    /// setup/submit-choices — domknięcie onboarding (theme/console pozostają = "zostaw").
    SetupSubmitChoices,
    /// identity/read — /whoami (login, org).
    IdentityRead,
    /// account/read — AccountView (plan.title, planOffer, rateLimitAction, apiKey.preview).
    AccountRead,
    /// Local dependency install (uv → mistral-vibe → verify
    /// vibe-app-server). Handled engine-side (never reaches a
    /// transport): the CLI is a prerequisite for starting any agent.
    SetupInstall,
}
