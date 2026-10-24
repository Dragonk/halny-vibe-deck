//! Chat view: message list with user bubbles, AI rows (thinking + tool
//! calls + content) and system notes. Minimal markdown: paragraphs and
//! fenced code blocks.

use dioxus::prelude::*;
use vibedeck_core::snapshot::{PendingApproval, PendingQuestion};
use vibedeck_core::types::{Message, MessageSender, ToolStatus};

use crate::state::{token, ClientState};

pub fn Chat() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let messages = state.read().snapshot.current_messages.clone();

    rsx! {
        div { class: "chat",
            for msg in messages {
                match msg.sender {
                    MessageSender::User => rsx! {
                        div { class: "msg-row user",
                            div { class: "bubble", "{msg.content}" }
                        }
                    },
                    MessageSender::Ai => rsx! { AiRow { msg: msg.clone() } },
                    MessageSender::System => rsx! {
                        div { class: "system-row", "{msg.content}" }
                    },
                }
            }
        }
    }
}

fn thinking_title(done: bool) -> &'static str {
    if done { "Myślę" } else { "Myślę…" }
}

#[component]
pub fn AiRow(msg: Message) -> Element {
    rsx! {
        div { class: "msg-row",
            div { class: "ai-row",
                div { class: "avatar", "V" }
                div { class: "ai-body",
                    for block in &msg.thinking {
                        details { class: "think",
                            summary { "{thinking_title(block.is_complete)}" }
                            div { "{block.content}" }
                        }
                    }
                    for tc in &msg.tool_calls {
                        ToolCallBlock { tc: tc.clone() }
                    }
                    MarkdownLite { text: msg.content.clone() }
                }
            }
        }
    }
}

#[component]
fn ToolCallBlock(tc: vibedeck_core::types::ToolCall) -> Element {
    let (icon, cls) = match tc.status {
        ToolStatus::Done => ("✔", "done"),
        ToolStatus::Error => ("✖", "error"),
        ToolStatus::Running => ("⏳", "running"),
    };
    rsx! {
        details { class: "tool",
            summary {
                span { class: "tool-status {cls}", "{icon}" }
                "{tc.title}"
            }
            if let Some(output) = &tc.output {
                div { "{output}" }
            }
        }
    }
}

/// Minimal markdown renderer: fenced code blocks + paragraphs.
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
            rsx! { pre { key: "{k}", "{chunk}" } }
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

    rsx! { div { class: "ai-content", {blocks.iter().cloned()} } }
}

/// Agent question modal (choices + free-form).
#[component]
pub fn QuestionCard(question: PendingQuestion) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut answer = use_signal(String::new);

    rsx! {
        div { class: "approval",
            div { "❓ {question.question}" }
            div { class: "approval-actions",
                for choice in question.choices.iter() {
                    button {
                        class: "approval-btn primary",
                        onclick: {
                            let state = state.clone();
                            let req = question.request_id.clone();
                            let ch = choice.clone();
                            move |_| {
                                let req = req.clone();
                                let ch = ch.clone();
                                async move {
                                    let _ = crate::api::api_answer_question(token(&state), req, ch).await;
                                    crate::state::refresh(state).await;
                                }
                            }
                        },
                        "{choice}"
                    }
                }
            }
            input {
                style: "background:var(--panel);border:1px solid var(--border);border-radius:8px;padding:8px 10px;color:var(--text);outline:none;",
                placeholder: "Odpowiedz…",
                value: "{answer()}",
                oninput: move |e| answer.set(e.value()),
            }
            button {
                class: "approval-btn",
                onclick: {
                    let state = state.clone();
                    let req = question.request_id.clone();
                    move |_| {
                        let req = req.clone();
                        async move {
                            let _ = crate::api::api_answer_question(token(&state), req, answer().clone()).await;
                            crate::state::refresh(state).await;
                        }
                    }
                },
                "Wyślij"
            }
        }
    }
}

/// Tool approval card.
#[component]
pub fn ApprovalCard(approval: PendingApproval) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let title = approval.tool_name.clone().unwrap_or_else(|| approval.method.clone());
    rsx! {
        div { class: "approval",
            div { "🔐 {title}" }
            div { style: "color:var(--muted);font-size:11px;white-space:pre-wrap;", "{approval.params_json}" }
            div { class: "approval-actions",
                button {
                    class: "approval-btn primary",
                    onclick: {
                        let state = state.clone();
                        let req = approval.request_id.clone();
                        move |_| {
                            let req = req.clone();
                            async move {
                                let _ = crate::api::api_approve(token(&state), req, "proceed_once".into()).await;
                                crate::state::refresh(state).await;
                            }
                        }
                    },
                    "Zatwierdź"
                }
                button {
                    class: "approval-btn",
                    onclick: {
                        let state = state.clone();
                        let req = approval.request_id.clone();
                        move |_| {
                            let req = req.clone();
                            async move {
                                let _ = crate::api::api_approve(token(&state), req, "reject".into()).await;
                                crate::state::refresh(state).await;
                            }
                        }
                    },
                    "Odrzuć"
                }
            }
        }
    }
}
