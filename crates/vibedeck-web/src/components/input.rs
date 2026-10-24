//! Input bar: textarea + send/stop button.

use dioxus::prelude::*;

use crate::state::{token, ClientState};

pub fn InputBar() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut draft = use_signal(String::new);
    let busy = state.read().snapshot.busy;
    let model = current_model(&state);

    let send = move |_| async move {
        let text = draft().trim().to_string();
        if text.is_empty() {
            return;
        }
        draft.set(String::new());
        let _ = crate::api::api_send_message(token(&state), text).await;
    };
    let stop = move |_| async move {
        let _ = crate::api::api_cancel(token(&state)).await;
    };
    let on_key = move |e: Event<KeyboardData>| {
        if e.key() == Key::Enter && !e.modifiers().shift() {
            e.prevent_default();
            let text = draft().trim().to_string();
            if !text.is_empty() {
                draft.set(String::new());
                let state = state.clone();
                spawn(async move {
                    let _ = crate::api::api_send_message(token(&state), text).await;
                });
            }
        }
    };

    rsx! {
        div { class: "input-bar",
            div { class: "input-card",
                textarea {
                    class: "vd-input",
                    rows: "1",
                    placeholder: "Napisz wiadomość… (Shift+Enter = nowa linia)",
                    value: "{draft()}",
                    oninput: move |e| draft.set(e.value()),
                    onkeydown: on_key,
                }
                div { class: "toolbar",
                    span { class: "chip", "Model: {model}" }
                    div { class: "toolbar-spacer" }
                    if busy {
                        button { class: "send-btn stop", onclick: stop, "■" }
                    } else {
                        button {
                            class: "send-btn",
                            disabled: draft().trim().is_empty(),
                            onclick: send,
                            "➤"
                        }
                    }
                }
            }
        }
    }
}

fn current_model(state: &Signal<ClientState>) -> String {
    let s = state.read();
    s.snapshot
        .config_options
        .iter()
        .find(|o| o.id == "model")
        .map(|o| o.current_label())
        .unwrap_or_else(|| "—".to_string())
}
