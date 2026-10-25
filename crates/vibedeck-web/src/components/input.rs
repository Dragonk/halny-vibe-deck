//! Input bar: textarea + project chip + model/mode/thinking chips with
//! dropdowns + send/stop, the live usage bar and the status bar.
//! Also the Nexus `@` file picker, `/` slash picker (commands + skills +
//! prompts) and the attach menu (file / directory / image / branch diff).
//! Ported from Nexus InputBar.tsx (same CSS classes).

use dioxus::prelude::*;
use vibedeck_core::proto::ConfigOption;

use crate::i18n::tr;
use crate::state::{lang, submit, token, ClientState};

/// One `/` picker entry (Nexus SlashItem).
#[derive(Clone, Debug, PartialEq)]
struct SlashItem {
    id: String,
    label: String,
    /// command | skill | prompt
    kind: &'static str,
    /// what gets inserted when the item is picked
    content: String,
    description: String,
    hint: String,
}

/// Query of the `@` marker the caret is inside, if any (Nexus /@([^\s@]*)$/).
fn at_query(text: &str) -> Option<String> {
    let after = text
        .rsplit(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("");
    let q = after.strip_prefix('@')?;
    if q.contains('@') {
        return None;
    }
    Some(q.to_string())
}

/// Query of the `/` marker the caret is inside (Nexus /(?:^|[\s])\/([^\s/]*)$/).
fn slash_query(text: &str) -> Option<String> {
    let after = text
        .rsplit(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("");
    let q = after.strip_prefix('/')?;
    if q.contains('/') || q.contains('@') {
        return None;
    }
    Some(q.to_string())
}

/// Slash items filtered by the current query (label substring, Nexus style).
fn filter_slash(items: &[SlashItem], q: &str) -> Vec<SlashItem> {
    if q.is_empty() {
        return items.to_vec();
    }
    let ql = q.to_lowercase();
    items
        .iter()
        .filter(|i| i.label.to_lowercase().contains(&ql))
        .cloned()
        .collect()
}

/// Replace the trailing `@query` token with `@path ` (dirs get a trailing
/// slash, like the Nexus insertAtFile).
fn insert_at_marker(draft: &str, path: &str, is_dir: bool) -> String {
    let marker_pos = draft.rfind('@').unwrap_or(draft.len());
    let suffix = if is_dir { "/ " } else { " " };
    format!("{}@{}{}", &draft[..marker_pos], path, suffix)
}

/// Replace the trailing `/query` token. Commands insert `/name hint `;
/// skills and prompts insert their whole content followed by a newline
/// (Nexus insertSlashItem).
fn insert_slash_item(draft: &str, item: &SlashItem) -> String {
    let slash_pos = draft.rfind('/').unwrap_or(draft.len());
    let head = &draft[..slash_pos];
    if item.kind == "command" {
        let hint = item.hint.trim();
        if hint.is_empty() {
            format!("{}{} ", head, item.content)
        } else {
            format!("{}{} {} ", head, item.content, hint)
        }
    } else {
        let nl = if item.content.ends_with('\n') { "" } else { "\n" };
        format!("{}{}{}", head, item.content, nl)
    }
}

fn loc(lang: &str, pl: &str, en: &str) -> String {
    if lang == "pl" {
        pl.to_string()
    } else {
        en.to_string()
    }
}

pub fn InputBar() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut draft = use_signal(String::new);
    let open_chip = use_signal(String::new);

    // @ picker
    let mut at_open = use_signal(|| false);
    let at_items: Signal<Vec<crate::fsgit::FileEntry>> = use_signal(Vec::new);
    let mut at_idx = use_signal(|| 0usize);

    // / picker
    let mut slash_open = use_signal(|| false);
    let slash_items: Signal<Vec<SlashItem>> = use_signal(Vec::new);
    let mut slash_idx = use_signal(|| 0usize);
    let slash_loaded = use_signal(|| false);

    // attach menu
    let mut attach_open = use_signal(|| false);

    let s = state.read();
    let lang = lang(&state);
    let busy = s.snapshot.busy;
    let wd = s.snapshot.working_directory.clone();
    let cfg_wd = s.snapshot.config.working_directory.clone();
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

    // ---- precomputed picker rows (rsx children cannot contain statements) ----
    let at_item_els: Vec<Element> = {
        let items = at_items.read().clone();
        let cur = *at_idx.read();
        items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let mut draft = draft.clone();
                let mut at_open = at_open.clone();
                let path = it.path.clone();
                let is_dir = it.is_dir;
                let name = it.name.clone();
                let hint = it.path.clone();
                let item_cls = if i == cur {
                    "input-picker-item active"
                } else {
                    "input-picker-item"
                };
                let icon = if is_dir { "📁" } else { "📄" };
                rsx! {
                    button {
                        class: "{item_cls}",
                        onclick: move |_| {
                            let d = draft.read().clone();
                            let new_draft = insert_at_marker(&d, &path, is_dir);
                            draft.set(new_draft);
                            at_open.set(false);
                        },
                        span { class: "picker-item-icon", "{icon}" }
                        span { class: "picker-item-body",
                            span { class: "picker-item-kind", "{name}" }
                            if !hint.is_empty() {
                                span { class: "picker-item-hint", "{hint}" }
                            }
                        }
                    }
                }
            })
            .collect()
    };

    let slash_filtered: Vec<SlashItem> = {
        let d = draft.read().clone();
        let full = slash_items.read().clone();
        match slash_query(&d) {
            Some(q) => filter_slash(&full, &q),
            None => full,
        }
    };
    let slash_item_els: Vec<Element> = slash_filtered
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let mut draft = draft.clone();
            let mut slash_open = slash_open.clone();
            let item = it.clone();
            let cur = *slash_idx.read();
            let item_cls = if i == cur {
                "input-picker-item active"
            } else {
                "input-picker-item"
            };
            let has_desc = !item.description.is_empty();
            rsx! {
                button {
                    class: "{item_cls}",
                    onclick: move |_| {
                        let d = draft.read().clone();
                        let new_draft = insert_slash_item(&d, &item);
                        draft.set(new_draft);
                        slash_open.set(false);
                    },
                    span { class: "picker-item-icon", "{item.kind}" }
                    span { class: "picker-item-body",
                        span { class: "picker-item-kind", "{item.label}" }
                        if has_desc {
                            span { class: "picker-item-desc", "{item.description}" }
                        }
                    }
                }
            }
        })
        .collect();

    let at_open_now = *at_open.read() && !at_item_els.is_empty();
    let slash_open_now = *slash_open.read() && !slash_item_els.is_empty();
    let at_len = at_item_els.len();
    let slash_len = slash_item_els.len();

    let send_btn: Element = if busy {
        rsx! {
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
                svg {
                    xmlns: "http://www.w3.org/2000/svg",
                    width: "12", height: "12",
                    view_box: "0 0 24 24",
                    fill: "currentColor",
                    rect { x: "4", y: "4", width: "16", height: "16", rx: "2" }
                }
            }
        }
    } else {
        let can_send = !draft().trim().is_empty();
        let send_cls = if can_send {
            "ic-send-btn enabled"
        } else {
            "ic-send-btn"
        };
        rsx! {
            button {
                class: "{send_cls}",
                disabled: !can_send,
                title: "{tr(&lang, \"sendEnter\")}",
                onclick: {
                    let state = state.clone();
                    let mut draft = draft.clone();
                    move |_| {
                        let state = state.clone();
                        let text = draft.read().clone();
                        draft.set(String::new());
                        async move {
                            if !text.trim().is_empty() {
                                submit(state, text).await;
                            }
                        }
                    }
                },
                svg {
                    xmlns: "http://www.w3.org/2000/svg",
                    width: "14", height: "14",
                    view_box: "0 0 24 24",
                    fill: "currentColor",
                    style: "transform:rotate(-90deg);",
                    path { d: "M12 18v4h4v-4h-4ZM16 14v4h4v-4h-4ZM20 10v4h4v-4h-4ZM16 6v4h4V6h-4ZM12 2v4h4V2h-4ZM12 10v4h4v-4h-4ZM8 10v4h4v-4H8ZM4 10v4h4v-4H4ZM0 10v4h4v-4H0Z" }
                }
            }
        }
    };

    rsx! {
        div { class: "input-card-wrap",
            div { class: "input-card",
                div { class: "input-editor-wrap",
                    textarea {
                        class: "input-card-editor",
                        rows: "1",
                        placeholder: "{tr(&lang, \"typeMessage\")}",
                        value: "{draft()}",
                        oninput: move |e| {
                            let v = e.value();
                            draft.set(v.clone());
                            if let Some(q) = at_query(&v) {
                                at_open.set(true);
                                slash_open.set(false);
                                attach_open.set(false);
                                let state = state.clone();
                                let mut at_idx = at_idx.clone();
                                let mut at_items = at_items.clone();
                                spawn(async move {
                                    let res = crate::fsgit::api_search_files(token(&state), q).await;
                                    match res {
                                        Ok(items) => {
                                            at_items.set(items);
                                            at_idx.set(0);
                                        }
                                        Err(_) => {
                                            at_items.set(Vec::new());
                                        }
                                    }
                                });
                            } else if let Some(_q) = slash_query(&v) {
                                slash_open.set(true);
                                at_open.set(false);
                                attach_open.set(false);
                                let state = state.clone();
                                let mut slash_idx = slash_idx.clone();
                                let mut slash_items = slash_items.clone();
                                let mut slash_loaded = slash_loaded.clone();
                                spawn(async move {
                                    if !*slash_loaded.read() {
                                        slash_loaded.set(true);
                                        let sk = crate::vibeconfig::api_list_vibe_files(
                                            token(&state),
                                            "global".to_string(),
                                            "skills".to_string(),
                                        )
                                        .await;
                                        let pr = crate::vibeconfig::api_list_vibe_files(
                                            token(&state),
                                            "global".to_string(),
                                            "prompts".to_string(),
                                        )
                                        .await;
                                        let mut items: Vec<SlashItem> = Vec::new();
                                        for c in state.read().snapshot.commands.clone() {
                                            items.push(SlashItem {
                                                id: format!("cmd:{}", c.name),
                                                label: c.name.clone(),
                                                kind: "command",
                                                content: format!("/{}", c.name),
                                                description: c.description.clone(),
                                                hint: c.hint.clone(),
                                            });
                                        }
                                        if let Ok(files) = sk {
                                            for f in files {
                                                items.push(SlashItem {
                                                    id: format!("skill:{}", f.display_name),
                                                    label: f.display_name.clone(),
                                                    kind: "skill",
                                                    content: f.content.clone(),
                                                    description: String::new(),
                                                    hint: String::new(),
                                                });
                                            }
                                        }
                                        if let Ok(files) = pr {
                                            for f in files {
                                                items.push(SlashItem {
                                                    id: format!("prompt:{}", f.display_name),
                                                    label: f.display_name.clone(),
                                                    kind: "prompt",
                                                    content: f.content.clone(),
                                                    description: String::new(),
                                                    hint: String::new(),
                                                });
                                            }
                                        }
                                        slash_items.set(items);
                                        slash_idx.set(0);
                                    } else {
                                        slash_idx.set(0);
                                    }
                                });
                            } else {
                                at_open.set(false);
                                slash_open.set(false);
                                attach_open.set(false);
                            }
                        },
                        onkeydown: move |e: Event<KeyboardData>| {
                            let key = e.key();
                            if at_open_now && at_len > 0 {
                                match key {
                                    Key::ArrowDown => {
                                        e.prevent_default();
                                        let cur = *at_idx.read();
                                        at_idx.set((cur + 1) % at_len);
                                        return;
                                    }
                                    Key::ArrowUp => {
                                        e.prevent_default();
                                        let cur = *at_idx.read();
                                        at_idx.set(if cur == 0 { at_len - 1 } else { cur - 1 });
                                        return;
                                    }
                                    Key::Enter => {
                                        e.prevent_default();
                                        let items = at_items.read().clone();
                                        let cur = *at_idx.read();
                                        if let Some(it) = items.get(cur) {
                                            let d = draft.read().clone();
                                            let new_draft = insert_at_marker(&d, &it.path, it.is_dir);
                                            draft.set(new_draft);
                                        }
                                        at_open.set(false);
                                        return;
                                    }
                                    Key::Escape => {
                                        at_open.set(false);
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            if slash_open_now && slash_len > 0 {
                                match key {
                                    Key::ArrowDown => {
                                        e.prevent_default();
                                        let cur = *slash_idx.read();
                                        slash_idx.set((cur + 1) % slash_len);
                                        return;
                                    }
                                    Key::ArrowUp => {
                                        e.prevent_default();
                                        let cur = *slash_idx.read();
                                        slash_idx.set(if cur == 0 { slash_len - 1 } else { cur - 1 });
                                        return;
                                    }
                                    Key::Enter => {
                                        e.prevent_default();
                                        let d = draft.read().clone();
                                        let q = slash_query(&d);
                                        let full = slash_items.read().clone();
                                        let filtered = match &q {
                                            Some(qq) => filter_slash(&full, qq),
                                            None => full,
                                        };
                                        let cur = *slash_idx.read();
                                        if let Some(it) = filtered.get(cur) {
                                            let new_draft = insert_slash_item(&d, it);
                                            draft.set(new_draft);
                                        }
                                        slash_open.set(false);
                                        return;
                                    }
                                    Key::Escape => {
                                        slash_open.set(false);
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            if key == Key::Enter && !e.modifiers().shift() {
                                e.prevent_default();
                                let text = draft.read().clone();
                                draft.set(String::new());
                                if !text.trim().is_empty() {
                                    spawn(send_text(text));
                                }
                            }
                        },
                    }
                    // --- @ file picker popup (opens upward, Nexus markup) ---
                    if at_open_now {
                        div { class: "input-picker-dropdown",
                            {at_item_els.iter().cloned()}
                        }
                    }
                    // --- / slash picker popup ---
                    if slash_open_now {
                        div { class: "input-picker-dropdown",
                            {slash_item_els.iter().cloned()}
                        }
                    }
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
                        // --- attach menu (Nexus 📎) ---
                        div { class: "attach-menu-wrap",
                            button {
                                class: "ic-folder-chip",
                                title: "{tr(&lang, \"attachments\")}",
                                onclick: {
                                    let mut attach_open = attach_open.clone();
                                    move |_| {
                                        let cur = *attach_open.read();
                                        attach_open.set(!cur);
                                    }
                                },
                                "📎"
                            }
                            if *attach_open.read() {
                                div { class: "folder-picker-dropdown attach-dropdown",
                                    button {
                                        class: "folder-picker-item",
                                        onclick: {
                                            let mut draft = draft.clone();
                                            let mut attach_open = attach_open.clone();
                                            let mut at_open = at_open.clone();
                                            move |_| {
                                                attach_open.set(false);
                                                let d = format!("{}@", draft.read().clone());
                                                draft.set(d);
                                                at_open.set(true);
                                            }
                                        },
                                        "📄 {loc(&lang, \"Plik\", \"File\")}"
                                    }
                                    button {
                                        class: "folder-picker-item",
                                        onclick: {
                                            let mut draft = draft.clone();
                                            let mut attach_open = attach_open.clone();
                                            let mut at_open = at_open.clone();
                                            move |_| {
                                                attach_open.set(false);
                                                let d = format!("{}@", draft.read().clone());
                                                draft.set(d);
                                                at_open.set(true);
                                            }
                                        },
                                        "📁 {loc(&lang, \"Katalog\", \"Directory\")}"
                                    }
                                    button {
                                        class: "folder-picker-item",
                                        onclick: {
                                            let state = state.clone();
                                            let lang_c = lang.clone();
                                            let draft = draft.clone();
                                            let mut attach_open = attach_open.clone();
                                            move |_| {
                                                let state = state.clone();
                                                let lang = lang_c.clone();
                                                let mut draft = draft.clone();
                                                async move {
                                                    attach_open.set(false);
                                                    match crate::fsgit::api_git_diff_all(token(&state)).await {
                                                        Ok(diff) if !diff.trim().is_empty() => {
                                                            let capped = if diff.len() > 100_000 {
                                                                format!("{}\n… (truncated)", &diff[..100_000])
                                                            } else {
                                                                diff
                                                            };
                                                            let block = format!("```diff\n{}\n```\n", capped);
                                                            let d = format!("{}{}", draft.read().clone(), block);
                                                            draft.set(d);
                                                        }
                                                        Ok(_) => {
                                                            let hint = loc(&lang, "Brak zmian do wklejenia.", "No changes to paste.");
                                                            let d = format!("{}{} ", draft.read().clone(), hint);
                                                            draft.set(d);
                                                        }
                                                        Err(_) => {}
                                                    }
                                                }
                                            }
                                        },
                                        "🔀 {loc(&lang, \"Diff gałęzi\", \"Branch diff\")}"
                                    }
                                }
                            }
                        }
                    }
                    div { class: "input-card-right",
                        { config_chip(state.clone(), lang.clone(), "model", tr(&lang, "modelPicker").to_string(), open_chip.clone()) }
                        { config_chip(state.clone(), lang.clone(), "mode", tr(&lang, "modePicker").to_string(), open_chip.clone()) }
                        { config_chip(state.clone(), lang.clone(), "thinking", tr(&lang, "thinkingPicker").to_string(), open_chip.clone()) }
                        {send_btn}
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

    // Only the model chip locks while busy — mode/thinking can be switched
    // mid-session, so their dropdowns must stay clickable (bug: lists did not
    // open after the first message).
    let disabled = option.is_none() || (id == "model" && busy);
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
