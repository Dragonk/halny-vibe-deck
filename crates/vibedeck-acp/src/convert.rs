//! Conversion of raw ACP JSON into [`AcpEvent`]s.
//!
//! `vibe-acp` sends `session/update` notifications in two shapes
//! (modern `update.sessionUpdate` + inline fields, and an older
//! `update.type` + `update.chunk/toolCall` shape). Both are supported,
//! mirroring the tolerant parsing of the original TypeScript client.

use serde_json::Value;
use vibedeck_core::proto::AcpEvent;
use vibedeck_core::types::{FileAction, FileChange, ToolCall, ToolStatus};

/// Convert one `session/update` params object into zero or more events.
pub fn convert_update(params: &Value) -> Vec<AcpEvent> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let Some(update) = params.get("update") else {
        return Vec::new();
    };

    let mut events = Vec::new();
    let session_update = update
        .get("sessionUpdate")
        .or_else(|| update.get("session_update"))
        .and_then(Value::as_str);
    let type_tag = update.get("type").and_then(Value::as_str);

    // --- streaming text (modern shape) -----------------------------------
    if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
        match session_update {
            Some("agent_thought_chunk") => events.push(AcpEvent::ThinkingChunk {
                session_id: session_id.clone(),
                text: text.to_string(),
            }),
            Some("agent_message_chunk") => events.push(AcpEvent::MessageChunk {
                session_id: session_id.clone(),
                text: text.to_string(),
            }),
            // session/load history replay: the user's own messages
            Some("user_message_chunk") => events.push(AcpEvent::UserMessageChunk {
                session_id: session_id.clone(),
                text: text.to_string(),
            }),
            _ => {}
        }
    }

    if session_update == Some("agent_thought_end") {
        events.push(AcpEvent::ThinkingComplete {
            session_id: session_id.clone(),
        });
    }

    // --- available slash commands ------------------------------------------
    if let Some(cmds) = update.get("availableCommands").and_then(Value::as_array) {
        let commands = cmds
            .iter()
            .filter_map(|c| {
                let name = c.get("name").and_then(Value::as_str)?.to_string();
                Some(vibedeck_core::proto::AvailableCommand {
                    name,
                    description: c
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    hint: c
                        .pointer("/input/hint")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
            })
            .collect();
        events.push(AcpEvent::AvailableCommands {
            session_id: session_id.clone(),
            commands,
        });
    }

    // --- live usage (vibe-acp ≥ 3.0: `usage_update`) -----------------------
    if session_update == Some("usage_update") {
        let meta = update.get("_meta");
        let f = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(0.0);
        let (used, size) = (f(update.get("used")), f(update.get("size")));
        events.push(AcpEvent::UsageUpdate {
            session_id: session_id.clone(),
            prompt_tokens: f(meta.and_then(|m| m.get("promptTokens"))),
            completion_tokens: f(meta.and_then(|m| m.get("completionTokens"))),
            cached_tokens: f(meta.and_then(|m| m.get("cachedTokens"))),
            total_tokens: f(meta.and_then(|m| m.get("totalTokens"))),
            tokens_per_second: f(meta.and_then(|m| m.get("tokensPerSecond"))),
            last_turn_duration_ms: f(meta.and_then(|m| m.get("lastTurnDuration"))),
            context_used: used,
            context_size: size,
        });
    }

    // --- tool calls (modern shape) ----------------------------------------
    if session_update == Some("tool_call") || type_tag == Some("tool_call") {
        if let Some(ev) = tool_call_started(&session_id, update) {
            events.push(ev);
        }
    }

    if session_update == Some("tool_call_update") {
        let tool_call_id = str_field(update, &["toolCallId", "tool_call_id"]);
        if !tool_call_id.is_empty() {
            let raw_status = update.get("status").and_then(Value::as_str).unwrap_or("");
            let raw_output = update
                .get("output")
                .or_else(|| update.get("result"))
                .or_else(|| update.get("content"));
            let output = raw_output.map(|o| match o {
                Value::String(s) => s.clone(),
                other => serde_json::to_string_pretty(other).unwrap_or_default(),
            });
            events.push(AcpEvent::ToolCallUpdated {
                session_id: session_id.clone(),
                tool_call_id,
                status: ToolStatus::normalize(raw_status),
                output,
            });
        }
    }

    // --- older/alternative shape (type-tagged) ----------------------------
    match type_tag {
        Some("user_message_chunk") => {
            if let Some(text) = update.pointer("/chunk/text").and_then(Value::as_str) {
                events.push(AcpEvent::UserMessageChunk {
                    session_id: session_id.clone(),
                    text: text.to_string(),
                });
            }
        }
        Some("agent_message_chunk") => {
            if session_update != Some("agent_message_chunk") {
                if let Some(text) = update.pointer("/chunk/text").and_then(Value::as_str) {
                    events.push(AcpEvent::MessageChunk {
                        session_id: session_id.clone(),
                        text: text.to_string(),
                    });
                }
                if update.pointer("/chunk/type").and_then(Value::as_str) == Some("thinking") {
                    if let Some(text) = update.pointer("/chunk/thinking").and_then(Value::as_str) {
                        let block_id = update
                            .pointer("/chunk/id")
                            .and_then(Value::as_str)
                            .unwrap_or("thinking")
                            .to_string();
                        events.push(AcpEvent::ThinkingChunk {
                            session_id: session_id.clone(),
                            text: text.to_string(),
                        });
                        let _ = block_id; // single thinking block per message in practice
                    }
                }
                if update.pointer("/chunk/type").and_then(Value::as_str) == Some("thinking_end") {
                    events.push(AcpEvent::ThinkingComplete {
                        session_id: session_id.clone(),
                    });
                }
            }
        }
        Some("tool_call") => { /* handled above via session_update/type branch */ }
        Some("tool_call_update") => {
            if session_update != Some("tool_call_update") {
                let tool_call_id = update
                    .pointer("/toolCall/id")
                    .or_else(|| update.pointer("/toolCall/toolCallId"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !tool_call_id.is_empty() {
                    let raw_status = update
                        .pointer("/toolCall/status")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let raw_output = update
                        .pointer("/toolCall/output")
                        .or_else(|| update.pointer("/toolCall/result"))
                        .or_else(|| update.pointer("/toolCall/content"));
                    let output = raw_output.map(|o| match o {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string_pretty(other).unwrap_or_default(),
                    });
                    events.push(AcpEvent::ToolCallUpdated {
                        session_id: session_id.clone(),
                        tool_call_id,
                        status: ToolStatus::normalize(raw_status),
                        output,
                    });
                }
            }
        }
        Some("plan") => {
            let entries = update
                .get("entries")
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));
            events.push(AcpEvent::PlanUpdate {
                session_id: session_id.clone(),
                entries_json: serde_json::to_string(&entries).unwrap_or_default(),
            });
        }
        _ => {}
    }

    events
}

fn str_field(v: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(s) = v.get(*key).and_then(Value::as_str) {
            return s.to_string();
        }
    }
    String::new()
}

fn tool_call_started(session_id: &str, update: &Value) -> Option<AcpEvent> {
    let tool_call_id = str_field(update, &["toolCallId", "tool_call_id"]);
    let title = str_field(update, &["title", "name"]);
    if tool_call_id.is_empty() || title.is_empty() {
        return None;
    }
    let kind = update
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("execute")
        .to_string();
    let raw_input = update.get("rawInput").or_else(|| update.get("raw_input"));
    let input_json = raw_input.map(|r| match r {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    });

    let file_changes = input_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Value>(json).ok())
        .map(|input| extract_file_changes(&title, &input))
        .unwrap_or_default();

    Some(AcpEvent::ToolCallStarted {
        session_id: session_id.to_string(),
        tool_call: ToolCall {
            id: tool_call_id,
            title,
            kind,
            status: ToolStatus::Running,
            input_json,
            output: None,
            file_changes,
        },
    })
}

fn extract_file_changes(tool_name: &str, input: &Value) -> Vec<FileChange> {
    let path = str_field(input, &["path", "file_path", "filename"]);
    if path.is_empty() {
        return Vec::new();
    }
    let lower = tool_name.to_lowercase();
    let action = if lower.contains("creat") || lower.contains("write") || lower.contains("new") {
        FileAction::Added
    } else if lower.contains("delet") || lower.contains("remov") {
        FileAction::Deleted
    } else {
        FileAction::Modified
    };
    vec![FileChange { path, action }]
}

/// Convert an agent-initiated request (needs user interaction) into an event.
pub fn convert_agent_request(id: Value, method: &str, params: &Value) -> Vec<AcpEvent> {
    let request_id = serde_json::to_string(&id).unwrap_or_default();
    let is_question =
        method == "ask_user_question" || method.contains("ask_user") || method == "user/question";
    if is_question {
        let question = params
            .get("question")
            .or_else(|| params.get("prompt"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let choices = params
            .get("choices")
            .or_else(|| params.get("options"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| match c {
                        Value::String(s) => Some(s.clone()),
                        Value::Object(_) => c
                            .get("label")
                            .or_else(|| c.get("text"))
                            .or_else(|| c.get("id"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        other => serde_json::to_string(other).ok(),
                    })
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        return vec![AcpEvent::AskUserQuestion {
            request_id,
            question,
            choices,
        }];
    }

    // Approval / permission / tool-call requests.
    let tool_name = params
        .pointer("/toolCall/toolCallId")
        .or_else(|| params.get("toolCallId"))
        .and_then(Value::as_str)
        .map(|tool_call_id| tool_call_id.to_string());
    vec![AcpEvent::ToolApprovalRequest {
        request_id,
        method: method.to_string(),
        params_json: serde_json::to_string_pretty(params).unwrap_or_default(),
        tool_name,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_chunk_modern_shape() {
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "Hello" }
            }
        });
        let events = convert_update(&params);
        assert!(
            matches!(events.as_slice(), [AcpEvent::MessageChunk { text, .. }] if text == "Hello")
        );
    }

    #[test]
    fn thought_chunk_and_end() {
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_thought_chunk",
                "content": { "text": "thinking..." }
            }
        });
        assert!(matches!(
            convert_update(&params).as_slice(),
            [AcpEvent::ThinkingChunk { .. }]
        ));

        let params = serde_json::json!({
            "sessionId": "s1",
            "update": { "sessionUpdate": "agent_thought_end" }
        });
        assert!(matches!(
            convert_update(&params).as_slice(),
            [AcpEvent::ThinkingComplete { .. }]
        ));
    }

    #[test]
    fn tool_call_roundtrip() {
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tc1",
                "title": "Write file",
                "kind": "edit",
                "rawInput": { "path": "/tmp/x.txt" }
            }
        });
        let events = convert_update(&params);
        match events.as_slice() {
            [AcpEvent::ToolCallStarted { tool_call, .. }] => {
                assert_eq!(tool_call.title, "Write file");
                assert_eq!(tool_call.file_changes.len(), 1);
                assert_eq!(tool_call.file_changes[0].action, FileAction::Added);
            }
            other => panic!("unexpected events: {other:?}"),
        }

        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tc1",
                "status": "completed",
                "output": { "ok": true }
            }
        });
        let events = convert_update(&params);
        assert!(matches!(
            events.as_slice(),
            [AcpEvent::ToolCallUpdated {
                status: ToolStatus::Done,
                ..
            }]
        ));
    }

    #[test]
    fn older_chunk_shape() {
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "type": "agent_message_chunk",
                "chunk": { "type": "text", "text": "Old shape" }
            }
        });
        assert!(
            matches!(convert_update(&params).as_slice(), [AcpEvent::MessageChunk { text, .. }] if text == "Old shape")
        );
    }

    #[test]
    fn usage_update_event() {
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "usage_update",
                "used": 8000,
                "size": 800000,
                "_meta": {
                    "steps": 2,
                    "promptTokens": 1000,
                    "completionTokens": 500,
                    "cachedTokens": 600,
                    "totalTokens": 1500,
                    "tokensPerSecond": 42.5,
                    "lastTurnDuration": 3.2
                }
            }
        });
        let events = convert_update(&params);
        match events.as_slice() {
            [AcpEvent::UsageUpdate { prompt_tokens, cached_tokens, context_size, .. }] => {
                assert_eq!(*prompt_tokens, 1000.0);
                assert_eq!(*cached_tokens, 600.0);
                assert_eq!(*context_size, 800000.0);
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn agent_question_request() {
        let events = convert_agent_request(
            Value::from(7),
            "ask_user_question",
            &serde_json::json!({ "question": "Which region?", "choices": ["EU", "US"] }),
        );
        assert!(matches!(
            events.as_slice(),
            [AcpEvent::AskUserQuestion { question, choices, .. }] if question == "Which region?" && choices.len() == 2
        ));
    }
}
