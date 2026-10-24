//! Settings view: General (language / theme / sidebar limit), ACP
//! (paths + status), and Remote control (login + port + enabled).
//! Remaining Nexus tabs (agents, MCP, prompts, skills, providers,
//! models, raw TOML) arrive in the next parity iterations.

use dioxus::prelude::*;

use crate::i18n::tr;
use crate::state::{lang, save_prefs, token, ClientState};

fn loc(lang: &str, pl: &str, en: &str) -> String {
    if lang == "pl" { pl.to_string() } else { en.to_string() }
}

pub fn SettingsView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut tab = use_signal(|| "general".to_string());
    let saved = use_signal(String::new);

    let s = state.read();
    let lang = lang(&state);
    let language = s.snapshot.config.language.clone();
    let theme = s.snapshot.config.theme.clone();
    let dock_open = s.snapshot.config.dock_open;
    let limit = s.snapshot.config.sidebar_sessions_limit.unwrap_or(0);
    let acp_path = s.snapshot.config.acp_path.clone();
    let wd = s.snapshot.config.working_directory.clone();
    let remote_enabled = s.snapshot.config.remote_enabled;
    let auth_configured = s.snapshot.config.auth_configured;
    let remote_port = s.snapshot.config.remote_port;
    let status = s.snapshot.status.clone();
    drop(s);

    // Local form state for the remote-control tab.
    let mut r_user = use_signal(String::new);
    let mut r_pass = use_signal(String::new);
    let mut r_port = use_signal(move || remote_port.to_string());
    let mut r_enable = use_signal(move || remote_enabled || auth_configured);

    let tab_cls = |name: &str| {
        if *tab.read() == name {
            "scope-pill active".to_string()
        } else {
            "scope-pill".to_string()
        }
    };

    let body_el = match tab.read().as_str() {
        "acp" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "ACP" }
                }
                div { class: "field",
                    div { class: "field-label", "vibe-acp" }
                    input { class: "field-input", readonly: true, value: "{acp_path}" }
                    div { class: "field-hint", "{loc(&lang, \"Zmiana ścieżki wymaga edycji pliku konfiguracyjnego i restartu.\", \"Changing the path requires editing the config file and a restart.\")}" }
                }
                div { class: "field",
                    div { class: "field-label", "{tr(&lang, \"workingDirectory\")}" }
                    input { class: "field-input", readonly: true, value: "{wd}" }
                }
                div { class: "field",
                    div { class: "field-label", "Status" }
                    div { class: "acp-status {status}",
                        span { class: "status-dot" }
                        span { class: "status-label", "{status}" }
                    }
                }
            }
        },
        "remote" => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title",
                        "{loc(&lang, \"Zdalne sterowanie\", \"Remote control\")}"
                    }
                }
                div { class: "field-row",
                    label { class: "checkbox-label",
                        input {
                            r#type: "checkbox",
                            checked: *r_enable.read(),
                            onchange: move |e| r_enable.set(e.checked()),
                        }
                        "{loc(&lang, \"Włącz dostęp z przeglądarki\", \"Enable browser access\")}"
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Login\", \"Username\")}" }
                    input {
                        class: "field-input",
                        r#type: "text",
                        autocomplete: "off",
                        value: "{*r_user.read()}",
                        oninput: move |e| r_user.set(e.value()),
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Hasło\", \"Password\")}" }
                    input {
                        class: "field-input",
                        r#type: "password",
                        autocomplete: "new-password",
                        value: "{*r_pass.read()}",
                        oninput: move |e| r_pass.set(e.value()),
                    }
                    div { class: "field-hint",
                        "{loc(&lang, \"Hasło zapisywane jest wyłącznie jako skrót (sha256 + sól) — nigdy w jawnej postaci i nigdy nie jest wyświetlane.\", \"The password is stored only as a salted sha256 hash — never in plain text and never displayed.\")}"
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Port\", \"Port\")}" }
                    input {
                        class: "field-input",
                        r#type: "number",
                        value: "{*r_port.read()}",
                        oninput: move |e| r_port.set(e.value()),
                    }
                    div { class: "field-hint",
                        "{loc(&lang, \"Zmiana portu/interfejsu wchodzi w życie po restarcie serwera VibeDeck.\", \"Port/interface changes take effect after a VibeDeck server restart.\")}"
                    }
                }
                div { class: "field",
                    button {
                        class: "save-btn",
                        onclick: {
                            let state = state.clone();
                            let saved = saved.clone();
                            let lang_c = lang.clone();
                            move |_| {
                                let state = state.clone();
                                let mut saved = saved.clone();
                                let lang = lang_c.clone();
                                async move {
                                    let user = r_user.read().clone();
                                    let pass = r_pass.read().clone();
                                    let enable = *r_enable.read();
                                    let port = r_port.read().parse::<u16>().unwrap_or(8080);
                                    if user.trim().is_empty() || pass.is_empty() {
                                        saved.set(loc(&lang, "Podaj login i hasło.", "Enter username and password."));
                                        return;
                                    }
                                    let res = crate::api::api_set_remote(
                                        token(&state), user, pass, port, enable,
                                    )
                                    .await;
                                    match res {
                                        Ok(()) => {
                                            saved.set(String::new());
                                            refresh_after_save(state).await;
                                        }
                                        Err(_) => saved.set(loc(&lang, "Nie udało się zapisać.", "Failed to save.")),
                                    }
                                }
                            }
                        },
                        "{tr(&lang, \"saveLabel\")}"
                    }
                    if !saved.read().is_empty() {
                        span { class: "field-hint", style: "margin-left:10px;", "{*saved.read()}" }
                    }
                }
                div { class: "field",
                    div { class: "field-hint",
                        if remote_enabled || auth_configured {
                            "{loc(&lang, \"Zdalne sterowanie aktywne — logowanie z przeglądarki wymagane.\", \"Remote control active — browser login required.\")}"
                        } else {
                            "{loc(&lang, \"Zdalne sterowanie wyłączone.\", \"Remote control disabled.\")}"
                        }
                    }
                }
            }
        },
        _ => rsx! {
            div { class: "settings-section",
                div { class: "settings-section-header",
                    div { class: "settings-section-title", "{tr(&lang, \"general\")}" }
                }
                div { class: "field",
                    div { class: "field-label", "{tr(&lang, \"language\")}" }
                    select {
                        class: "field-select",
                        value: "{language}",
                        onchange: {
                            let state = state.clone();
                            let theme = theme.clone();
                            let dock_open = dock_open;
                            let limit = limit;
                            move |e| {
                                let state = state.clone();
                                let theme = theme.clone();
                                let dock_open = dock_open;
                                let limit = limit;
                                async move {
                                    save_prefs(state, e.value(), theme, dock_open, limit).await;
                                }
                            }
                        },
                        option { value: "auto", selected: language == "auto", "Auto (EN)" }
                        option { value: "en", selected: language == "en", "English" }
                        option { value: "pl", selected: language == "pl", "Polski" }
                        option { value: "de", selected: language == "de", "Deutsch" }
                        option { value: "fr", selected: language == "fr", "Français" }
                        option { value: "es", selected: language == "es", "Español" }
                        option { value: "it", selected: language == "it", "Italiano" }
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{tr(&lang, \"theme\")}" }
                    select {
                        class: "field-select",
                        value: "{theme}",
                        onchange: {
                            let state = state.clone();
                            let language = language.clone();
                            let dock_open = dock_open;
                            let limit = limit;
                            move |e| {
                                let state = state.clone();
                                let language = language.clone();
                                let dock_open = dock_open;
                                let limit = limit;
                                async move {
                                    save_prefs(state, language, e.value(), dock_open, limit).await;
                                }
                            }
                        },
                        option { value: "auto", selected: theme == "auto", "{tr(&lang, \"auto\")}" }
                        option { value: "dark", selected: theme == "dark", "{tr(&lang, \"dark\")}" }
                        option { value: "light", selected: theme == "light", "{tr(&lang, \"light\")}" }
                    }
                }
                div { class: "field",
                    div { class: "field-label", "{loc(&lang, \"Limit sesji w panelu\", \"Sidebar sessions limit\")}" }
                    input {
                        class: "field-input",
                        r#type: "number",
                        value: "{limit}",
                        onchange: {
                            let state = state.clone();
                            let language = language.clone();
                            let theme = theme.clone();
                            let dock_open = dock_open;
                            move |e| {
                                let state = state.clone();
                                let language = language.clone();
                                let theme = theme.clone();
                                let dock_open = dock_open;
                                async move {
                                    let n = e.value().parse::<u32>().unwrap_or(0);
                                    save_prefs(state, language, theme, dock_open, n).await;
                                }
                            }
                        },
                    }
                    div { class: "field-hint", "0 = {loc(&lang, \"bez limitu\", \"no limit\")}" }
                }
            }
        },
    };

    rsx! {
        div { class: "settings-view",
            div { class: "settings-header",
                div { class: "settings-title-row",
                    h2 { class: "settings-title", "{tr(&lang, \"settings\")}" }
                }
            }
            div { class: "settings-tabs scope-switch",
                button { class: "{tab_cls(\"general\")}", onclick: move |_| tab.set("general".into()), "{tr(&lang, \"general\")}" }
                button { class: "{tab_cls(\"acp\")}", onclick: move |_| tab.set("acp".into()), "ACP" }
                button { class: "{tab_cls(\"remote\")}", onclick: move |_| tab.set("remote".into()), "{loc(&lang, \"Zdalne\", \"Remote\")}" }
            }
            div { class: "settings-content", {body_el} }
        }
    }
}

async fn refresh_after_save(state: Signal<ClientState>) {
    crate::state::refresh(state).await;
}
