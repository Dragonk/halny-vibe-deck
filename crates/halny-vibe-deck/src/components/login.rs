//! Login wizard (L1 — TUI `setup/wizard` parity): method choice,
//! browser PKCE stepper (Open browser → Waiting → Exchanging →
//! Complete) and the API-key input. Shown instead of the Shell when
//! `setup/status` reports no API key (app.rs gate), or on demand
//! from Settings ("Sign in again") / the rate-limit banner.
//!
//! Polling runs client-side every 3 s (`gloo-timers` sleep — never
//! `tokio::time::sleep`, which panics on wasm); the PKCE HTTP flow
//! itself lives engine-side (`auth_browser.rs`).

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{lang, refresh, sleep_ms, token, ClientState};

pub fn LoginView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    // phase: method | browser | apikey | done
    let phase = use_signal(|| "method".to_string());
    // browser stepper: idle | waiting | complete
    let step = use_signal(|| "idle".to_string());
    let error: Signal<String> = use_signal(String::new);
    let sign_in_url = use_signal(String::new);
    let mut api_key = use_signal(String::new);
    let mut custom_domain = use_signal(String::new);
    let copied = use_signal(|| false);

    let setup = state.read().snapshot.setup.clone();
    let can_go_back = state
        .read()
        .snapshot
        .setup
        .as_ref()
        .map(|s| s.has_api_key)
        .unwrap_or(false);

    // Bootstrap: make sure setup/status + account are loaded (the
    // gate only renders this view when setup exists, but a manual
    // "sign in again" can arrive before the preflow finished).
    {
        let state = state.clone();
        use_future(move || async move {
            if state.read().snapshot.setup.is_none() {
                let _ = crate::api::api_setup_account_status(token(&state)).await;
                refresh(state).await;
            }
        });
    }

    let lang_future = lang.clone();
    // Browser flow poller: one loop for the lifetime of the view;
    // polls only while the stepper is in "waiting" (signal reads
    // after awaits are not tracked — exactly what we want here).
    {
        let state = state.clone();
        let mut step = step.clone();
        let mut error = error.clone();
        use_future(move || {
            let lang = lang_future.clone();
            async move {
            loop {
                sleep_ms(3_000).await;
                if step.read().as_str() != "waiting" {
                    continue;
                }
                match crate::api::api_login_poll(token(&state)).await {
                    Ok(p) if p == "completed" => {
                        step.set("complete".to_string());
                        refresh(state).await;
                    }
                    Ok(p) if p == "expired" => {
                        error.set(tr(&lang, "loginExpired").to_string());
                        step.set("idle".to_string());
                    }
                    Ok(p) if p == "denied" => {
                        error.set(tr(&lang, "loginDenied").to_string());
                        step.set("idle".to_string());
                    }
                    Ok(_) => {} // pending — keep polling
                    Err(e) => {
                        let msg = e.to_string();
                        error.set(if msg.contains("not connected") {
                            tr(&lang, "loginNotConnected").to_string()
                        } else {
                            msg
                        });
                        step.set("idle".to_string());
                    }
                }
            }
            }
        });
    }

    let start_browser = {
        let state = state.clone();
        let step = step.clone();
        let error = error.clone();
        let sign_in_url = sign_in_url.clone();
        move |_| {
            let state = state.clone();
            let mut step = step.clone();
            let mut error = error.clone();
            let mut sign_in_url = sign_in_url.clone();
            async move {
                error.set(String::new());
                match crate::api::api_login_start(token(&state)).await {
                    Ok(v) => {
                        // Open the browser tab (TUI external_url::open —
                        // the browser lives on the user's side).
                        let url = v.sign_in_url.replace('\'', "%27");
                        dioxus::document::eval(&format!(
                            "try{{window.open('{url}','_blank','noopener')}}catch(e){{}}"
                        ));
                        sign_in_url.set(v.sign_in_url);
                        step.set("waiting".to_string());
                    }
                    Err(e) => error.set(e.to_string()),
                }
            }
        }
    };

    let copy_url = {
        let copied = copied.clone();
        let url = sign_in_url.read().clone();
        move |_| {
            let mut copied = copied.clone();
            let url = url.clone();
            async move {
                dioxus::document::eval(&format!(
                    "try{{navigator.clipboard.writeText('{url}')}}catch(e){{}}"
                ));
                copied.set(true);
            }
        }
    };

    let submit_api_key = {
        let state = state.clone();
        let phase = phase.clone();
        let error = error.clone();
        let lang = lang.clone();
        let api_key = api_key.read().trim().to_string();
        let custom_domain = custom_domain.read().trim().to_string();
        move |_| {
            let lang = lang.clone();
            let state = state.clone();
            let mut phase = phase.clone();
            let mut error = error.clone();
            let api_key = api_key.clone();
            let custom_domain = custom_domain.clone();
            async move {
                error.set(String::new());
                match crate::api::api_login_with_api_key(
                    token(&state),
                    api_key,
                    if custom_domain.is_empty() {
                        None
                    } else {
                        Some(custom_domain)
                    },
                )
                .await
                {
                    Ok(()) => {
                        phase.set("done".to_string());
                        refresh(state).await;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        error.set(if msg.contains("not connected") {
                            tr(&lang, "loginNotConnected").to_string()
                        } else {
                            msg
                        });
                    }
                }
            }
        }
    };

    let go_back = {
        let mut state = state.clone();
        move |_| {
            state.write().login_requested = false;
        }
    };

    // --- body per phase ---
    let pinned = setup
        .as_ref()
        .and_then(|s| s.pinned_console_url.clone())
        .is_some();
    let env_var = setup
        .as_ref()
        .and_then(|s| s.provider.as_ref())
        .map(|p| p.api_key_env_var.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "MISTRAL_API_KEY".to_string());

    let body: Element = if setup.as_ref().map(|s| s.unsupported).unwrap_or(false) {
        // Old CLI: no wizard — manual env-var instructions (plan §1.4).
        rsx! {
            p { class: "setup-desc",
                "{tf(&lang, \"loginUnsupported\", &[(\"env_var\", env_var.as_str())])}"
            }
            if can_go_back {
                button { class: "setup-btn-ghost", onclick: go_back, "{tr(&lang, \"loginBack\")}" }
            }
        }
    } else {
        match phase.read().as_str() {
            "browser" => {
                let step_now = step.read().clone();
                let url = sign_in_url.read().clone();
                let copied_now = *copied.read();
                let stepper_label = match step_now.as_str() {
                    "waiting" => tr(&lang, "loginStepWaiting"),
                    "complete" => tr(&lang, "loginStepComplete"),
                    _ => tr(&lang, "loginStepOpen"),
                };
                rsx! {
                    div { class: "setup-steps",
                        div { class: "setup-step step-done",
                            span { class: "step-num", "✓" }
                            div { div { class: "step-name", "{tr(&lang, \"loginStepOpen\")}" } }
                        }
                        div { class: "setup-step",
                            span { class: "step-num",
                                if step_now == "complete" { "✓" } else if step_now == "waiting" { "⟳" } else { "2" }
                            }
                            div { div { class: "step-name", "{stepper_label}" } }
                        }
                    }
                    if !url.is_empty() {
                        div { class: "setup-actions",
                            button { class: "setup-btn-ghost", onclick: copy_url,
                                if copied_now { "{tr(&lang, \"loginCopied\")}" } else { "{tr(&lang, \"loginCopyUrl\")}" }
                            }
                        }
                    }
                    if step_now == "waiting" && !url.is_empty() {
                        // fallback visible while the browser tab is open
                        div { class: "field-hint", "{url}" }
                    }
                }
            }
            "apikey" => rsx! {
                div { class: "setup-body",
                    label { class: "field-label", "{tr(&lang, \"loginApiKeyLabel\")}" }
                    input {
                        r#type: "password",
                        class: "field-input",
                        placeholder: "{tr(&lang, \"loginApiKeyPlaceholder\")}",
                        value: "{api_key.read()}",
                        oninput: move |e| api_key.set(e.value()),
                    }
                    details { style: "margin:10px 0;",
                        summary { style: "cursor:pointer;font-size:12px;color:var(--text-muted);",
                            "{tr(&lang, \"loginCustomDomain\")}"
                        }
                        if !pinned {
                            input {
                                class: "field-input",
                                placeholder: "https://console.example.com",
                                value: "{custom_domain.read()}",
                                oninput: move |e| custom_domain.set(e.value()),
                            }
                            div { class: "field-hint", "{tr(&lang, \"loginCustomDomainHint\")}" }
                        } else {
                            div { class: "field-hint", "{tr(&lang, \"loginCustomDomainHint\")}" }
                        }
                    }
                    button { class: "setup-btn-primary", onclick: submit_api_key,
                        "{tr(&lang, \"loginApiKeySubmit\")}"
                    }
                }
            },
            "done" => rsx! {
                div { class: "setup-steps",
                    div { class: "setup-step step-done",
                        span { class: "step-num", "✓" }
                        div { div { class: "step-name", "{tr(&lang, \"loginStepComplete\")}" } }
                    }
                }
                button { class: "setup-btn-primary", onclick: go_back, "{tr(&lang, \"loginBack\")}" }
            },
            _ => {
                // method choice
                let browser_supported = setup
                    .as_ref()
                    .map(|s| s.supports_browser_sign_in)
                    .unwrap_or(false);
                rsx! {
                    div { class: "setup-steps",
                        if browser_supported {
                            button { class: "setup-btn-primary", onclick: start_browser,
                                "{tr(&lang, \"loginBrowser\")}"
                            }
                            div { class: "field-hint", "{tr(&lang, \"loginBrowserDesc\")}" }
                        }
                        button {
                            class: if browser_supported { "setup-btn-ghost" } else { "setup-btn-primary" },
                            onclick: move |_| {
                                let mut phase = phase.clone();
                                phase.set("apikey".to_string());
                            },
                            "{tr(&lang, \"loginApiKey\")}"
                        }
                        div { class: "field-hint", "{tr(&lang, \"loginApiKeyDesc\")}" }
                    }
                }
            }
        }
    };

    let error_el: Element = {
        let err = error.read().clone();
        if err.is_empty() {
            VNode::empty()
        } else {
            rsx! {
                div { class: "chat-error-banner", span { "⚠" } " {err}"
                    button {
                        class: "setup-btn-ghost",
                        style: "margin-left:12px;",
                        onclick: move |_| {
                            let mut phase = phase.clone();
                            let mut step = step.clone();
                            let mut error = error.clone();
                            phase.set("method".to_string());
                            step.set("idle".to_string());
                            error.set(String::new());
                        },
                        "{tr(&lang, \"loginRetry\")}"
                    }
                }
            }
        }
    };

    rsx! {
        div { class: "setup-overlay",
            div { class: "setup-card",
                div { class: "setup-header",
                    div { class: "setup-title", "{tr(&lang, \"loginTitle\")}" }
                    div { class: "setup-subtitle", "{tr(&lang, \"loginSubtitle\")}" }
                }
                {error_el}
                {body}
            }
        }
    }
}
