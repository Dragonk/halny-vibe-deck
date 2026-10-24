//! Input bar: textarea + project chip + model/mode/thinking chips with
//! dropdowns + send/stop, the live usage bar and the status bar.
//! Ported from Nexus InputBar.tsx (same CSS classes).

use dioxus::prelude::*;
use vibedeck_core::proto::ConfigOption;

use crate::i18n::tr;
use crate::state::{lang, submit, token, ClientState};

pub fn InputBar() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut draft = use_signal(String::new);
    let open_chip = use_signal(String::new);

    let s = state.read();
    let lang = lang(&state);
    let busy = s.snapshot.busy;
    let wd = s.snapshot.working_directory.clone();
    let cfg_wd = s.snapshot.config.working_directory.clone();
    let _options = s.snapshot.config_options.clone();
    let status = s.snapshot.status.clone();
    drop(s);

    let cwd = if wd.is_empty() { cfg_wd } else { wd };
    let project_label = short_dir(&cwd);
    let queue_len = s_read_queue(&state);

    let send_text = move |text: String| {
        let state = state.clone();
        async move {
            submit(state, text).await;
        }
    };

    rsx! {
        div { class: "input-card-wrap",
            div { class: "input-card",
                textarea {
                    class: "input-card-editor",
                    rows: "1",
                    placeholder: "{tr(&lang, \"typeMessage\")}",
                    value: "{draft()}",
                    oninput: move |e| draft.set(e.value()),
                    onkeydown: move |e: Event<KeyboardData>| {
                        if e.key() == Key::Enter && !e.modifiers().shift() {
                            e.prevent_default();
                            let text = draft().clone();
                            draft.set(String::new());
                            if !text.trim().is_empty() {
                                spawn(send_text(text));
                            }
                        }
                    },
                }
                if queue_len > 0 {
                    div { class: "input-attachments",
                        div { class: "attachment-chip",
                            span { class: "attachment-name", "⏳ {queue_len}" }
                        }
                    }
                }
                div { class: "input-card-toolbar",
                    div { class: "input-card-left",
                        button {
                            class: "ic-folder-chip",
                            title: "{cwd}",
                            disabled: true,
                            span { class: "ic-folder-name", "{project_label}" }
                        }
                    }
                    div { class: "input-card-right",
                        { config_chip(state.clone(), lang.clone(), "model", tr(&lang, "modelPicker").to_string(), open_chip.clone()) }
                        { config_chip(state.clone(), lang.clone(), "mode", tr(&lang, "modePicker").to_string(), open_chip.clone()) }
                        { config_chip(state.clone(), lang.clone(), "thinking", tr(&lang, "thinkingPicker").to_string(), open_chip.clone()) }
                        if busy {
                            button {
                                class: "ic-stop-btn",
                                title: "Stop",
                                onclick: {
                                    let state = state.clone();
                                    move |_| {
                                        let state = state.clone();
                                        async move {
                                            let _ = crate::api::api_cancel(token(&state)).await;
                                        }
                                    }
                                },
                                "■"
                            }
                        } else {
                            button {
                                class: "ic-send-btn",
                                disabled: draft().trim().is_empty(),
                                title: "{tr(&lang, \"sendEnter\")}",
                                onclick: {
                                    let state = state.clone();
                                    move |_| {
                                        let text = draft().clone();
                                        draft.set(String::new());
                                        async move {
                                            if !text.trim().is_empty() {
                                                submit(state, text).await;
                                            }
                                        }
                                    }
                                },
                                "➤"
                            }
                        }
                    }
                }
            }
            crate::components::usagebar::UsageBar {}
            div { class: "input-status-bar",
                div { class: "acp-status {status}",
                    span { class: "status-dot" }
                    span { class: "status-label", "{status}" }
                }
                if !cwd.is_empty() {
                    span { class: "status-cwd", title: "{cwd}", "{project_label}" }
                }
            }
        }
    }
}

fn s_read_queue(state: &Signal<ClientState>) -> usize {
    state.read().queue.len()
}

fn short_dir(dir: &str) -> String {
    let name = dir.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("");
    if name.is_empty() { dir.to_string() } else { name.to_string() }
}

/// One model/mode/thinking chip with a dropdown (Nexus ConfigChip).
fn config_chip(
    state: Signal<ClientState>,
    _lang: String,
    id: &'static str,
    label: String,
    open: Signal<String>,
) -> Element {
    let s = state.read();
    let busy = s.snapshot.busy;
    let option: Option<ConfigOption> = s.snapshot.config_options.iter().find(|o| o.id == id).cloned();
    drop(s);

    let disabled = busy || option.is_none();
    let items: Vec<(String, String, String)> = option
        .as_ref()
        .map(|o| {
            o.choices
                .iter()
                .map(|c| (c.value.clone(), c.name.clone(), c.description.clone()))
                .collect()
        })
        .unwrap_or_default();
    let is_open = open.read().clone() == id;
    let current_value = option
        .as_ref()
        .map(|o| o.current_value.clone())
        .unwrap_or_default();
    let current_label = option
        .as_ref()
        .map(|o| o.current_label())
        .unwrap_or_else(|| "—".to_string());
    let cls = if disabled { "ic-chip ic-chip-disabled" } else { "ic-chip" };

    // Precompute the dropdown items (rsx children cannot contain raw
    // statements) — one element per choice.
    let item_els: Vec<Element> = items
        .into_iter()
        .map(|(value, name, desc)| {
            let state = state.clone();
            let open = open.clone();
            let active = value == current_value;
            let item_cls = if active { "ic-chip-item active" } else { "ic-chip-item" };
            let check: &str = if active { "✓" } else { "" };
            let has_desc = !desc.is_empty();
            rsx! {
                button {
                    class: "{item_cls}",
                    title: "{desc}",
                    onclick: move |_| {
                        let mut state = state.clone();
                        let mut open = open.clone();
                        let value = value.clone();
                        async move {
                            open.set(String::new());
                            {
                                let mut s = state.write();
                                if let Some(o) = s.snapshot.config_options.iter_mut().find(|o| o.id == id) {
                                    o.current_value = value.clone();
                                }
                            }
                            let _ = crate::api::api_set_config_option(
                                token(&state),
                                id.to_string(),
                                value.clone(),
                            )
                            .await;
                        }
                    },
                    span { class: "ic-chip-item-check", "{check}" }
                    span { class: "ic-chip-item-body",
                        span { class: "ic-chip-item-name", "{name}" }
                        if has_desc {
                            span { class: "ic-chip-item-desc", "{desc}" }
                        }
                    }
                }
            }
        })
        .collect();

    let dropdown_el: Element = if is_open && option.is_some() {
        rsx! {
            div { class: "ic-chip-dropdown",
                {item_els.iter().cloned()}
            }
        }
    } else {
        VNode::empty()
    };

    rsx! {
        div { class: "ic-chip-wrap",
            button {
                class: "{cls}",
                disabled: disabled,
                title: "{label}",
                onclick: {
                    let mut open = open.clone();
                    move |_| {
                        if disabled {
                            return;
                        }
                        let cur = open.read().clone();
                        if cur == id {
                            open.set(String::new());
                        } else {
                            open.set(id.to_string());
                        }
                    }
                },
                span { class: "ic-chip-label", "{label}" }
                span { class: "ic-chip-value", "{current_label}" }
                span { class: "ic-chip-chevron", "▾" }
            }
            {dropdown_el}
        }
    }
}
