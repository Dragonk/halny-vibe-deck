//! Header: brand, agent activity status with tooltip, dock toggle,
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
    let agent_error = s.snapshot.agent_error.clone();
    let theme = s.snapshot.config.theme.clone();
    let dock_open = s.snapshot.config.dock_open;
    let limit = s.snapshot.config.sidebar_sessions_limit.unwrap_or(0);
    let language = s.snapshot.config.language.clone();
    drop(s);

    let (activity_cls, activity_text) = match status.as_str() {
        "connected" => ("activity-connected", tr(&lang, "statusConnected").to_string()),
        "connecting" => ("activity-connecting", tr(&lang, "statusConnecting").to_string()),
        "error" => (
            "activity-error",
            tf(&lang, "statusError", &[("error", agent_error.as_str())]),
        ),
        _ => ("activity-disconnected", tr(&lang, "statusDisconnected").to_string()),
    };
    let activity_cls = format!("header-activity {activity_cls}");

    // L1: account banner (TUI ui/banner + startup/banners): the
    // plan title next to the status and the planOffer CTA link.
    // Only for a ready account; unsupported CLI → no banner.
    let account = state.read().snapshot.account.clone();
    let (plan_title, plan_offer) = match account.as_ref() {
        Some(acc) if acc.status == "ready" && !acc.unsupported => (
            acc.plan.as_ref().map(|p| p.title.clone()).filter(|t| !t.is_empty()),
            acc.plan_offer
                .as_ref()
                .map(|o| (o.kind.clone(), o.url.clone()))
                .filter(|(_, u)| !u.is_empty()),
        ),
        _ => (None, None),
    };

    // Theme naming: the config default is "" (no theme set) and the CLI
    // world also says "system" — all three mean the OS preference
    // (apply_theme falls back to prefers-color-scheme for anything that
    // is not dark/light). The cycle persists the canonical "auto".
    let theme_is_auto = matches!(theme.as_str(), "" | "system" | "auto");
    let (theme_icon, theme_title) = if theme_is_auto {
        ('\u{25D1}', tr(&lang, "themeAuto"))
    } else if theme == "dark" {
        ('\u{263c}', tr(&lang, "themeDark"))
    } else {
        ('\u{263e}', tr(&lang, "themeLight"))
    };
    let next_theme = if theme_is_auto {
        "dark"
    } else if theme == "dark" {
        "light"
    } else {
        "auto"
    };
    let dock_cls = if dock_open {
        "header-btn dock-toggle-btn active".to_string()
    } else {
        "header-btn dock-toggle-btn".to_string()
    };

    rsx! {
        header { class: "header",
            div { class: "header-left",
                button {
                    class: "header-btn header-menu-btn",
                    title: "{tr(&lang, \"menuLabel\")}",
                    onclick: {
                        let mut state = state.clone();
                        move |_| {
                            let mut s = state.write();
                            s.mobile_sidebar_open = !s.mobile_sidebar_open;
                        }
                    },
                    svg {
                        width: "14", height: "14",
                        view_box: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor",
                        stroke_width: "2",
                        stroke_linecap: "round",
                        line { x1: "3", y1: "6", x2: "21", y2: "6" },
                        line { x1: "3", y1: "12", x2: "21", y2: "12" },
                        line { x1: "3", y1: "18", x2: "21", y2: "18" },
                    }
                }
                div { class: "header-brand",
                    crate::components::brand::BrandMark { size: 0.24, class: "header-brand-mark".to_string() }
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
                        if status == "error" && !agent_error.is_empty() {
                            div { style: "margin-top:2px;color:#f87171;", "{agent_error}" }
                        }
                    }
                }
                // L1: plan banner + planOffer CTA (Nexus banner parity).
                if let Some(title) = &plan_title {
                    span {
                        class: "header-plan",
                        style: "margin-left:8px;font-size:12px;color:var(--text-muted);white-space:nowrap;",
                        "{title}"
                    }
                }
                if let Some((kind, url)) = &plan_offer {
                    a {
                        class: "header-plan-cta",
                        href: "{url}",
                        target: "_blank",
                        rel: "noopener",
                        style: "margin-left:6px;font-size:12px;color:var(--accent);text-decoration:none;white-space:nowrap;",
                        if kind == "upgrade_to_pro" { "{tr(&lang, \"rateLimitUpgrade\")}" } else { "{tr(&lang, \"rateLimitSwitchKey\")}" }
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
                }
            }
        }
    }
}
