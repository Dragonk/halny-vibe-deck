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

/// One agent tool call (execute / read / edit / …).
#[derive(Clone, Debug, Serialize, Deserialize)]
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
}

/// A reasoning/thinking stream block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ThinkingBlock {
    pub id: String,
    pub content: String,
    pub is_complete: bool,
}

/// A file or folder attached to a prompt via `@` mention.
#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
