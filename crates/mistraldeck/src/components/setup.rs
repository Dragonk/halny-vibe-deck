//! First-run setup screen (Nexus SetupView): detects uv, installs
//! mistral-vibe through uv with a live progress log; the engine then
//! re-detects vibe-app-server and connects. Shown when the agent
//! binary is missing; skippable.

use dioxus::prelude::*;

use crate::i18n::tr;
use crate::setup::SetupLine;
use crate::state::{lang, refresh, ClientState};

fn loc(lang: &str, pl: &str, en: &str) -> String {
    if lang == "pl" {
        pl.to_string()
    } else {
        en.to_string()
    }
}

#[component]
pub fn SetupView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    // confirm | installing | done | error
    let phase = use_signal(|| "confirm".to_string());
    let mut uv_found = use_signal(|| false);
    let lines: Signal<Vec<SetupLine>> = use_signal(Vec::new);

    // Detect uv for the confirmation screen.
    use_future(move || async move {
        if let Ok(st) = crate::setup::api_setup_status().await {
            uv_found.set(st.uv_found);
        }
    });

    let phase_now = phase.read().clone();
    let is_confirm = phase_now == "confirm";
    let is_installing = phase_now == "installing";
    let is_done = phase_now == "done";
    let is_error = phase_now == "error";

    // ---- confirm body (precomputed; rsx children cannot hold statements) ----
    let confirm_body: Element = if is_confirm {
        let uv_ok = *uv_found.read();
        let step1_cls = if uv_ok {
            "setup-step step-done"
        } else {
            "setup-step"
        };
        let step1_num = if uv_ok { "✓" } else { "1" };
        let step2_num = if uv_ok { "1" } else { "2" };
        rsx! {
            div { class: "setup-body",
                p { class: "setup-desc", "{tr(&lang, \"cliNotDetected\")}" }
                div { class: "setup-steps",
                    div { class: "{step1_cls}",
                        span { class: "step-num", "{step1_num}" }
                        div {
                            div { class: "step-name", "{tr(&lang, \"uvPackageManager\")}" }
                            if uv_ok {
                                div { class: "step-hint ok", "{tr(&lang, \"alreadyInstalled\")}" }
                            } else {
                                div { class: "step-hint", "{tr(&lang, \"downloadedFromAstral\")}" }
                            }
                        }
                    }
                    div { class: "setup-step",
                        span { class: "step-num", "{step2_num}" }
                        div {
                            div { class: "step-name", "{tr(&lang, \"mistralVibe\")}" }
                            div { class: "step-hint",
                                code { "{tr(&lang, \"uvToolInstall\")}" }
                            }
                        }
                    }
                }
                div { class: "setup-actions",
                    button {
                        class: "setup-btn-primary",
                        onclick: {
                            let state = state.clone();
                            let phase = phase.clone();
                            let lines = lines.clone();
                            move |_| {
                                let state = state.clone();
                                let mut phase = phase.clone();
                                let mut lines = lines.clone();
                                async move {
                                    phase.set("installing".to_string());
                                    lines.set(Vec::new());
                                    let _ = crate::setup::api_setup_install(
                                        crate::state::token(&state),
                                    )
                                    .await;
                                    loop {
                                        crate::state::sleep_ms(800)
                                            .await;
                                        match crate::setup::api_setup_log(
                                            crate::state::token(&state),
                                        )
                                        .await
                                        {
                                            Ok((ls, running, finished)) => {
                                                lines.set(ls);
                                                if let Some(ok) = finished {
                                                    phase.set(
                                                        if ok { "done" } else { "error" }.to_string(),
                                                    );
                                                    break;
                                                }
                                                if !running {
                                                    phase.set("error".to_string());
                                                    break;
                                                }
                                            }
                                            Err(_) => {
                                                phase.set("error".to_string());
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        "{tr(&lang, \"installAutomatically\")}"
                    }
                    button {
                        class: "setup-btn-ghost",
                        onclick: {
                            let mut state = state.clone();
                            move |_| {
                                state.write().setup_skipped = true;
                            }
                        },
                        "{tr(&lang, \"skipManualConfigure\")}"
                    }
                }
            }
        }
    } else {
        VNode::empty()
    };

    // ---- installing / done / error body ----
    let log_body: Element = if is_installing || is_done || is_error {
        let line_els: Vec<Element> = lines
            .read()
            .clone()
            .into_iter()
            .map(|l| {
                let row_cls = format!("log-line log-{}", l.kind);
                let prefix = match l.kind.as_str() {
                    "success" => "✓",
                    "error" => "✗",
                    _ => "›",
                };
                rsx! {
                    div { class: "{row_cls}",
                        span { class: "log-prefix", "{prefix}" }
                        "{l.message}"
                    }
                }
            })
            .collect();
        let actions_el: Element = if is_done {
            rsx! {
                div { class: "setup-actions",
                    div { class: "field-hint", style: "margin-bottom:8px;",
                        "{loc(&lang, \"Zainstalowano — agent łączy się automatycznie.\", \"Installed — the agent connects automatically.\")}"
                    }
                    button {
                        class: "setup-btn-primary",
                        onclick: {
                            let state = state.clone();
                            move |_| {
                                let mut state = state.clone();
                                async move {
                                    if let Ok(st) = crate::setup::api_setup_status().await {
                                        if st.agent_found {
                                            state.write().setup_needed = false;
                                            refresh(state).await;
                                        }
                                    }
                                }
                            }
                        },
                        "{tr(&lang, \"doneLaunchApp\")}"
                    }
                }
            }
        } else if is_error {
            rsx! {
                div { class: "setup-actions",
                    button {
                        class: "setup-btn-primary",
                        onclick: {
                            let mut phase = phase.clone();
                            move |_| {
                                phase.set("confirm".to_string());
                            }
                        },
                        "{tr(&lang, \"tryAgain\")}"
                    }
                    button {
                        class: "setup-btn-ghost",
                        onclick: {
                            let mut state = state.clone();
                            move |_| {
                                state.write().setup_skipped = true;
                            }
                        },
                        "{tr(&lang, \"skipManualConfigure\")}"
                    }
                }
            }
        } else {
            VNode::empty()
        };
        rsx! {
            div { class: "setup-body",
                div { class: "setup-log",
                    {line_els.iter().cloned()}
                    if is_installing {
                        div { class: "log-line log-info",
                            span { class: "log-spinner", "⟳" }
                            "{tr(&lang, \"installing\")}"
                        }
                    }
                }
                {actions_el}
            }
        }
    } else {
        VNode::empty()
    };

    rsx! {
        div { class: "setup-overlay",
            div { class: "setup-card",
                div { class: "setup-header",
                    div { class: "setup-logo",
                        svg {
                            xmlns: "http://www.w3.org/2000/svg",
                            width: "32", height: "32",
                            view_box: "0 0 24 24",
                            fill: "var(--accent)",
                            path { d: "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-1 14H9V8h2v8zm4 0h-2V8h2v8z" }
                        }
                    }
                    div {
                        div { class: "setup-title", "{tr(&lang, \"appName\")}" }
                        div { class: "setup-subtitle", "{tr(&lang, \"configurationRequired\")}" }
                    }
                }
                {confirm_body}
                {log_body}
            }
        }
    }
}
