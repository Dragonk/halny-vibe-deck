//! Login card for headless / remote-control mode.

use dioxus::prelude::*;

use crate::state::{refresh, ClientState};

pub fn Login() -> Element {
    let mut state = use_context::<Signal<ClientState>>();
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let do_login = move |_| async move {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(String::new());
        match crate::api::api_login(username().clone(), password().clone()).await {
            Ok(Some(token)) => {
                state.write().token = Some(token);
                refresh(state).await;
                state.write().loaded = true;
            }
            Ok(None) => error.set("Nieprawidłowy login lub hasło.".into()),
            Err(e) => error.set(format!("Błąd połączenia: {e}")),
        }
        busy.set(false);
    };

    rsx! {
        div { class: "login-wrap",
            div { class: "login-card",
                h1 {
                    span { class: "flame", "V" }
                    "VibeDeck"
                }
                input {
                    r#type: "text",
                    placeholder: "Login",
                    value: "{username}",
                    oninput: move |e| username.set(e.value()),
                }
                input {
                    r#type: "password",
                    placeholder: "Hasło",
                    value: "{password}",
                    oninput: move |e| password.set(e.value()),
                    onkeydown: move |e| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            e.stop_propagation();
                        }
                    },
                }
                if !error().is_empty() {
                    div { class: "login-error", "{error}" }
                }
                button {
                    class: "login-btn",
                    disabled: busy(),
                    onclick: do_login,
                    if busy() { "Łączenie…" } else { "Zaloguj" }
                }
                div { class: "login-hint",
                    "Nieoficjalny projekt społecznościowy — niepowiązany z Mistral AI."
                }
            }
        }
    }
}
