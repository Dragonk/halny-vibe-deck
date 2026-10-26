//! Chat view: empty state with the pixel flame, message list with user
//! bubbles (inline `@` attachment chips like Nexus `UserMessageContent`),
//! AI rows (thinking, tool calls, markdown), the agent busy indicator and
//! subagent task blocks with lazily loaded transcripts.
//! Ported from Nexus ChatView + MessageBubble (same CSS classes).

use dioxus::prelude::*;
use vibedeck_core::types::{Attachment, Message, MessageSender, PlanEntry, ToolStatus};

use crate::i18n::tr;
use crate::state::{lang, ClientState};
use crate::vibestats::SubagentSessionView;

pub fn ChatView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let s = state.read();
    let messages = s.snapshot.current_messages.clone();
    let plan = s.snapshot.current_plan.clone();
    let busy = s.snapshot.busy;
    let status = s.snapshot.status.clone();
    let acp_error = s.snapshot.acp_error.clone();
    drop(s);

    // one-shot: install the mermaid MutationObserver renderer (client only;
    // the server SSR build never evaluates JS)
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let mut mermaid_init = use_signal(|| false);
        if !*mermaid_init.read() {
            mermaid_init.set(true);
            spawn(async move {
                dioxus::document::eval(MERMAID_JS);
            });
        }
    }

    let subtitle = match status.as_str() {
        "connected" => tr(&lang, "agentReady").to_string(),
        "connecting" => tr(&lang, "statusConnecting").to_string(),
        "error" => tr(&lang, "agentConnectionError").to_string(),
        _ => tr(&lang, "configureAcpInSettings").to_string(),
    };

    rsx! {
        div { class: "chat-view-wrap",
            div { class: "chat-messages",
                div { class: "chat-inner",
                    if messages.is_empty() {
                        div { class: "chat-empty",
                            Flame {}
                            div { class: "chat-empty-title", "{tr(&lang, \"appName\")}" }
                            div { class: "chat-empty-subtitle", "{subtitle}" }
                            if status == "error" && !acp_error.is_empty() {
                                div { class: "chat-error-banner", span { "⚠" } " {acp_error}" }
                            }
                        }
                    } else {
                        div { class: "message-list",
                            for (idx, msg) in messages.iter().enumerate() {
                                MessageBubble { msg: msg.clone(), streaming: !msg.is_complete && busy && idx == messages.len() - 1 }
                            }
                            if !plan.is_empty() {
                                PlanCard { entries: plan.clone() }
                            }
                            if busy {
                                div { class: "agent-busy-row",
                                    span { class: "agent-busy-dots", span {}, span {}, span {} }
                                    "{tr(&lang, \"agentWorkingLabel\")}"
                                }
                            }
                        }
                    }
                }
            }
            crate::components::input::InputBar {}
        }
    }
}

/// The pixel-art flame from the Nexus empty state.
#[component]
fn Flame() -> Element {
    rsx! {
        svg {
            class: "chat-empty-icon",
            xmlns: "http://www.w3.org/2000/svg",
            width: "95", height: "68",
            view_box: "0 0 212.121 151.515",
            version: "1.1",
            rect { fill: "var(--yellowbright-500)", x: "30.303001", y: "0", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--yellowbright-500)", x: "151.515", y: "0", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--yellow-500)", x: "30.303001", y: "30.303001", width: "60.605999", height: "30.302999" }
            rect { fill: "var(--yellow-500)", x: "121.21201", y: "30.303001", width: "60.605999", height: "30.302999" }
            rect { fill: "var(--orangebright-500)", x: "30.303001", y: "60.606003", width: "151.515", height: "30.302999" }
            rect { fill: "var(--orange-500)", x: "30.303001", y: "90.908997", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--orange-500)", x: "90.908997", y: "90.908997", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--orange-500)", x: "151.515", y: "90.908997", width: "30.302999", height: "30.302999" }
            rect { fill: "var(--red-700)", x: "0", y: "121.21201", width: "90.908997", height: "30.302999" }
            rect { fill: "var(--red-700)", x: "121.21201", y: "121.21201", width: "90.908997", height: "30.302999" }
        }
    }
}

/// One segment of a user message: plain text or an `@marker`.
enum UserSeg {
    Text(String),
    Marker { raw: String, att: Option<Attachment> },
}

/// Split a user message into text/`@marker` segments (Nexus `@([^\s]+)`),
/// resolving each marker against the message attachments.
fn split_user_segments(content: &str, atts: &[Attachment]) -> Vec<UserSeg> {
    let mut segs: Vec<UserSeg> = Vec::new();
    let bytes = content.as_bytes();
    let mut last = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let raw = &content[start + 1..j];
            if raw.is_empty() {
                i += 1;
                continue;
            }
            if start > last {
                segs.push(UserSeg::Text(content[last..start].to_string()));
            }
            let stripped = raw.strip_suffix('/').unwrap_or(raw);
            let att = atts
                .iter()
                .find(|a| a.path == raw || a.path == stripped || format!("{}/", a.path) == raw)
                .cloned();
            segs.push(UserSeg::Marker {
                raw: raw.to_string(),
                att,
            });
            last = j;
            i = j;
        } else {
            i += 1;
        }
    }
    if last < content.len() {
        segs.push(UserSeg::Text(content[last..].to_string()));
    }
    segs
}

/// The Mistral logo mark (Nexus `MISTRAL_LOGO`), rendered inside the AI
/// message avatar.
#[component]
fn MistralLogo() -> Element {
    rsx! {
        svg {
            view_box: "0 0 24 24",
            fill: "currentColor",
            path { d: "M13.3715 16.358H16.1144V13.6486H13.3712L13.3715 16.358H10.6283V13.6486H7.88568V16.358H10.6283V19.0676H2.3999V16.358H5.14279V5.52002H7.88568V8.22963H10.6286V10.939H13.3715V8.22963H16.1144V5.52002H18.8572V16.358H21.5999V19.0676H13.3715V16.358Z" }
        }
    }
}

/// Copy-to-clipboard action button (Nexus `msg-action-btn`).
#[component]
fn CopyButton(text: String) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    rsx! {
        button {
            class: "msg-action-btn",
            title: "{tr(&lang, \"copy\")}",
            onclick: move |_| {
                let js = format!(
                    "try{{navigator.clipboard.writeText(\"{}\")}}catch(e){{}}",
                    js_escape(&text)
                );
                dioxus::document::eval(&js);
            },
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                width: "16", height: "16",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                rect { width: "14", height: "14", x: "8", y: "8", rx: "2", ry: "2" }
                path { d: "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2" }
            }
        }
    }
}

/// Escape a string for embedding inside a JS double-quoted string literal.
fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// `HH:MM` for a message timestamp, in the browser's local timezone.
fn msg_time(ms: i64) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
        format!("{:02}:{:02}", d.get_hours(), d.get_minutes())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // SSR fallback (no JS): UTC
        let secs = ms.div_euclid(1000);
        format!("{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60)
    }
}

#[component]
pub fn MessageBubble(msg: Message, streaming: bool) -> Element {
    let time_str = msg_time(msg.timestamp_ms);
    match msg.sender {
        MessageSender::User => {
            // Inline `@marker` rendering with expandable attachment chips
            // (Nexus UserMessageContent) — precomputed, rsx children cannot
            // contain statements. Track matched markers so unmatched
            // attachments still render as chips below the text.
            let mut matched_paths: Vec<String> = Vec::new();
            let seg_els: Vec<Element> = split_user_segments(&msg.content, &msg.attachments)
                .into_iter()
                .map(|seg| match seg {
                    UserSeg::Text(t) => rsx! { div { class: "msg-text", "{t}" } },
                    UserSeg::Marker { att: Some(a), .. } => {
                        matched_paths.push(a.path.clone());
                        rsx! {
                            AttachmentFile { path: a.path.clone(), is_dir: a.is_dir, content: a.content.clone() }
                        }
                    }
                    UserSeg::Marker { raw, att: None } => rsx! {
                        span { class: "attachment-marker", "@{raw}" }
                    },
                })
                .collect();
            // Attachments that never matched a marker still show as chips.
            let extra_els: Vec<Element> = msg
                .attachments
                .iter()
                .filter(|a| !matched_paths.contains(&a.path))
                .map(|a| {
                    rsx! {
                        AttachmentFile { path: a.path.clone(), is_dir: a.is_dir, content: a.content.clone() }
                    }
                })
                .collect();
            rsx! {
                div { class: "msg-row msg-user",
                    div { class: "msg-bubble",
                        div { class: "msg-text",
                            {seg_els.iter().cloned()}
                            {extra_els.iter().cloned()}
                        }
                        div { class: "msg-actions-row",
                            div { class: "msg-actions-left" }
                            div { class: "msg-actions-right",
                                div { class: "msg-time", "{time_str}" }
                                CopyButton { text: msg.content.clone() }
                            }
                        }
                    }
                }
            }
        }
        MessageSender::Ai => {
            let avatar_cls = if streaming {
                "msg-avatar-wrap msg-avatar-busy"
            } else {
                "msg-avatar-wrap"
            };
            rsx! {
                div { class: "msg-row msg-ai",
                    div { class: "{avatar_cls}",
                        div { class: "msg-avatar", MistralLogo {} }
                        if streaming {
                            svg {
                                class: "msg-avatar-spinner",
                                view_box: "0 0 36 36",
                                xmlns: "http://www.w3.org/2000/svg",
                                circle {
                                    cx: "18", cy: "18", r: "15",
                                    fill: "none",
                                    stroke: "currentColor",
                                    stroke_width: "2.5",
                                    stroke_linecap: "round",
                                    stroke_dasharray: "60 40",
                                }
                            }
                        }
                    }
                    div { class: "msg-content",
                        for block in &msg.thinking {
                            ThinkingSection { block: block.clone() }
                        }
                        if !msg.tool_calls.is_empty() {
                            div { class: "tool-calls-list",
                                for tc in &msg.tool_calls {
                                    ToolCallBlock { tc: tc.clone() }
                                }
                            }
                        }
                        if !msg.content.is_empty() {
                            div { class: "msg-text",
                                MarkdownLite { text: msg.content.clone() }
                            }
                        }
                        if streaming {
                            span { class: "cursor-blink" }
                        }
                        div { class: "msg-actions-row",
                            div { class: "msg-actions-left",
                                div { class: "msg-time", "{time_str}" }
                            }
                            div { class: "msg-actions-right",
                                CopyButton { text: msg.content.clone() }
                            }
                        }
                    }
                }
            }
        }
        MessageSender::System => rsx! {
            div { class: "msg-row msg-notice",
                div { class: "msg-content", "{msg.content}" }
            }
        },
    }
}

/// Expandable attachment chip (Nexus AttachmentFile): header with icon, path
/// and chevron; content is a `<pre>` revealed on click.
#[component]
fn AttachmentFile(path: String, is_dir: bool, content: String) -> Element {
    let expanded = use_signal(|| false);
    let icon = if is_dir { "📁" } else { "📄" };
    let chev = if *expanded.read() { "▲" } else { "▼" };
    rsx! {
        div { class: "attachment-file",
            button {
                class: "attachment-header",
                onclick: {
                    let mut expanded = expanded.clone();
                    move |_| {
                        let v = *expanded.read();
                        expanded.set(!v);
                    }
                },
                span { class: "attachment-icon", "{icon}" }
                span { class: "attachment-path", "{path}" }
                span { class: "attachment-chevron", "{chev}" }
            }
            if *expanded.read() {
                div { class: "attachment-content",
                    pre { class: "attachment-code", "{content}" }
                }
            }
        }
    }
}

#[component]
fn ThinkingSection(block: vibedeck_core::types::ThinkingBlock) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let expanded = use_signal(|| false);
    let label = if block.is_complete {
        tr(&lang, "thinking").to_string()
    } else {
        tr(&lang, "thinkingThinking").to_string()
    };
    let chev = if *expanded.read() { "▲" } else { "▼" };
    let paragraphs: Vec<&str> = block.content.split("\n\n").collect();
    rsx! {
        div { class: "thinking-section",
            button {
                class: "thinking-toggle",
                onclick: {
                    let mut expanded = expanded.clone();
                    move |_| {
                        let v = *expanded.read();
                        expanded.set(!v);
                    }
                },
                span { class: "thinking-dot" }
                span { class: "thinking-label", "{label}" }
                span { class: "thinking-chevron", "{chev}" }
            }
            if *expanded.read() {
                div { class: "thinking-content",
                    div { class: "thinking-paragraphs",
                        for para in paragraphs {
                            p { class: "thinking-paragraph", "{para}" }
                        }
                    }
                }
            }
        }
    }
}

/// Extract a top-level string field from a JSON object without a parser
/// (tool inputs like `{"agent": "...", "task": "..."}`).
fn json_str_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = json.find(&needle)? + needle.len();
    let rest = json[pos..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut esc = false;
    for ch in rest.chars() {
        if esc {
            esc = false;
            out.push(ch);
            continue;
        }
        match ch {
            '\\' => esc = true,
            '"' => return Some(out),
            _ => out.push(ch),
        }
    }
    Some(out)
}

/// Nexus TaskToolBlock detection: title "task"/"task: …" or the input object
/// carries both `agent` and `task` keys.
fn is_task_tool(title: &str, kind: &str, input_json: &str) -> bool {
    let hay = format!("{} {}", title, kind).to_lowercase();
    if hay.contains("subagent") {
        return true;
    }
    if title == "task" || title.starts_with("task:") {
        return true;
    }
    input_json.contains("\"agent\"") && input_json.contains("\"task\"")
}

#[component]
fn ToolCallBlock(tc: vibedeck_core::types::ToolCall) -> Element {
    let input = tc.input_json.clone().unwrap_or_default();
    if is_task_tool(&tc.title, &tc.kind, &input) {
        return rsx! { TaskToolBlock { tc: tc.clone() } };
    }

    let expanded = use_signal(|| false);
    let status_str = match tc.status {
        ToolStatus::Done => "done",
        ToolStatus::Error => "error",
        ToolStatus::Running => "running",
    };
    let block_cls = format!("tool-call-block tool-call-{status_str}");
    let icon_cls = format!("tool-call-icon tool-call-icon-{status_str}");
    let chev = if *expanded.read() { "▲" } else { "▼" };
    let kind = tc.kind.clone();
    let title = tc.title.clone();
    let output = tc.output.clone().unwrap_or_default();
    let has_changes = !tc.file_changes.is_empty();

    rsx! {
        div { class: "{block_cls}",
            button {
                class: "tool-call-header",
                onclick: {
                    let mut expanded = expanded.clone();
                    move |_| {
                        let v = *expanded.read();
                        expanded.set(!v);
                    }
                },
                span { class: "{icon_cls}", StatusIcon { status: tc.status } }
                span { class: "tool-call-title", "{title}" }
                span { class: "tool-call-chevron", "{chev}" }
            }
            if *expanded.read() {
                div { class: "tool-call-content",
                    if !kind.is_empty() {
                        span { class: "tool-kind-chip", "{kind}" }
                    }
                    if !input.is_empty() {
                        div { class: "tool-call-input", "{input}" }
                    }
                    if has_changes {
                        div { class: "tool-call-path",
                            for ch in &tc.file_changes {
                                span { class: "tool-file-path", "{ch.path}" }
                            }
                        }
                    }
                    if !output.is_empty() {
                        div { class: "tool-call-output", "{output}" }
                    }
                }
            }
        }
    }
}

/// Extract the subagent response text and `turns_used` count from a task
/// tool output (structured `response: <text>\nturns_used: N\ncompleted: …`).
fn task_output_meta(output: &str) -> Option<(Option<String>, Option<u64>)> {
    let rest = output.find("response:")?;
    let after = &output[rest + "response:".len()..];
    let body_end = after
        .find("\nturns_used:")
        .or_else(|| after.find("\ncompleted:"))
        .unwrap_or(after.len());
    let response = after[..body_end].trim().to_string();
    let response = if response.is_empty() {
        None
    } else {
        Some(response)
    };
    let turns = output.find("turns_used:").and_then(|p| {
        output[p + "turns_used:".len()..]
            .trim_start()
            .split(|c: char| c.is_whitespace())
            .next()
            .and_then(|t| t.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u64>().ok())
    });
    Some((response, turns))
}

/// Fetch + narrow subagent transcripts (Nexus fetchSessions): filter by
/// agent name, then — when several sessions share the agent — narrow by
/// task text matching the first user prompt.
fn fetch_subagent_sessions(
    state: Signal<ClientState>,
    parent: Option<String>,
    agent: String,
    task: String,
    mut sessions: Signal<Option<Vec<SubagentSessionView>>>,
    mut loading: Signal<bool>,
) {
    spawn(async move {
        let Some(parent) = parent else {
            sessions.set(Some(Vec::new()));
            return;
        };
        loading.set(true);
        let result = crate::vibestats::api_subagent_logs(crate::state::token(&state), parent).await;
        let all = match result {
            Ok(v) => v,
            Err(_) => Vec::new(),
        };
        let by_agent: Vec<SubagentSessionView> =
            all.into_iter().filter(|s| s.agent_name == agent).collect();
        let narrowed = if by_agent.len() <= 1 || task.is_empty() {
            by_agent
        } else {
            let norm = |s: &str| -> String {
                s.split_whitespace().collect::<Vec<_>>().join(" ")
            };
            let task_text = norm(&task);
            let matched: Vec<SubagentSessionView> = by_agent
                .iter()
                .filter(|s| {
                    match s.messages.iter().find(|m| m.role == "user") {
                        Some(m) if !m.content.is_empty() => {
                            let msg_text = norm(&m.content);
                            msg_text == task_text
                                || msg_text.contains(&task_text)
                                || task_text.contains(&msg_text)
                        }
                        _ => true,
                    }
                })
                .cloned()
                .collect();
            if matched.is_empty() {
                by_agent
            } else {
                matched
            }
        };
        sessions.set(Some(narrowed));
        loading.set(false);
    });
}

/// Nexus TaskToolBlock: subagent task header with agent label + task preview,
/// lazily loaded transcript sessions, `turns_used` meta.
#[component]
fn TaskToolBlock(tc: vibedeck_core::types::ToolCall) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let expanded = use_signal(|| false);
    let sessions: Signal<Option<Vec<SubagentSessionView>>> = use_signal(|| None);
    let loading = use_signal(|| false);
    let mut auto_loaded = use_signal(|| false);

    let input = tc.input_json.clone().unwrap_or_default();
    let agent_name =
        json_str_field(&input, "agent").unwrap_or_else(|| "subagent".to_string());
    let task = json_str_field(&input, "task").unwrap_or_default();

    let parent_acp_id = {
        let s = state.read();
        s.snapshot
            .current
            .and_then(|i| s.snapshot.sessions.get(i))
            .and_then(|m| m.acp_session_id.clone())
    };

    // Auto-load once the call completes (Nexus does it with a 600ms flush
    // delay); expanding also triggers a load when not loaded yet.
    if !*auto_loaded.read() && tc.status == ToolStatus::Done {
        auto_loaded.set(true);
        fetch_subagent_sessions(
            state.clone(),
            parent_acp_id.clone(),
            agent_name.clone(),
            task.clone(),
            sessions.clone(),
            loading.clone(),
        );
    }

    let status_str = match tc.status {
        ToolStatus::Done => "done",
        ToolStatus::Error => "error",
        ToolStatus::Running => "running",
    };
    let icon_cls = format!("tool-call-icon tool-call-icon-{}", status_str);
    let block_cls = format!("tool-call-block tool-call-{}", status_str);
    let chev = if *expanded.read() { "▲" } else { "▼" };
    let task_preview = if task.chars().count() > 60 {
        format!("{}…", task.chars().take(60).collect::<String>())
    } else {
        task.clone()
    };
    let has_task = !task.is_empty();

    let output = tc.output.clone().unwrap_or_default();
    let turns_used = task_output_meta(&output).and_then(|(_, t)| t);
    let show_turns = turns_used.is_some();
    let turns_val = turns_used.unwrap_or(0);

    let session_els: Vec<Element> = sessions
        .read()
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            rsx! { SubagentSessionBlock { session: s } }
        })
        .collect();
    let is_loading = *loading.read();

    rsx! {
        div { class: "{block_cls}",
            button {
                class: "tool-call-header",
                onclick: {
                    let mut expanded = expanded.clone();
                    let state = state.clone();
                    let parent = parent_acp_id.clone();
                    let agent_c = agent_name.clone();
                    let task_c = task.clone();
                    let sessions = sessions.clone();
                    let loading = loading.clone();
                    move |_| {
                        let next = !*expanded.read();
                        expanded.set(next);
                        if next && sessions.read().is_none() {
                            fetch_subagent_sessions(
                                state.clone(),
                                parent.clone(),
                                agent_c.clone(),
                                task_c.clone(),
                                sessions.clone(),
                                loading.clone(),
                            );
                        }
                    }
                },
                span { class: "{icon_cls}", StatusIcon { status: tc.status } }
                span { class: "tool-call-title",
                    span { class: "task-agent-label", "{tr(&lang, \"subagent\")}: {agent_name}" }
                    if has_task {
                        span { class: "task-preview", "{task_preview}" }
                    }
                }
                span { class: "tool-call-chevron", "{chev}" }
            }
            if *expanded.read() {
                div { class: "tool-call-content",
                    if has_task {
                        div { class: "task-prompt", "{task}" }
                    }
                    if is_loading {
                        div { class: "subagent-loading", "{tr(&lang, \"subagentLoading\")}" }
                    }
                    {session_els.iter().cloned()}
                    if show_turns {
                        div { class: "task-output-text",
                            div { class: "tool-read-meta", "{tr(&lang, \"turnsUsed\")}: {turns_val}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StatusIcon(status: ToolStatus) -> Element {
    match status {
        ToolStatus::Running => rsx! {
            svg {
                class: "tool-spin",
                xmlns: "http://www.w3.org/2000/svg",
                width: "12", height: "12",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2.5",
                stroke_linecap: "round",
                path { d: "M21 12a9 9 0 1 1-6.219-8.56" }
            }
        },
        ToolStatus::Done => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                width: "12", height: "12",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2.5",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                polyline { points: "20 6 9 17 4 12" }
            }
        },
        ToolStatus::Error => rsx! {
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                width: "12", height: "12",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2.5",
                stroke_linecap: "round",
                line { x1: "18", y1: "6", x2: "6", y2: "18" }
                line { x1: "6", y1: "6", x2: "18", y2: "18" }
            }
        },
    }
}

/// One subagent transcript (Nexus SubagentSessionBlock): collapsible header
/// with agent name and prompt/response counts, then the messages.
#[component]
fn SubagentSessionBlock(session: SubagentSessionView) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let open = use_signal(|| false);
    let visible: Vec<&crate::vibestats::SubagentMsg> = session
        .messages
        .iter()
        .filter(|m| m.role == "user" || m.role == "assistant")
        .collect();
    let user_count = visible.iter().filter(|m| m.role == "user").count();
    let assistant_count = visible.iter().filter(|m| m.role == "assistant").count();
    let chev_cls = if *open.read() {
        "subagent-chevron open"
    } else {
        "subagent-chevron"
    };

    let msg_els: Vec<Element> = visible
        .iter()
        .map(|m| {
            let row_cls = format!("subagent-msg subagent-msg-{}", m.role);
            let role_cls = format!("subagent-msg-role subagent-msg-role-{}", m.role);
            let role_label = if m.role == "user" {
                tr(&lang, "subagentPrompt").to_string()
            } else {
                tr(&lang, "subagentResponse").to_string()
            };
            let content = m.content.clone();
            rsx! {
                div { class: "{row_cls}",
                    span { class: "{role_cls}", "{role_label}" }
                    div { class: "subagent-msg-body",
                        if !content.is_empty() {
                            MarkdownLite { text: content }
                        }
                    }
                }
            }
        })
        .collect();

    rsx! {
        div { class: "subagent-session-block",
            button {
                class: "subagent-session-header",
                onclick: {
                    let mut open = open.clone();
                    move |_| {
                        let v = *open.read();
                        open.set(!v);
                    }
                },
                svg {
                    class: "{chev_cls}",
                    xmlns: "http://www.w3.org/2000/svg",
                    width: "12", height: "12",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2.5",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    polyline { points: "9 18 15 12 9 6" }
                }
                span { class: "subagent-agent-name", "{tr(&lang, \"subagent\")}: {session.agent_name}" }
                span { class: "subagent-meta", "{user_count} · {assistant_count}" }
            }
            if *open.read() {
                div { class: "subagent-messages",
                    {msg_els.iter().cloned()}
                }
            }
        }
    }
}

/// Full markdown renderer (see `crate::md`): blocks are rendered to HTML and
/// mounted through `dangerous_inner_html`. The HTML is fully escaped by the
/// renderer, so model output can never inject raw markup. Mermaid fences are
/// emitted as `.mermaid-container[data-src]` divs and rendered to SVG by
/// MERMAID_JS below (mermaid@10 from CDN, loaded lazily once per page).
#[component]
fn MarkdownLite(text: String) -> Element {
    if text.is_empty() {
        return VNode::empty();
    }
    let html = crate::md::to_html(&text);
    rsx! { div { class: "markdown-content", dangerous_inner_html: html } }
}

/// Client-side mermaid bootstrap. Runs once per page load: lazily injects the
/// mermaid script, then a MutationObserver renders every new
/// `.mermaid-container` (attribute `data-src` holds the escaped diagram
/// source). Dioxus never manages children of these divs, so hydration and
/// re-renders stay safe.
#[cfg(any(feature = "web", feature = "desktop"))]
const MERMAID_JS: &str = r#"
(function(){
  if (window.__vdMermaid) return; window.__vdMermaid = true;
  function load(cb){
    if (window.mermaid) return cb();
    if (window.__vdMermaidLoading) return window.__vdMermaidLoading.then(cb);
    window.__vdMermaidLoading = new Promise(function(res){
      var s = document.createElement('script');
      s.src = 'https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js';
      s.onload = res; s.onerror = res;
      document.head.appendChild(s);
    });
    window.__vdMermaidLoading.then(cb);
  }
  function renderEl(el){
    load(function(){
      if (!window.mermaid) {
        el.classList.add('mermaid-error');
        el.innerHTML = '<div class="mermaid-error-title">mermaid.js unavailable</div>';
        return;
      }
      try {
        window.mermaid.initialize({ startOnLoad: false, securityLevel: 'loose', theme: 'dark' });
        var id = 'vdGraph' + (el.__vdId || (el.__vdId = Math.random().toString(36).slice(2)));
        window.mermaid.render(id, el.getAttribute('data-src') || '', function(svg){
          el.innerHTML = svg; el.classList.add('mermaid-ready');
        }, function(err){
          el.classList.add('mermaid-error');
          el.innerHTML = '<div class="mermaid-error-title">Mermaid error</div><pre></pre>';
          el.querySelector('pre').textContent = String(err && err.message ? err.message : err);
        });
      } catch (e) {
        el.classList.add('mermaid-error');
        el.innerHTML = '<div class="mermaid-error-title">Mermaid error</div><pre></pre>';
        el.querySelector('pre').textContent = String(e);
      }
    });
  }
  function scan(){
    document.querySelectorAll('.mermaid-container:not([data-vd-done])').forEach(function(el){
      el.setAttribute('data-vd-done', '1');
      renderEl(el);
    });
  }
  var mo = new MutationObserver(function(){ scan(); });
  mo.observe(document.body, { childList: true, subtree: true });
  scan();
})();
"#;

/// Nexus v0.11.1: agent plan / todo checklist card, rendered after the
/// messages (cumulative state — the last `plan` update wins). Collapses
/// to the current entry; auto-collapses once everything is done.
#[component]
pub fn PlanCard(entries: Vec<PlanEntry>) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let mut collapsed_manual: Signal<Option<bool>> = use_signal(|| None);

    let all_done = !entries.is_empty() && entries.iter().all(|e| e.status == "completed");
    let collapsed = collapsed_manual().unwrap_or(all_done);
    let current = entries
        .iter()
        .find(|e| e.status == "in_progress")
        .or_else(|| entries.last())
        .cloned();
    let done = entries.iter().filter(|e| e.status == "completed").count();
    let total = entries.len();
    let card_cls = if collapsed {
        "plan-card plan-card-collapsed"
    } else {
        "plan-card"
    };
    let chevron_cls = if collapsed {
        "plan-card-chevron"
    } else {
        "plan-card-chevron open"
    };

    // Precompute the rows: collapsed -> only the current entry,
    // expanded -> the full checklist (Nexus ChatView markup 1:1).
    let rows: Vec<Element> = if collapsed {
        current
            .map(|e| rsx! { PlanRow { entry: e } })
            .into_iter()
            .collect()
    } else {
        entries
            .iter()
            .map(|e| rsx! { PlanRow { entry: e.clone() } })
            .collect()
    };

    rsx! {
        div { class: "{card_cls}",
            button {
                class: "plan-card-header",
                onclick: move |_| collapsed_manual.set(Some(!collapsed)),
                span { class: "plan-card-title", "{tr(&lang, \"planLabel\")}" }
                span { class: "plan-card-meta",
                    span { class: "plan-card-count", "{done}/{total}" }
                    span { class: "{chevron_cls}",
                        svg {
                            width: "10",
                            height: "10",
                            view_box: "0 0 10 10",
                            fill: "currentColor",
                            path { d: "M1 3l4 4 4-4" }
                        }
                    }
                }
            }
            {rows.iter().cloned()}
        }
    }
}

/// One plan row: status icon (✓ / ◐ / ○) + content text.
#[component]
fn PlanRow(entry: PlanEntry) -> Element {
    let icon = match entry.status.as_str() {
        "completed" => "✓",
        "in_progress" => "◐",
        _ => "○",
    };
    let cls = format!("plan-row plan-{}", entry.status);
    rsx! {
        div { class: "{cls}",
            span { class: "plan-icon", "{icon}" }
            span { class: "plan-content", "{entry.content}" }
        }
    }
}
