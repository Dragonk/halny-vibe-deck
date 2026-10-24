//! Chat view: empty state with the pixel flame, message list with user
//! bubbles / AI rows (thinking, tool calls, markdown-lite), the agent
//! busy indicator. Ported from Nexus ChatView + MessageBubble (same
//! CSS classes).

use dioxus::prelude::*;
use vibedeck_core::types::{Message, MessageSender, ToolStatus};

use crate::i18n::tr;
use crate::state::{lang, ClientState};

pub fn ChatView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let s = state.read();
    let messages = s.snapshot.current_messages.clone();
    let busy = s.snapshot.busy;
    let status = s.snapshot.status.clone();
    let acp_error = s.snapshot.acp_error.clone();
    let queue_len = s.queue.len();
    drop(s);

    let subtitle = match status.as_str() {
        "connected" => tr(&lang, "agentReady").to_string(),
        "connecting" => tr(&lang, "statusConnecting").to_string(),
        "error" => tr(&lang, "agentConnectionError").to_string(),
        _ => tr(&lang, "configureAcpInSettings").to_string(),
    };

    rsx! {
        div { class: "chat-view-wrap",
            if queue_len > 0 {
                div { class: "tool-approval-queue-hint", style: "position:static;margin:0 16px 4px;", "⏳ {queue_len}" }
            }
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

#[component]
pub fn MessageBubble(msg: Message, streaming: bool) -> Element {
    match msg.sender {
        MessageSender::User => rsx! {
            div { class: "msg-row msg-user",
                div { class: "msg-bubble",
                    div { class: "msg-text", "{msg.content}" }
                }
            }
        },
        MessageSender::Ai => rsx! {
            div { class: "msg-row msg-ai",
                span { class: "msg-avatar", "V" }
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
                    MarkdownLite { text: msg.content.clone() }
                    if streaming {
                        span { class: "cursor-blink" }
                    }
                }
            }
        },
        MessageSender::System => rsx! {
            div { class: "msg-row msg-notice",
                div { class: "msg-content", "{msg.content}" }
            }
        },
    }
}

#[component]
fn ThinkingSection(block: vibedeck_core::types::ThinkingBlock) -> Element {
    let label = if block.is_complete { "Myślę" } else { "Myślę…" };
    let paragraphs: Vec<&str> = block.content.split("\n\n").collect();
    rsx! {
        details { class: "thinking-section",
            summary { class: "thinking-toggle",
                span { class: "thinking-dot" }
                span { class: "thinking-label", "{label}" }
                span { class: "thinking-chevron" }
            }
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

#[component]
fn ToolCallBlock(tc: vibedeck_core::types::ToolCall) -> Element {
    let (icon, done_cls) = match tc.status {
        ToolStatus::Done => ("✔", "tool-done"),
        ToolStatus::Error => ("✖", "tool-error"),
        ToolStatus::Running => ("⏳", "tool-running"),
    };
    let kind = tc.kind.clone();
    let title = tc.title.clone();
    let input = tc.input_json.clone().unwrap_or_default();
    let output = tc.output.clone().unwrap_or_default();
    let has_changes = !tc.file_changes.is_empty();

    rsx! {
        details { class: "tool-rich-bash",
            summary { class: "tool-call-header",
                span { class: "tool-status {done_cls}", "{icon}" }
                span { class: "tool-call-title", "{title}" }
                span { class: "tool-call-chevron" }
            }
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

/// Minimal markdown renderer: fenced code blocks + paragraphs
/// (full GFM port arrives with the dock/editor iteration).
#[component]
fn MarkdownLite(text: String) -> Element {
    if text.is_empty() {
        return VNode::empty();
    }
    let mut blocks: Vec<Element> = Vec::new();
    let mut code = false;
    let mut current = String::new();
    let mut key = 0usize;
    let flush = |blocks: &mut Vec<Element>, current: &mut String, key: &mut usize, code: bool| {
        if current.is_empty() {
            return;
        }
        let k = *key;
        *key += 1;
        let chunk = std::mem::take(current);
        let node: Element = if code {
            rsx! { pre { key: "{k}", code { "{chunk}" } } }
        } else {
            rsx! { p { key: "{k}", "{chunk}" } }
        };
        blocks.push(node);
    };

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            flush(&mut blocks, &mut current, &mut key, code);
            code = !code;
            continue;
        }
        current.push_str(line);
        current.push('\n');
    }
    flush(&mut blocks, &mut current, &mut key, code);

    rsx! { div { class: "markdown-content", {blocks.iter().cloned()} } }
}
