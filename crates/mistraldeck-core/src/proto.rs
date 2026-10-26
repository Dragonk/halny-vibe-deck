//! Event & command protocol between the UI layer and the ACP worker.
//!
//! The UI never blocks: it sends [`AcpCommand`]s to the worker thread
//! and consumes [`AcpEvent`]s every frame (the same shape as the old
//! Electron IPC bridge, minus Electron).

use crate::types::{ToolCall, ToolStatus};

use serde::{Deserialize, Serialize};

/// A session previously created with the `vibe` CLI, reported by `session/list`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpSessionInfo {
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub start_time: String,
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

/// A session config option advertised by vibe-acp
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

/// Events emitted by the ACP worker towards the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AcpEvent {
    /// `initialize` handshake finished.
    Initialized { protocol_version: u32 },
    /// The vibe-acp subprocess terminated.
    Disconnected { code: Option<i32> },
    /// `session/list` result.
    SessionsList(Vec<AcpSessionInfo>),
    /// A session was created / loaded / resumed and is ready for prompts.
    SessionReady {
        session_id: String,
        modes: Vec<ModeInfo>,
        /// model / mode / thinking / … — the source of truth for the
        /// toolbar pickers (vibe-acp ≥ 3.0 reports these as
        /// `configOptions`; older servers fall back to `models`/`modes`).
        config_options: Vec<ConfigOption>,
        title: Option<String>,
        /// `modes.currentModeId` from the wire (Nexus `parseModes`: falls
        /// back to the first mode) — updates the mode chip display.
        #[serde(default)]
        current_mode_id: Option<String>,
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
        /// vibe-acp ≥ 3.0 reports it with every usage update.
        #[serde(default)]
        cost: f64,
    },
    /// A worker operation failed.
    SessionError { context: String, message: String },
    /// Streaming assistant text.
    MessageChunk { session_id: String, text: String },
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
    ThinkingChunk { session_id: String, text: String },
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
}

/// Commands from the UI to the ACP worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AcpCommand {
    /// Spawn the agent from `argv` and run the `initialize` handshake.
    /// For local sessions this is `[acp_path]`; for SSH remotes it is the
    /// tokenized `ssh` command with `bash -lc "exec vibe-acp"` appended -
    /// the JSON-RPC protocol is identical over the ssh pipe.
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
    /// Answer a [`AcpEvent::ToolApprovalRequest`].
    Approve {
        request_id: String,
        option_id: String,
    },
    /// Answer a [`AcpEvent::AskUserQuestion`].
    AnswerQuestion {
        request_id: String,
        answer: String,
    },
}
