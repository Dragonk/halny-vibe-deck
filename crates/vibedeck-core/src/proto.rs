//! Event & command protocol between the UI layer and the ACP worker.
//!
//! The UI never blocks: it sends [`AcpCommand`]s to the worker thread
//! and consumes [`AcpEvent`]s every frame (the same shape as the old
//! Electron IPC bridge, minus Electron).

use crate::types::{ToolCall, ToolStatus};

/// A session previously created with the `vibe` CLI, reported by `session/list`.
#[derive(Clone, Debug)]
pub struct AcpSessionInfo {
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub start_time: String,
}

#[derive(Clone, Debug)]
pub struct ModeInfo {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
}

/// Events emitted by the ACP worker towards the UI.
#[derive(Clone, Debug)]
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
        models: Vec<String>,
        title: Option<String>,
    },
    /// A worker operation failed.
    SessionError { context: String, message: String },
    /// Streaming assistant text.
    MessageChunk { session_id: String, text: String },
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
#[derive(Clone, Debug)]
pub enum AcpCommand {
    /// Spawn vibe-acp at `acp_path` and run the `initialize` handshake.
    Start {
        acp_path: String,
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
