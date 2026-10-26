//! Conversion of raw ACP JSON into [`AcpEvent`]s.
//!
//! `vibe-acp` sends `session/update` notifications in two shapes
//! (modern `update.sessionUpdate` + inline fields, and an older
//! `update.type` + `update.chunk/toolCall` shape). Both are supported,
//! mirroring the tolerant parsing of the original TypeScript client.

use serde_json::Value;
use vibedeck_core::proto::AcpEvent;
use vibedeck_core::types::{
    DiffHunk, FileAction, FileChange, SubagentInfo, ToolCall, ToolStatus,
};

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
        // v0.10.2: the server reports the running session cost directly
        // (`update.cost.amount`, USD) — no local price estimation needed.
        let cost = update.pointer("/cost/amount").and_then(Value::as_f64).unwrap_or(0.0);
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
            cost,
        });
    }

    // --- agent plan / todo checklist (modern wire) ------------------------
    // Nexus v0.11.1: `sessionUpdate: "plan"` (older servers: "plan_update")
    // carries the cumulative checklist snapshot in `entries`.
    if session_update == Some("plan") || session_update == Some("plan_update") {
        let entries = update
            .get("entries")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        events.push(AcpEvent::PlanUpdate {
            session_id: session_id.clone(),
            entries_json: serde_json::to_string(&entries).unwrap_or_default(),
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
                subagent: meta_subagent(update),
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
                        subagent: meta_subagent(update),
                    });
                }
            }
        }
        Some("plan") => {
            // Guard: the modern `sessionUpdate: "plan"` branch already
            // emitted — never double-emit the same snapshot.
            if session_update != Some("plan") && session_update != Some("plan_update") {
                let entries = update
                    .get("entries")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new()));
                events.push(AcpEvent::PlanUpdate {
                    session_id: session_id.clone(),
                    entries_json: serde_json::to_string(&entries).unwrap_or_default(),
                });
            }
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

/// v0.13 subagent invocation meta (agents.spawn / task): `_meta.effect_kind
/// == "subagent"` with `agent`, `task` and — added later via
/// `tool_call_update` — `child_session_id`. Port of Nexus `metaSubagent`.
fn meta_subagent(update: &Value) -> Option<SubagentInfo> {
    let meta = update.get("_meta")?;
    if meta.get("effect_kind").and_then(Value::as_str) != Some("subagent") {
        return None;
    }
    let mut info = SubagentInfo::default();
    if let Some(agent) = meta.get("agent").and_then(Value::as_str) {
        info.agent = Some(agent.to_string());
    }
    if let Some(task) = meta.get("task").and_then(Value::as_str) {
        info.task = Some(task.to_string());
    }
    let child = meta
        .get("child_session_id")
        .or_else(|| meta.get("childSessionId"))
        .and_then(Value::as_str);
    if let Some(child) = child {
        if !child.is_empty() {
            info.child_session_id = Some(child.to_string());
        }
    }
    let empty = info.agent.is_none()
        && info.task.is_none()
        && info.child_session_id.is_none();
    if empty {
        None
    } else {
        Some(info)
    }
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

    let diffs = extract_diffs(&title, update, input_json.as_deref());

    // Canonical tool name from `_meta` (e.g. "file_system.edit") — used for
    // rich content routing in the chat (Nexus `metaToolName`).
    let tool_name = update
        .pointer("/_meta/tool_name")
        .or_else(|| update.pointer("/_meta/toolName"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    // Pre-edit file content carried by edit tools in `rawOutput`.
    let previous_content = update
        .pointer("/rawOutput/previousContent")
        .or_else(|| update.pointer("/raw_output/previous_content"))
        .or_else(|| update.pointer("/rawOutput/previous_content"))
        .and_then(Value::as_str)
        .map(str::to_string);

    // Raw structured output (process tools table/status).
    let raw_output_json = update
        .get("rawOutput")
        .or_else(|| update.get("raw_output"))
        .filter(|r| r.is_object())
        .map(|r| serde_json::to_string(r).unwrap_or_default());

    // v0.13 subagent invocation meta (agents.spawn / task).
    let subagent = meta_subagent(update);

    // session/load replay carries the final status and the output inside the
    // same `tool_call` event — live calls get them via `tool_call_update`.
    let status = ToolStatus::normalize(
        update.get("status").and_then(Value::as_str).unwrap_or("in_progress"),
    );
    let output = update
        .get("output")
        .or_else(|| update.get("result"))
        .map(|o| match o {
            Value::String(s) => s.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_default(),
        })
        .or_else(|| output_from_content(update.get("content")));

    Some(AcpEvent::ToolCallStarted {
        session_id: session_id.to_string(),
        tool_call: ToolCall {
            id: tool_call_id,
            title,
            kind,
            status,
            input_json,
            output,
            file_changes,
            diffs,
            tool_name,
            previous_content,
            raw_output_json,
            subagent,
            started_at: vibedeck_core::types::now_ms(),
        },
    })
}

/// The replay `tool_call` event carries its output as a content array:
/// `content: [ { content: { text, type } } ]` — join the text parts.
fn output_from_content(content: Option<&Value>) -> Option<String> {
    let items = content?.as_array()?;
    let mut text = String::new();
    for item in items {
        if let Some(t) = item.pointer("/content/text").and_then(Value::as_str) {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(t);
        }
    }
    if text.is_empty() { None } else { Some(text) }
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

/// Extract structured edit diffs for the chat's git-style diff card.
/// Sources (Nexus parity priority):
/// 1. wire `content` blocks with `type: "diff"` — `{ path, oldText, newText }`;
/// 2. the edit tool input — `filePath` + `changes: [{ oldString, newString }]`
///    (or a single oldString/newString pair);
/// 3. optional line numbers from `rawOutput.occurrences[0]` (startLine /
///    newStartLine) attached to the first hunk.
fn extract_diffs(title: &str, update: &Value, input_json: Option<&str>) -> Vec<DiffHunk> {
    let lower_title = title.to_lowercase();
    let looks_like_edit = lower_title.contains("edit")
        || lower_title.contains("replace")
        || lower_title.contains("write")
        || lower_title.contains("creat");

    let mut hunks: Vec<DiffHunk> = Vec::new();

    // 1. wire content diff blocks
    if let Some(items) = update.get("content").and_then(Value::as_array) {
        for item in items {
            let block = match item.get("type").and_then(Value::as_str) {
                Some("diff") => item,
                _ => match item.pointer("/content/type").and_then(Value::as_str) {
                    Some("diff") => item.get("content").unwrap_or(item),
                    _ => continue,
                },
            };
            let path = block
                .get("path")
                .or_else(|| block.get("filePath"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let new_text = block
                .get("newText")
                .or_else(|| block.get("new_text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if path.is_empty() && new_text.is_empty() {
                continue;
            }
            hunks.push(DiffHunk {
                path,
                old_text: block
                    .get("oldText")
                    .or_else(|| block.get("old_text"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                new_text,
                start_line: None,
                new_start_line: None,
            });
        }
    }

    // 2. edit tool input fallback
    if hunks.is_empty() && looks_like_edit {
        if let Some(json) = input_json {
            if let Ok(input) = serde_json::from_str::<Value>(json) {
                let path = str_field(&input, &["filePath", "file_path", "path", "filename"]);
                let pairs: Vec<(Option<String>, String)> = input
                    .get("changes")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| {
                                let old = c.get("oldString").or_else(|| c.get("old_string")).and_then(Value::as_str);
                                let new = c.get("newString").or_else(|| c.get("new_string")).and_then(Value::as_str)?;
                                Some((old.map(str::to_string), new.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_else(|| {
                        // single oldString/newString pair
                        match (
                            input.get("oldString").or_else(|| input.get("old_string")).and_then(Value::as_str),
                            input.get("newString").or_else(|| input.get("new_string")).and_then(Value::as_str),
                        ) {
                            (old, Some(new)) => vec![(old.map(str::to_string), new.to_string())],
                            _ => Vec::new(),
                        }
                    });
                for (old, new) in pairs {
                    hunks.push(DiffHunk {
                        path: path.clone(),
                        old_text: old,
                        new_text: new,
                        start_line: None,
                        new_start_line: None,
                    });
                }
            }
        }
    }

    // 3. line numbers from rawOutput.occurrences (first hunk)
    if let Some(first) = hunks.first_mut() {
        let occ = update
            .pointer("/rawOutput/occurrences/0")
            .or_else(|| update.pointer("/raw_output/occurrences/0"));
        if let Some(occ) = occ {
            let sl = occ.get("startLine").or_else(|| occ.get("start_line")).and_then(Value::as_i64);
            let nsl = occ
                .get("newStartLine")
                .or_else(|| occ.get("new_start_line"))
                .and_then(Value::as_i64);
            if let Some(sl) = sl {
                first.start_line = Some(sl);
                first.new_start_line = Some(nsl.unwrap_or(sl));
            }
        }
    }

    hunks
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
    fn subagent_meta_extraction() {
        // agents.spawn tool call with subagent _meta on the wire.
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tc1",
                "title": "task",
                "_meta": {
                    "tool_name": "agents.spawn",
                    "effect_kind": "subagent",
                    "agent": "general-purpose",
                    "task": "Fix the bug"
                }
            }
        });
        let events = convert_update(&params);
        match events.as_slice() {
            [AcpEvent::ToolCallStarted { tool_call, .. }] => {
                let sa = tool_call.subagent.as_ref().expect("subagent meta");
                assert_eq!(sa.agent.as_deref(), Some("general-purpose"));
                assert_eq!(sa.task.as_deref(), Some("Fix the bug"));
                assert!(sa.child_session_id.is_none());
                assert_eq!(tool_call.tool_name, "agents.spawn");
            }
            other => panic!("unexpected events: {other:?}"),
        }

        // The child session id arrives later via tool_call_update.
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tc1",
                "status": "in_progress",
                "_meta": {
                    "effect_kind": "subagent",
                    "child_session_id": "child-abc"
                }
            }
        });
        let events = convert_update(&params);
        match events.as_slice() {
            [AcpEvent::ToolCallUpdated { subagent: Some(sa), .. }] => {
                assert_eq!(sa.child_session_id.as_deref(), Some("child-abc"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn replayed_tool_call_has_status_and_output() {
        // session/load replay: status + output inside the tool_call event.
        let params = serde_json::json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tc9",
                "title": "bash: ls",
                "kind": "execute",
                "status": "completed",
                "rawInput": { "command": "ls" },
                "content": [
                    { "content": { "text": "file-a\nfile-b", "type": "text" } }
                ]
            }
        });
        let events = convert_update(&params);
        match events.as_slice() {
            [AcpEvent::ToolCallStarted { tool_call, .. }] => {
                assert_eq!(tool_call.status, ToolStatus::Done);
                assert!(tool_call.input_json.is_some());
                assert_eq!(tool_call.output.as_deref(), Some("file-a\nfile-b"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
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
