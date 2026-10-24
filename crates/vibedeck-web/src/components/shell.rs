//! App shell: header (brand + status + usage), sidebar, chat + input.

use dioxus::prelude::*;

use crate::state::ClientState;

pub fn Shell() -> Element {
    let state = use_context::<Signal<ClientState>>();

    // The engine event stream (mounted only when authed).
    let tok = crate::state::token(&state);
    let mut socket = crate::events::use_engine_socket(tok);
    use_future(move || async move {
        loop {
            match socket.recv().await {
                Ok(ev) => {
                    let refetch = crate::state::apply_event(state, ev);
                    if refetch {
                        crate::state::refresh(state).await;
                    }
                }
                Err(_) => {
                    // Stream dropped (server restart, token expiry).
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    crate::state::refresh(state).await;
                }
            }
        }
    });

    let s = state.read();
    let status = s.snapshot.status.clone();
    let status_class = match status.as_str() {
        "connected" => "connected",
        "connecting" => "connecting",
        "error" => "error",
        _ => "",
    };
    let usage = s.snapshot.usage.clone();
    let busy = s.snapshot.busy;
    let sidebar_open = s.sidebar_open;
    let question = s.snapshot.question.clone();
    let approval = s.snapshot.approval.clone();
    drop(s);

    let hit = (usage.cache_hit_rate() * 100.0).round() as u64;
    let inp = usage.prompt_tokens as u64;
    let outp = usage.completion_tokens as u64;
    let sidebar_cls = if sidebar_open { "sidebar open" } else { "sidebar" };

    rsx! {
        link { rel: "stylesheet", href: asset!("/assets/main.css") }
        div { id: "main",
            header { class: "header",
                div { class: "brand",
                    span { class: "flame", "V" }
                    span { class: "brand-text", "VibeDeck" }
                }
                div { class: "status",
                    span { class: "dot {status_class}" }
                    "{status}"
                    if busy {
                        span { class: "spin" }
                    }
                }
                div { class: "header-right",
                    span { class: "usage",
                        "in {inp} · out {outp} · cache {hit}%"
                    }
                }
            }
            div { class: "shell",
                div { class: sidebar_cls,
                    crate::components::sidebar::Sidebar {}
                }
                div { class: "main",
                    crate::components::chat::Chat {}
                    if let Some(q) = &question {
                        crate::components::chat::QuestionCard { question: q.clone() }
                    }
                    if let Some(a) = &approval {
                        crate::components::chat::ApprovalCard { approval: a.clone() }
                    }
                    crate::components::input::InputBar {}
                }
            }
        }
    }
}
