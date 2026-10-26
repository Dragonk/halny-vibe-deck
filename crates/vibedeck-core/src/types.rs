//! Core domain types: messages, tool calls, sessions.

use serde::{Deserialize, Serialize};

/// Status of a single agent tool call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Running,
    Done,
    Error,
}

impl ToolStatus {
    /// Normalize raw ACP status strings (`completed`, `failed`, …).
    pub fn normalize(raw: &str) -> Self {
        match raw {
            "completed" | "done" | "success" => ToolStatus::Done,
            "failed" | "error" => ToolStatus::Error,
            _ => ToolStatus::Running,
        }
    }
}

/// A file touched by a tool call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub action: FileAction,
}

/// One structured edit diff hunk (git-style rendering in the chat).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffHunk {
    pub path: String,
    #[serde(default)]
    pub old_text: Option<String>,
    #[serde(default)]
    pub new_text: String,
    #[serde(default)]
    pub start_line: Option<i64>,
    #[serde(default)]
    pub new_start_line: Option<i64>,
}

/// v0.13 subagent invocation meta (`_meta.effect_kind == "subagent"` on
/// `tool_call` / `tool_call_update` — agents.spawn / task tools).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SubagentInfo {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub child_session_id: Option<String>,
}

/// One agent tool call (execute / read / edit / …).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: ToolStatus,
    #[serde(default)]
    pub input_json: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub file_changes: Vec<FileChange>,
    /// Structured edit diffs (edit tool) — git-style hunks in the chat.
    #[serde(default)]
    pub diffs: Vec<DiffHunk>,
    /// Canonical tool name from the ACP `_meta.tool_name` (rich routing).
    #[serde(default)]
    pub tool_name: String,
    /// Pre-edit file content (edit tool `rawOutput.previousContent`) —
    /// enables git-style context lines around hunks.
    #[serde(default)]
    pub previous_content: Option<String>,
    /// Raw structured output (`rawOutput`) serialized as JSON — process
    /// tool tables/status; unwrapped like Nexus `processRawFrom`.
    #[serde(default)]
    pub raw_output_json: Option<String>,
    /// v0.13 subagent invocation meta (agents.spawn / task).
    #[serde(default)]
    pub subagent: Option<SubagentInfo>,
    /// Unix epoch ms when the call was first seen (steps timeline ordering).
    #[serde(default)]
    pub started_at: i64,
}

/// A reasoning/thinking stream block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThinkingBlock {
    pub id: String,
    pub content: String,
    pub is_complete: bool,
    /// Unix epoch ms when the block started streaming.
    #[serde(default)]
    pub started_at: i64,
    /// Unix epoch ms when the block was finalized (0 while streaming).
    #[serde(default)]
    pub completed_at: i64,
}

/// A file or folder attached to a prompt via `@` mention.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub path: String,
    pub is_dir: bool,
    pub content: String,
}

/// Who authored a chat message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageSender {
    User,
    Ai,
    System,
}

/// One chat message (user, assistant or system).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: MessageSender,
    pub content: String,
    pub is_complete: bool,
    /// Unix epoch milliseconds.
    pub timestamp_ms: i64,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub thinking: Vec<ThinkingBlock>,
    #[serde(default)]
    pub is_context: bool,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

impl Message {
    pub fn new_user(content: &str) -> Self {
        Self {
            id: crate::types::new_id(),
            sender: MessageSender::User,
            content: content.to_string(),
            is_complete: true,
            timestamp_ms: now_ms(),
            tool_calls: Vec::new(),
            thinking: Vec::new(),
            is_context: false,
            attachments: Vec::new(),
        }
    }

    pub fn new_ai() -> Self {
        Self {
            id: crate::types::new_id(),
            sender: MessageSender::Ai,
            content: String::new(),
            is_complete: false,
            timestamp_ms: now_ms(),
            tool_calls: Vec::new(),
            thinking: Vec::new(),
            is_context: false,
            attachments: Vec::new(),
        }
    }

    pub fn new_system(content: &str) -> Self {
        Self {
            id: crate::types::new_id(),
            sender: MessageSender::System,
            content: content.to_string(),
            is_complete: true,
            timestamp_ms: now_ms(),
            tool_calls: Vec::new(),
            thinking: Vec::new(),
            is_context: false,
            attachments: Vec::new(),
        }
    }
}

/// One entry of the agent plan / todo checklist (`sessionUpdate: "plan"`).
/// Mirrors Nexus `PlanEntry`: status is one of pending / in_progress /
/// completed (anything else on the wire normalizes to "pending").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
}

impl PlanEntry {
    /// Normalize a raw wire status the same way Nexus does.
    pub fn normalize_status(raw: &str) -> String {
        match raw {
            "completed" | "in_progress" => raw.to_string(),
            _ => "pending".to_string(),
        }
    }

    /// Parse the `entries` array of a plan update / replay snapshot.
    pub fn parse_list(entries_json: &str) -> Vec<Self> {
        use serde_json::Value;
        let Ok(Value::Array(entries)) =
            serde_json::from_str::<Value>(entries_json)
        else {
            return Vec::new();
        };
        entries
            .iter()
            .map(|e| Self {
                content: e
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                status: Self::normalize_status(
                    e.get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("pending"),
                ),
                priority: e.get("priority").map(|p| match p {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                }),
            })
            .collect()
    }
}

/// A local chat session, optionally linked to a backend ACP session.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub acp_session_id: Option<String>,
    pub messages: Vec<Message>,
    /// Unix epoch milliseconds.
    pub created_at_ms: i64,
    pub last_active_ms: i64,
    #[serde(default)]
    pub working_directory: String,
    /// SSH remote connection this session runs on (None = local).
    #[serde(default)]
    pub remote_id: Option<String>,
    /// Current agent plan / todo checklist (cumulative — the last
    /// `plan` update wins; mirrors Nexus v0.11.1 `session.plans`).
    #[serde(default)]
    pub plan: Vec<PlanEntry>,
    /// Last reported config options (model/mode/thinking) — per-session
    /// chips survive slot-less switching and lazy respawns.
    #[serde(default)]
    pub config_options: Vec<crate::proto::ConfigOption>,
    /// Last config actually used by this session (Nexus `lastModel`/
    /// `lastMode`/`lastThinking`): recorded on every confirmed config
    /// change and on SessionReady, restored from disk
    /// (`runtime-state.json`) for sessions never spawned this run.
    #[serde(default)]
    pub last_model: Option<String>,
    #[serde(default)]
    pub last_mode: Option<String>,
    #[serde(default)]
    pub last_thinking: Option<String>,
    /// Usage mirrored onto the session (every `usage_update` is applied
    /// here too): survives slot death and feeds the UsageBar for
    /// lazy-resumed sessions whose preview came from disk (Nexus
    /// `setUsage` in `previewSession`).
    #[serde(default)]
    pub usage: crate::snapshot::UsageStats,
}

impl ChatSession {
    pub fn new(name: &str, working_directory: &str) -> Self {
        let now = now_ms();
        Self {
            id: new_id(),
            name: name.to_string(),
            acp_session_id: None,
            messages: Vec::new(),
            created_at_ms: now,
            last_active_ms: now,
            working_directory: working_directory.to_string(),
            remote_id: None,
            plan: Vec::new(),
            config_options: Vec::new(),
            last_model: None,
            last_mode: None,
            last_thinking: None,
            usage: crate::snapshot::UsageStats::default(),
        }
    }
}

pub fn now_ms() -> i64 {
    // `std::time::SystemTime` panics on wasm ("time not implemented on this
    // platform") — that crash took down the whole Stats and History views in
    // the browser. On wasm use JS Date::now() instead.
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Short random identifier (session/message ids are local-only;
/// ACP session ids always come from the vibe-acp server).
pub fn new_id() -> String {
    let t = now_ms() as u128;
    // xorshift from the clock + address entropy: good enough for UI keys.
    let mut x = t ^ (&t as *const u128 as usize as u128);
    let mut out = String::with_capacity(16);
    for _ in 0..16 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push(char::from(b'a' + (x % 26) as u8));
    }
    out
}
