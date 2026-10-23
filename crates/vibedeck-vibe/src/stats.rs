//! Session statistics from the Vibe CLI logs (`~/.vibe/logs/session/`).
//!
//! Layout written by the CLI:
//! - `<session>/meta.json` — session metadata + stats
//! - `<session>/messages.jsonl` — message log
//! - `<session>/agents/<agent>/<…>` — subagent sessions
//!
//! All fields are optional and snake_case (the CLI's native shape),
//! so schema drift degrades gracefully instead of failing the whole view.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::vibe_home;

fn sessions_log_dir() -> PathBuf {
    vibe_home().join("logs").join("session")
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct VibeSessionStats {
    #[serde(default)]
    pub steps: f64,
    #[serde(default)]
    pub session_prompt_tokens: f64,
    /// Cached (reused) prompt tokens reported by the CLI, when present.
    #[serde(default, alias = "session_cached_tokens", alias = "cached_input_tokens")]
    pub cached_tokens: f64,
    #[serde(default)]
    pub session_completion_tokens: f64,
    #[serde(default)]
    pub tool_calls_agreed: f64,
    #[serde(default)]
    pub tool_calls_rejected: f64,
    #[serde(default)]
    pub tool_calls_failed: f64,
    #[serde(default)]
    pub tool_calls_succeeded: f64,
    #[serde(default)]
    pub context_tokens: f64,
    #[serde(default)]
    pub last_turn_duration: f64,
    #[serde(default)]
    pub tokens_per_second: f64,
    #[serde(default)]
    pub session_total_llm_tokens: f64,
    #[serde(default)]
    pub session_cost: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct VibeSession {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub start_time: String,
    #[serde(default)]
    pub end_time: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub total_messages: Option<u64>,
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub environment: Option<VibeEnvironment>,
    #[serde(default)]
    pub stats: VibeSessionStats,
    #[serde(default)]
    pub config: Option<VibeSessionConfig>,
    #[serde(default)]
    pub agent_profile: Option<VibeAgentProfile>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct VibeEnvironment {
    #[serde(default)]
    pub working_directory: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct VibeSessionConfig {
    #[serde(default)]
    pub active_model: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct VibeAgentProfile {
    #[serde(default)]
    pub name: Option<String>,
}

/// One entry from `messages.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VibeMessage {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<serde_json::Value>,
}

/// A subagent session nested under a parent session folder.
#[derive(Clone, Debug)]
pub struct VibeSubagentSession {
    pub session_id: String,
    pub agent_name: String,
    pub start_time: String,
    pub messages: Vec<VibeMessage>,
}

/// All sessions, newest first.
///
/// Two on-disk layouts are supported:
/// - classic: `logs/session/<id>/meta.json` (stats included),
/// - vibe-acp ≥ 3.0: `logs/session/unified/<id>/meta.json`
///   (no `stats` block — token/cost metrics come from the live
///   `usage_update` ACP notifications instead).
pub fn load_sessions() -> Vec<VibeSession> {
    let dir = sessions_log_dir();
    let mut sessions: Vec<VibeSession> = Vec::new();
    // classic layout
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || entry.file_name().to_str() == Some("unified") {
                continue;
            }
            if let Some(s) = load_meta(&path.join("meta.json")) {
                sessions.push(s);
            }
        }
    }
    // unified layout (newer CLI)
    let unified = dir.join("unified");
    if let Ok(entries) = std::fs::read_dir(&unified) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(s) = load_meta(&path.join("meta.json")) {
                sessions.push(s);
            }
        }
    }
    sessions.sort_by(|a, b| b.start_time.cmp(&a.start_time));
    sessions.dedup_by(|a, b| a.session_id == b.session_id && a.start_time == b.start_time);
    sessions
}

/// Parse one `meta.json`; falls back to the folder name for the id.
fn load_meta(meta: &Path) -> Option<VibeSession> {
    let raw = std::fs::read_to_string(meta).ok()?;
    let mut session: VibeSession = serde_json::from_str(&raw).ok()?;
    if session.session_id.is_empty() {
        session.session_id = meta
            .parent()?
            .file_name()?
            .to_str()?
            .to_string();
    }
    if session.session_id.is_empty() || session.start_time.is_empty() {
        return None;
    }
    Some(session)
}

/// Locate the on-disk folder of a session by id (folder name or meta.json).
fn find_session_dir(session_id: &str) -> Option<PathBuf> {
    let dir = sessions_log_dir();
    // Fast path: folder named after the session (classic layout).
    let direct = dir.join(session_id);
    if direct.join("meta.json").is_file() {
        return Some(direct);
    }
    // Unified layout (newer CLI): `logs/session/unified/<id>/`.
    let unified = dir.join("unified").join(session_id);
    if unified.join("meta.json").is_file() {
        return Some(unified);
    }
    // Scan both layouts for a meta.json with a matching session id.
    let scan = |base: &Path| -> Option<PathBuf> {
        for entry in std::fs::read_dir(base).ok().into_iter().flatten().flatten() {
            let path = entry.path();
            if !path.join("meta.json").is_file() {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(path.join("meta.json")) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            if value.get("session_id").and_then(|v| v.as_str()) == Some(session_id) {
                return Some(path);
            }
        }
        None
    };
    scan(&dir).or_else(|| scan(&dir.join("unified")))
}

/// Load the message log of a session.
pub fn load_session_messages(session_id: &str) -> Vec<VibeMessage> {
    let Some(dir) = find_session_dir(session_id) else {
        return Vec::new();
    };
    let classic = read_messages(&dir.join("messages.jsonl"));
    if !classic.is_empty() {
        return classic;
    }
    // Newer CLI ("unified") layout: the transcript lives in chunk snapshots.
    read_unified_transcript(&dir.join("chunks"))
}

/// Newer CLI ("unified") sessions keep the transcript inside
/// `chunks/*.json` snapshots: each file is a list of entries, and entries
/// with `source: "history"` carry one message of the conversation
/// (`message: { role, content: [...] }`). Snapshots cover single turns and
/// overlap, so per turn the most complete snapshot is used; user entries
/// (turn starters) are collected from every snapshot.
fn read_unified_transcript(chunks_dir: &Path) -> Vec<VibeMessage> {
    let Ok(entries) = std::fs::read_dir(chunks_dir) else {
        return Vec::new();
    };
    // turn -> [(file, non-user history entries of that turn)]
    let mut turns: std::collections::BTreeMap<i64, Vec<(String, Vec<serde_json::Value>)>> =
        std::collections::BTreeMap::new();
    // user entries deduped by content, kept per turn
    let mut user_msgs: Vec<(i64, serde_json::Value)> = Vec::new();
    let mut seen_user: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) else {
            continue;
        };
        let mut per_turn: std::collections::HashMap<i64, Vec<serde_json::Value>> =
            std::collections::HashMap::new();
        for e in list {
            if e.get("source").and_then(|s| s.as_str()) != Some("history") {
                continue;
            }
            let turn = e.get("turn_index").and_then(|t| t.as_i64()).unwrap_or(0);
            if e.pointer("/message/role").and_then(|r| r.as_str()) == Some("user") {
                let key = match e.pointer("/message/content") {
                    Some(v) => serde_json::to_string(v).unwrap_or_default(),
                    None => String::new(),
                };
                if seen_user.insert(key) {
                    user_msgs.push((turn, e));
                }
                continue;
            }
            per_turn.entry(turn).or_default().push(e);
        }
        for (turn, list) in per_turn {
            turns
                .entry(turn)
                .or_default()
                .push((path.display().to_string(), list));
        }
    }
    let mut messages: Vec<VibeMessage> = Vec::new();
    let mut idx = 0usize;
    for (turn, files) in turns {
        // user message(s) that started this turn
        for (uturn, e) in &user_msgs {
            if *uturn == turn {
                if let Some(msg) = unified_entry_to_message(e, &mut idx) {
                    messages.push(msg);
                }
            }
        }
        // most complete snapshot of this turn
        if let Some((_, list)) = files.iter().max_by_key(|(_, l)| l.len()) {
            for e in list {
                if let Some(msg) = unified_entry_to_message(e, &mut idx) {
                    messages.push(msg);
                }
            }
        }
    }
    messages
}

/// Convert one `source: "history"` chunk entry into a [`VibeMessage`].
fn unified_entry_to_message(entry: &serde_json::Value, idx: &mut usize) -> Option<VibeMessage> {
    let message = entry.get("message")?;
    let role = message
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("tool")
        .to_string();
    let mut content = String::new();
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    if let Some(items) = message.get("content").and_then(|c| c.as_array()) {
        for item in items {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        if !content.is_empty() {
                            content.push('\n');
                        }
                        content.push_str(text);
                    }
                }
                Some("tool_call") => tool_calls.push(item.clone()),
                _ => {}
            }
        }
    }
    if content.is_empty() && tool_calls.is_empty() {
        return None;
    }
    *idx += 1;
    Some(VibeMessage {
        role,
        content: (!content.is_empty()).then_some(content),
        message_id: Some(format!("unified-{idx}")),
        tool_call_id: None,
        tool_calls,
    })
}

fn read_messages(path: &Path) -> Vec<VibeMessage> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let msg: VibeMessage = serde_json::from_str(l).ok()?;
            // Tool responses have tool_call_id but no message_id.
            (msg.message_id.is_some() || msg.tool_call_id.is_some()).then_some(msg)
        })
        .collect()
}

/// Load subagent sessions spawned by a parent session.
pub fn load_subagent_sessions(parent_session_id: &str) -> Vec<VibeSubagentSession> {
    let Some(parent) = find_session_dir(parent_session_id) else {
        return Vec::new();
    };
    let agents_dir = parent.join("agents");
    let Ok(entries) = std::fs::read_dir(&agents_dir) else {
        return Vec::new();
    };
    let mut subs: Vec<VibeSubagentSession> = entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let Ok(meta) = std::fs::read_to_string(dir.join("meta.json")) else {
                return None;
            };
            let value: serde_json::Value = serde_json::from_str(&meta).ok()?;
            let entry_name = entry.file_name().to_str().unwrap_or_default().to_string();
            let agent_name = value
                .pointer("/agent_profile/name")
                .and_then(|v| v.as_str())
                .unwrap_or(&entry_name)
                .to_string();
            Some(VibeSubagentSession {
                session_id: value
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                agent_name,
                start_time: value
                    .get("start_time")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                messages: read_messages(&dir.join("messages.jsonl")),
            })
        })
        .collect();
    subs.sort_by(|a, b| a.start_time.cmp(&b.start_time));
    subs
}

/// Totals across all sessions, for the Stats dashboard.
#[derive(Clone, Copy, Debug, Default)]
pub struct AggregateStats {
    pub sessions: usize,
    pub prompt_tokens: f64,
    pub completion_tokens: f64,
    pub total_llm_tokens: f64,
    pub cost: f64,
    pub tool_calls_succeeded: f64,
    pub tool_calls_failed: f64,
    pub tool_calls_rejected: f64,
}

pub fn aggregate(sessions: &[VibeSession]) -> AggregateStats {
    sessions.iter().fold(
        AggregateStats {
            sessions: sessions.len(),
            ..Default::default()
        },
        |acc, s| AggregateStats {
            sessions: acc.sessions,
            prompt_tokens: acc.prompt_tokens + s.stats.session_prompt_tokens,
            completion_tokens: acc.completion_tokens + s.stats.session_completion_tokens,
            total_llm_tokens: acc.total_llm_tokens + s.stats.session_total_llm_tokens,
            cost: acc.cost + s.stats.session_cost,
            tool_calls_succeeded: acc.tool_calls_succeeded + s.stats.tool_calls_succeeded,
            tool_calls_failed: acc.tool_calls_failed + s.stats.tool_calls_failed,
            tool_calls_rejected: acc.tool_calls_rejected + s.stats.tool_calls_rejected,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unified_transcript_from_chunks() {
        let dir = std::env::temp_dir().join(format!("vd-chunks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.json"),
            r#"[
              {"source":"history","turn_index":0,"message":{"role":"user","content":[{"type":"text","text":"hello"}]}},
              {"source":"history","turn_index":0,"message":{"role":"assistant","content":[{"type":"text","text":"running"},{"type":"tool_call","name":"bash","arguments":{"raw":"{}"}}]}},
              {"source":"history","turn_index":0,"message":{"role":"tool","content":[{"type":"text","text":"out"}]}}
            ]"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("b.json"),
            r#"[
              {"source":"history","turn_index":0,"message":{"role":"assistant","content":[{"type":"text","text":"running"}]}},
              {"source":"history","turn_index":1,"message":{"role":"user","content":[{"type":"text","text":"again"}]}},
              {"source":"history","turn_index":1,"message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
            ]"#,
        )
        .unwrap();
        let msgs = read_unified_transcript(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let roles: Vec<&str> = msgs.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "user", "assistant"]);
        assert_eq!(msgs[0].content.as_deref(), Some("hello"));
        assert_eq!(msgs[1].tool_calls.len(), 1);
        assert_eq!(msgs[1].content.as_deref(), Some("running"));
        assert_eq!(msgs[4].content.as_deref(), Some("done"));
    }

    fn sample_meta() -> &'static str {
        r#"{
            "session_id": "s-42",
            "start_time": "2026-10-01T10:00:00",
            "end_time": "2026-10-01T11:00:00",
            "title": "Refactor parser",
            "git_branch": "main",
            "environment": { "working_directory": "/home/kamil/proj" },
            "stats": {
                "steps": 12,
                "session_prompt_tokens": 1000.5,
                "session_completion_tokens": 2000.25,
                "tool_calls_succeeded": 8,
                "tool_calls_failed": 1,
                "session_total_llm_tokens": 3000.75,
                "session_cost": 0.42
            },
            "config": { "active_model": "mistral-medium-3.5" },
            "agent_profile": { "name": "default" }
        }"#
    }

    #[test]
    fn parses_meta_json() {
        let session: VibeSession = serde_json::from_str(sample_meta()).unwrap();
        assert_eq!(session.session_id, "s-42");
        assert_eq!(session.title.as_deref(), Some("Refactor parser"));
        assert_eq!(
            session.config.as_ref().unwrap().active_model.as_deref(),
            Some("mistral-medium-3.5")
        );
        assert_eq!(session.stats.session_cost, 0.42);
    }

    #[test]
    fn parses_messages_jsonl() {
        let dir = std::env::temp_dir().join("vibedeck-test-msgs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("messages.jsonl"),
            "{\"role\":\"user\",\"content\":\"hi\",\"message_id\":\"m1\"}\n{\"role\":\"tool\",\"content\":\"ok\",\"tool_call_id\":\"t1\"}\nnot json\n",
        )
        .unwrap();
        let messages = read_messages(&dir.join("messages.jsonl"));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].role, "tool");
    }

    #[test]
    fn aggregates() {
        let session: VibeSession = serde_json::from_str(sample_meta()).unwrap();
        let totals = aggregate(&[session]);
        assert_eq!(totals.sessions, 1);
        assert_eq!(totals.cost, 0.42);
        assert_eq!(totals.prompt_tokens, 1000.5);
        assert_eq!(totals.tool_calls_failed, 1.0);
    }
}
