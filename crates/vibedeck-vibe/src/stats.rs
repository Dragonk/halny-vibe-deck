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
    /// All models used across generations (unified layout; Nexus
    /// `loadUnifiedSessionModels`). Empty for classic sessions.
    #[serde(default)]
    pub models: Vec<String>,
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
/// - vibe-acp >= 3.0: `logs/session/unified/<id>/meta.json` (no `stats`
///   block - token metrics are augmented from
///   `generations/<gen>/projection-state.json` and costs from the
///   local price table, like Nexus `loadUnifiedSessionUsage`).
pub fn load_sessions() -> Vec<VibeSession> {
    let dir = sessions_log_dir();
    let prices = crate::config::model_prices();
    let mut sessions: Vec<VibeSession> = Vec::new();
    // classic layout
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || entry.file_name().to_str() == Some("unified") {
                continue;
            }
            if let Some(mut s) = load_meta(&path.join("meta.json")) {
                // Nexus: recompute the cost when the legacy log carries
                // no pricing info (zero cost with real prompt tokens).
                if s.stats.session_cost == 0.0 && s.stats.session_prompt_tokens > 0.0 {
                    let model = s.config.as_ref().and_then(|c| c.active_model.clone());
                    s.stats.session_cost = session_cost_from_prices(
                        &prices,
                        model.as_deref(),
                        s.stats.session_prompt_tokens,
                        s.stats.cached_tokens,
                        s.stats.session_completion_tokens,
                    );
                }
                sessions.push(s);
            }
        }
    }
    // unified layout (newer CLI): token/cost stats live in
    // generations/<gen>/projection-state.json, not meta.json
    let unified = dir.join("unified");
    if let Ok(entries) = std::fs::read_dir(&unified) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(mut s) = load_meta(&path.join("meta.json")) {
                if let Some(u) = load_unified_usage(&path) {
                    let model = s.config.as_ref().and_then(|c| c.active_model.clone());
                    s.stats.steps = u.tool_calls as f64;
                    s.stats.session_prompt_tokens = u.input_tokens;
                    s.stats.cached_tokens = u.cached_tokens;
                    s.stats.session_completion_tokens = u.output_tokens;
                    s.stats.tool_calls_agreed = u.tool_calls as f64;
                    s.stats.tool_calls_succeeded = u.tool_calls as f64;
                    s.stats.context_tokens = u.context_tokens;
                    s.stats.session_total_llm_tokens = u.total_tokens;
                    s.total_messages = Some(u.message_count);
                    s.stats.session_cost = session_cost_from_prices(
                        &prices,
                        model.as_deref(),
                        u.input_tokens,
                        u.cached_tokens,
                        u.output_tokens,
                    );
                }
                s.models = load_unified_models(
                    &path,
                    s.config.as_ref().and_then(|c| c.active_model.as_deref()),
                );
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

/// The newest generation of a unified session (Nexus `latestGeneration`):
/// the `CURRENT` file when present, else the highest numbered
/// `generations/<N>/` directory.
fn latest_generation(session_dir: &Path) -> Option<String> {
    if let Ok(raw) = std::fs::read_to_string(session_dir.join("CURRENT")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(gen) = v.get("generation").and_then(|g| g.as_str()) {
                return Some(gen.to_string());
            }
        }
    }
    let mut best: Option<u64> = None;
    for entry in std::fs::read_dir(session_dir.join("generations")).ok()?.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(n) = name.parse::<u64>() {
                if best.map_or(true, |b| n > b) {
                    best = Some(n);
                }
            }
        }
    }
    best.map(|n| n.to_string())
}

/// Token/message/tool counts of a unified session (Nexus
/// `loadUnifiedSessionUsage`): usage lives in
/// `generations/<gen>/projection-state.json` under
/// `snapshot.session.tokenUsage`, not in `meta.json`.
struct UnifiedUsage {
    input_tokens: f64,
    cached_tokens: f64,
    output_tokens: f64,
    total_tokens: f64,
    context_tokens: f64,
    tool_calls: u64,
    message_count: u64,
}

fn load_unified_usage(session_dir: &Path) -> Option<UnifiedUsage> {
    let gen = latest_generation(session_dir)?;
    let raw = std::fs::read_to_string(
        session_dir
            .join("generations")
            .join(&gen)
            .join("projection-state.json"),
    )
    .ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let usage = v
        .pointer("/snapshot/session/tokenUsage")
        .cloned()
        .unwrap_or_default();
    let get = |k: &str| usage.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let input = get("inputTokens");
    let cached = get("cachedInputTokens");
    let output = get("outputTokens");
    let total = get("totalTokens");
    let total = if total > 0.0 { total } else { input + output };
    let context = v
        .pointer("/snapshot/session/contextUsage/totalTokens")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    let count_dir = |name: &str| {
        std::fs::read_dir(session_dir.join(name))
            .map(|d| d.count() as u64)
            .unwrap_or(0)
    };
    Some(UnifiedUsage {
        input_tokens: input,
        cached_tokens: cached,
        output_tokens: output,
        total_tokens: total,
        context_tokens: if context > 0.0 { context } else { input + output },
        tool_calls: count_dir("tool-results"),
        message_count: count_dir("chunks"),
    })
}

/// All models used across the generations of a unified session (Nexus
/// `loadUnifiedSessionModels`), from `runtime-state.json ->
/// session_metadata.active_model`; falls back to the meta model.
fn load_unified_models(session_dir: &Path, fallback: Option<&str>) -> Vec<String> {
    let mut used: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(session_dir.join("generations")) {
        let mut gens: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        gens.sort();
        for gen in gens {
            let Ok(raw) = std::fs::read_to_string(
                session_dir.join("generations").join(&gen).join("runtime-state.json"),
            ) else {
                continue;
            };
            let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
                continue;
            };
            if let Some(model) = state
                .pointer("/session_metadata/active_model")
                .and_then(|m| m.as_str())
            {
                if !used.iter().any(|u| u == model) {
                    used.push(model.to_string());
                }
            }
        }
    }
    if used.is_empty() {
        if let Some(f) = fallback {
            used.push(f.to_string());
        }
    }
    used
}

/// Session cost from the local price table (Nexus `computeSessionCost`
/// via `modelPriceFor`); 0 when the model has no price entry.
pub fn session_cost_from_prices(
    prices: &[crate::config::ModelPrice],
    model: Option<&str>,
    prompt_tokens: f64,
    cached_tokens: f64,
    completion_tokens: f64,
) -> f64 {
    match model {
        Some(m) => crate::config::estimate_cost(prices, m, prompt_tokens, completion_tokens, cached_tokens),
        None => 0.0,
    }
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

// --- v0.13 session preview (subagent "Show work") -------------------------

/// One thinking block of a preview message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewThinking {
    pub id: String,
    pub content: String,
}

/// One tool call line of a preview message (`status`: `done` | `error`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewToolCall {
    pub title: String,
    pub status: String,
}

/// One message of a session preview (Nexus `SubagentWorkData.messages`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub thinking: Vec<PreviewThinking>,
    #[serde(default)]
    pub tool_calls: Vec<PreviewToolCall>,
}

const PREVIEW_MAX_CHUNKS: usize = 48;
const PREVIEW_MAX_MESSAGES: usize = 2000;

/// Locate the unified session dir for an id. Also accepts child sessions
/// (`unified/child-<id>/` — those carry no `meta.json`, so the directory
/// itself is the only marker).
fn find_preview_dir(session_id: &str) -> Option<PathBuf> {
    let base = sessions_log_dir().join("unified");
    let stripped = session_id.strip_prefix("child-").unwrap_or(session_id);
    for name in [
        session_id.to_string(),
        format!("child-{session_id}"),
        stripped.to_string(),
    ] {
        let dir = base.join(name);
        if dir.join("CURRENT").is_file() || dir.join("chunks").is_dir() {
            return Some(dir);
        }
    }
    None
}

/// Read the raw projection entries from the session's chunk files (Nexus
/// `readUnifiedChunks`): `CURRENT` → generation → `manifest.json` →
/// `projection_state.chunks` (last 48 shas) → `chunks/<sha>.json`. Falls
/// back to reading every `chunks/*.json` when the manifest is unavailable.
fn read_preview_entries(dir: &Path) -> Option<Vec<serde_json::Value>> {
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let via_manifest = (|| -> Option<Vec<String>> {
        let cur: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("CURRENT")).ok()?).ok()?;
        let gen = cur.get("generation").and_then(|g| g.as_str()).unwrap_or("");
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(
                dir.join("generations").join(gen).join("manifest.json"),
            )
            .ok()?,
        )
        .ok()?;
        let list = manifest
            .pointer("/projection_state/chunks")?
            .as_array()?
        .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
        Some(list)
    })();
    if let Some(mut shas) = via_manifest {
        if shas.len() > PREVIEW_MAX_CHUNKS {
            let skip = shas.len() - PREVIEW_MAX_CHUNKS;
            shas.drain(..skip);
        }
        for sha in &shas {
            if let Ok(raw) =
                std::fs::read_to_string(dir.join("chunks").join(format!("{sha}.json")))
            {
                if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
                    entries.extend(list);
                }
            }
        }
        return Some(entries);
    }
    // Fallback: every chunk file, sorted for a stable order.
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("chunks")).ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    if files.is_empty() {
        return None;
    }
    for path in files {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
                entries.extend(list);
            }
        }
    }
    Some(entries)
}

/// Build the UI-friendly history from raw projection entries (Nexus
/// `buildSessionPreview`, trimmed to what the subagent work card shows:
/// user/ai text, thinking blocks, tool call lines).
fn build_preview_messages(entries: &[serde_json::Value]) -> Vec<PreviewMessage> {
    let mut messages: Vec<PreviewMessage> = Vec::new();
    let mut cur_ai: Option<PreviewMessage> = None;
    let mut pending_thinking: Vec<PreviewThinking> = Vec::new();
    for e in entries {
        if messages.len() > PREVIEW_MAX_MESSAGES {
            break;
        }
        match e.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                let is_user = e.get("role").and_then(|r| r.as_str()) == Some("user");
                let mut text = String::new();
                if let Some(blocks) = e.get("content").and_then(|c| c.as_array()) {
                    for b in blocks {
                        if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                text.push_str(t);
                            }
                        }
                    }
                }
                if is_user {
                    if let Some(msg) = cur_ai.take() {
                        messages.push(msg);
                    }
                    pending_thinking.clear();
                    messages.push(PreviewMessage {
                        role: "user".to_string(),
                        content: text,
                        thinking: Vec::new(),
                        tool_calls: Vec::new(),
                    });
                } else {
                    if let Some(msg) = cur_ai.take() {
                        messages.push(msg);
                    }
                    let thinking = std::mem::take(&mut pending_thinking);
                    cur_ai = Some(PreviewMessage {
                        role: "ai".to_string(),
                        content: text,
                        thinking,
                        tool_calls: Vec::new(),
                    });
                }
            }
            Some("reasoning") => {
                let block = PreviewThinking {
                    id: e
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("reasoning")
                        .to_string(),
                    content: e
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                };
                match &mut cur_ai {
                    Some(ai) => ai.thinking.push(block),
                    None => pending_thinking.push(block),
                }
            }
            Some("effect") => {
                if cur_ai.is_none() {
                    let thinking = std::mem::take(&mut pending_thinking);
                    cur_ai = Some(PreviewMessage {
                        role: "ai".to_string(),
                        content: String::new(),
                        thinking,
                        tool_calls: Vec::new(),
                    });
                }
                let det = e.get("detail");
                let display = det.and_then(|d| d.get("display"));
                let state = e.get("state");
                let decision = state
                    .and_then(|s| s.get("decision"))
                    .and_then(|d| d.as_str());
                let success = display
                    .and_then(|d| d.get("success"))
                    .and_then(|s| s.as_bool())
                    != Some(false)
                    && matches!(decision, None | Some("execute"));
                let title = display
                    .and_then(|d| d.get("summary"))
                    .and_then(|s| s.as_str())
                    .or_else(|| {
                        det.and_then(|d| d.get("toolName"))
                            .and_then(|s| s.as_str())
                    })
                    .unwrap_or("tool");
                if let Some(ai) = &mut cur_ai {
                    ai.tool_calls.push(PreviewToolCall {
                        title: title.to_string(),
                        status: if success { "done" } else { "error" }.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(msg) = cur_ai.take() {
        messages.push(msg);
    }
    messages
}

/// Session preview for the subagent "Show work" card (Nexus
/// `loadSessionPreview`): the child session transcript as UI-friendly
/// messages. `None` when the session (or its chunks) cannot be found.
pub fn load_session_preview(session_id: &str) -> Option<Vec<PreviewMessage>> {
    let dir = find_preview_dir(session_id)?;
    let entries = read_preview_entries(&dir)?;
    Some(build_preview_messages(&entries))
}

/// Last config actually used by a session (Nexus `loadSessionLastConfig`):
/// the newest generation of `unified/<id>/generations/*/runtime-state.json`
/// carrying `session_metadata.active_model` wins; `agent_name` is the
/// mode, `reasoning_effort` the thinking level.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionLastConfig {
    pub model: Option<String>,
    pub mode: Option<String>,
    pub thinking: Option<String>,
}

pub fn load_session_last_config(session_id: &str) -> Option<SessionLastConfig> {
    let stripped = session_id.strip_prefix("child-").unwrap_or(session_id);
    let generations_dir = sessions_log_dir()
        .join("unified")
        .join(stripped)
        .join("generations");
    let mut gens: Vec<String> = std::fs::read_dir(&generations_dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    gens.sort();
    gens.reverse();
    for gen in gens {
        let raw = std::fs::read_to_string(generations_dir.join(&gen).join("runtime-state.json"))
            .ok()?;
        let Ok(state) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let Some(meta) = state.get("session_metadata") else {
            continue;
        };
        let model = meta
            .get("active_model")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(model) = model else {
            continue;
        };
        return Some(SessionLastConfig {
            model: Some(model),
            mode: meta
                .get("agent_name")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            thinking: meta
                .get("reasoning_effort")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        });
    }
    None
}

// --- v0.12 lazy resume preview (Nexus buildSessionPreview, §2.2) -----------

/// One entry of the session plan recovered from the unified store
/// (`vibe.todo` effects — the last write wins).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewPlanEntry {
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
}

/// Usage snapshot recovered from `.session-index.json` (Nexus
/// `readSessionIndexEntry.usage`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PreviewUsage {
    pub context_used: f64,
    pub prompt_tokens: f64,
    pub completion_tokens: f64,
    pub cached_tokens: f64,
    pub total_tokens: f64,
}

impl PreviewUsage {
    /// True when any token figure exists (Nexus: `usage` only when the
    /// index entry carries token totals).
    pub fn is_known(&self) -> bool {
        self.total_tokens > 0.0
            || self.prompt_tokens > 0.0
            || self.completion_tokens > 0.0
            || self.cached_tokens > 0.0
            || self.context_used > 0.0
    }
}

/// Everything the lazy-resume preview needs (Nexus `SessionPreview`):
/// transcript + plan + title + usage + last config — all read from
/// disk, no agent process involved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionPreviewData {
    pub messages: Vec<PreviewMessage>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub updated_at: Option<i64>,
    pub config: Option<SessionLastConfig>,
    pub usage: PreviewUsage,
    pub plan: Vec<PreviewPlanEntry>,
}

/// Read one entry from `unified/.session-index.json` (Nexus
/// `readSessionIndexEntry`): title/updatedAt from the session block,
/// cwd + config from the metadata block, usage from the token totals.
fn read_session_index_entry_at(
    index_path: &Path,
    session_id: &str,
) -> Option<(Option<String>, Option<String>, Option<i64>, Option<SessionLastConfig>, PreviewUsage)> {
    let raw = std::fs::read_to_string(index_path).ok()?;
    let idx: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let entries = idx.get("entries")?.as_array()?;
    let entry = entries.iter().find(|e| {
        e.pointer("/identity/session_id").and_then(|v| v.as_str()) == Some(session_id)
            || e.get("session_id").and_then(|v| v.as_str()) == Some(session_id)
    })?;
    let sess = entry.get("session");
    let meta = entry.get("metadata");
    let title = sess
        .and_then(|s| s.get("title"))
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_string);
    let updated_at = sess
        .and_then(|s| s.get("updatedAt"))
        .and_then(|v| v.as_i64())
        .or_else(|| {
            sess.and_then(|s| s.get("updatedAt"))
                .and_then(|v| v.as_f64())
                .map(|f| f as i64)
        });
    let config = meta.and_then(|m| {
        let model = m.get("active_model").and_then(|v| v.as_str());
        if model.is_none() {
            return None;
        }
        Some(SessionLastConfig {
            model: model.map(str::to_string),
            mode: m.get("agent_name").and_then(|v| v.as_str()).map(str::to_string),
            thinking: m
                .get("reasoning_effort")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    });
    let num = |v: Option<&serde_json::Value>| v.and_then(|x| x.as_f64()).unwrap_or(0.0);
    let tok = || sess.and_then(|s| s.get("tokenUsage"));
    let usage = PreviewUsage {
        context_used: num(sess.and_then(|s| s.get("contextUsage")).and_then(|c| c.get("totalTokens"))),
        prompt_tokens: num(tok().and_then(|t| t.get("inputTokens"))),
        completion_tokens: num(tok().and_then(|t| t.get("outputTokens"))),
        cached_tokens: num(tok().and_then(|t| t.get("cachedInputTokens"))),
        total_tokens: num(tok().and_then(|t| t.get("totalTokens"))),
    };
    let cwd = meta
        .and_then(|m| m.get("cwd"))
        .and_then(|v| v.as_str())
        .filter(|c| !c.is_empty())
        .map(str::to_string);
    Some((title, cwd, updated_at, config, if usage.is_known() { usage } else { PreviewUsage::default() }))
}

/// The todo list recovered from the last `vibe.todo` effect in the
/// projection entries (Nexus `buildSessionPreview.plan`): todos come
/// from the tool input first, then from the structured output.
fn build_preview_plan(entries: &[serde_json::Value]) -> Vec<PreviewPlanEntry> {
    let mut plan: Vec<PreviewPlanEntry> = Vec::new();
    for e in entries {
        if e.get("type").and_then(|t| t.as_str()) != Some("effect") {
            continue;
        }
        let Some(det) = e.get("detail") else { continue };
        if det.get("toolName").and_then(|t| t.as_str()) != Some("vibe.todo") {
            continue;
        }
        // A cancelled/rejected write must not clobber the last good list.
        let decision = e
            .pointer("/state/decision")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        if matches!(decision, "reject" | "cancelled" | "cancel") {
            continue;
        }
        let todos = det
            .get("input")
            .and_then(|i| i.get("todos"))
            .and_then(|t| t.as_array())
            .or_else(|| {
                e.pointer("/state/output/structured_content/todos")
                    .and_then(|t| t.as_array())
            });
        let Some(todos) = todos else { continue };
        let parsed: Vec<PreviewPlanEntry> = todos
            .iter()
            .filter_map(|t| {
                let content = t.get("content").and_then(|c| c.as_str())?;
                if content.is_empty() {
                    return None;
                }
                Some(PreviewPlanEntry {
                    content: content.to_string(),
                    status: t
                        .get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("pending")
                        .to_string(),
                    priority: t
                        .get("priority")
                        .and_then(|p| p.as_str())
                        .map(str::to_string),
                })
            })
            .collect();
        if !parsed.is_empty() {
            plan = parsed; // the last write wins
        }
    }
    plan
}

/// Lazy-resume session preview (Nexus `previewSession`): the full
/// transcript, plan, title and usage read straight from the unified
/// store — no agent process spawned. `None` when the session (or its
/// chunks) cannot be found on disk.
pub fn load_full_session_preview(session_id: &str) -> Option<SessionPreviewData> {
    let dir = find_preview_dir(session_id)?;
    let entries = read_preview_entries(&dir)?;
    let (title, cwd, updated_at, config, usage) = read_session_index_entry_at(
        &sessions_log_dir().join("unified").join(".session-index.json"),
        session_id,
    )
    .unwrap_or((None, None, None, None, PreviewUsage::default()));
    let mut messages = build_preview_messages(&entries);
    messages.truncate(PREVIEW_MAX_MESSAGES);
    Some(SessionPreviewData {
        messages,
        title,
        cwd,
        updated_at,
        config: config.or_else(|| load_session_last_config(session_id)),
        usage,
        plan: build_preview_plan(&entries),
    })
}

// ---------------------------------------------------------------------------
// History content search (Nexus `searchSessionContent`)
// ---------------------------------------------------------------------------

/// One content-search hit: the session id plus a short snippet around the
/// first match (+/-70 chars, whitespace collapsed, capped at 200 chars).
#[derive(Clone, Debug, PartialEq)]
pub struct ContentMatch {
    pub session_id: String,
    pub snippet: String,
}

/// First-match snippet for `q` inside `text` (case-insensitive): +/-70 chars
/// around the first hit, whitespace collapsed, max 200 chars. Byte offsets
/// refer to the lowercased text; when lowercasing changed the string's
/// shape the original slice can miss char boundaries - fall back to the
/// lowercased slice instead of panicking.
fn content_snippet(text: &str, q: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let idx = lower.find(q)?;
    let from = idx.saturating_sub(70);
    let to = (idx + q.len() + 70).min(lower.len());
    let raw = text.get(from..to).or_else(|| lower.get(from..to)).unwrap_or("");
    let mut snippet = String::new();
    for word in raw.split_whitespace() {
        if !snippet.is_empty() {
            snippet.push(' ');
        }
        snippet.push_str(word);
    }
    Some(snippet.chars().take(200).collect())
}

/// Search every session transcript for `query` (case-insensitive): unified
/// chunk snapshots first, then legacy `messages.jsonl` logs - the first hit
/// per session wins (Nexus `searchSessionContent`).
pub fn search_session_content(query: &str) -> Vec<ContentMatch> {
    search_session_content_in(&sessions_log_dir(), query)
}

fn search_session_content_in(base: &Path, query: &str) -> Vec<ContentMatch> {
    let q = query.to_lowercase();
    let mut out: Vec<ContentMatch> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if q.is_empty() {
        return out;
    }
    // Defensive: chunk snapshots are small, but skip anything huge.
    const MAX_FILE: u64 = 16 * 1024 * 1024;

    // Unified store: <base>/unified/<id>/chunks/*
    let unified = base.join("unified");
    let mut unified_dirs: Vec<PathBuf> = std::fs::read_dir(&unified)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    unified_dirs.sort();
    for dir in unified_dirs {
        let session_id = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let chunks = dir.join("chunks");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&chunks)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for f in files {
            if f.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            if let Some(snippet) = content_snippet(&text, &q) {
                if seen.insert(session_id.clone()) {
                    out.push(ContentMatch {
                        session_id: session_id.clone(),
                        snippet,
                    });
                }
                break;
            }
        }
    }

    // Legacy store: <base>/<entry>/messages.jsonl
    let mut classic_dirs: Vec<PathBuf> = std::fs::read_dir(base)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && *p != unified)
        .collect();
    classic_dirs.sort();
    for dir in classic_dirs {
        let msg_path = dir.join("messages.jsonl");
        if !msg_path.is_file() || msg_path.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE {
            continue;
        }
        let session_id = std::fs::read_to_string(dir.join("meta.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| {
                v.get("session_id")
                    .and_then(|sid| sid.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        if seen.contains(&session_id) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&msg_path) else { continue };
        if let Some(snippet) = content_snippet(&text, &q) {
            if seen.insert(session_id.clone()) {
                out.push(ContentMatch { session_id, snippet });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_search_snippets_across_layouts() {
        let base = std::env::temp_dir().join(format!("vd-content-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // Classic layout: session id comes from meta.json.
        let classic = base.join("sess-a");
        std::fs::create_dir_all(&classic).unwrap();
        std::fs::write(classic.join("meta.json"), r#"{"session_id":"abc-123"}"#).unwrap();
        std::fs::write(
            classic.join("messages.jsonl"),
            r#"{"message_id":"m1","role":"user","content":[{"type":"text","text":"fix the gearbox issue now"}]}"#,
        )
        .unwrap();
        // Unified layout: chunk snapshot with messy whitespace.
        let unified = base.join("unified").join("u-42");
        std::fs::create_dir_all(unified.join("chunks")).unwrap();
        std::fs::write(unified.join("meta.json"), r#"{"session_id":"u-42"}"#).unwrap();
        std::fs::write(
            unified.join("chunks").join("a.json"),
            r#"[{"role":"user","content":[{"type":"text","text":"needle in   the   haystack"}]}]"#,
        )
        .unwrap();
        // Junk without messages.jsonl must be skipped.
        std::fs::create_dir_all(base.join("junk")).unwrap();

        let matches = search_session_content_in(&base, "gearbox");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].session_id, "abc-123");
        assert!(matches[0].snippet.contains("gearbox"));
        assert!(matches[0].snippet.chars().count() <= 200);

        let matches = search_session_content_in(&base, "needle");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].session_id, "u-42");
        assert!(matches[0].snippet.contains("needle in the haystack"));

        assert!(search_session_content_in(&base, "").is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }
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

    #[test]
    fn preview_messages_from_raw_entries() {
        let entries: Vec<serde_json::Value> = serde_json::json!([
            { "type": "message", "role": "user", "content": [
                { "type": "text", "text": "fix the bug" }
            ] },
            { "type": "reasoning", "id": "r1", "text": "thinking about it" },
            { "type": "message", "role": "assistant", "content": [
                { "type": "text", "text": "on it" }
            ] },
            { "type": "reasoning", "id": "r2", "text": "more thought" },
            { "type": "effect", "detail": { "toolName": "bash", "display": {
                "summary": "bash: ls", "success": true
            } }, "state": { "decision": "execute" } },
            { "type": "effect", "detail": { "toolName": "edit", "display": {
                "summary": "edit: x.rs"
            } }, "state": { "decision": "reject" } },
            { "type": "message", "role": "assistant", "content": [
                { "type": "text", "text": "done" }
            ] }
        ])
        .as_array()
        .unwrap()
        .clone();
        let msgs = build_preview_messages(&entries);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "fix the bug");
        assert_eq!(msgs[1].role, "ai");
        assert_eq!(msgs[1].content, "on it");
        assert_eq!(msgs[1].thinking.len(), 2);
        assert_eq!(msgs[1].thinking[1].content, "more thought");
        assert_eq!(msgs[1].tool_calls.len(), 2);
        assert_eq!(msgs[1].tool_calls[0].status, "done");
        assert_eq!(msgs[1].tool_calls[1].status, "error");
        assert_eq!(msgs[2].content, "done");
        assert!(msgs[2].thinking.is_empty());
    }

    #[test]
    fn reads_session_index_entry() {
        let dir = std::env::temp_dir().join(format!("vd-idx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let index = r#"{
            "catalog_version": 1,
            "entries": [
              {
                "identity": { "session_id": "s-77", "kind": "root" },
                "metadata": {
                  "active_model": "glm-5-3-flash",
                  "agent_name": "auto-approve",
                  "cwd": "/home/k/proj",
                  "reasoning_effort": "low"
                },
                "session": {
                  "id": "s-77",
                  "title": "Inboxora security audit",
                  "updatedAt": 1792865238183,
                  "contextUsage": { "cachedInputTokens": 82944, "inputTokens": 87592, "outputTokens": 290, "totalTokens": 87882 },
                  "tokenUsage": { "cachedInputTokens": 1889280, "inputTokens": 2150303, "outputTokens": 9548, "totalTokens": 2159851 }
                },
                "session_id": "s-77"
              }
            ]
        }"#;
        let path = dir.join(".session-index.json");
        std::fs::write(&path, index).unwrap();
        let (title, cwd, updated_at, config, usage) =
            read_session_index_entry_at(&path, "s-77").expect("entry exists");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(title.as_deref(), Some("Inboxora security audit"));
        assert_eq!(cwd.as_deref(), Some("/home/k/proj"));
        assert_eq!(updated_at, Some(1792865238183));
        let config = config.expect("config present");
        assert_eq!(config.model.as_deref(), Some("glm-5-3-flash"));
        assert_eq!(config.mode.as_deref(), Some("auto-approve"));
        assert_eq!(config.thinking.as_deref(), Some("low"));
        assert_eq!(usage.prompt_tokens, 2150303.0);
        assert_eq!(usage.completion_tokens, 9548.0);
        assert_eq!(usage.cached_tokens, 1889280.0);
        assert_eq!(usage.total_tokens, 2159851.0);
        assert_eq!(usage.context_used, 87882.0);
    }

    #[test]
    fn index_entry_without_tokens_gives_unknown_usage() {
        let dir = std::env::temp_dir().join(format!("vd-idx2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".session-index.json");
        std::fs::write(
            &path,
            r#"{ "entries": [ { "identity": { "session_id": "s-1" },
                "session": { "id": "s-1", "title": "T" } } ] }"#,
        )
        .unwrap();
        let (_, _, _, config, usage) =
            read_session_index_entry_at(&path, "s-1").expect("entry exists");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(config.is_none());
        assert!(!usage.is_known());
    }

    #[test]
    fn latest_generation_reads_current_file() {
        let dir = std::env::temp_dir().join("vd-gen-current");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("generations/1")).unwrap();
        std::fs::create_dir_all(dir.join("generations/3")).unwrap();
        std::fs::write(dir.join("CURRENT"), r#"{"generation":"3"}"#).unwrap();
        assert_eq!(latest_generation(&dir).as_deref(), Some("3"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_generation_falls_back_to_max_numeric() {
        let dir = std::env::temp_dir().join("vd-gen-max");
        let _ = std::fs::remove_dir_all(&dir);
        for g in ["1", "2", "10"] {
            std::fs::create_dir_all(dir.join("generations").join(g)).unwrap();
        }
        assert_eq!(latest_generation(&dir).as_deref(), Some("10"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unified_usage_from_projection_state() {
        let dir = std::env::temp_dir().join("vd-usage");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("generations/2")).unwrap();
        std::fs::write(
            dir.join("generations/2/projection-state.json"),
            r#"{"snapshot":{"session":{"tokenUsage":{"inputTokens":2150303,"cachedInputTokens":1889280,"outputTokens":9548,"totalTokens":2159851},"contextUsage":{"totalTokens":120000}}}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("tool-results")).unwrap();
        std::fs::write(dir.join("tool-results/a.json"), "{}").unwrap();
        std::fs::write(dir.join("tool-results/b.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.join("chunks")).unwrap();
        for i in 0..3 {
            std::fs::write(dir.join("chunks").join(i.to_string()), "{}").unwrap();
        }
        let u = load_unified_usage(&dir).unwrap();
        assert_eq!(u.input_tokens, 2150303.0);
        assert_eq!(u.cached_tokens, 1889280.0);
        assert_eq!(u.output_tokens, 9548.0);
        assert_eq!(u.total_tokens, 2159851.0);
        assert_eq!(u.context_tokens, 120000.0);
        assert_eq!(u.tool_calls, 2);
        assert_eq!(u.message_count, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unified_usage_total_falls_back_to_in_plus_out() {
        let dir = std::env::temp_dir().join("vd-usage-fb");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("generations/1")).unwrap();
        std::fs::write(
            dir.join("generations/1/projection-state.json"),
            r#"{"snapshot":{"session":{"tokenUsage":{"inputTokens":100,"outputTokens":50}}}}"#,
        )
        .unwrap();
        let u = load_unified_usage(&dir).unwrap();
        assert_eq!(u.total_tokens, 150.0);
        assert_eq!(u.context_tokens, 150.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unified_models_across_generations() {
        let dir = std::env::temp_dir().join("vd-models");
        let _ = std::fs::remove_dir_all(&dir);
        for (gen, model) in [("1", "a-model"), ("2", "b-model")] {
            std::fs::create_dir_all(dir.join("generations").join(gen)).unwrap();
            std::fs::write(
                dir.join("generations").join(gen).join("runtime-state.json"),
                format!(r#"{{"session_metadata":{{"active_model":"{model}"}}}}"#),
            )
            .unwrap();
        }
        let models = load_unified_models(&dir, None);
        assert_eq!(models, vec!["a-model", "b-model"]);
        assert_eq!(
            load_unified_models(&dir, Some("fallback")),
            vec!["a-model", "b-model"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn preview_plan_last_todo_write_wins() {
        let entries: Vec<serde_json::Value> = serde_json::json!([
            { "type": "effect", "detail": { "toolName": "bash", "kind": "tool" } },
            { "type": "effect", "detail": { "toolName": "vibe.todo", "kind": "tool",
                "input": { "action": "write", "todos": [
                    { "content": "first", "status": "completed", "priority": "high" },
                    { "content": "second", "status": "in_progress" }
                ] } },
              "state": { "decision": "execute" } },
            { "type": "effect", "detail": { "toolName": "vibe.todo", "kind": "tool",
                "input": { "action": "write", "todos": [
                    { "content": "third", "status": "pending" }
                ] } },
              "state": { "decision": "reject" } },
            { "type": "effect", "detail": { "toolName": "vibe.todo", "kind": "tool",
                "input": { "action": "write", "todos": [
                    { "content": "final", "status": "pending", "priority": "medium" },
                    { "content": "", "status": "pending" }
                ] } },
              "state": { "decision": "execute" } }
        ])
        .as_array()
        .unwrap()
        .clone();
        let plan = build_preview_plan(&entries);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].content, "final");
        assert_eq!(plan[0].status, "pending");
        assert_eq!(plan[0].priority.as_deref(), Some("medium"));
    }
}
