//! Chat view: empty state with the pixel flame, message list with user
//! bubbles (inline `@` attachment chips like Nexus `UserMessageContent`),
//! AI rows (thinking, tool calls, markdown), the agent busy indicator and
//! subagent task blocks with lazily loaded transcripts.
//! Ported from Nexus ChatView + MessageBubble (same CSS classes).

use dioxus::prelude::*;
use mistraldeck_core::types::{Attachment, Message, MessageSender, PlanEntry, ToolStatus};

use crate::i18n::{tr, tf};
use crate::state::{lang, ClientState};
use crate::vibestats::{PreviewMessageView, SubagentSessionView};

pub fn ChatView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let s = state.read();
    let messages = s.snapshot.current_messages.clone();
    let plan = s.snapshot.current_plan.clone();
    let busy = s.snapshot.busy;
    let status = s.snapshot.status.clone();
    let acp_error = s.snapshot.acp_error.clone();
    // Stop feedback (Nexus v0.10.5): session/cancel in flight — the busy
    // row label switches to "Stopping…".
    let stopping = s
        .snapshot
        .current
        .and_then(|i| s.snapshot.sessions.get(i))
        .map(|m| m.stopping)
        .unwrap_or(false);
    let busy_label = tr(
        &lang,
        if stopping { "stoppingLabel" } else { "agentWorkingLabel" },
    );
    // Lazy-resume overlay (Nexus isResuming): the current session's agent
    // is waking via session/load — spinner + "Resuming session… (N)" with
    // the replay chunk count.
    let current_acp_id = s
        .snapshot
        .current
        .and_then(|i| s.snapshot.sessions.get(i))
        .and_then(|m| m.acp_session_id.clone());
    let load_progress = s.snapshot.session_load_progress.clone();
    // CLI notice translation (Nexus translateCliNotice): technical
    // model names resolve to display names from the model option.
    let model_names: std::collections::HashMap<String, String> = s
        .snapshot
        .config_options
        .iter()
        .find(|o| o.id == "model")
        .map(|o| {
            o.choices
                .iter()
                .map(|c| (c.value.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    drop(s);
    let is_resuming = current_acp_id
        .as_deref()
        .zip(load_progress.as_ref().map(|p| p.session_id.as_str()))
        .map(|(a, p)| a == p)
        .unwrap_or(false);
    let resume_count = if is_resuming {
        load_progress.map(|p| p.count).unwrap_or(0)
    } else {
        0
    };

    // one-shot: install the mermaid MutationObserver renderer and the
    // markdown path-link click bridge (client only; the server SSR build
    // never evaluates JS)
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let mut js_init = use_signal(|| false);
        if !*js_init.read() {
            js_init.set(true);
            spawn(async move {
                dioxus::document::eval(MERMAID_JS);
                dioxus::document::eval(PATH_CLICK_JS);
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
            // hidden bridge for clicks on dangerously-mounted markdown path
            // links: PATH_CLICK_JS stores the raw path on window.__vdPath
            // and clicks this button — Rust cannot attach listeners to the
            // inner HTML itself.
            button {
                id: "vd-path-bridge",
                class: "vd-path-bridge",
                tabindex: "-1",
                onclick: move |_| {
                    let st = state.clone();
                    spawn(async move {
                        let eval = dioxus::document::eval("return window.__vdPath || '';");
                        if let Ok(p) = eval.join::<String>().await {
                            if !p.is_empty() {
                                dioxus::document::eval("window.__vdPath = '';");
                                crate::state::open_path(st, &p).await;
                            }
                        }
                    });
                }
            }
            div { class: "chat-messages",
                div { class: "chat-inner",
                    if messages.is_empty() && !is_resuming {
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
                                MessageBubble { msg: msg.clone(), streaming: !msg.is_complete && busy && idx == messages.len() - 1, lang: lang.clone(), model_names: model_names.clone() }
                            }
                            if !plan.is_empty() {
                                PlanCard { entries: plan.clone() }
                            }
                            if busy && !is_resuming {
                                div { class: "agent-busy-row",
                                    span { class: "agent-busy-dots", span {}, span {}, span {} }
                                    "{busy_label}"
                                }
                            }
                        }
                    }
                    // Lazy-resume overlay (Nexus ChatView isResuming):
                    // spinner + label, chunk count once the replay streams.
                    if is_resuming {
                        div { class: "agent-busy-row",
                            span { class: "agent-busy-spinner" }
                            "{tr(&lang, \"resumingSessionLabel\")}"
                            if resume_count > 0 {
                                " ({resume_count})"
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
pub fn MessageBubble(
    msg: Message,
    streaming: bool,
    lang: String,
    model_names: std::collections::HashMap<String, String>,
) -> Element {
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
                    div { class: "msg-user-col",
                        div { class: "msg-bubble",
                            div { class: "msg-text",
                                {seg_els.iter().cloned()}
                                {extra_els.iter().cloned()}
                            }
                        }
                        div { class: "msg-actions-row",
                            div { class: "msg-actions-left" }
                            div { class: "msg-actions-right",
                                CopyButton { text: msg.content.clone() }
                                div { class: "msg-time", "{time_str}" }
                            }
                        }
                    }
                }
            }
        }
        MessageSender::Ai => {
            rsx! {
                div { class: "msg-row msg-ai",
                    div { class: "msg-content",
                        StepsTimeline { msg: msg.clone(), is_agent_busy: streaming }
                        if !msg.content.is_empty() {
                            div { class: "msg-text",
                                MarkdownLite { text: msg.content.clone() }
                            }
                        }
                        if streaming {
                            span { class: "cursor-blink" }
                        }
                        div { class: "msg-actions-row",
                            div { class: "msg-actions-left" }
                            div { class: "msg-actions-right",
                                CopyButton { text: msg.content.clone() }
                                div { class: "msg-time", "{time_str}" }
                            }
                        }
                    }
                }
            }
        }
        MessageSender::System => {
            // CLI checkpoints ("Model changed to ...", "Compacting context",
            // ...) render as subtle notice rows - localized like Nexus
            // translateCliNotice; unknown notices pass through unchanged.
            let text = translate_cli_notice(&lang, &msg.content, &model_names);
            rsx! {
                div { class: "msg-row msg-notice",
                    div { class: "msg-content", "{text}" }
                }
            }
        }
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

// ===== Work timeline (harness steps shown above the assistant answer) =====
// Nexus v0.9.1 StepsTimeline 1:1: one muted summary line that expands into
// a vertical timeline of thinking + tool steps (chronological, per-step
// collapse), spinner + shimmer on the running step while the agent works.

#[component]
fn StepCheckIcon() -> Element {
    rsx! {
        svg {
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            circle { cx: "12", cy: "12", r: "10" }
            path { d: "M7.5 12L10.5 15L16.5 9" }
        }
    }
}

#[component]
fn StepSpinnerIcon() -> Element {
    rsx! {
        svg {
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2.5",
            stroke_linecap: "round",
            path { d: "M21 12a9 9 0 1 1-6.219-8.56" }
        }
    }
}

#[component]
fn StepErrorIcon() -> Element {
    rsx! {
        svg {
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            circle { cx: "12", cy: "12", r: "10" }
            path { d: "M9 9L15 15" }
            path { d: "M15 9L9 15" }
        }
    }
}

#[component]
fn StepChevronIcon() -> Element {
    rsx! {
        svg {
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "2",
            path { d: "M10.5 8L14.5 12L10.5 16" }
        }
    }
}

/// One chronological step of the timeline: a thinking block or a tool call.
#[derive(Clone, PartialEq)]
enum TimelineStep {
    Thinking(mistraldeck_core::types::ThinkingBlock),
    Tool(mistraldeck_core::types::ToolCall),
}

/// Merge thinking + tool steps, sorted by `started_at` (stable; tools without
/// timestamps fall back to the end, thinking to the front — Nexus parity).
fn build_timeline_steps(msg: &Message) -> Vec<TimelineStep> {
    let tools_len = msg.tool_calls.len();
    let mut keyed: Vec<(i64, usize, TimelineStep)> = Vec::new();
    for (i, b) in msg.thinking.iter().enumerate() {
        let order = if b.started_at > 0 { b.started_at } else { 0 };
        keyed.push((order, i, TimelineStep::Thinking(b.clone())));
    }
    for (i, tc) in msg.tool_calls.iter().enumerate() {
        let order = if tc.started_at > 0 {
            tc.started_at
        } else {
            i64::MAX - (tools_len - i) as i64
        };
        keyed.push((order, 100_000 + i, TimelineStep::Tool(tc.clone())));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    keyed.into_iter().map(|(_, _, s)| s).collect()
}

/// Short label for a step (timeline rows + the summary line).
fn step_label(step: &TimelineStep, lang: &str) -> String {
    match step {
        TimelineStep::Thinking(b) => {
            if !b.is_complete {
                return tr(lang, "thinkingThinking").to_string();
            }
            if b.started_at > 0 {
                let end = if b.completed_at > 0 {
                    b.completed_at
                } else {
                    mistraldeck_core::types::now_ms()
                };
                let seconds = ((end - b.started_at) / 1000).max(0);
                if seconds >= 1 {
                    return tf(
                        lang,
                        "thoughtForLabel",
                        &[("seconds", &seconds.to_string())],
                    );
                }
            }
            tr(lang, "thinking").to_string()
        }
        TimelineStep::Tool(tc) => {
            // Nexus stepLabel: translate action prefixes ("Applied …",
            // "bash: …"), then build a title from the raw function name +
            // input (file names, commands, patterns).
            let translated = translate_tool_title(&tc.title, lang);
            if translated != tc.title {
                return translated;
            }
            raw_function_name_to_title(&tc.title, &tool_input(tc), lang)
        }
    }
}

#[component]
fn StepsTimeline(msg: Message, is_agent_busy: bool) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let manual: Signal<Option<bool>> = use_signal(|| None);

    let steps = build_timeline_steps(&msg);
    if steps.is_empty() {
        return VNode::empty();
    }

    let any_running = steps.iter().any(|s| match s {
        TimelineStep::Thinking(b) => !b.is_complete,
        TimelineStep::Tool(tc) => tc.status == ToolStatus::Running,
    });
    let active = any_running || is_agent_busy;
    let open = (*manual.read()).unwrap_or(active);
    let summary = steps
        .iter()
        .take(4)
        .map(|s| step_label(s, &lang))
        .collect::<Vec<_>>()
        .join(" · ");

    let mut section_cls = String::from("steps-section");
    if open {
        section_cls.push_str(" open");
    }
    if active {
        section_cls.push_str(" steps-active");
    }
    let summary_cls = if active {
        "steps-summary-text steps-shimmer"
    } else {
        "steps-summary-text"
    };

    let steps_len = steps.len();
    let row_els: Vec<Element> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            rsx! {
                StepRow { step: s.clone(), last: i == steps_len - 1 }
            }
        })
        .collect();

    rsx! {
        div { class: "{section_cls}",
            button {
                class: "steps-summary",
                onclick: {
                    let mut manual = manual.clone();
                    move |_| {
                        manual.set(Some(!open));
                    }
                },
                span { class: "{summary_cls}", "{summary}" }
                span { class: "steps-chevron", StepChevronIcon {} }
            }
            if open {
                div { class: "steps-list",
                    {row_els.iter().cloned()}
                }
            }
        }
    }
}

#[component]
fn StepRow(step: TimelineStep, last: bool) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let open_sig: Signal<bool> = use_signal(|| false);

    let (running, error) = match &step {
        TimelineStep::Thinking(b) => (!b.is_complete, false),
        TimelineStep::Tool(tc) => (tc.status == ToolStatus::Running, tc.status == ToolStatus::Error),
    };
    let status_cls = if running {
        "step-running"
    } else if error {
        "step-error"
    } else {
        "step-done"
    };
    let mut row_cls = format!("step-row {status_cls}");
    if *open_sig.read() {
        row_cls.push_str(" open");
    }
    let label = step_label(&step, &lang);
    let label_cls = if running {
        "step-label steps-shimmer"
    } else {
        "step-label"
    };

    let content: Element = match &step {
        TimelineStep::Thinking(b) => {
            let para_els: Vec<Element> = b
                .content
                .split("\n\n")
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .map(|p| rsx! { p { "{p}" } })
                .collect();
            rsx! {
                div { class: "step-reasoning",
                    {para_els.iter().cloned()}
                }
            }
        }
        TimelineStep::Tool(tc) => {
            let input = tc.input_json.clone().unwrap_or_default();
            if is_subagent_tool(tc) {
                rsx! { SubagentBlock { tc: tc.clone() } }
            } else if is_task_tool(&tc.title, &tc.kind, &input) {
                rsx! { TaskToolBlock { tc: tc.clone() } }
            } else if let Some(rich) = tool_call_rich(tc, &lang) {
                rich
            } else {
                rsx! { ToolCallBody { tc: tc.clone() } }
            }
        }
    };

    rsx! {
        div { class: "{row_cls}",
            div { class: "step-rail",
                span { class: "step-icon",
                    if running {
                        StepSpinnerIcon {}
                    } else if error {
                        StepErrorIcon {}
                    } else {
                        StepCheckIcon {}
                    }
                }
                if !last {
                    span { class: "step-connector" }
                }
            }
            div { class: "step-body",
                button {
                    class: "step-header",
                    onclick: {
                        let mut open_sig = open_sig.clone();
                        move |_| {
                            let v = *open_sig.read();
                            open_sig.set(!v);
                        }
                    },
                    span { class: "{label_cls}", "{label}" }
                    span { class: "step-chevron", StepChevronIcon {} }
                }
                if *open_sig.read() {
                    div { class: "step-content",
                        {content}
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

/// Nexus v0.13 subagent routing (ToolCallRichContent): calls with subagent
/// `_meta` or the canonical `agents.spawn` / `task` tool name render as the
/// SubagentBlock card — intercepted before the post-hoc TaskToolBlock.
fn is_subagent_tool(tc: &mistraldeck_core::types::ToolCall) -> bool {
    if tc.subagent.is_some() {
        return true;
    }
    let tn = tc.tool_name.to_lowercase();
    tn == "agents.spawn" || tn == "task"
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

// ---------------------------------------------------------------------------
// Rich tool call content — port of Nexus v0.12 `ToolCallRichContent` routing:
// EditDiffBlock (git-style hunks), BashBlock, ReadFileBlock, ProcessBlock,
// ToolLineBlock and the SEARCH/REPLACE edit fallback with grep/glob cards.
// ---------------------------------------------------------------------------

/// Parsed tool input as a JSON value (empty object when absent/broken).
fn tool_input(tc: &mistraldeck_core::types::ToolCall) -> serde_json::Value {
    tc.input_json
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()))
}

/// First non-empty string field among the given keys.
fn inp_str(input: &serde_json::Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(s) = input.get(*key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

/// Port of Nexus `parseToolOutput`: JSON array of content blocks → joined
/// texts; object with text/result/output → that string; otherwise the
/// trimmed raw text (None when empty).
fn parse_tool_output(raw: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        match &v {
            serde_json::Value::Array(items) => {
                let texts: Vec<String> = items
                    .iter()
                    .filter_map(|e| {
                        let ty = e.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if ty == "content" || ty == "tool_error" {
                            e.pointer("/content/text")
                                .and_then(|t| t.as_str())
                                .map(str::to_string)
                        } else {
                            None
                        }
                    })
                    .collect();
                if !texts.is_empty() {
                    return Some(texts.join("\n"));
                }
            }
            obj @ serde_json::Value::Object(_) => {
                let val = obj
                    .get("text")
                    .or_else(|| obj.get("result"))
                    .or_else(|| obj.get("output"));
                if let Some(s) = val.and_then(|v| v.as_str()) {
                    return Some(s.to_string());
                }
            }
            _ => {}
        }
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// --- step label helpers (Nexus `translateToolTitle` + -----------------------
// `rawFunctionNameToTitleFromInput`) ---------------------------------------

/// Strip a trailing parenthetical suffix with surrounding whitespace
/// (JS `/\s*\([^)]*\)\s*$/`).
fn strip_trailing_paren(s: &str) -> String {
    let t = s.trim_end();
    if t.ends_with(')') {
        if let Some(open) = t.rfind('(') {
            let inner = &t[open + 1..t.len() - 1];
            if !inner.contains(')') && !inner.contains('(') {
                return t[..open].trim_end().to_string();
            }
        }
    }
    t.to_string()
}

/// Port of Nexus `translateToolTitle`: translated prefixes for action words,
/// the "bash: <cmd>" format, and the "Applied …" confirmation labels.
fn translate_tool_title(title: &str, lang: &str) -> String {
    const TOOL_ACTION_KEYS: &[(&str, &str)] = &[
        ("reading", "toolReading"),
        ("patching", "toolPatching"),
        ("running", "toolRunning"),
        ("searching", "toolSearching"),
        ("listing", "toolListing"),
        ("writing", "toolWriting"),
        ("creating", "toolCreating"),
        ("deleting", "toolDeleting"),
    ];
    let lower = title.to_lowercase();
    for (word, key) in TOOL_ACTION_KEYS {
        if lower.starts_with(word) {
            let suffix = strip_trailing_paren(&title[word.len()..]);
            return format!("{}{}", tr(lang, key), suffix);
        }
    }
    if lower.starts_with("bash:") {
        let cmd = title[5..].trim();
        if !cmd.is_empty() {
            return format!("{} {}", tr(lang, "toolRunning"), strip_trailing_paren(cmd));
        }
    }
    if lower.starts_with("applied") {
        let rest = title[7..].trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && rest[digits.len()..].trim_start().starts_with("block") {
            return tf(lang, "toolApplied", &[("count", &digits)]);
        }
        if rest.is_empty() || !rest.as_bytes()[0].is_ascii_alphanumeric() {
            return tr(lang, "toolAppliedOk").to_string();
        }
    }
    title.to_string()
}

/// Port of Nexus `rawFunctionNameToTitleFromInput`: display title built from
/// the raw ACP function name + parsed input (file names, commands, patterns).
fn raw_function_name_to_title(
    title: &str,
    input: &serde_json::Value,
    lang: &str,
) -> String {
    let lower = title.to_lowercase();
    let file = inp_str(input, &["path", "file_path", "filePath", "filename"]);
    let file = if file.is_empty() {
        String::new()
    } else {
        let name = file.rsplit(['/', '\\']).next().unwrap_or_default();
        format!(" {name}")
    };
    if lower == "read_file" || lower == "view" || lower == "read" {
        return format!("{}{}", tr(lang, "toolReading"), file);
    }
    if lower == "search_replace"
        || lower == "str_replace"
        || lower == "edit_file"
        || lower == "edit"
        || lower.ends_with(".edit")
    {
        return format!("{}{}", tr(lang, "toolPatching"), file);
    }
    if lower == "bash"
        || lower == "execute"
        || lower == "run_command"
        || lower == "shell"
        || lower.ends_with(".bash")
        || lower.ends_with(".execute")
    {
        let cmd: String = inp_str(input, &["command", "cmd", "script"])
            .chars()
            .take(50)
            .collect();
        let suffix = if cmd.is_empty() { String::new() } else { format!(" {cmd}") };
        return format!("{}{}", tr(lang, "toolRunning"), suffix);
    }
    if lower == "grep" || lower == "find_in_file" || lower == "ripgrep" {
        let pattern: String = inp_str(input, &["pattern", "query", "regex"])
            .chars()
            .take(50)
            .collect();
        let suffix = if pattern.is_empty() { String::new() } else { format!(" {pattern}") };
        return format!("{}{}", tr(lang, "toolSearching"), suffix);
    }
    if lower == "glob" || lower == "list_dir" || lower == "ls" {
        let pattern: String = inp_str(input, &["pattern"]).chars().take(50).collect();
        if pattern.is_empty() {
            return format!("{}{}", tr(lang, "toolListing"), file);
        }
        return format!("{} {}", tr(lang, "toolListing"), pattern);
    }
    if lower == "write_file" || lower == "create_file" {
        return format!("{}{}", tr(lang, "toolCreating"), file);
    }
    if lower == "delete_file" {
        return format!("{}{}", tr(lang, "toolDeleting"), file);
    }
    title.to_string()
}

// --- EditDiffBlock (git-style hunks) ---------------------------------------

/// One git-style diff row (context / deleted / added).
#[derive(Clone)]
struct DiffRow {
    kind: &'static str,
    old_no: Option<i64>,
    new_no: Option<i64>,
    text: String,
}

/// Split into lines, dropping a single trailing empty line (files normally
/// end with a newline). Port of Nexus `cutLines`.
fn cut_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if lines.last().map_or(false, |l| l.is_empty()) {
        lines.pop();
    }
    lines
}

/// Port of Nexus `buildDiffRows`: git-style rows for one edit hunk with
/// 2 context lines before/after when the pre-edit content is known.
fn build_diff_rows(
    d: &mistraldeck_core::types::DiffHunk,
    previous_content: Option<&str>,
) -> Vec<DiffRow> {
    let old_lines: Vec<String> = d.old_text.as_deref().map(cut_lines).unwrap_or_default();
    let new_lines = cut_lines(&d.new_text);
    let sl = d.start_line.filter(|n| *n > 0);
    let nsl = d.new_start_line.filter(|n| *n > 0).or(sl);
    let prev_lines: Option<Vec<String>> = match (sl, previous_content) {
        (Some(_), Some(pc)) => Some(cut_lines(pc)),
        _ => None,
    };
    let old_start = sl.unwrap_or(1);
    let new_start = nsl.unwrap_or(old_start);
    let mut rows: Vec<DiffRow> = Vec::new();
    if let Some(prev) = &prev_lines {
        let from = old_start - 3;
        for i in from..(old_start - 1) {
            if i >= 0 && (i as usize) < prev.len() {
                rows.push(DiffRow {
                    kind: "ctx",
                    old_no: Some(i + 1),
                    new_no: Some(i + 1),
                    text: prev[i as usize].clone(),
                });
            }
        }
    }
    for (i, l) in old_lines.iter().enumerate() {
        rows.push(DiffRow {
            kind: "del",
            old_no: Some(old_start + i as i64),
            new_no: None,
            text: l.clone(),
        });
    }
    for (i, l) in new_lines.iter().enumerate() {
        rows.push(DiffRow {
            kind: "add",
            old_no: None,
            new_no: Some(new_start + i as i64),
            text: l.clone(),
        });
    }
    if let Some(prev) = &prev_lines {
        let delta = new_lines.len() as i64 - old_lines.len() as i64;
        let after_from = old_start - 1 + old_lines.len() as i64;
        let mut i = after_from;
        while i < after_from + 2 && (i as usize) < prev.len() {
            rows.push(DiffRow {
                kind: "ctx",
                old_no: Some(i + 1),
                new_no: Some(i + 1 + delta),
                text: prev[i as usize].clone(),
            });
            i += 1;
        }
    }
    rows
}

/// Port of Nexus `ToolFilePathLink`: tool-call path row with a ↗ quick
/// preview button (directories reveal in the dock Files tab, files open
/// a preview tab, the dock opens itself if closed).
fn tool_path_chip(path: &str) -> Element {
    rsx! { ToolPathChip { path: path.to_string() } }
}

#[component]
fn ToolPathChip(path: String) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    rsx! {
        div { class: "tool-file-path-row",
            span { class: "tool-file-path", "{path}" }
            button {
                class: "tool-path-preview-btn",
                title: "{tr(&lang, \"openPreview\")}",
                onclick: move |_| {
                    let st = state.clone();
                    let raw = path.clone();
                    spawn(async move { crate::state::open_path(st, &raw).await; });
                },
                "↗"
            }
        }
    }
}

/// Port of Nexus `extractToolCallPath`: primary file/directory path from a
/// tool-call input, if present.
fn extract_tool_call_path(input_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(input_json).ok()?;
    for key in ["path", "file_path", "filePath", "filename", "file", "dir", "directory"] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Port of Nexus `ToolCallPathChip`: clickable path chip shown in expanded
/// tool calls — opens the file preview (directories reveal in the Files
/// tab of the dock).
#[component]
fn ToolCallPathChip(path: String) -> Element {
    let state = use_context::<Signal<ClientState>>();
    rsx! {
        button {
            class: "tool-call-path",
            title: "{path}",
            onclick: move |_| {
                let st = state.clone();
                let raw = path.clone();
                spawn(async move { crate::state::open_path(st, &raw).await; });
            },
            "{path}"
        }
    }
}

/// Port of Nexus `EditDiffBlock`: structured edit diffs (ACP diff content
/// blocks / edit tool input) rendered as git-style hunks with line numbers,
/// `@@` headers, +/- coloring and an error/success footer.
fn edit_diff_block(tc: &mistraldeck_core::types::ToolCall, lang: &str) -> Element {
    let out_str = tc.output.clone().unwrap_or_default();
    let is_error = out_str.contains("<tool_error>");
    let err_text = out_str
        .replace("<tool_error>", "")
        .replace("</tool_error>", "")
        .trim()
        .to_string();
    let ok_label = tr(lang, "toolAppliedOk").to_string();
    let wrap_cls = if is_error { "tool-rich-error" } else { "tool-rich-diff" };

    let pair_els: Vec<Element> = tc
        .diffs
        .iter()
        .map(|d| {
            let rows: Vec<DiffRow> = build_diff_rows(d, tc.previous_content.as_deref())
                .into_iter()
                .take(400)
                .collect();
            let sl = d.start_line.filter(|n| *n > 0);
            let nsl = d.new_start_line.filter(|n| *n > 0).or(sl);
            let old_count = rows.iter().filter(|r| r.kind != "add").count();
            let new_count = rows.iter().filter(|r| r.kind != "del").count();
            let hunk = match (sl, nsl) {
                (Some(sl), Some(nsl)) => format!("@@ -{sl},{old_count} +{nsl},{new_count} @@"),
                _ => String::new(),
            };
            let path = d.path.clone();
            let path_el: Element = if path.is_empty() {
                VNode::empty()
            } else {
                tool_path_chip(&path)
            };
            let hunk_el: Element = if hunk.is_empty() {
                VNode::empty()
            } else {
                rsx! { div { class: "tool-diff-hunk", "{hunk}" } }
            };
            let row_els: Vec<Element> = rows
                .iter()
                .map(|r| {
                    let cls = format!("tool-diff-row tool-diff-{}", r.kind);
                    let sign = match r.kind {
                        "del" => "-",
                        "add" => "+",
                        _ => " ",
                    };
                    let old = r.old_no.map(|n| n.to_string()).unwrap_or_default();
                    let new = r.new_no.map(|n| n.to_string()).unwrap_or_default();
                    let text = r.text.clone();
                    rsx! {
                        div { class: "{cls}",
                            span { class: "tool-diff-lno", "{old}" }
                            span { class: "tool-diff-lno", "{new}" }
                            span { class: "tool-diff-sign", "{sign}" }
                            span { class: "tool-diff-text", "{text}" }
                        }
                    }
                })
                .collect();
            rsx! {
                div { class: "tool-diff-pair",
                    {path_el}
                    {hunk_el}
                    div { class: "tool-diff-table",
                        {row_els.iter().cloned()}
                    }
                }
            }
        })
        .collect();

    let footer_el: Element = if is_error {
        rsx! { pre { class: "tool-error-output", "{err_text}" } }
    } else {
        rsx! { span { class: "tool-success-badge", "✓ {ok_label}" } }
    };

    rsx! {
        div { class: "{wrap_cls}",
            {pair_els.iter().cloned()}
            {footer_el}
        }
    }
}

// --- BashBlock --------------------------------------------------------------

/// Structured `command:/stdout:/stderr:/returncode:` bash output.
struct BashOut {
    stdout: Option<String>,
    stderr: Option<String>,
    rc: Option<i64>,
}

fn parse_bash_output(raw: &str) -> BashOut {
    let mut sections: std::collections::BTreeMap<String, String> = Default::default();
    let mut current: Option<String> = None;
    for line in raw.split('\n') {
        let mut matched: Option<(String, String)> = None;
        for key in ["command", "stdout", "stderr", "returncode"] {
            if line.starts_with(key) && line[key.len()..].starts_with(": ") {
                matched = Some((key.to_string(), line[key.len() + 2..].to_string()));
                break;
            }
        }
        match matched {
            Some((k, v)) => {
                sections.insert(k.clone(), v);
                current = Some(k);
            }
            None => {
                if let Some(cur) = &current {
                    let entry = sections.entry(cur.clone()).or_default();
                    entry.push('\n');
                    entry.push_str(line);
                }
            }
        }
    }
    BashOut {
        stdout: sections.get("stdout").cloned(),
        stderr: sections.get("stderr").map(|s| s.trim_end().to_string()),
        rc: sections
            .get("returncode")
            .and_then(|s| s.trim().parse::<i64>().ok()),
    }
}

/// Port of Nexus `BashBlock`: `$ cmd` line + stdout, stderr only when the
/// return code is non-zero; the one-line "Ran X" summary shows no output.
fn bash_block(tc: &mistraldeck_core::types::ToolCall) -> Element {
    let input = tool_input(tc);
    let title_cmd = if tc.title.len() >= 5 && tc.title[..5].eq_ignore_ascii_case("bash:") {
        tc.title[5..].trim().to_string()
    } else {
        String::new()
    };
    let command = {
        let c = inp_str(&input, &["command", "cmd", "script", "code"]);
        if c.is_empty() {
            title_cmd
        } else {
            c
        }
    }
    .trim()
    .to_string();

    let raw_out = parse_tool_output(&tc.output.clone().unwrap_or_default());
    let is_summary_only = match &raw_out {
        Some(out) => {
            let t = out.trim();
            t.len() >= 3
                && t[..3].eq_ignore_ascii_case("ran")
                && t.as_bytes().get(3) == Some(&b' ')
                && !t[3..].trim_start().is_empty()
                && !out.contains('\n')
        }
        None => true,
    };
    let display_out: Option<String> = if is_summary_only { None } else { raw_out };

    let (stdout_text, stderr_text) = match &display_out {
        Some(out) if out.contains("stdout:") || out.contains("returncode:") => {
            let parsed = parse_bash_output(out);
            let stdout = parsed
                .stdout
                .map(|s| s.trim_end().to_string())
                .filter(|s| !s.is_empty());
            let stderr = match (&parsed.stderr, parsed.rc) {
                (Some(e), Some(rc)) if !e.is_empty() && rc != 0 => Some(e.clone()),
                _ => None,
            };
            (stdout, stderr)
        }
        Some(out) => (Some(out.clone()), None),
        None => (None, None),
    };

    let cmd_el: Element = if command.is_empty() {
        VNode::empty()
    } else {
        rsx! {
            div { class: "tool-bash-command",
                span { class: "tool-bash-prompt", "$" }
                span { class: "tool-bash-cmd", "{command}" }
            }
        }
    };
    let out_el: Element = match &stdout_text {
        Some(out) => rsx! { pre { class: "tool-bash-output", "{out}" } },
        None => VNode::empty(),
    };
    let err_el: Element = match &stderr_text {
        Some(err) => rsx! { pre { class: "tool-bash-output tool-bash-stderr", "{err}" } },
        None => VNode::empty(),
    };

    rsx! {
        div { class: "tool-rich-bash",
            {cmd_el}
            {out_el}
            {err_el}
        }
    }
}

// --- ReadFileBlock ----------------------------------------------------------

/// Translate `lines_read:` / `was_truncated:` meta keys (Nexus parity).
fn translate_meta_line(line: &str, lang: &str) -> String {
    if let Some(rest) = line.strip_prefix("lines_read:") {
        return format!("{}: {}", tr(lang, "linesRead"), rest.trim());
    }
    if let Some(rest) = line.strip_prefix("was_truncated:") {
        let val = rest.trim().to_lowercase();
        let word = if val == "true" { tr(lang, "yes") } else { tr(lang, "no") };
        return format!("{}: {}", tr(lang, "truncated"), word);
    }
    line.to_string()
}

/// "Read N lines from <file>" summary (JS `/Read\s+(\d+)\s+lines?\s+from/i`).
fn lines_read_summary(out: &str, lang: &str) -> Option<String> {
    let lower = out.to_lowercase();
    let toks: Vec<&str> = lower.split_whitespace().collect();
    if toks.len() >= 3
        && toks[0] == "read"
        && !toks[1].is_empty()
        && toks[1].chars().all(|c| c.is_ascii_digit())
        && toks[2].starts_with("lines")
    {
        return Some(format!("{}: {}", tr(lang, "linesRead"), toks[1]));
    }
    None
}

/// Port of Nexus `ReadFileBlock`: file path chip (+ `:start-end` range) and
/// the structured `path:/content:/lines_read:/was_truncated:` output.
fn read_file_block(tc: &mistraldeck_core::types::ToolCall, lang: &str) -> Element {
    let input = tool_input(tc);
    let input_path = inp_str(&input, &["path", "file_path", "filePath"]);
    let start_line = input
        .get("start_line")
        .or_else(|| input.get("startLine"))
        .and_then(|v| v.as_i64());
    let end_line = input
        .get("end_line")
        .or_else(|| input.get("endLine"))
        .and_then(|v| v.as_i64());
    let line_range = match start_line {
        Some(s) => {
            let end = end_line.map(|e| format!("-{e}")).unwrap_or_default();
            format!(":{s}{end}")
        }
        None => String::new(),
    };
    let raw_out = parse_tool_output(&tc.output.clone().unwrap_or_default());
    let has_structured = raw_out
        .as_deref()
        .map(|o| o.contains("\ncontent:") || o.starts_with("content:"))
        .unwrap_or(false);

    // Structured branch: classify path/meta/content lines.
    let mut chip = String::new();
    let mut meta_lines: Vec<String> = Vec::new();
    let mut content_lines: Vec<String> = Vec::new();
    let mut in_content = false;
    if has_structured {
        if let Some(out) = &raw_out {
            let lines: Vec<&str> = out.split('\n').collect();
            for i in 0..lines.len() {
                let line = lines[i];
                if line.starts_with("lines_read:") || line.starts_with("was_truncated:") {
                    if in_content {
                        content_lines.push(line.to_string());
                    } else {
                        meta_lines.push(translate_meta_line(line, lang));
                    }
                } else if line.starts_with("path:") {
                    if !in_content && chip.is_empty() {
                        chip = line[5..].trim().to_string();
                    } else {
                        meta_lines.push(line.to_string());
                    }
                } else if line.starts_with("content:") {
                    in_content = true;
                    let part = line[8..].trim_start();
                    if !part.is_empty() {
                        content_lines.push(part.to_string());
                    }
                } else if in_content {
                    content_lines.push(line.to_string());
                } else if chip.is_empty() {
                    // pre-content line: the path (with or without the prefix)
                    chip = if i + 1 < lines.len() && lines[i + 1].starts_with("lines_read:") {
                        line.to_string()
                    } else {
                        line.to_string()
                    };
                } else {
                    meta_lines.push(line.to_string());
                }
            }
        }
        chip = format!("{}{}", chip, line_range);
    } else {
        chip = format!("{}{}", input_path, line_range);
        if let Some(out) = &raw_out {
            if let Some(meta) = lines_read_summary(out, lang) {
                meta_lines.push(meta);
            }
        }
    }
    let content_text = if content_lines.is_empty() {
        String::new()
    } else {
        content_lines.join("\n")
    };
    // Non-structured outputs that are not the "Read N lines" summary carry
    // the file content themselves (older CLI sessions).
    let fallback_content = if !has_structured {
        raw_out
            .as_ref()
            .filter(|o| lines_read_summary(o, lang).is_none())
            .cloned()
            .unwrap_or_default()
    } else {
        String::new()
    };
    let content_text = if content_text.is_empty() {
        fallback_content
    } else {
        content_text
    };

    let meta_els: Vec<Element> = meta_lines
        .iter()
        .map(|m| rsx! { div { class: "tool-read-meta", "{m}" } })
        .collect();
    let content_el: Element = if content_text.is_empty() {
        VNode::empty()
    } else {
        rsx! { pre { class: "tool-file-content", "{content_text}" } }
    };

    rsx! {
        div { class: "tool-rich-read",
            if !chip.is_empty() {
                div { class: "tool-file-path", "{chip}" }
            }
            {meta_els.iter().cloned()}
            {content_el}
        }
    }
}

// --- ProcessBlock -----------------------------------------------------------

/// Port of Nexus `processRawFrom`: unwrap `structured_content` from the raw
/// output object; fall back to a JSON dump in the output text.
fn process_raw_from(tc: &mistraldeck_core::types::ToolCall) -> serde_json::Value {
    if let Some(raw) = &tc.raw_output_json {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
            if v.is_object() {
                if let Some(sc) = v.get("structured_content").or_else(|| v.get("structuredContent")) {
                    if sc.is_object() {
                        return sc.clone();
                    }
                }
                return v;
            }
        }
    }
    if let Some(out) = &tc.output {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(out) {
            if v.is_object() {
                return v;
            }
        }
    }
    serde_json::Value::Object(Default::default())
}

/// Port of Nexus `ProcessBlock`: process.start/list/stop card — command,
/// process table [command, status · exit N], and output text.
fn process_block(tc: &mistraldeck_core::types::ToolCall) -> Element {
    let input = tool_input(tc);
    let ro = process_raw_from(tc);

    let cmd = {
        let c = inp_str(&input, &["command", "cmd"]);
        if !c.is_empty() {
            c
        } else {
            match ro.get("processId").or_else(|| ro.get("process_id")) {
                Some(serde_json::Value::Number(n)) => n.to_string(),
                Some(serde_json::Value::String(s)) => s.clone(),
                _ => String::new(),
            }
        }
    };
    let processes = ro.get("processes").and_then(|p| p.as_array()).cloned();
    let out_text = {
        let o = ro
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if !o.is_empty() {
            o
        } else {
            tc.output
                .clone()
                .and_then(|raw| parse_tool_output(&raw))
                .unwrap_or_default()
        }
    };
    let status_str = match ro.get("status") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let exit_top = ro
        .get("exitCode")
        .or_else(|| ro.get("exit_code"))
        .and_then(|v| v.as_i64());

    let cmd_el: Element = if cmd.is_empty() {
        VNode::empty()
    } else {
        rsx! { div { class: "proc-cmd", "{cmd}" } }
    };

    let row_els: Vec<Element> = processes
        .iter()
        .flatten()
        .map(|p| {
            let cmd = p
                .get("command")
                .or_else(|| p.get("cmd"))
                .or_else(|| p.get("processId"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let status = p
                .get("status")
                .map(|s| match s {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .unwrap_or_default();
            let exit = p
                .get("exitCode")
                .or_else(|| p.get("exit_code"))
                .and_then(|v| v.as_i64());
            let mut cls = String::from("proc-status");
            if status == "running" {
                cls.push_str(" running");
            }
            let text = format!(
                "{}{}",
                status,
                exit.map(|e| format!(" · exit {e}")).unwrap_or_default()
            );
            rsx! {
                div { class: "proc-tr",
                    span { class: "proc-tr-cmd", "{cmd}" }
                    span { class: "{cls}", "{text}" }
                }
            }
        })
        .collect();

    let meta_el: Element = if processes.is_some() {
        rsx! {
            div { class: "proc-table",
                {row_els.iter().cloned()}
            }
        }
    } else {
        let mut cls = String::from("proc-status");
        if status_str == "running" {
            cls.push_str(" running");
        }
        let text = format!(
            "{}{}",
            status_str,
            exit_top.map(|e| format!(" · exit {e}")).unwrap_or_default()
        );
        let status_el: Element = if status_str.is_empty() {
            VNode::empty()
        } else {
            rsx! { span { class: "{cls}", "{text}" } }
        };
        let out_el: Element = if out_text.is_empty() {
            VNode::empty()
        } else {
            rsx! { pre { class: "proc-out", "{out_text}" } }
        };
        rsx! {
            div { class: "proc-card-meta",
                {status_el}
                {out_el}
            }
        }
    };

    rsx! {
        div { class: "proc-card",
            {cmd_el}
            {meta_el}
        }
    }
}

// --- ToolLineBlock ----------------------------------------------------------

/// Port of Nexus `ToolLineBlock`: muted one-liner for discovery-style tools
/// (`search_tool_functions`, `vibe.todo`).
fn tool_line_block(tc: &mistraldeck_core::types::ToolCall) -> Element {
    let text = parse_tool_output(&tc.output.clone().unwrap_or_default())
        .filter(|o| !o.is_empty())
        .unwrap_or_else(|| tc.title.clone());
    rsx! {
        div { class: "tool-discovery-line",
            span { class: "tool-discovery-dot", "·" }
            span { "{text}" }
        }
    }
}

// --- SEARCH/REPLACE edit fallback -------------------------------------------

/// Strip SEARCH/REPLACE marker lines from a block.
fn parse_diff_block(block: &str) -> Vec<String> {
    block
        .split('\n')
        .filter(|l| {
            !l.starts_with("<<<<<<<") && !l.starts_with("=======") && !l.starts_with(">>>>>>>")
        })
        .map(str::to_string)
        .collect()
}

/// Split a full `<<<<<<< … ======= … >>>>>>>` block into (old, new).
fn split_search_replace(text: &str) -> Option<(String, String)> {
    let start = text.find("<<<<<<<")?;
    let after_marker = text[start..].find('\n')? + start + 1;
    let sep = text[after_marker..].find("=======")? + after_marker;
    let end = text[sep..].find(">>>>>>>")? + sep;
    let mut old = text[after_marker..sep].to_string();
    if old.ends_with('\n') {
        old.pop();
    }
    let mut mid = sep + 7;
    if text[mid..].starts_with('\n') {
        mid += 1;
    }
    let mut new = text[mid..end].to_string();
    if new.ends_with('\n') {
        new.pop();
    }
    if old.is_empty() && new.is_empty() {
        return None;
    }
    Some((old, new))
}

/// One batch edit pair (`changes: [{ oldString, newString, replaceAll }]`).
struct BatchPair {
    old_str: String,
    new_str: String,
    replace_all: bool,
}

/// "Applied N block(s)" count (JS `/^Applied\s+(\d+)\s+block/i`).
fn applied_count(out: &str) -> Option<String> {
    let lower = out.to_lowercase();
    if !lower.starts_with("applied") {
        return None;
    }
    let rest = lower[7..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    if rest[digits.len()..].trim_start().starts_with("block") {
        Some(digits)
    } else {
        None
    }
}

/// The ✓ success badge with the check icon (Nexus `statusBadge`).
fn applied_badge(out_str: &str, lang: &str) -> Element {
    let label = match applied_count(out_str) {
        Some(count) => tf(lang, "toolApplied", &[("count", &count)]),
        None => tr(lang, "toolAppliedOk").to_string(),
    };
    rsx! {
        span { class: "tool-success-badge",
            svg { width: "12", height: "12", view_box: "0 0 24 24", fill: "none",
                stroke: "currentColor", stroke_width: "2.5", stroke_linecap: "round", stroke_linejoin: "round",
                polyline { points: "20 6 9 17 4 12" }
            }
            "{label}"
        }
    }
}

/// Port of the Nexus edit branch of `ToolCallRichContent`: SEARCH/REPLACE
/// blocks, batch `changes[]`, error output, and success badges.
fn edit_fallback_block(
    tc: &mistraldeck_core::types::ToolCall,
    input: &serde_json::Value,
    lang: &str,
) -> Element {
    let path = inp_str(input, &["path", "file_path", "filePath", "filename"]);
    let out = parse_tool_output(&tc.output.clone().unwrap_or_default());
    let out_str = out.clone().unwrap_or_default();
    let is_error = out_str.contains("<tool_error>");

    let mut old_str = inp_str(
        input,
        &[
            "oldString",
            "old_string",
            "old_str",
            "oldText",
            "search",
            "search_text",
            "target",
            "old_content",
        ],
    );
    let mut new_str = inp_str(
        input,
        &[
            "newString",
            "new_string",
            "new_str",
            "newText",
            "replace",
            "replace_text",
            "new_content",
        ],
    );

    // The full SEARCH/REPLACE block may live in `text`/`content`.
    if old_str.is_empty() && new_str.is_empty() {
        let text = inp_str(input, &["text", "content"]);
        if let Some((o, n)) = split_search_replace(&text) {
            old_str = o;
            new_str = n;
        }
    }

    // Inline markers with the pair split across one field.
    if !old_str.is_empty() && new_str.is_empty() && old_str.contains("<<<<<<<") {
        if let Some(sep) = old_str.find("=======") {
            if let Some(end) = old_str.find(">>>>>>>") {
                if sep > 0 && end > sep {
                    new_str = old_str[sep + 7..end].to_string();
                    old_str = old_str[..sep].to_string();
                }
            }
        }
    }
    if old_str.is_empty() && !new_str.is_empty() && new_str.contains("<<<<<<<") {
        if let Some(sep) = new_str.find("=======") {
            if let Some(end) = new_str.find(">>>>>>>") {
                if sep > 0 && end > sep {
                    old_str = new_str[..sep].to_string();
                    new_str = new_str[sep + 7..end].to_string();
                }
            }
        }
    }

    let old_lines = if old_str.is_empty() { Vec::new() } else { parse_diff_block(&old_str) };
    let new_lines = if new_str.is_empty() { Vec::new() } else { parse_diff_block(&new_str) };

    // Batch of change objects (new CLI edit tool).
    let batch_pairs: Vec<BatchPair> = input
        .get("changes")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|ch| {
                    let o = ch
                        .get("oldString")
                        .or_else(|| ch.get("old_string"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let n = ch
                        .get("newString")
                        .or_else(|| ch.get("new_string"))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let ra = ch.get("replaceAll").map(|v| *v == serde_json::Value::Bool(true)).unwrap_or(false)
                        || ch.get("replace_all").map(|v| *v == serde_json::Value::Bool(true)).unwrap_or(false);
                    Some(BatchPair { old_str: o, new_str: n, replace_all: ra })
                })
                .collect()
        })
        .unwrap_or_default();

    let has_diff = !old_lines.is_empty() || !new_lines.is_empty() || !batch_pairs.is_empty();
    let badge: Element = if is_error || !out_str.contains("Applied") {
        VNode::empty()
    } else {
        applied_badge(&out_str, lang)
    };
    let err_text = out_str
        .replace("<tool_error>", "")
        .replace("</tool_error>", "")
        .trim()
        .to_string();

    let path_el: Element = if path.is_empty() {
        VNode::empty()
    } else {
        tool_path_chip(&path)
    };
    let old_pre: Element = if old_lines.is_empty() {
        VNode::empty()
    } else {
        let text = old_lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n");
        rsx! { pre { class: "tool-diff-removed", "{text}" } }
    };
    let new_pre: Element = if new_lines.is_empty() {
        VNode::empty()
    } else {
        let text = new_lines.iter().map(|l| format!("+ {l}")).collect::<Vec<_>>().join("\n");
        rsx! { pre { class: "tool-diff-added", "{text}" } }
    };
    let batch_els: Vec<Element> = batch_pairs
        .iter()
        .map(|p| {
            let old_text = if p.old_str.is_empty() {
                String::new()
            } else {
                p.old_str
                    .split('\n')
                    .map(|l| format!("- {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let new_text = if p.new_str.is_empty() {
                String::new()
            } else {
                p.new_str
                    .split('\n')
                    .map(|l| format!("+ {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let ra_el: Element = if p.replace_all {
                rsx! { div { class: "tool-read-meta", "replace_all" } }
            } else {
                VNode::empty()
            };
            let o_el: Element = if old_text.is_empty() {
                VNode::empty()
            } else {
                rsx! { pre { class: "tool-diff-removed", "{old_text}" } }
            };
            let n_el: Element = if new_text.is_empty() {
                VNode::empty()
            } else {
                rsx! { pre { class: "tool-diff-added", "{new_text}" } }
            };
            rsx! {
                div { class: "tool-diff-pair",
                    {ra_el}
                    {o_el}
                    {n_el}
                }
            }
        })
        .collect();

    if is_error {
        let err_el: Element = if err_text.is_empty() {
            VNode::empty()
        } else {
            rsx! { pre { class: "tool-error-output", "{err_text}" } }
        };
        return rsx! {
            div { class: "tool-rich-error",
                {path_el}
                {old_pre}
                {new_pre}
                {batch_els.iter().cloned()}
                {err_el}
            }
        };
    }
    if has_diff {
        return rsx! {
            div { class: "tool-rich-diff",
                {path_el}
                {badge}
                {old_pre}
                {new_pre}
                {batch_els.iter().cloned()}
            }
        };
    }
    if let Some(out_text) = &out {
        let body: Element = if out_str.contains("Applied") {
            badge
        } else {
            rsx! { pre { class: "tool-bash-output", "{out_text}" } }
        };
        return rsx! {
            div { class: "tool-rich-diff",
                {path_el}
                {body}
            }
        };
    }
    rsx! {
        div { class: "tool-rich-result",
            {path_el}
            {badge}
        }
    }
}

// --- grep / glob cards -------------------------------------------------------

/// Grep-style card: pattern + path header with the raw output.
fn grep_block(tc: &mistraldeck_core::types::ToolCall, input: &serde_json::Value) -> Element {
    let pattern = inp_str(input, &["pattern", "query", "regex"]);
    let path = inp_str(input, &["path", "directory", "dir", "filePath"]);
    let out = parse_tool_output(&tc.output.clone().unwrap_or_default());
    let pattern_el: Element = if pattern.is_empty() {
        VNode::empty()
    } else {
        rsx! { code { class: "tool-grep-pattern", "{pattern}" } }
    };
    let path_el: Element = if path.is_empty() {
        VNode::empty()
    } else {
        rsx! { span { class: "tool-grep-path", "{path}" } }
    };
    let out_el: Element = match &out {
        Some(text) => rsx! { pre { class: "tool-grep-output", "{text}" } },
        None => VNode::empty(),
    };
    rsx! {
        div { class: "tool-rich-grep",
            div { class: "tool-grep-header",
                {pattern_el}
                {path_el}
            }
            {out_el}
        }
    }
}

/// Glob/list-style card (same layout as grep).
fn glob_block(tc: &mistraldeck_core::types::ToolCall, input: &serde_json::Value) -> Element {
    let pattern = inp_str(input, &["pattern"]);
    let path = inp_str(input, &["path", "directory", "dir", "filePath"]);
    let out = parse_tool_output(&tc.output.clone().unwrap_or_default());
    let pattern_el: Element = if pattern.is_empty() {
        VNode::empty()
    } else {
        rsx! { code { class: "tool-grep-pattern", "{pattern}" } }
    };
    let path_el: Element = if path.is_empty() {
        VNode::empty()
    } else {
        rsx! { span { class: "tool-grep-path", "{path}" } }
    };
    let out_el: Element = match &out {
        Some(text) => rsx! { pre { class: "tool-grep-output", "{text}" } },
        None => VNode::empty(),
    };
    rsx! {
        div { class: "tool-rich-grep",
            div { class: "tool-grep-header",
                {pattern_el}
                {path_el}
            }
            {out_el}
        }
    }
}

// --- routing ----------------------------------------------------------------

/// Port of Nexus `ToolCallRichContent`: route a tool call to a rich block by
/// diffs → canonical tool name → title/input heuristics. None = fall through
/// to the generic JSON body.
fn tool_call_rich(tc: &mistraldeck_core::types::ToolCall, lang: &str) -> Option<Element> {
    let name = tc.title.to_lowercase();
    let input = tool_input(tc);
    let tn = tc.tool_name.to_lowercase();

    if !tc.diffs.is_empty() {
        return Some(edit_diff_block(tc, lang));
    }
    // Canonical tool name from the ACP `_meta` — precise routing before the
    // title/input heuristics below.
    if tn == "bash" || tn == "file_system.bash" {
        return Some(bash_block(tc));
    }
    if tn == "read_file" || tn == "file_system.read_file" || tn.ends_with(".read_file") {
        return Some(read_file_block(tc, lang));
    }
    if tn.starts_with("process.") {
        return Some(process_block(tc));
    }
    if tn == "search_tool_functions" || tn == "vibe.todo" {
        return Some(tool_line_block(tc));
    }

    let has_bash_input = input.get("command").map_or(false, |v| v.is_string())
        || input.get("cmd").map_or(false, |v| v.is_string())
        || input.get("script").map_or(false, |v| v.is_string());
    let has_read_file_input = (input.get("path").map_or(false, |v| v.is_string())
        || input.get("file_path").map_or(false, |v| v.is_string())
        || input.get("filePath").map_or(false, |v| v.is_string()))
        && (input.get("offset").is_some()
            || input.get("limit").is_some()
            || input.get("start_line").is_some());
    let content_str = input.get("content").and_then(|v| v.as_str());
    let has_search_replace_input = input.get("old_string").is_some()
        || input.get("oldString").is_some()
        || input.get("old_str").is_some()
        || content_str.map_or(false, |c| c.contains("<<<<<<<"))
        || (input
            .get("file_path")
            .and_then(|v| v.as_str())
            .map_or(false, |p| !p.is_empty())
            && content_str.map_or(false, |c| !c.is_empty()));
    let has_grep_input = (input.get("pattern").is_some()
        || input.get("regex").is_some()
        || input.get("query").is_some())
        && (input.get("path").is_some() || input.get("directory").is_some());

    // bash / execute / run_command / shell / terminal / command / cmd
    if name.contains("bash")
        || name.contains("execute")
        || name.contains("run_command")
        || name.contains("shell")
        || name.contains("terminal")
        || name.contains("command")
        || name.contains("cmd")
        || has_bash_input
    {
        return Some(bash_block(tc));
    }
    // read_file / view / cat
    if name == "read_file"
        || name == "view"
        || name == "read"
        || name.contains("read_file")
        || has_read_file_input
    {
        return Some(read_file_block(tc, lang));
    }
    // search_replace / str_replace / edit_file / create_file / write
    if name == "edit"
        || name.ends_with(".edit")
        || name.contains("replace")
        || name.contains("str_replace")
        || name.contains("edit_file")
        || name.contains("create_file")
        || name.contains("write")
        || has_search_replace_input
    {
        return Some(edit_fallback_block(tc, &input, lang));
    }
    // grep / search / find_in_file / ripgrep
    if name.contains("grep")
        || name.contains("search")
        || name == "find_in_file"
        || name == "ripgrep"
        || has_grep_input
    {
        return Some(grep_block(tc, &input));
    }
    // glob / list_dir / ls
    if name.contains("glob") || name.contains("list") || name == "ls" {
        return Some(glob_block(tc, &input));
    }
    None
}

#[component]
fn ToolCallBlock(tc: mistraldeck_core::types::ToolCall) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let input = tc.input_json.clone().unwrap_or_default();
    if is_subagent_tool(&tc) {
        return rsx! { SubagentBlock { tc: tc.clone() } };
    }
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
    let title = tc.title.clone();
    // Rich content routing (Nexus ToolCallRichContent) with the generic JSON
    // body as fallback.
    let body: Element = match tool_call_rich(&tc, &lang) {
        Some(rich) => rich,
        None => rsx! { ToolCallBody { tc: tc.clone() } },
    };

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
                {body}
            }
        }
    }
}

/// The expanded body of a tool call (kind chip, input, file changes, output) —
/// shared by the legacy ToolCallBlock and the StepsTimeline step content.
#[component]
fn ToolCallBody(tc: mistraldeck_core::types::ToolCall) -> Element {
    let input = tc.input_json.clone().unwrap_or_default();
    let kind = tc.kind.clone();
    let output = tc.output.clone().unwrap_or_default();
    let has_changes = !tc.file_changes.is_empty();
    // Nexus `extractToolCallPath`: the fallback path chip shown before the
    // input/output in the generic block (file changes already render their
    // own chips, so only the empty case falls back to the input scan).
    let chip_path: Option<String> = if has_changes {
        None
    } else {
        tc.input_json.as_deref().and_then(extract_tool_call_path)
    };

    rsx! {
        div { class: "tool-call-content",
            if !kind.is_empty() {
                span { class: "tool-kind-chip", "{kind}" }
            }
            if let Some(p) = chip_path.clone() {
                ToolCallPathChip { path: p }
            }
            if !input.is_empty() {
                div { class: "tool-call-input", "{input}" }
            }
            if has_changes {
                for ch in &tc.file_changes {
                    ToolCallPathChip { path: ch.path.clone() }
                }
            }
            if !output.is_empty() {
                div { class: "tool-call-output", "{output}" }
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
fn TaskToolBlock(tc: mistraldeck_core::types::ToolCall) -> Element {
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

/// Nexus v0.13 SubagentBlock: subagent invocation card (agents.spawn /
/// task) — badge + agent name, live status (✓/✕/…, pulsing while running),
/// the task prompt (line-clamp 2, click expands), error output in a pre,
/// and the child session transcript behind "Show work" (loaded on demand
/// from the unified session store; unavailable sessions show a notice).
#[component]
fn SubagentBlock(tc: mistraldeck_core::types::ToolCall) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let prompt_open = use_signal(|| false);
    let work: Signal<Option<Vec<PreviewMessageView>>> = use_signal(|| None);
    let work_err = use_signal(|| false);
    let work_loading = use_signal(|| false);

    let input = tc.input_json.clone().unwrap_or_default();
    // Wire meta first; calls restored from disk without `_meta` fall back
    // to the tool input (Nexus reads agent/task the same way from the disk
    // detail).
    let (meta_agent, meta_task, child_id) = match &tc.subagent {
        Some(sa) => (
            sa.agent
                .clone()
                .filter(|a| !a.is_empty())
                .or_else(|| sa.agent_name.clone().filter(|a| !a.is_empty())),
            sa.task.clone(),
            sa.child_session_id.clone().filter(|c| !c.is_empty()),
        ),
        None => (None, None, None),
    };
    let agent = meta_agent
        .or_else(|| json_str_field(&input, "agent").filter(|a| !a.is_empty()))
        .unwrap_or_else(|| tr(&lang, "subagent").to_string());
    let task = meta_task
        .or_else(|| json_str_field(&input, "task"))
        .unwrap_or_default();

    let (status_txt, status_cls) = match tc.status {
        ToolStatus::Error => ("✕", "subagent-status error"),
        ToolStatus::Running => ("…", "subagent-status running"),
        ToolStatus::Done => ("✓", "subagent-status"),
    };

    let task_cls = if *prompt_open.read() {
        "subagent-task open"
    } else {
        "subagent-task"
    };
    let has_task = !task.is_empty();

    let error_out = if tc.status == ToolStatus::Error {
        tc.output.clone().filter(|o| !o.is_empty())
    } else {
        None
    };

    let has_child = child_id.is_some();
    let show_work_label = if work.read().is_some() {
        tr(&lang, "subagentHideWork")
    } else {
        tr(&lang, "subagentShowWork")
    };
    let is_loading = *work_loading.read();
    let has_work_err = *work_err.read();
    let has_work = work.read().is_some();
    let work_els: Vec<Element> = work
        .read()
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let role = m.role.clone();
            let content = m.content.clone();
            let think_els: Vec<Element> = m
                .thinking
                .iter()
                .map(|th| {
                    let text = th.content.clone();
                    rsx! { div { class: "subagent-work-thinking", "{text}" } }
                })
                .collect();
            let tool_els: Vec<Element> = m
                .tool_calls
                .iter()
                .map(|t| {
                    let line = if t.status == "error" {
                        format!("· {} ✕", t.title)
                    } else {
                        format!("· {}", t.title)
                    };
                    rsx! { div { class: "subagent-work-tool", "{line}" } }
                })
                .collect();
            if role == "user" {
                rsx! { div { class: "subagent-work-msg",
                    div { class: "subagent-work-user", "{content}" }
                } }
            } else {
                rsx! { div { class: "subagent-work-msg",
                    {think_els.iter().cloned()}
                    {tool_els.iter().cloned()}
                    if !content.is_empty() {
                        div { class: "subagent-work-answer",
                            MarkdownLite { text: content.clone() }
                        }
                    }
                } }
            }
        })
        .collect();

    rsx! {
        div { class: "subagent-card",
            div { class: "subagent-header",
                span { class: "subagent-badge", "{tr(&lang, \"subagent\")}" }
                span { class: "subagent-agent", "{agent}" }
                span { class: "{status_cls}", "{status_txt}" }
            }
            if has_task {
                div {
                    class: "{task_cls}",
                    onclick: {
                        let mut prompt_open = prompt_open.clone();
                        move |_| {
                            let v = *prompt_open.read();
                            prompt_open.set(!v);
                        }
                    },
                    span { class: "subagent-task-label",
                        "{tr(&lang, \"subagentPromptLabel\")}"
                    }
                    span { class: "subagent-task-text", "{task}" }
                }
            }
            if let Some(out) = error_out {
                pre { class: "subagent-error-out", "{out}" }
            }
            if has_child {
                button {
                    class: "subagent-work-btn",
                    disabled: is_loading,
                    onclick: {
                        let state = state.clone();
                        let child = child_id.clone();
                        let mut work = work.clone();
                        let mut work_err = work_err.clone();
                        let mut work_loading = work_loading.clone();
                        move |_| {
                            if *work_loading.read() {
                                return;
                            }
                            if work.read().is_some() {
                                work.set(None);
                                return;
                            }
                            let Some(child) = child.clone() else {
                                return;
                            };
                            work_loading.set(true);
                            work_err.set(false);
                            let state = state.clone();
                            let mut work = work.clone();
                            let mut work_err = work_err.clone();
                            let mut work_loading = work_loading.clone();
                            spawn(async move {
                                match crate::vibestats::api_session_preview(
                                    crate::state::token(&state),
                                    child.clone(),
                                )
                                .await
                                {
                                    Ok(Some(msgs)) => work.set(Some(msgs)),
                                    Ok(None) => work_err.set(true),
                                    Err(_) => work_err.set(true),
                                }
                                work_loading.set(false);
                            });
                        }
                    },
                    "{show_work_label}"
                }
            }
            if has_work_err {
                div { class: "subagent-work-error",
                    "{tr(&lang, \"subagentWorkUnavailable\")}"
                }
            }
            if has_work {
                div { class: "subagent-work",
                    {work_els.iter().cloned()}
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
pub(crate) fn MarkdownLite(text: String) -> Element {
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

/// Click bridge for `.markdown-path-link` inline code (mounted through
/// `dangerous_inner_html`, so listeners must live at the document level).
/// A click stores the raw path on `window.__vdPath` and clicks the hidden
/// `#vd-path-bridge` button, whose handler resolves and opens the preview.
/// Enter/Space mirror the Nexus keydown handler (the links carry
/// tabindex=0 so they are focusable).
#[cfg(any(feature = "web", feature = "desktop"))]
const PATH_CLICK_JS: &str = r#"
(function(){
  if (window.__vdPathBridge) return; window.__vdPathBridge = true;
  function fire(el){
    var p = el.getAttribute('data-path');
    if (!p) return;
    window.__vdPath = p;
    var btn = document.getElementById('vd-path-bridge');
    if (btn) btn.click();
  }
  function hit(t){
    while (t && t !== document.body) {
      if (t.classList && t.classList.contains('markdown-path-link')) return t;
      t = t.parentElement;
    }
    return null;
  }
  document.addEventListener('click', function(ev){
    var el = hit(ev.target);
    if (el) fire(el);
  }, true);
  document.addEventListener('keydown', function(ev){
    if (ev.key !== 'Enter' && ev.key !== ' ') return;
    var el = hit(ev.target);
    if (el) { ev.preventDefault(); fire(el); }
  });
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

/// Localize a CLI checkpoint notice (Nexus translateCliNotice):
/// "Model changed to <tech>" resolves the technical model name to its
/// display name via the model config option choices.
fn translate_cli_notice(
    lang: &str,
    text: &str,
    model_names: &std::collections::HashMap<String, String>,
) -> String {
    let t = text.trim();
    if let Some(tech) = t.strip_prefix("Model changed to ") {
        let tech = tech.trim();
        let name = model_names
            .get(tech)
            .cloned()
            .unwrap_or_else(|| tech.to_string());
        return tf(lang, "modelChangedTo", &[("model", &name)]);
    }
    if let Some(agent) = t.strip_prefix("Agent changed to ") {
        return tf(lang, "agentChangedTo", &[("agent", agent.trim())]);
    }
    match t {
        "Compacting context" => tr(lang, "compactingContext").to_string(),
        "Context compacted" => tr(lang, "contextCompacted").to_string(),
        "Compaction failed" => tr(lang, "compactionFailed").to_string(),
        "Compacting conversation history..." => tr(lang, "compactingHistory").to_string(),
        "Compacted conversation history" => tr(lang, "compactedHistory").to_string(),
        "Session title updated" => tr(lang, "sessionTitleUpdated").to_string(),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod cli_notice_tests {
    use super::*;

    #[test]
    fn translates_model_change_with_display_name() {
        let mut models = std::collections::HashMap::new();
        models.insert(
            "mistral-large-latest".to_string(),
            "Mistral Large".to_string(),
        );
        assert_eq!(
            translate_cli_notice("pl", "Model changed to mistral-large-latest", &models),
            "Zmieniono model na: Mistral Large"
        );
        // Unknown technical model falls back to the raw name.
        assert_eq!(
            translate_cli_notice("pl", "Model changed to mystery-model", &models),
            "Zmieniono model na: mystery-model"
        );
    }

    #[test]
    fn translates_known_phrases_and_passes_through_unknown() {
        let models = std::collections::HashMap::new();
        assert_eq!(
            translate_cli_notice("pl", "Compacting context", &models),
            "Kompaktowanie kontekstu"
        );
        assert_eq!(
            translate_cli_notice("pl", "Session title updated", &models),
            "Zaktualizowano tytuł sesji"
        );
        assert_eq!(
            translate_cli_notice("en", "Some other system notice", &models),
            "Some other system notice"
        );
    }
}
