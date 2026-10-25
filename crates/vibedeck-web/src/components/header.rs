//! Header: brand, ACP activity status with tooltip, dock toggle,
//! theme cycle (auto -> dark -> light). Ported 1:1 from Nexus Header.tsx
//! (window buttons are Electron-only and stay out of the web build).

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{is_desktop, lang, save_prefs, ClientState};

pub fn Header() -> Element {
    let state = use_context::<Signal<ClientState>>();

    let s = state.read();
    let lang = lang(&state);
    let status = s.snapshot.status.clone();
    let acp_error = s.snapshot.acp_error.clone();
    let theme = s.snapshot.config.theme.clone();
    let dock_open = s.snapshot.config.dock_open;
    let limit = s.snapshot.config.sidebar_sessions_limit.unwrap_or(0);
    let language = s.snapshot.config.language.clone();
    let auth_required = s.auth_required;
    drop(s);

    let (activity_cls, activity_text) = match status.as_str() {
        "connected" => ("activity-connected", tr(&lang, "statusConnected").to_string()),
        "connecting" => ("activity-connecting", tr(&lang, "statusConnecting").to_string()),
        "error" => (
            "activity-error",
            tf(&lang, "statusError", &[("error", acp_error.as_str())]),
        ),
        _ => ("activity-disconnected", tr(&lang, "statusDisconnected").to_string()),
    };
    let activity_cls = format!("header-activity {activity_cls}");

    let (theme_icon, theme_title) = match theme.as_str() {
        "auto" => ('\u{25D1}', tr(&lang, "themeAuto")),
        "dark" => ('\u{263c}', tr(&lang, "themeDark")),
        _ => ('\u{263e}', tr(&lang, "themeLight")),
    };
    let next_theme = match theme.as_str() {
        "auto" => "dark",
        "dark" => "light",
        _ => "auto",
    };
    let dock_cls = if dock_open {
        "header-btn dock-toggle-btn active".to_string()
    } else {
        "header-btn dock-toggle-btn".to_string()
    };

    rsx! {
        header { class: "header",
            div { class: "header-left",
                div { class: "header-brand",
                    span { class: "header-brand-name", "{tr(&lang, \"appName\")}" }
                }
                div { class: "{activity_cls}",
                    span { class: "activity-icon" }
                    span { class: "activity-text", "{activity_text}" }
                    div { class: "activity-tooltip chart-tooltip",
                        div { "{tf(&lang, \"statusLabel\", &[(\"status\", activity_text.as_str())])}" }
                        if status == "connected" {
                            div { style: "margin-top:2px;", "{tr(&lang, \"agentLabel\")}" }
                        }
                        if status == "error" && !acp_error.is_empty() {
                            div { style: "margin-top:2px;color:#f87171;", "{acp_error}" }
                        }
                    }
                }
            }
            div { class: "header-controls",
                button {
                    class: "{dock_cls}",
                    title: "{tr(&lang, \"dockToggle\")}",
                    onclick: {
                        let state = state.clone();
                        let language = language.clone();
                        let theme = theme.clone();
                        let limit = limit;
                        move |_| {
                            let state = state.clone();
                            let language = language.clone();
                            let theme = theme.clone();
                            let dock = !dock_open;
                            let limit = limit;
                            async move {
                                save_prefs(state, language, theme, dock, limit).await;
                            }
                        }
                    },
                    svg {
                        width: "14", height: "14", view_box: "0 0 24 24",
                        fill: "none", stroke: "currentColor",
                        "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
                        rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
                        line { x1: "15", y1: "3", x2: "15", y2: "21" }
                    }
                }
                button {
                    class: "header-btn theme-btn",
                    title: "{theme_title}",
                    onclick: {
                        let state = state.clone();
                        let language = language.clone();
                        let next_theme = next_theme.to_string();
                        let limit = limit;
                        move |_| {
                            let state = state.clone();
                            let language = language.clone();
                            let next_theme = next_theme.clone();
                            let limit = limit;
                            async move {
                                save_prefs(state, language, next_theme, dock_open, limit).await;
                            }
                        }
                    },
                    "{theme_icon}"
                }
                // Frameless desktop window: the window buttons live in the
                // header and talk to the wry window over IPC (main.rs).
                if is_desktop() {
                    button {
                        class: "header-btn",
                        title: "{tr(&lang, \"minimize\")}",
                        onclick: move |_| {
                            dioxus::document::eval("window.ipc.postMessage('vd-min')");
                        },
                        "─"
                    }
                    button {
                        class: "header-btn",
                        title: "{tr(&lang, \"maximize\")}",
                        onclick: move |_| {
                            dioxus::document::eval("window.ipc.postMessage('vd-max')");
                        },
                        "□"
                    }
                    button {
                        class: "header-btn close-btn",
                        title: "{tr(&lang, \"close\")}",
                        onclick: move |_| {
                            dioxus::document::eval("window.ipc.postMessage('vd-close')");
                        },
                        "✕"
                    }
                } else if auth_required {
                    // Remote access in a browser: offer a logout (⏻) instead
                    // of the window buttons.
                    button {
                        class: "header-btn",
                        title: "{tr(&lang, \"logoutLabel\")}",
                        onclick: {
                            let state = state.clone();
                            move |_| {
                                let mut state = state.clone();
                                async move {
                                    state.write().token = None;
                                    state.write().loaded = false;
                                }
                            }
                        },
                        "⏻"
                    }
                }
            }
        }
    }
}
