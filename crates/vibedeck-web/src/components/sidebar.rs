//! Session list sidebar: local sessions + CLI (backend) sessions.

use dioxus::prelude::*;

use crate::state::{token, ClientState};

pub fn Sidebar() -> Element {
    let state = use_context::<Signal<ClientState>>();

    let new_session = move |_| async move {
        let _ = crate::api::api_create_session(token(&state)).await;
        crate::state::refresh(state).await;
    };
    let refresh_btn = move |_| async move {
        let _ = crate::api::api_refresh_sessions(token(&state)).await;
        // SessionsList event triggers a snapshot refetch via the stream.
    };

    let s = state.read();
    let sessions = s.snapshot.sessions.clone();
    let current = s.snapshot.current;
    let backend = s.snapshot.backend_sessions.clone();
    drop(s);

    rsx! {
        button { class: "new-session", onclick: new_session, "+ Nowa sesja" }
        div { class: "sessions",
            div { class: "side-section", "Sesje" }
            for (i, meta) in sessions.iter().enumerate() {
                {
                    let state = state.clone();
                    let idx = i;
                    let cls = if current == Some(i) { "session active" } else { "session" };
                    rsx! {
                        div {
                            class: cls,
                            onclick: move |_| async move {
                                let _ = crate::api::api_switch_session(token(&state), idx).await;
                                crate::state::refresh(state).await;
                            },
                            span { class: "session-name", "{meta.name}" }
                            span { class: "session-meta",
                                "{meta.message_count} wiadomości · {short_dir(&meta.working_directory)}"
                            }
                        }
                    }
                }
            }
            div { class: "side-section",
                "Historia z CLI "
                button {
                    style: "background:none;border:none;color:var(--muted);cursor:pointer;",
                    onclick: refresh_btn,
                    "⟳"
                }
            }
            for info in backend.iter() {
                {
                    let state = state.clone();
                    let sid = info.session_id.clone();
                    let name = if info.name.is_empty() { "(bez nazwy)".to_string() } else { info.name.clone() };
                    let cwd = short_dir(&info.cwd);
                    rsx! {
                        div {
                            class: "session",
                            onclick: move |_| {
                                let sid = sid.clone();
                                async move {
                                    let _ = crate::api::api_open_backend_session(token(&state), sid).await;
                                    crate::state::refresh(state).await;
                                }
                            },
                            span { class: "session-name", "{name}" }
                            span { class: "session-meta", "{cwd}" }
                        }
                    }
                }
            }
        }
    }
}

fn short_dir(dir: &str) -> String {
    let short: String = if let Some(pos) = dir.rfind('/') {
        dir[pos..].to_string()
    } else {
        dir.to_string()
    };
    if short.is_empty() { dir.chars().take(18).collect() } else { short }
}
