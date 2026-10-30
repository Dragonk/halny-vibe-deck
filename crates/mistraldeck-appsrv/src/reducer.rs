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
    /// round 8: callback_id → the question texts of a user_input request, so
    /// the answer can be wrapped in the wire's UserQuestionResult shape
    /// (`answers: [{question, answer, isOther}]`)
    pub callback_questions: HashMap<String, Vec<String>>,
    /// PID of our own agent child process — a lease held by this PID is
    /// OURS, not "another process" (the "open elsewhere" badge).
    pub own_pid: Option<u32>,
}

impl Projection {
    pub fn new() -> Self {
        Self::default()
    }

    /// `apply_snapshot` from a raw `state` value (fork/rewind responses).
    pub fn apply_snapshot_value(&mut self, v: Value) -> Vec<AgentEvent> {
        match serde_json::from_value::<SessionState>(v) {
            Ok(st) => self.apply_snapshot(st),
            Err(_) => Vec::new(),
        }
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
            let child_sessions = state
                .child_sessions
                .as_ref()
                .map(|cs| cs.iter().map(|c| c.to_string()).collect())
                .unwrap_or_default();
            events.push(AgentEvent::SessionReady {
                session_id: s.id.clone(),
                modes: Vec::new(),
                config_options: Vec::new(),
                title: s.title.clone(),
                current_mode_id: None,
                child_sessions,
            });
        }
        if let Some(entries) = &state.history {
            let msgs = entries_to_messages(entries);
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
                        // round 8: assistant entries have kind "message" (never
                        // "assistant"), so the old inner check primed them with
                        // len 0 and the next entryUpdated re-emitted the FULL
                        // text as a delta (duplicated assistant bubbles)
                        let len = if is_assistant(e) {
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
        // Session-level token usage rides on the snapshot
        // (`session.tokenUsage` / `session.contextUsage`, wire TokenUsage:
        // inputTokens/outputTokens/totalTokens/cachedInputTokens) — emit it
        // as a UsageUpdate so a resync/reconnect restores the usage bar
        // instead of blanking it (user-reported round 4, E12).
        if let Some(s) = &state.session {
            if let Some(tu) = &s.token_usage {
                let (prompt, completion, cached, total, _cost) = stats_numbers(Some(tu));
                let ctx_used = s
                    .context_usage
                    .as_ref()
                    .map(|c| {
                        usage_f64(Some(c), &["inputTokens", "input_tokens", "totalTokens", "total_tokens"])
                    })
                    .unwrap_or(0.0);
                let cost = {
                    let prices = mistraldeck_vibe::config::model_prices();
                    if prices.is_empty() {
                        0.0
                    } else {
                        mistraldeck_vibe::stats::session_cost_from_prices(
                            &prices,
                            s.model.as_deref(),
                            prompt,
                            cached,
                            completion,
                        )
                    }
                };
                if total > 0.0 || prompt > 0.0 || completion > 0.0 {
                    events.push(AgentEvent::UsageUpdate {
                        session_id: s.id.clone(),
                        prompt_tokens: prompt,
                        completion_tokens: completion,
                        cached_tokens: cached,
                        total_tokens: if total > 0.0 { total } else { prompt + completion },
                        tokens_per_second: 0.0,
                        last_turn_duration_ms: 0.0,
                        context_used: ctx_used,
                        context_size: 0.0,
                        cost,
                    });
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
                let raw_ops: Vec<PatchOp> = params
                    .get("patch")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().cloned())
                    .unwrap_or_default()
                    .filter_map(|v| serde_json::from_value(v).ok())
                    .collect();
                // Round 6 (R8): the wire patch ops are ENTRY-RELATIVE
                // (`/text`, `/content/0/text`) — prefix them with the
                // entry's path in the mirror doc, otherwise they never land
                // and reasoning deltas (thinking) are lost.
                // round 8: an unknown entryId must NOT fall back to index 0 —
                // the entry-relative ops would corrupt the FIRST history
                // entry. Flag a resync instead.
                let Some(idx) = self.index_of(entry_id) else {
                    self.resync_needed = true;
                    return events;
                };
                let base = format!("/history/{idx}");
                let ops: Vec<PatchOp> = raw_ops
                    .into_iter()
                    .map(|mut op| {
                        if op.path.starts_with("/history/") {
                            op
                        } else if op.path.is_empty() || op.path == "/" {
                            op.path = base.clone();
                            op
                        } else {
                            op.path = format!("{base}{}", op.path);
                            op
                        }
                    })
                    .collect();
                if apply_all(&mut self.doc, &ops).is_err() {
                    self.resync_needed = true;
                }
                let Some(entry) = self.entry_by_id(entry_id) else {
                    return events;
                };
                events.extend(self.deltas_for(&base, entry_id, &entry));
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
                let (mut prompt, mut completion, mut cached, mut total, mut cost) =
                    stats_numbers(stats);
                // v1.99.9: the CLI reports subagent child-session usage
                // separately from the parent's stats — roll the child
                // sessions' token totals into the usage bar.
                let sub = mistraldeck_vibe::stats::subagent_usage_totals(&self.session_id);
                prompt += sub.usage.prompt_tokens;
                completion += sub.usage.completion_tokens;
                cached += sub.usage.cached_tokens;
                total += sub.usage.total_tokens;
                cost += sub.cost;
                let ctx_used = usage_f64(
                    stats,
                    &["contextTokens", "context_tokens", "tokens"],
                );
                // Wire: `contextWindow` (StatsUpdatedParams.context_window,
                // camelCase alias on the wire).
                let ctx_size = params
                    .get("contextWindow")
                    .or_else(|| params.get("context_window"))
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
                    // Wire: `lastTurnDuration` is a float of SECONDS
                    // (AgentStatsSnapshot.last_turn_duration) — the GUI
                    // wants milliseconds.
                    last_turn_duration_ms: {
                        let ms = usage_f64(
                            stats,
                            &["lastTurnDurationMs", "last_turn_duration_ms"],
                        );
                        if ms > 0.0 {
                            ms
                        } else {
                            usage_f64(stats, &["lastTurnDuration", "last_turn_duration"]) * 1000.0
                        }
                    },
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
                // native queue — the GUI queues locally and drains on turn
                // end. Round 11 regression: forcing a full session/resume
                // here stormed the server on every queue update (slow
                // streaming, busy conflicts mid-turn) — log only.
                events.push(AgentEvent::DebugLog(
                    "app-server: turn queue updated".into(),
                ));
            }
            "runtime/updated" => {
                // Round 11 regression: a resync per runtime/updated stormed
                // the server the same way — the periodic snapshot already
                // picks runtime changes up; log only.
                events.push(AgentEvent::DebugLog(
                    "app-server: runtime updated".into(),
                ));
            }
            "turn/retrying" => {
                // TUI shows a "retrying" banner from the snapshot's
                // retrying state — resync so the client sees it
                self.resync_needed = true;
            }
            "session/childSessionUpdated" => {
                // subagent (child session) status change — forward raw so the
                // GUI's subagent panel stays live
                if let Some(child) = params.get("childSession") {
                    events.push(AgentEvent::ChildSessionUpdated {
                        session_id: self.session_id.clone(),
                        child_json: child.to_string(),
                    });
                }
            }
            "vibeCode/teleport/event" => {
                // teleport progress (summarizing_context → checking_git →
                // push_required → pushing → starting_workflow → complete|failed)
                if let Some(ev) = params.get("event") {
                    events.push(AgentEvent::TeleportProgress {
                        session_id: self.session_id.clone(),
                        event_json: ev.to_string(),
                    });
                }
            }
            "artifacts/created" | "artifacts/updated" => {
                events.push(AgentEvent::ArtifactsChanged {
                    session_id: self.session_id.clone(),
                });
            }
            "session/lease/updated" | "session/lease/ended" => {
                // wire: SessionLeaseUpdate {sessionId, leaseHolder|null};
                // "ended" carries no holder → released. A holder whose
                // processId is OUR OWN agent child is not "elsewhere".
                let holder_pid = params
                    .get("leaseHolder")
                    .and_then(|h| h.get("processId").or_else(|| h.get("process_id")))
                    .and_then(Value::as_u64);
                // show "open elsewhere" ONLY when the holder PID is known and
                // differs from our own agent child — a null/unknown PID can't
                // be proven to be another process (round 12: the badge lit up
                // on every session start because of this)
                let held = method == "session/lease/updated"
                    && params.get("leaseHolder").map(|h| !h.is_null()).unwrap_or(false)
                    && match (holder_pid, self.own_pid) {
                        (Some(hp), Some(own)) => hp != own as u64,
                        (Some(_), None) => true,
                        _ => false,
                    };
                events.push(AgentEvent::LeaseUpdated {
                    session_id: self.session_id.clone(),
                    held_elsewhere: held,
                });
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

    fn index_of(&self, id: &str) -> Option<usize> {
        self.doc["history"]
            .as_array()
            .and_then(|arr| {
                arr.iter().position(|e| {
                    e.get("id").and_then(Value::as_str) == Some(id)
                })
            })
    }

    /// Server→client callback → GUI approval/question event.
    pub(crate) fn callback_event(
        &mut self,
        cb: &crate::proto::CallbackEntry,
    ) -> Option<AgentEvent> {
        let callback_id = cb.callback_id.clone();
        self.callback_sessions
            .insert(callback_id.clone(), self.session_id.clone());
        match cb.effective_kind().as_deref() {
            Some("approval") => Some(AgentEvent::ToolApprovalRequest {
                request_id: callback_id,
                method: "callback/approval".into(),
                params_json: serde_json::to_string(
                    cb.detail.as_ref()?,
                )
                .ok()?,
                tool_name: cb.detail.as_ref().and_then(|d| effect_tool_name(d)),
            }),
            Some("user_input") => {
                let detail = cb.detail.as_ref()?;
                let req = detail.get("request")?;
                // round 8: the wire is `request.questions[]` (each with
                // `question` and `options[].label`), not a flat
                // `question`/`choices` pair — join all questions and collect
                // all options so multi-question forms render fully
                let questions = req.get("questions").and_then(Value::as_array);
                let question_texts: Vec<String> = match questions {
                    Some(arr) if !arr.is_empty() => arr
                        .iter()
                        .filter_map(|q| {
                            q.get("question")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                if !question_texts.is_empty() {
                    self.callback_questions
                        .insert(callback_id.clone(), question_texts.clone());
                }
                let question = if !question_texts.is_empty() {
                    question_texts.join("\n")
                } else {
                    req.get("question")
                        .or_else(|| req.get("title"))
                        .and_then(Value::as_str)
                        .unwrap_or("Question")
                        .to_string()
                };
                let mut choices: Vec<String> = Vec::new();
                if let Some(arr) = questions {
                    for q in arr {
                        if let Some(opts) = q.get("options").and_then(Value::as_array) {
                            for o in opts {
                                if let Some(l) = o.as_str().map(str::to_string).or_else(|| {
                                    o.get("label").and_then(Value::as_str).map(str::to_string)
                                }) {
                                    if !choices.contains(&l) {
                                        choices.push(l);
                                    }
                                }
                            }
                        }
                    }
                }
                if choices.is_empty() {
                    if let Some(arr) = req.get("choices").and_then(Value::as_array) {
                        choices = arr
                            .iter()
                            .filter_map(|c| {
                                c.as_str().map(str::to_string).or_else(|| {
                                    c.get("label")
                                        .and_then(Value::as_str)
                                        .map(str::to_string)
                                })
                            })
                            .collect();
                    }
                }
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
    // round 8: approval details nest the tool under `effect.toolName`
    // (ApprovalCallbackDetail.effect); accept the flat shapes too
    detail
        .get("effect")
        .and_then(|e| {
            e.get("toolName")
                .or_else(|| e.get("tool_name"))
                .and_then(Value::as_str)
        })
        .or_else(|| detail.get("toolName").and_then(Value::as_str))
        .or_else(|| detail.get("tool_name").and_then(Value::as_str))
        .or_else(|| detail.get("title").and_then(Value::as_str))
        .map(str::to_string)
}

/// Build the GUI [`ToolCall`] for one `effect` history entry (shared by the
/// live stream and the snapshot/resume grouping).
///
/// Wire shape (vibe/app_server/_effect_models.py, models.py): the entry's
/// `detail` carries `{toolName, kind, display: {summary, verb, message,
/// settledVerb, settledMessage, statusText, content, suffix}, input}` and the
/// entry's `state` carries `{status, output, outputText/output_text,
/// display}`. Round 5 (R13): without this mapping a resumed session rendered
/// the raw effect JSON instead of the rich tool card.
fn effect_tool_call(entry: &HistoryEntry) -> ToolCall {
    let id = entry
        .id
        .clone()
        .unwrap_or_else(|| entry.title.clone().unwrap_or_default());
    let status = ToolStatus::normalize(&status_str(entry.state.as_ref()));
    let detail = entry.detail.clone().unwrap_or(Value::Null);
    let display = detail.get("display").cloned().unwrap_or(Value::Null);
    let str_field = |v: &Value, keys: &[&str]| -> String {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string()
    };
    let tool_name_raw = str_field(&detail, &["toolName", "tool_name"]);
    let tool_name = if tool_name_raw.is_empty() {
        entry.title.clone().unwrap_or_default()
    } else {
        tool_name_raw
    };
    let kind = str_field(&detail, &["kind"]);
    let title_raw = str_field(&display, &["summary", "message", "verb"]);
    let title = if title_raw.is_empty() {
        entry.title.clone().unwrap_or_default()
    } else {
        title_raw
    };
    let input_json = detail
        .get("input")
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::to_string(v).ok());
    // output: structured `state.output` when present, else the text fields.
    // Round 7 (R9): some servers mirror the whole effect DETAIL into
    // `state.output` (keys toolName/kind/input/display) — that is not tool
    // output; fall back to the plain text fields instead of rendering it.
    let state = entry.state.clone().unwrap_or(Value::Null);
    let output = state
        .get("output")
        .filter(|v| !v.is_null())
        .filter(|v| {
            !(v.get("toolName").is_some()
                && v.get("input").is_some()
                && v.get("display").is_some())
        })
        .and_then(|v| serde_json::to_string(v).ok())
        .or_else(|| {
            let t = str_field(&state, &["outputText", "output_text"]);
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        });
    let subagent = detail
        .get("childSessionId")
        .or_else(|| detail.get("child_session_id"))
        .and_then(Value::as_str)
        .map(|cid| mistraldeck_core::types::SubagentInfo {
            agent: detail
                .get("agent")
                .and_then(Value::as_str)
                .map(str::to_string),
            agent_name: detail
                .get("agentName")
                .or_else(|| detail.get("agent_name"))
                .and_then(Value::as_str)
                .map(str::to_string),
            task: detail
                .get("task")
                .or_else(|| detail.get("prompt"))
                .and_then(Value::as_str)
                .map(str::to_string),
            child_session_id: Some(cid.to_string()),
        });
    // Round 6 (R5): file_edit effects carry git-style diffs — build
    // DiffHunks from state.output.occurrences (or the old/new string pair)
    // so the chat renders the edit as a diff block, not raw JSON.
    let (diffs, previous_content) = if kind == "file_edit" {
        let out = state.get("output");
        let path = detail
            .pointer("/input/filePath")
            .or_else(|| detail.pointer("/input/file_path"))
            .and_then(Value::as_str)
            .or_else(|| out.and_then(|o| o.get("file")).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        let mut hunks = Vec::new();
        if let Some(occs) = out.and_then(|o| o.get("occurrences")).and_then(Value::as_array) {
            for o in occs {
                let get_str = |keys: &[&str]| -> Option<String> {
                    keys.iter().find_map(|k| o.get(*k).and_then(Value::as_str)).map(str::to_string)
                };
                let get_i64 = |keys: &[&str]| -> Option<i64> {
                    keys.iter().find_map(|k| o.get(*k).and_then(Value::as_i64))
                };
                hunks.push(mistraldeck_core::types::DiffHunk {
                    path: path.clone(),
                    old_text: get_str(&["oldText", "old_text"]),
                    new_text: get_str(&["newText", "new_text"]).unwrap_or_default(),
                    start_line: get_i64(&["startLine", "start_line"]),
                    new_start_line: get_i64(&["newStartLine", "new_start_line"]),
                });
            }
        }
        if hunks.is_empty() {
            let old = out.and_then(|o| o.get("oldString")).and_then(Value::as_str);
            let new = out.and_then(|o| o.get("newString")).and_then(Value::as_str);
            if let (Some(old), Some(new)) = (old, new) {
                hunks.push(mistraldeck_core::types::DiffHunk {
                    path: path.clone(),
                    old_text: Some(old.to_string()),
                    new_text: new.to_string(),
                    start_line: None,
                    new_start_line: None,
                });
            }
        }
        let prev = out
            .and_then(|o| o.get("previousContent"))
            .and_then(Value::as_str)
            .map(str::to_string);
        (hunks, prev)
    } else {
        (Vec::new(), None)
    };
    ToolCall {
        id,
        title,
        kind: if kind.is_empty() { "appserver-effect".into() } else { kind },
        status,
        input_json,
        output,
        file_changes: Vec::new(),
        diffs,
        tool_name,
        previous_content,
        raw_output_json: None,
        subagent,
        started_at: 0,
    }
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
    let tc = effect_tool_call(entry);
    let status = tc.status;
    let output = tc.output.clone();
    let subagent = tc.subagent.clone();
    let mut events = Vec::new();
    // round 8: todo_write effects feed the plan card — PlanUpdate had no
    // producer at all, so the todo list never rendered live
    if entry
        .detail
        .as_ref()
        .and_then(|d| d.get("kind"))
        .and_then(Value::as_str)
        == Some("todo")
    {
        let todos = entry
            .detail
            .as_ref()
            .and_then(|d| d.get("input"))
            .and_then(|i| i.get("todos"))
            .or_else(|| {
                entry
                    .state
                    .as_ref()
                    .and_then(|s| s.get("output"))
                    .and_then(|o| o.get("todos"))
            });
        if let Some(todos) = todos {
            if let Ok(entries_json) = serde_json::to_string(todos) {
                events.push(AgentEvent::PlanUpdate {
                    session_id: session_id.to_string(),
                    entries_json,
                });
            }
        }
    }
    events.push(if started {
        AgentEvent::ToolCallStarted {
            session_id: session_id.to_string(),
            tool_call: tc,
        }
    } else {
        AgentEvent::ToolCallUpdated {
            session_id: session_id.to_string(),
            tool_call_id: id,
            status,
            output,
            subagent,
        }
    });
    events
}

/// History entry → GUI message (snapshot/resume path).
/// wire kind + role → is this an assistant message entry?
fn is_assistant(entry: &HistoryEntry) -> bool {
    entry.kind == "message" && entry.role.as_deref() == Some("assistant")
}

/// Group wire history entries into GUI messages (snapshot / resume path).
///
/// Wire layout (vibe/app_server/_history_projection.py): per turn one user
/// `message` entry, then per assistant LLMMessage a `reasoning` entry
/// (thinking), an assistant `message` entry (text) and one `effect` entry
/// per tool call (the tool result updates the same effect's `state`).
/// `notice` entries are system lines. Reasoning buffered before an
/// assistant text belongs to that text's message; effects attach to the
/// last assistant message — so a resumed session keeps thinking blocks and
/// tool calls instead of flattening them to plain text (round 4, E5).
pub fn entries_to_messages(entries: &[HistoryEntry]) -> Vec<Message> {
    use mistraldeck_core::types::{MessageSender, ThinkingBlock};
    let mut out: Vec<Message> = Vec::new();
    let mut pending_thinking: Vec<ThinkingBlock> = Vec::new();
    for e in entries {
        match e.kind.as_str() {
            "message" if e.role.as_deref() == Some("user") => {
                pending_thinking.clear();
                let mut m = Message::new_user(&e.text_content());
                m.id = e.id.clone().unwrap_or(m.id);
                out.push(m);
            }
            "message" if e.role.as_deref() == Some("assistant") => {
                let mut m = Message::new_ai();
                m.content = e.text_content();
                m.is_complete = true;
                m.id = e.id.clone().unwrap_or(m.id);
                m.thinking = std::mem::take(&mut pending_thinking);
                out.push(m);
            }
            "message" if e.role.as_deref() == Some("system") => {
                pending_thinking.clear();
                let mut m = Message::new_system(&e.text_content());
                m.id = e.id.clone().unwrap_or(m.id);
                out.push(m);
            }
            "reasoning" => {
                let text = e.text.clone().unwrap_or_default();
                if text.is_empty() {
                    continue;
                }
                pending_thinking.push(ThinkingBlock {
                    id: e
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("t-{}", pending_thinking.len())),
                    content: text,
                    is_complete: true,
                    started_at: 0,
                    completed_at: 0,
                });
            }
            "effect" => {
                // attach to the last AI message (create one if the turn had
                // tool calls but no assistant text yet)
                let needs_new = !matches!(
                    out.last(),
                    Some(m) if m.sender == MessageSender::Ai
                );
                if needs_new {
                    out.push(Message::new_ai());
                    out.last_mut().unwrap().is_complete = true;
                }
                let msg = out.last_mut().unwrap();
                let tc = effect_tool_call(e);
                match msg.tool_calls.iter_mut().find(|t| t.id == tc.id) {
                    Some(existing) => *existing = tc,
                    None => msg.tool_calls.push(tc),
                }
            }
            "notice" => {
                pending_thinking.clear();
                let text = e.message.clone().unwrap_or_default();
                if text.is_empty() {
                    continue;
                }
                let mut m = Message::new_system(&text);
                m.id = e.id.clone().unwrap_or(m.id);
                out.push(m);
            }
            _ => {}
        }
    }
    // trailing reasoning with no following assistant text still deserves a bubble
    if !pending_thinking.is_empty() {
        let mut m = Message::new_ai();
        m.is_complete = true;
        m.thinking = pending_thinking;
        out.push(m);
    }
    out
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

    /// Exact sequence the e2e stub agent emits for one turn: snapshot →
    /// turn/started → entryAdded (empty text) → entryUpdated patches →
    /// turn/completed. Every step must produce the event the UI needs, and
    /// the session id of the snapshot must be the one chunks carry.
    #[test]
    fn stub_agent_turn_sequence_produces_streamed_chunks() {
        let mut p = proj();
        let snapshot = json!({
            "state": {
                "eventId": 0,
                "session": {"id": "s1", "title": "e2e", "status": "idle", "cwd": "/tmp"},
                "history": [],
                "turns": [],
                "isQuiescent": true,
                "activeCallbacks": []
            },
            "lastEventId": 0
        });
        let ready = p.apply_snapshot(
            serde_json::from_value(snapshot["state"].clone()).expect("session state"),
        );
        assert!(
            ready
                .iter()
                .any(|e| matches!(e, AgentEvent::SessionReady { session_id, .. } if session_id == "s1")),
            "snapshot must emit SessionReady for the agent session id"
        );

        p.consume(
            "turn/started",
            &json!({"sessionId": "s1", "turn": {"id": "t1", "status": "running"}}),
        );
        assert_eq!(p.current_turn.as_ref().and_then(|t| t.id.clone()).as_deref(), Some("t1"));

        p.consume(
            "history/entryAdded",
            &json!({"sessionId": "s1", "entry": {"type": "message", "role": "assistant",
                     "id": "e1", "turnId": "t1",
                     "content": [{"type": "text", "text": ""}]}}),
        );
        let updated = p.consume(
            "history/entryUpdated",
            &json!({"sessionId": "s1", "entryId": "e1", "patch": [{
                "op": "replace", "path": "/history/0/content",
                "value": [{"type": "text", "text": "Working on it. "}]
            }]}),
        );
        assert!(
            updated
                .iter()
                .any(|e| matches!(e, AgentEvent::MessageChunk { session_id, text, .. }
                                  if session_id == "s1" && text == "Working on it. ")),
            "entryUpdated must stream the text for the same session id, got {updated:?}"
        );
    }

    /// The e2e stub agent (scripts/e2e/stub-agent.py) streams a reply by
    /// replacing an assistant entry's content through `history/entryUpdated`
    /// json-patch ops. This test pins that shape: only the appended part
    /// becomes a MessageChunk (no duplicated text).
    #[test]
    fn entry_update_patch_emits_only_the_delta() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let added = p.consume(
            "history/entryAdded",
            &json!({"entry": {"type": "message", "role": "assistant", "id": "e1",
                              "content": [{"type": "text", "text": "Working"}]}}),
        );
        assert!(added
            .iter()
            .any(|e| matches!(e, AgentEvent::MessageChunk { text, .. } if text == "Working")));

        let evs = p.consume(
            "history/entryUpdated",
            &json!({
                "entryId": "e1",
                "patch": [{
                    "op": "replace",
                    "path": "/history/0/content",
                    "value": [{"type": "text", "text": "Working on it."}],
                }],
            }),
        );
        let deltas: Vec<String> = evs
            .iter()
            .filter_map(|e| match e {
                AgentEvent::MessageChunk { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec![" on it.".to_string()]);
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

    /// Round 4 (E5): a resumed session's snapshot history must keep
    /// reasoning as thinking blocks and effects as tool calls — not
    /// flatten them to plain assistant text.
    #[test]
    fn snapshot_history_groups_reasoning_and_effects() {
        let entries = vec![
            json!({"type": "message", "role": "user", "id": "u1",
                   "content": [{"type": "text", "text": "hi"}]}),
            json!({"type": "reasoning", "id": "r1", "text": "thinking hard"}),
            json!({"type": "message", "role": "assistant", "id": "a1",
                   "content": [{"type": "text", "text": "answer"}]}),
            json!({"type": "effect", "id": "e1", "title": "bash",
                   "detail": {"toolName": "bash", "output": "ok"},
                   "state": {"type": "completed"}}),
            json!({"type": "notice", "id": "n1", "message": "note"}),
        ];
        let list: Vec<HistoryEntry> = entries
            .iter()
            .filter_map(|e| serde_json::from_value(e.clone()).ok())
            .collect();
        let msgs = entries_to_messages(&list);
        assert_eq!(msgs.len(), 3, "{msgs:?}");
        assert_eq!(msgs[0].content, "hi");
        assert_eq!(msgs[1].content, "answer");
        assert_eq!(msgs[1].thinking.len(), 1);
        assert_eq!(msgs[1].thinking[0].content, "thinking hard");
        assert!(msgs[1].thinking[0].is_complete);
        assert_eq!(msgs[1].tool_calls.len(), 1);
        assert_eq!(msgs[1].tool_calls[0].tool_name, "bash");
        assert!(matches!(msgs[1].tool_calls[0].status, ToolStatus::Done));
        assert_eq!(msgs[2].content, "note");
    }

    /// Round 5 (R13): a wire `effect` entry must map to a rich ToolCall —
    /// tool_name from detail.toolName, title from display.summary, input
    /// from detail.input, output from state.output, status from
    /// state.status — not the raw detail JSON (that rendered as raw JSON in
    /// resumed sessions).
    #[test]
    fn effect_entry_maps_to_rich_tool_call() {
        let entry: HistoryEntry = serde_json::from_value(json!({
            "type": "effect",
            "id": "e1",
            "title": "file_system.bash",
            "detail": {
                "toolName": "file_system.bash",
                "kind": "shell",
                "display": {"summary": "ls Astrafox", "verb": "Running", "statusText": "Running command"},
                "input": {"command": "ls Astrafox"}
            },
            "state": {
                "status": "completed",
                "output": {"stdout": "a.txt\nb.txt", "stderr": "", "output": "a.txt\nb.txt"},
                "outputText": "a.txt\nb.txt"
            }
        }))
        .unwrap();
        let tc = effect_tool_call(&entry);
        assert_eq!(tc.tool_name, "file_system.bash");
        assert_eq!(tc.kind, "shell");
        assert_eq!(tc.title, "ls Astrafox");
        assert!(matches!(tc.status, ToolStatus::Done));
        assert_eq!(tc.input_json.as_deref().unwrap(), "{\"command\":\"ls Astrafox\"}");
        let out = tc.output.clone().unwrap();
        assert!(out.contains("a.txt"), "{out}");
    }

    /// Round 4 (E12): the session snapshot's tokenUsage/contextUsage must
    /// produce a UsageUpdate so a resync does not blank the usage bar.
    #[test]
    fn snapshot_emits_usage_from_session_token_usage() {
        let mut p = proj();
        let evs = p.consume(
            "session/snapshot",
            &json!({
                "state": {
                    "eventId": 5,
                    "session": {"id": "s1", "title": "t",
                        "status": {"type": "idle"},
                        "model": "mistral-large-latest",
                        "tokenUsage": {"inputTokens": 100, "outputTokens": 20,
                                       "totalTokens": 120, "cachedInputTokens": 40},
                        "contextUsage": {"inputTokens": 120, "totalTokens": 120}},
                    "history": [],
                    "activeCallbacks": []
                },
                "lastEventId": 5
            }),
        );
        let usage = evs.iter().find_map(|e| match e {
            AgentEvent::UsageUpdate {
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                total_tokens,
                context_used,
                ..
            } => Some((*prompt_tokens, *completion_tokens, *cached_tokens, *total_tokens, *context_used)),
            _ => None,
        });
        assert_eq!(usage, Some((100.0, 20.0, 40.0, 120.0, 120.0)), "{evs:?}");
    }

    /// Round 8: on the real wire the callback discriminator lives INSIDE
    /// `detail.kind` — entry-level `kind` must not be required, and the
    /// approval tool name is nested at `detail.effect.toolName`.
    #[test]
    fn callback_detail_kind_and_effect_tool_name() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let cb: crate::proto::CallbackEntry = serde_json::from_value(json!({
            "callbackId": "cb-9",
            "detail": {
                "kind": "approval",
                "effect": {"toolName": "bash", "input": {"command": "ls"}},
                "request": {"choices": ["approve", "deny"]}
            }
        }))
        .unwrap();
        let ev = p.callback_event(&cb).expect("approval event");
        match ev {
            AgentEvent::ToolApprovalRequest { request_id, tool_name, .. } => {
                assert_eq!(request_id, "cb-9");
                assert_eq!(tool_name.as_deref(), Some("bash"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Round 8: user_input requests carry `request.questions[]` with
    /// `options[].label` — not a flat question/choices pair.
    #[test]
    fn callback_user_input_questions_array() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let cb: crate::proto::CallbackEntry = serde_json::from_value(json!({
            "callbackId": "cb-10",
            "detail": {
                "kind": "user_input",
                "request": {"questions": [
                    {"question": "Which file?", "options": [{"label": "a.md"}, {"label": "b.md"}]},
                    {"question": "Continue?", "options": [{"label": "yes"}]}
                ]}
            }
        }))
        .unwrap();
        let ev = p.callback_event(&cb).expect("question event");
        match ev {
            AgentEvent::AskUserQuestion { question, choices, .. } => {
                assert_eq!(question, "Which file?\nContinue?");
                assert_eq!(choices, vec!["a.md".to_string(), "b.md".to_string(), "yes".to_string()]);
            }
            other => panic!("unexpected: {other:?}"),
        }
        // the stored questions feed the UserQuestionResult-shaped answer
        assert_eq!(
            p.callback_questions.get("cb-10").map(|v| v.len()),
            Some(2)
        );
    }

    /// Round 8: an entryUpdated for an unknown entryId must NOT patch
    /// /history/0 (that corrupted the first entry) — it flags a resync.
    #[test]
    fn entry_updated_unknown_id_does_not_corrupt_first_entry() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        p.consume("history/entryAdded", &json!({
            "eventId": 6,
            "entry": {"type": "message", "role": "user", "id": "u1",
                      "content": [{"type": "text", "text": "keep me"}]}
        }));
        let before = p.doc["history"][0].clone();
        let evs = p.consume("history/entryUpdated", &json!({
            "eventId": 7, "entryId": "does-not-exist",
            "patch": [{"op": "replace", "path": "/content/0/text", "value": "CORRUPTED"}]
        }));
        assert!(evs.is_empty());
        assert!(p.resync_needed);
        assert_eq!(p.doc["history"][0], before);
    }

    /// Round 8: after a snapshot, an assistant entry's emitted_len is primed
    /// with the FULL text — a same-length update must not re-emit the whole
    /// message as a fresh delta (duplicated bubbles).
    #[test]
    fn snapshot_primes_assistant_emitted_len() {
        let mut p = proj();
        let evs = p.consume("session/snapshot", &json!({
            "state": {
                "eventId": 5,
                "session": {"id": "s1", "title": "t", "status": {"type": "idle"}},
                "history": [
                    {"type": "message", "role": "assistant", "id": "a1",
                     "content": [{"type": "text", "text": "Hello"}]}
                ],
                "activeCallbacks": []
            },
            "lastEventId": 5
        }));
        // snapshot itself emits the history but no MessageChunk replay
        assert!(evs.iter().all(|e| !matches!(e, AgentEvent::MessageChunk { .. })));
        let evs = p.consume("history/entryUpdated", &json!({
            "eventId": 6, "entryId": "a1",
            "patch": [{"op": "replace", "path": "/content/0/text", "value": "Hello"}]
        }));
        assert!(
            evs.iter().all(|e| !matches!(e, AgentEvent::MessageChunk { .. })),
            "same-length patch re-emitted full text: {evs:?}"
        );
    }

    /// Round 8: todo_write effects emit PlanUpdate (the plan card had no
    /// producer) alongside the tool-call event.
    #[test]
    fn todo_effect_emits_plan_update() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("history/entryAdded", &json!({
            "eventId": 6,
            "entry": {"type": "effect", "id": "td1", "title": "todo_write",
                      "detail": {"kind": "todo",
                                 "input": {"todos": [
                                     {"content": "step 1", "status": "in_progress"},
                                     {"content": "step 2", "status": "pending"}]}}}
        }));
        let plan = evs.iter().find_map(|e| match e {
            AgentEvent::PlanUpdate { entries_json, .. } => Some(entries_json.clone()),
            _ => None,
        });
        let plan = plan.expect("PlanUpdate emitted");
        let entries = mistraldeck_core::types::PlanEntry::parse_list(&plan);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, "in_progress");
        assert_eq!(entries[1].content, "step 2");
    }

    /// Round 10: `session/childSessionUpdated` forwards the raw child so the
    /// GUI subagent shelf stays live.
    #[test]
    fn child_session_updated_event() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("session/childSessionUpdated", &json!({
            "eventId": 6, "sessionId": "s1",
            "childSession": {"id": "c1", "name": "explore", "agentType": "explore",
                             "status": {"type": "running"}}
        }));
        assert!(evs.iter().any(|e| matches!(
            e,
            AgentEvent::ChildSessionUpdated { child_json, .. } if child_json.contains("\"c1\"")
        )));
    }

    /// Round 10: lease notifications map to held_elsewhere (leaseHolder
    /// present = held by another process; "ended" = released).
    #[test]
    fn lease_notifications() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("session/lease/updated", &json!({
            "eventId": 6, "sessionId": "s1", "leaseHolder": {"processId": 42}
        }));
        assert!(evs.iter().any(|e| matches!(
            e, AgentEvent::LeaseUpdated { held_elsewhere: true, .. }
        )));
        let evs = p.consume("session/lease/ended", &json!({
            "eventId": 7, "sessionId": "s1"
        }));
        assert!(evs.iter().any(|e| matches!(
            e, AgentEvent::LeaseUpdated { held_elsewhere: false, .. }
        )));
    }

    /// Round 11: a lease held by OUR OWN agent child pid is not "elsewhere".
    #[test]
    fn lease_own_pid_not_elsewhere() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        p.own_pid = Some(4242);
        let evs = p.consume("session/lease/updated", &json!({
            "eventId": 6, "sessionId": "s1", "leaseHolder": {"processId": 4242}
        }));
        assert!(evs.iter().any(|e| matches!(
            e, AgentEvent::LeaseUpdated { held_elsewhere: false, .. }
        )));
        let evs = p.consume("session/lease/updated", &json!({
            "eventId": 7, "sessionId": "s1", "leaseHolder": {"processId": 9999}
        }));
        assert!(evs.iter().any(|e| matches!(
            e, AgentEvent::LeaseUpdated { held_elsewhere: true, .. }
        )));
    }

    /// Round 11: `vibeCode/teleport/event` progress events are forwarded raw.
    #[test]
    fn teleport_event_forwarded() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("vibeCode/teleport/event", &json!({
            "eventId": 6,
            "event": {"kind": "complete", "operationId": "op-1", "url": "https://code.mistral.ai/x"}
        }));
        assert!(evs.iter().any(|e| matches!(
            e,
            AgentEvent::TeleportProgress { event_json, .. } if event_json.contains("\"complete\"")
        )));
    }

    /// Round 11: artifacts/created|updated trigger a refetch event.
    #[test]
    fn artifacts_changed_event() {
        let mut p = proj();
        p.consume("session/snapshot", &snapshot_json("s1"));
        let evs = p.consume("artifacts/created", &json!({
            "eventId": 6, "artifact": {"id": "a1", "kind": "pull_request"}
        }));
        assert!(evs.iter().any(|e| matches!(e, AgentEvent::ArtifactsChanged { .. })));
    }

    /// Round 10: snapshots carry childSessions into the SessionReady event.
    #[test]
    fn snapshot_carries_child_sessions() {
        let mut p = proj();
        let evs = p.consume("session/snapshot", &json!({
            "state": {
                "eventId": 5,
                "session": {"id": "s1", "title": "t", "status": {"type": "idle"}},
                "history": [],
                "activeCallbacks": [],
                "childSessions": [{"id": "c9", "name": "lean", "agentType": "lean",
                                   "status": {"type": "running"}}]
            },
            "lastEventId": 5
        }));
        assert!(evs.iter().any(|e| matches!(
            e,
            AgentEvent::SessionReady { child_sessions, .. } if child_sessions.len() == 1 && child_sessions[0].contains("c9")
        )));
    }
}

