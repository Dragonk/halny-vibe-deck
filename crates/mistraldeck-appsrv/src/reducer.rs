//! Reducer: project vibe-app-server notifications into the
//! GUI-facing [`AgentEvent`] stream + maintain the session mirror.
//!
//! Port of the reference `ClientProjection` semantics
//! (`vibe/app_server/events.py`) at the fidelity the GUI needs:
//! streaming text as deltas, tool-call effects, callbacks, stats,
//! handoff on compact/context-cleared, watermark tracking with a
//! full-resync trigger on a gap (docs/appserver-protocol.md §8–§10).

use std::collections::HashMap;

use serde_json::Value;

use mistraldeck_core::proto::AgentEvent;
use mistraldeck_core::types::{ToolCall, ToolStatus};
use mistraldeck_core::types::Message;

use crate::patch::{apply_all, PatchOp};
use crate::proto::{
    stats_numbers, status_str, HistoryEntry, SessionState, Turn,
};

/// Everything the worker keeps per connection.
#[derive(Default)]
pub struct Projection {
    /// Mirror of the app-server session document (the assistant
    /// history entries carry the growing text; patches mutate it).
    pub doc: Value,
    pub session_id: String,
    /// Watermark: last notification event id applied.
    pub last_event_id: i64,
    /// Server watermark from the latest mutating response (gap
    /// detection: if response last_event_id > last_event_id + 1 we
    /// missed notifications → caller must resync via session/read).
    pub response_watermark: i64,
    pub resync_needed: bool,
    /// Current turn (from turn/started; cleared on turn/completed).
    pub current_turn: Option<Turn>,
    /// Streaming deltas: last emitted text length per entry id.
    pub emitted_len: HashMap<String, usize>,
    /// callback_id → session_id (for callback/result routing).
    pub callback_sessions: HashMap<String, String>,
}

impl Projection {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply an authoritative snapshot (session/start, session/resume,
    /// session/read, session/snapshot). Replaces the mirror.
    pub fn apply_snapshot(&mut self, state: SessionState) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        self.last_event_id = state.event_id.unwrap_or(self.last_event_id);
        self.response_watermark = self.last_event_id;
        self.emitted_len.clear();
        if let Some(s) = &state.session {
            self.session_id = s.id.clone();
            events.push(AgentEvent::SessionReady {
                session_id: s.id.clone(),
                modes: Vec::new(),
                config_options: Vec::new(),
                title: s.title.clone(),
                current_mode_id: None,
            });
        }
        if let Some(entries) = &state.history {
            let msgs: Vec<Message> = entries.iter().map(entry_to_message).collect();
            events.push(AgentEvent::SessionHistory {
                session_id: self.session_id.clone(),
                messages: msgs,
                plan: Vec::new(),
            });
            // prime the delta lengths so live patches continue
            // (not restart) the stream
            for e in entries {
                if is_assistant(e) || e.kind == "reasoning" {
                    if let Some(id) = &e.id {
                        let len = if e.kind == "assistant" {
                            e.text_content().len()
                        } else {
                            e.text.clone().unwrap_or_default().len()
                        };
                        self.emitted_len.insert(id.clone(), len);
                    }
                }
            }
        }
        // pending callbacks survive reconnects
        if let Some(cbs) = &state.active_callbacks {
            for cb in cbs {
                if let Some(ev) = self.callback_event(cb) {
                    events.push(ev);
                }
            }
        }
        self.doc = serde_json::to_value(&state).unwrap_or(Value::Null);
        events
    }

    /// Map one notification (method + params) to zero or more
    /// GUI events. Unknown methods are ignored (tolerant).
    pub fn consume(&mut self, method: &str, params: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        match method {
            "session/snapshot" => {
                if let Ok(st) =
                    serde_json::from_value::<SessionState>(params.get("state").cloned().unwrap_or(Value::Null))
                {
                    events.extend(self.apply_snapshot(st));
                }
            }
            "session/updated" => {
                let ops: Vec<PatchOp> = params
                    .get("patch")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().cloned())
                    .unwrap_or_default()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                if apply_all(&mut self.doc["session"], &ops).is_err() {
                    self.resync_needed = true;
                }
            }
            "history/entryAdded" => {
                let Ok(entry) = serde_json::from_value::<HistoryEntry>(
                    params.get("entry").cloned().unwrap_or(Value::Null),
                ) else {
                    return events;
                };
                self.push_entry(&entry, &mut events);
            }
            "history/entryUpdated" => {
                let Some(entry_id) = params.get("entryId").and_then(Value::as_str) else {
                    return events;
                };
                let ops: Vec<PatchOp> = params
                    .get("patch")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().cloned())
                    .unwrap_or_default()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                let path = format!("/history/{}", self.index_of(entry_id));
                if apply_all(&mut self.doc, &ops).is_err() {
                    self.resync_needed = true;
                }
                let Some(entry) = self.entry_by_id(entry_id) else {
                    return events;
                };
                events.extend(self.deltas_for(&path, entry_id, &entry));
            }
            "turn/started" => {
                if let Ok(t) = serde_json::from_value::<Turn>(
                    params.get("turn").cloned().unwrap_or(Value::Null),
                ) {
                    self.current_turn = Some(t);
                }
            }
            "turn/completed" => {
                let t: Option<Turn> = serde_json::from_value(
                    params.get("turn").cloned().unwrap_or(Value::Null),
                )
                .ok();
                let stop_reason = t
                    .as_ref()
                    .and_then(|t| t.stop_reason.clone())
                    .unwrap_or_else(|| "completed".into());
                // L1: wire `turn.error` is `{code, message}` (plain
                // strings never occur) — decode tolerantly and emit
                // TurnFailed alongside PromptCompleted (the UI keeps
                // its stop_reason handling untouched).
                if let Some(err) = t.as_ref().and_then(|t| t.error.as_ref()) {
                    let (code, message) = match err {
                        Value::Object(m) => (
                            m.get("code").and_then(Value::as_str).unwrap_or_default(),
                            m.get("message").and_then(Value::as_str).unwrap_or_default(),
                        ),
                        Value::String(s) => ("", s.as_str()),
                        _ => ("", ""),
                    };
                    if !code.is_empty() {
                        events.push(AgentEvent::TurnFailed {
                            session_id: self.session_id.clone(),
                            code: code.to_string(),
                            message: message.to_string(),
                        });
                    }
                }
                self.current_turn = None;
                events.push(AgentEvent::PromptCompleted {
                    session_id: self.session_id.clone(),
                    stop_reason,
                });
            }
            "session/statsUpdated" => {
                let stats = params.get("stats");
                let (prompt, completion, cached, total, cost) = stats_numbers(stats);
                let ctx_used = usage_f64(
                    stats,
                    &["contextTokens", "context_tokens", "tokens"],
                );
                let ctx_size = params
                    .get("context_window")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                events.push(AgentEvent::UsageUpdate {
                    session_id: self.session_id.clone(),
                    prompt_tokens: prompt,
                    completion_tokens: completion,
                    cached_tokens: cached,
                    total_tokens: total,
                    tokens_per_second: usage_f64(
                        stats,
                        &["tokensPerSecond", "tokens_per_second"],
                    ),
                    last_turn_duration_ms: usage_f64(
                        stats,
                        &["lastTurnDurationMs", "last_turn_duration_ms"],
                    ),
                    context_used: ctx_used,
                    context_size: ctx_size,
                    cost,
                });
            }
            "session/compacted" | "session/contextCleared" => {
                // Handoff: new authoritative state under a NEW
                // session id — remap everything GUI-side by emitting
                // SessionReady + fresh history for the new id.
                if let Ok(st) = serde_json::from_value::<SessionState>(
                    params.get("state").cloned().unwrap_or(Value::Null),
                ) {
                    events.push(AgentEvent::DebugLog(format!(
                        "{method}: session handoff (compact/context clear)"
                    )));
                    events.extend(self.apply_snapshot(st));
                }
            }
            "turn/queueUpdated" => {
                // native queue — the GUI gets prompts queued while
                // busy for free; v1 logs it (queue UI arrives with F5)
                events.push(AgentEvent::DebugLog(
                    "app-server: turn queue updated".into(),
                ));
            }
            "runtime/updated" => {
                events.push(AgentEvent::DebugLog(
                    "app-server: runtime updated".into(),
                ));
            }
            "warning" => {
                if let Some(m) = params.get("message").and_then(Value::as_str) {
                    events.push(AgentEvent::DebugLog(format!(
                        "app-server warning: {m}"
                    )));
                }
            }
            _ => {}
        }
        // track watermark for every event-bearing notification
        if let Some(id) = params.get("eventId").and_then(Value::as_i64) {
            if id <= self.last_event_id {
                // out-of-order/replayed event → full resync
                self.resync_needed = true;
            } else {
                self.last_event_id = id;
            }
        }
        events
    }

    fn push_entry(&mut self, entry: &HistoryEntry, events: &mut Vec<AgentEvent>) {
        let id = entry.id.clone().unwrap_or_default();
        // mirror into doc.history
        let v = serde_json::to_value(entry).unwrap_or(Value::Null);
        if let Some(arr) = self.doc["history"].as_array_mut() {
            arr.push(v);
        }
        match entry.kind.as_str() {
            "message" if entry.role.as_deref() == Some("assistant") => {
                let text = entry.text_content();
                self.emitted_len.insert(id.clone(), text.len());
                if !text.is_empty() {
                    events.push(AgentEvent::MessageChunk {
                        session_id: self.session_id.clone(),
                        message_id: id,
                        text,
                    });
                }
            }
            "reasoning" => {
                let text = entry.text.clone().unwrap_or_default();
                self.emitted_len.insert(id.clone(), text.len());
                if !text.is_empty() {
                    events.push(AgentEvent::ThinkingChunk {
                        session_id: self.session_id.clone(),
                        block_id: id,
                        text,
                    });
                }
            }
            "message" if entry.role.as_deref() == Some("user") => {
                let text = entry.text_content();
                if !text.is_empty() {
                    events.push(AgentEvent::UserMessageChunk {
                        session_id: self.session_id.clone(),
                        text,
                    });
                }
            }
            "effect" => {
                events.extend(effect_events(
                    &self.session_id,
                    entry,
                    true,
                ));
            }
            "callback" => {
                if let Some(cb) = &entry.callback {
                    if let Ok(cbe) = serde_json::from_value::<
                        crate::proto::CallbackEntry,
                    >(cb.clone())
                    {
                        if let Some(ev) = self.callback_event(&cbe) {
                            events.push(ev);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Emit only the NEW text of an entry after a patch.
    fn deltas_for(
        &mut self,
        _path: &str,
        entry_id: &str,
        entry: &HistoryEntry,
    ) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        let known = self.emitted_len.get(entry_id).cloned().unwrap_or(0);
        match entry.kind.as_str() {
            "message" if entry.role.as_deref() == Some("assistant") => {
                let text = entry.text_content();
                if text.len() > known {
                    events.push(AgentEvent::MessageChunk {
                        session_id: self.session_id.clone(),
                        message_id: entry_id.to_string(),
                        text: text[known..].to_string(),
                    });
                }
                self.emitted_len
                    .insert(entry_id.to_string(), text.len());
            }
            "reasoning" => {
                let text = entry.text.clone().unwrap_or_default();
                if text.len() > known {
                    events.push(AgentEvent::ThinkingChunk {
                        session_id: self.session_id.clone(),
                        block_id: entry_id.to_string(),
                        text: text[known..].to_string(),
                    });
                }
                self.emitted_len
                    .insert(entry_id.to_string(), text.len());
            }
            "effect" => {
                events.extend(effect_events(
                    &self.session_id,
                    entry,
                    false,
                ));
            }
            _ => {}
        }
        events
    }

    fn entry_by_id(&self, id: &str) -> Option<HistoryEntry> {
        self.doc["history"]
            .as_array()?
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|e| serde_json::from_value(e.clone()).ok())
    }

    fn index_of(&self, id: &str) -> usize {
        self.doc["history"]
            .as_array()
            .and_then(|arr| {
                arr.iter().position(|e| {
                    e.get("id").and_then(Value::as_str) == Some(id)
                })
            })
            .unwrap_or(0)
    }

    /// Server→client callback → GUI approval/question event.
    pub(crate) fn callback_event(
        &mut self,
        cb: &crate::proto::CallbackEntry,
    ) -> Option<AgentEvent> {
        let callback_id = cb.callback_id.clone();
        self.callback_sessions
            .insert(callback_id.clone(), self.session_id.clone());
        match cb.detail_kind.as_deref() {
            Some("approval") => Some(AgentEvent::ToolApprovalRequest {
                request_id: callback_id,
                method: "callback/approval".into(),
                params_json: serde_json::to_string(
                    cb.detail.as_ref()?,
                )
                .ok()?,
                tool_name: cb.detail
                    .as_ref()
                    .and_then(|d| effect_tool_name(d)),
            }),
            Some("user_input") => {
                let detail = cb.detail.as_ref()?;
                let req = detail.get("request")?;
                let question = req
                    .get("question")
                    .or_else(|| req.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Question")
                    .to_string();
                let choices = req
                    .get("choices")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| {
                                c.as_str().map(str::to_string).or_else(|| {
                                    c.get("label")
                                        .and_then(Value::as_str)
                                        .map(str::to_string)
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(AgentEvent::AskUserQuestion {
                    request_id: callback_id,
                    question,
                    choices,
                })
            }
            _ => None,
        }
    }

    /// Gap check against a mutating response watermark.
    pub fn note_response_watermark(&mut self, last_event_id: i64) {
        self.response_watermark = last_event_id;
        if last_event_id > self.last_event_id {
            // responses carry the authoritative watermark; the
            // caller should resync (session/read) to catch missed
            // notifications
            self.resync_needed = true;
        }
    }
}

fn usage_f64(v: Option<&Value>, keys: &[&str]) -> f64 {
    let Some(v) = v else { return 0.0 };
    for k in keys {
        if let Some(n) = v.get(k).and_then(Value::as_f64) {
            return n;
        }
    }
    0.0
}

fn effect_tool_name(detail: &Value) -> Option<String> {
    Some(
        detail
            .get("toolName")
            .or_else(|| detail.get("tool_name"))
            .or_else(|| detail.get("title"))?
            .as_str()?
            .to_string(),
    )
}

/// Map an effect entry to ToolCallStarted/Updated.
fn effect_events(
    session_id: &str,
    entry: &HistoryEntry,
    started: bool,
) -> Vec<AgentEvent> {
    let id = entry
        .id
        .clone()
        .unwrap_or_else(|| entry.title.clone().unwrap_or_default());
    let state_str = status_str(entry.state.as_ref());
    let status = ToolStatus::normalize(&state_str);
    let title = entry.title.clone().unwrap_or_default();
    let detail = entry.detail.clone().unwrap_or(Value::Null);
    let detail_json = serde_json::to_string(&detail).ok();
    let tc = ToolCall {
        id: id.clone(),
        title,
        kind: "appserver-effect".into(),
        status,
        input_json: None,
        output: detail_json.clone(),
        file_changes: Vec::new(),
        diffs: Vec::new(),
        tool_name: effect_tool_name(&detail).unwrap_or_default(),
        previous_content: None,
        raw_output_json: None,
        subagent: None,
        started_at: 0,
    };
    vec![if started {
        AgentEvent::ToolCallStarted {
            session_id: session_id.to_string(),
            tool_call: tc,
        }
    } else {
        AgentEvent::ToolCallUpdated {
            session_id: session_id.to_string(),
            tool_call_id: id,
            status,
            output: detail_json,
            subagent: None,
        }
    }]
}

/// History entry → GUI message (snapshot/resume path).
/// wire kind + role → is this an assistant message entry?
fn is_assistant(entry: &HistoryEntry) -> bool {
    entry.kind == "message" && entry.role.as_deref() == Some("assistant")
}

pub fn entry_to_message(entry: &HistoryEntry) -> Message {
    let text = match entry.kind.as_str() {
        "message" if entry.role.as_deref() == Some("assistant") => entry.text_content(),
        "message" if entry.role.as_deref() == Some("user") => entry.text_content(),
        "reasoning" => entry.text.clone().unwrap_or_default(),
        "effect" => entry.title.clone().unwrap_or_default(),
        "notice" => entry.message.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let mut msg = if entry.kind == "message" && entry.role.as_deref() == Some("user") {
        Message::new_user(&text)
    } else {
        let mut m = Message::new_ai();
        m.content = text;
        m.is_complete = true;
        m
    };
    msg.id = entry.id.clone().unwrap_or(msg.id);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn proj() -> Projection {
        Projection::new()
    }

    fn snapshot_json(id: &str) -> Value {
        json!({
            "state": {
                "eventId": 5,
                "session": {"id": id, "title": "t", "status": {"type": "idle"}},
                "history": [],
                "activeCallbacks": []
            },
            "lastEventId": 5
        })
    }

    /// F1 AC: snapshot → SessionReady + SessionHistory, no replay buffering.
    #[test]
    fn snapshot_emits_ready_and_history() {
        let mut p = proj();
        let evs = p.consume("session/snapshot", &snapshot_json("s1"));
        assert!(evs
            .iter()
            .any(|e| matches!(e, AgentEvent::SessionReady { session_id, .. } if session_id == "s1")));
        assert_eq!(p.last_event_id, 5);
        assert_eq!(p.session_id, "s1");
    }

    /// F1 AC: assistant entry appears once, then grows by patch deltas —
    /// the GUI sees only the NEW text (streaming without duplicates).
    #[test]
    fn streaming_deltas() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let added = json!({
            "eventId": 6, "sessionId": "s1",
            "entry": {"type": "message", "role": "assistant", "id": "e1",
                      "content": [{"type": "text", "text": "Hello"}]}
        });
        let evs = p.consume("history/entryAdded", &added);
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::MessageChunk { text, .. } if text == "Hello")));

        let updated = json!({
            "eventId": 7, "sessionId": "s1", "entryId": "e1",
            "patch": [{"op": "append", "path": "/history/0/content/0/text",
                       "value": " world"}]
        });
        let evs = p.consume("history/entryUpdated", &updated);
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::MessageChunk { text, .. } if text == " world")));
        // a patch that does NOT grow the text emits no delta (append
        // semantics: a repeated append really extends the text)
        let evs = p.consume("history/entryUpdated", &json!({
            "eventId": 8, "sessionId": "s1", "entryId": "e1",
            "patch": [{"op": "replace", "path": "/history/0/content/0/text",
                       "value": "Hello world"}]
        }));
        assert!(evs.iter().all(|e| !matches!(e, AgentEvent::MessageChunk { .. })));
    }

    /// Reasoning streams as ThinkingChunks.
    #[test]
    fn reasoning_deltas() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        p.consume("history/entryAdded", &json!({
            "eventId": 6, "entry": {"type": "reasoning", "id": "r1", "text": "thin"}
        }));
        let evs = p.consume("history/entryUpdated", &json!({
            "eventId": 7, "entryId": "r1",
            "patch": [{"op": "append", "path": "/history/0/text", "value": "king"}]
        }));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::ThinkingChunk { text, .. } if text == "king")));
    }

    /// Turn lifecycle: turn/started arms the interrupt target, turn/completed
    /// emits PromptCompleted and clears it (W9).
    #[test]
    fn turn_lifecycle() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        p.consume("turn/started", &json!({
            "eventId": 6, "turn": {"id": "t1", "status": {"type": "running"}}}));
        assert_eq!(p.current_turn.as_ref().unwrap().id.as_deref(), Some("t1"));
        let evs = p.consume("turn/completed", &json!({
            "eventId": 7, "turn": {"id": "t1", "status": {"type": "completed"},
                                   "stopReason": "end_turn"}}));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::PromptCompleted { stop_reason, .. } if stop_reason == "end_turn")));
    }

    /// L1: wire `turn.error` is an object `{code, message}` — the
    /// rate-limit case must emit TurnFailed alongside PromptCompleted.
    #[test]
    fn turn_error_emits_turn_failed() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("turn/completed", &json!({
            "eventId": 7, "turn": {"id": "t1", "status": {"type": "failed"},
                                   "stopReason": "error",
                                   "error": {"code": "rate_limit", "message": "Limit reached"}}}));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::TurnFailed { code, message, .. }
            if code == "rate_limit" && message == "Limit reached")));
        // stop_reason still reaches the UI (no regression).
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::PromptCompleted { stop_reason, .. } if stop_reason == "error")));
        // A turn without an error emits no TurnFailed.
        let evs = p.consume("turn/completed", &json!({
            "eventId": 8, "turn": {"id": "t2", "status": {"type": "completed"},
                                   "stopReason": "end_turn"}}));
        assert!(evs.iter().all(|e| !matches!(e, AgentEvent::TurnFailed { .. })));
        assert!(p.current_turn.is_none());
    }

    /// F6 AC: statsUpdated maps to UsageUpdate without touching the
    /// unified store (W4/W7).
    #[test]
    fn stats_update() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("session/statsUpdated", &json!({
            "eventId": 6, "stats": {"promptTokens": 10, "completionTokens": 20,
                                    "totalTokens": 30, "costUsd": 0.5},
            "context_window": 128000
        }));
        match &evs[0] {
            AgentEvent::UsageUpdate {
                prompt_tokens, completion_tokens, total_tokens, cost,
                context_size, ..
            } => {
                assert_eq!((*prompt_tokens, *completion_tokens, *total_tokens), (10.0, 20.0, 30.0));
                assert_eq!(*cost, 0.5);
                assert_eq!(*context_size, 128000.0);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// F5 AC: compact handoff — the session continues under a NEW id and
    /// the projection emits SessionReady for it (engine remaps by id).
    #[test]
    fn compact_handoff_remaps_session() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("session/compacted", &json!({
            "eventId": 9, "oldSessionId": "s1",
            "state": {"eventId": 9, "session": {"id": "s2", "status": {"type": "idle"}}},
            "summaryLength": 100
        }));
        assert_eq!(p.session_id, "s2");
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::SessionReady { session_id, .. } if session_id == "s2")));
    }

    /// F3 AC: a gap/replayed event_id (stale cursor) flips resync_needed —
    /// the worker then resyncs via session/resume (authoritative snapshot).
    #[test]
    fn stale_event_triggers_resync() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        assert!(!p.resync_needed);
        p.consume("history/entryAdded", &json!({
            "eventId": 3, "entry": {"type": "notice", "message": "replayed"}
        }));
        assert!(p.resync_needed);
    }

    /// F2 AC: server callbacks surface as approval/question events and the
    /// callback→session mapping is recorded for callback/result routing.
    #[test]
    fn callbacks_map_to_ui_events() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("history/entryAdded", &json!({
            "eventId": 6,
            "entry": {"type": "callback", "id": "cb-entry",
                      "callback": {"callbackId": "cb-1", "kind": "approval",
                                   "detail": {"toolName": "bash", "choices": ["approve", "deny"]}}}
        }));
        match &evs[0] {
            AgentEvent::ToolApprovalRequest { request_id, tool_name, .. } => {
                assert_eq!(request_id, "cb-1");
                assert_eq!(tool_name.as_deref(), Some("bash"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(p.callback_sessions.get("cb-1").map(String::as_str), Some("s1"));

        let evs = p.consume("history/entryAdded", &json!({
            "eventId": 7,
            "entry": {"type": "callback",
                      "callback": {"callbackId": "cb-2", "kind": "user_input",
                                   "detail": {"request": {"question": "Continue?",
                                                           "choices": [{"label": "Yes"}]}}}}
        }));
        match &evs[0] {
            AgentEvent::AskUserQuestion { request_id, question, choices } => {
                assert_eq!(request_id, "cb-2");
                assert_eq!(question, "Continue?");
                assert_eq!(choices, &vec!["Yes".to_string()]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Response watermark above the last seen event id also flags a gap
    /// (the response boundary — docs §10).
    #[test]
    fn response_watermark_gap() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        p.note_response_watermark(20);
        assert!(p.resync_needed);
    }
}
