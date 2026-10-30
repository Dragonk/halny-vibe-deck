//! Tolerant wire models for the vibe-app-server protocol
//! (docs/appserver-protocol.md §5–§9, source: `vibe/app_server/models.py`,
//! `protocol.py`). Unknown fields are ignored on purpose — the
//! app-server API is internal and drifts between CLI releases.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `{state, last_event_id}` — every mutating response.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StateResponse {
    #[serde(default)]
    pub state: Option<SessionState>,
    #[serde(default, rename = "lastEventId")]
    pub last_event_id: Option<i64>,
}

/// `PublicSessionState` (format `vibe.public-session-state/v1`).
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct SessionState {
    #[serde(default, rename = "eventId")]
    pub event_id: Option<i64>,
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub history: Option<Vec<HistoryEntry>>,
    #[serde(default, rename = "historyBeforeCursor")]
    pub history_before_cursor: Option<String>,
    #[serde(default)]
    pub turns: Option<Vec<Turn>>,
    #[serde(default, rename = "activeCallbacks")]
    pub active_callbacks: Option<Vec<CallbackEntry>>,
    #[serde(default, rename = "turnQueue")]
    pub turn_queue: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, rename = "reasoningEffort")]
    pub reasoning_effort: Option<String>,
    /// `idle | running | blocked | failed | archived` (possibly a
    /// tagged object — keep the raw for status mapping).
    #[serde(default)]
    pub status: Option<Value>,
    #[serde(default, rename = "tokenUsage")]
    pub token_usage: Option<Value>,
    #[serde(default, rename = "contextUsage")]
    pub context_usage: Option<Value>,
    /// epoch ms (int) or ISO string depending on the release — kept raw
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<Value>,
    #[serde(default)]
    pub agent: Option<Value>,
}

/// `PublicHistoryEntry` — discriminated by `type`. Messages are
/// `type: "message"` with a `role` (user/assistant/system) — the
/// installed 3.0.x wire differs from the draft protocol doc.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HistoryEntry {
    #[serde(rename = "type")]
    pub kind: String,
    /// message entries only
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// user/assistant content blocks
    #[serde(default)]
    pub content: Option<Value>,
    /// reasoning text
    #[serde(default)]
    pub text: Option<String>,
    /// effect: title + detail (tool calls, edits, commands)
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail: Option<Value>,
    /// effect/assistant state ("running", "done", "error", …)
    #[serde(default)]
    pub state: Option<Value>,
    /// callback entry
    #[serde(default)]
    pub callback_id: Option<String>,
    #[serde(default)]
    pub callback: Option<Value>,
    /// notice
    #[serde(default, rename = "noticeKind")]
    pub notice_kind: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

impl HistoryEntry {
    /// Concatenated text of the assistant/user content blocks.
    pub fn text_content(&self) -> String {
        content_text(self.content.as_ref())
    }
}

/// Extract the text of content blocks
/// (`[{type:"text", text}, {type:"image", …}]`).
pub fn content_text(content: Option<&Value>) -> String {
    let Some(c) = content else { return String::new() };
    match c {
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(Value::as_str) == Some("text") {
                    b.get("text").and_then(Value::as_str).map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// `PublicTurn` — enough for busy/stop-reason semantics.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Turn {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<Value>,
    /// Raw wire `turn.error` — an object `{code, message}` on current
    /// backends (a plain string was never the real shape; the old
    /// `Option<String>` parse silently dropped rate limits). Kept as
    /// `Value` and decoded by the reducer.
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default, rename = "stopReason")]
    pub stop_reason: Option<String>,
}

/// `PublicCallbackEntry` (server→client `callback/call` params and
/// `state.activeCallbacks`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CallbackEntry {
    #[serde(default, rename = "callbackId")]
    pub callback_id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// `approval | user_input | connector_auth` (raw value kept).
    #[serde(default, rename = "kind")]
    pub detail_kind: Option<String>,
    /// full detail object (approval choices / user_input request)
    #[serde(default)]
    pub detail: Option<Value>,
    /// present on callback history entries & requests
    #[serde(default)]
    pub callback: Option<Value>,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
}

/// `session/list` item.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionListItem {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    /// First user prompt of the session (PublicSession.preview) — the
    /// fallback display name when the CLI has not generated a title yet.
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// epoch ms (int) or ISO string depending on the release — kept raw
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<Value>,
    #[serde(default, rename = "status")]
    pub status: Option<Value>,
}

/// Extract a string field from a possibly-tagged status object
/// (`{"type": "running"}` or `"running"`).
pub fn status_str(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(map)) => map
            // round 5 (R13): EffectState uses `status` (pending/running/
            // blocked/completed/failed/cancelled); older docs used `type`
            .get("status")
            .or_else(|| map.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// Best-effort number extraction from `tokenUsage`-like objects
/// (field names drifted across releases; check the known ones).
pub fn usage_num(v: Option<&Value>, keys: &[&str]) -> f64 {
    let Some(v) = v else { return 0.0 };
    for k in keys {
        if let Some(n) = v.get(k).and_then(Value::as_f64) {
            return n;
        }
    }
    0.0
}

/// AgentStatsSnapshot → (prompt, completion, cached, total, cost).
///
/// The installed CLI 3.0.x serializes `AgentStatsSnapshot`
/// (vibe/app_server/models.py) with camelCase aliases:
/// `sessionPromptTokens`, `sessionCompletionTokens`, `sessionCachedTokens`,
/// … The snapshot carries NO cost field — cost is derived from the
/// per-million price fields (`inputPricePerMillion`, …), falling back to an
/// explicit cost key when a release adds one.
pub fn stats_numbers(stats: Option<&Value>) -> (f64, f64, f64, f64, f64) {
    let Some(s) = stats else { return (0.0, 0.0, 0.0, 0.0, 0.0) };
    let get = |keys: &[&str]| usage_num(Some(s), keys);
    let prompt = get(&[
        "sessionPromptTokens",
        "session_prompt_tokens",
        "promptTokens",
        "prompt_tokens",
        "inputTokens",
    ]);
    let completion = get(&[
        "sessionCompletionTokens",
        "session_completion_tokens",
        "completionTokens",
        "completion_tokens",
        "outputTokens",
    ]);
    let cached = get(&[
        "sessionCachedTokens",
        "session_cached_tokens",
        "cachedInputTokens",
        "cached_input_tokens",
        "cachedTokens",
        "cached_tokens",
    ]);
    let total = get(&[
        "sessionTotalLlmTokens",
        "totalTokens",
        "total_tokens",
    ]);
    let total = if total > 0.0 {
        total
    } else {
        prompt + completion
    };
    // Explicit cost first; otherwise derive from the price fields.
    let cost = get(&["costUsd", "cost_usd", "cost", "totalCostUsd"]);
    let cost = if cost > 0.0 {
        cost
    } else {
        let in_p = get(&["inputPricePerMillion", "input_price_per_million"]);
        let out_p = get(&["outputPricePerMillion", "output_price_per_million"]);
        let cached_p = get(&[
            "cachedInputPricePerMillion",
            "cached_input_price_per_million",
        ]);
        let cached_p = if cached_p > 0.0 { cached_p } else { in_p };
        (in_p * prompt + out_p * completion + cached_p * cached) / 1_000_000.0
    };
    (prompt, completion, cached, total, cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_state_response() {
        let r: StateResponse = serde_json::from_value(json!({
            "state": {
                "eventId": 12,
                "session": {"id": "s1", "title": "t", "status": {"type": "idle"},
                             "tokenUsage": {"totalTokens": 10}},
                "history": [{"type": "user", "id": "e1",
                              "content": [{"type": "text", "text": "hi"}]}],
                "activeCallbacks": []
            },
            "lastEventId": 12
        }))
        .unwrap();
        let st = r.state.unwrap();
        assert_eq!(st.event_id, Some(12));
        assert_eq!(r.last_event_id, Some(12));
        let s = st.session.unwrap();
        assert_eq!(s.id, "s1");
        assert_eq!(status_str(s.status.as_ref()), "idle");
        assert_eq!(usage_num(s.token_usage.as_ref(), &["totalTokens"]), 10.0);
        assert_eq!(st.history.unwrap()[0].text_content(), "hi");
    }

    #[test]
    fn content_text_concatenates_blocks() {
        assert_eq!(
            content_text(Some(&json!([
                {"type": "text", "text": "a"},
                {"type": "image", "attachment": {}},
                {"type": "text", "text": "b"},
            ]))),
            "ab"
        );
    }

    #[test]
    fn stats_numbers_tolerates_field_names() {
        // total falls back to prompt+completion (round 4, E12: the CLI's
        // AgentStatsSnapshot has no total field)
        assert_eq!(stats_numbers(Some(&json!({"prompt_tokens": 1, "completionTokens": 2}))), (1.0, 2.0, 0.0, 3.0, 0.0));
        assert_eq!(stats_numbers(None), (0.0, 0.0, 0.0, 0.0, 0.0));
        // the installed CLI 3.0.x wire shape (camelCase aliases)
        assert_eq!(
            stats_numbers(Some(&json!({
                "sessionPromptTokens": 10, "sessionCompletionTokens": 5,
                "sessionCachedTokens": 4, "inputPricePerMillion": 2.0,
                "outputPricePerMillion": 10.0, "cachedInputPricePerMillion": 1.0
            }))),
            (10.0, 5.0, 4.0, 15.0, (10.0 * 2.0 + 5.0 * 10.0 + 4.0 * 1.0) / 1_000_000.0)
        );
    }

    #[test]
    fn ignores_unknown_fields() {
        let s: Session = serde_json::from_value(json!({
            "id": "x", "futureField": {"a": 1}, "anotherOne": [1, 2]
        }))
        .unwrap();
        assert_eq!(s.id, "x");
    }
}
