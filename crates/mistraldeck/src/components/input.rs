//! Input bar: textarea + project chip + model/mode/thinking chips with
//! dropdowns + send/stop, the live usage bar and the status bar.
//! Also the Nexus `@` file picker, `/` slash picker (commands + skills +
//! prompts) and the attach menu (＋ → file or image only).
//! Ported from Nexus InputBar.tsx (same CSS classes).

use dioxus::prelude::*;
use mistraldeck_core::proto::ConfigOption;

use crate::i18n::{tf, tr};
use crate::state::{lang, submit, token, ClientState};

/// The CLI's built-in slash commands (vibe/acp/commands/registry.py
/// `_build_commands`, advertised via available_commands_update). Round 4
/// (E11): offered even before any agent slot is live, so `/` always shows a
/// real list — merged with whatever the running session reports.
#[cfg(any(feature = "web", feature = "desktop", feature = "server"))]
const BUILTIN_SLASH_COMMANDS: &[(&str, &str, &str)] = &[
    ("help", "Show available commands and keyboard shortcuts", ""),
    ("compact", "Compact conversation history by summarizing", "Optional instructions to guide the compaction summary"),
    ("reload", "Reload configuration, agent instructions, and skills from disk", ""),
    ("log", "Show path to current session log directory", ""),
    ("mcp", "Show MCP OAuth status, login guidance, or log out an OAuth MCP server", "status | login <alias> | logout <alias>"),
    ("teleport", "Teleport session to Vibe Code Web", ""),
    ("proxy-setup", "Configure proxy and SSL certificate settings", "KEY value to set, KEY to unset, or empty for help"),
    ("retry", "Continue an interrupted model response", "Optional additional instructions for the continuation"),
    ("leanstall", "Install the Lean 4 agent (leanstral)", ""),
    ("unleanstall", "Uninstall the Lean 4 agent", ""),
    ("data-retention", "Show data retention information", ""),
    // executed client-side by the GUI (like the cli-rust TUI), not sent to
    // the model — round 9
    ("clear", "Clear the conversation context", ""),
    ("new", "Start a new conversation (clears context)", ""),
    ("rename", "Rename this session", "new title"),
    ("model", "Switch the model for this session", "model name"),
    ("thinking", "Set the thinking level", "off | low | medium | high | xhigh"),
    ("agent", "Switch the agent (mode) for this session", "agent name"),
    ("retry", "Continue the last interrupted response", "optional extra instructions"),
    ("status", "Show session statistics", ""),
    ("whoami", "Show the logged-in account", ""),
    ("plugins", "List loaded plugins", ""),
    ("rewind", "Rewind the session to a checkpoint", "pick a checkpoint"),
    ("branch", "Fork the session at a checkpoint into a new session", "optional checkpoint"),
    ("fork", "Alias of /branch", ""),
    ("loop", "Schedule a recurring prompt", "schedule and prompt"),
    ("delete", "Delete the current session", ""),
    ("diff", "Open the Git diff view", ""),
    ("theme", "Cycle the theme", "light | dark | auto"),
    ("config", "Open Settings", ""),
    ("permissions", "Open tool permissions in Settings", ""),
    ("connect", "Open SSH connections in Settings", ""),
    ("agents", "Open agents in Settings", ""),
    ("resume", "Resume a session from the sidebar", ""),
    ("version", "Show app and CLI versions", ""),
    ("update", "Check for updates", ""),
    ("quit", "Close the window", ""),
    ("exit", "Close the window", ""),
];

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

/// Re-fit the chat textarea after a programmatic draft change (the input
/// event only fires for real typing).
pub fn fit_input_now() {
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        spawn(async move {
            let _ = dioxus::document::eval("window.__vdFitInput && window.__vdFitInput();").await;
        });
    }
}

/// Window-level drag & drop (stable v1.0.8 parity): files dropped over the
/// chat are read as base64, parked on `window.__vdDrop`, and the hidden
/// `#vd-drop-bridge` button is clicked — the Rust handler uploads them into
/// the session root (`.halny-attachments/`, local or over SSH) and inserts
/// `@path` markers into the draft. Drops over the sidebar/dock/modals are
/// ignored.
#[cfg(any(feature = "web", feature = "desktop"))]
const DROP_JS: &str = r#"
(function () {
  if (window.__vdDropInit) return; window.__vdDropInit = 1;
  function hasFiles(e) {
    try { return Array.from(e.dataTransfer.types || []).includes('Files'); } catch (err) { return false; }
  }
  function excluded(e) {
    var t = e.target;
    return !!(t && t.closest && t.closest('.sidebar, .dock, .modal-overlay'));
  }
  document.addEventListener('dragover', function (e) {
    if (!hasFiles(e) || excluded(e)) return;
    e.preventDefault();
  });
  document.addEventListener('drop', function (e) {
    if (!hasFiles(e) || excluded(e)) return;
    e.preventDefault();
    var files = Array.from(e.dataTransfer.files || []);
    if (!files.length) return;
    window.__vdDrop = [];
    var pending = files.length;
    files.forEach(function (f) {
      var reader = new FileReader();
      reader.onload = function () {
        var url = String(reader.result || '');
        window.__vdDrop.push({ name: f.name, b64: url.split(',')[1] || '' });
        if (--pending === 0) {
          var btn = document.getElementById('vd-drop-bridge');
          if (btn) btn.click();
        }
      };
      reader.onerror = function () { if (--pending === 0) { var b = document.getElementById('vd-drop-bridge'); if (b) b.click(); } };
      reader.readAsDataURL(f);
    });
  });
})();
"#;

/// Clipboard paste (TUI paste_image parity): an image pasted into the chat
/// is read as base64, parked on `window.__vdPaste`, and the hidden
/// `#vd-paste-bridge` button is clicked — the Rust handler uploads it into
/// the session root (`.halny-attachments/`, local or over SSH) and inserts
/// an `@path` marker, exactly like a dropped file. Non-image pastes pass
/// through untouched.
#[cfg(any(feature = "web", feature = "desktop"))]
const PASTE_JS: &str = r#"
(function () {
  if (window.__vdPasteInit) return; window.__vdPasteInit = 1;
  document.addEventListener('paste', function (e) {
    try {
      var items = e.clipboardData && e.clipboardData.items;
      if (!items) return;
      var imgs = Array.from(items).filter(function (it) {
        return it.kind === 'file' && it.type && it.type.startsWith('image/');
      });
      if (!imgs.length) return;
      var t = e.target;
      if (!(t && t.closest && t.closest('.input-card-editor'))) return;
      e.preventDefault();
      window.__vdPaste = [];
      var pending = imgs.length;
      imgs.forEach(function (it) {
        var f = it.getAsFile();
        if (!f) { if (--pending === 0) { var b = document.getElementById('vd-paste-bridge'); if (b) b.click(); } return; }
        var reader = new FileReader();
        reader.onload = function () {
          var url = String(reader.result || '');
          var ext = (f.type.split('/')[1] || 'png').replace(/[^a-z0-9]/g, '');
          window.__vdPaste.push({ name: f.name || ('paste-' + Date.now() + '.' + ext), b64: url.split(',')[1] || '' });
          if (--pending === 0) { var b = document.getElementById('vd-paste-bridge'); if (b) b.click(); }
        };
        reader.onerror = function () { if (--pending === 0) { var b = document.getElementById('vd-paste-bridge'); if (b) b.click(); } };
        reader.readAsDataURL(f);
      });
    } catch (err) {}
  });
})();
"#;

/// Injected once (client only): the chat textarea grows with its content
/// (one line at a time) up to the CSS max-height (160px), and never offers
/// a manual resize handle (`resize: none` in main.css). Also installs the
/// click-outside handler that closes every open picker (model/mode/
/// thinking chips, the attach menu, the @ and / pickers) — it clicks the
/// hidden `#vd-close-pickers` bridge button.
#[cfg(any(feature = "web", feature = "desktop"))]
const INPUT_JS: &str = r#"
(function () {
  if (window.__vdInputInit) return; window.__vdInputInit = 1;
  var MAXH = 160;
  window.__vdFitInput = function () {
    var ta = document.querySelector('.input-card-editor');
    if (!ta) return;
    ta.style.height = 'auto';
    ta.style.height = Math.min(ta.scrollHeight, MAXH) + 'px';
  };
  document.addEventListener('input', function (e) {
    if (e.target && e.target.classList && e.target.classList.contains('input-card-editor')) {
      window.__vdFitInput();
    }
  }, true);
  // Clicking outside any picker closes all of them.
  document.addEventListener('pointerdown', function (e) {
    var t = e.target;
    if (!t || !t.closest) return;
    if (t.closest('.ic-chip-wrap, .attach-menu-wrap, .input-card-textarea-wrap, .input-picker-dropdown')) return;
    var btn = document.getElementById('vd-close-pickers');
    if (btn) btn.click();
  }, true);
})();
"#;

/// i18n key for the wire status string (Nexus InputBar status bar:
/// connected / connecting / error / disconnected).
fn status_label_key(status: &str) -> &'static str {
    match status {
        "connected" => "connected",
        "connecting" => "connecting",
        "error" => "error",
        _ => "disconnected",
    }
}

pub fn InputBar() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut draft = use_signal(String::new);
    let open_chip = use_signal(String::new);

    // one-shot: install the textarea auto-fit + click-outside-to-close JS
    // (client only; the SSR build never evaluates JS)
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let mut js_init = use_signal(|| false);
        if !*js_init.read() {
            js_init.set(true);
            spawn(async move {
                dioxus::document::eval(INPUT_JS);
                dioxus::document::eval(DROP_JS);
                dioxus::document::eval(PASTE_JS);
            });
        }
    }

    // Project agents (Nexus extraModeChoices) — loaded per working
    // directory of the current session (`<cwd>/.vibe/agents/`).
    let project_agents: Signal<Vec<crate::api::AgentChoiceView>> = use_signal(Vec::new);
    let project_agents_cwd = use_signal(String::new);

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

    // queued-message panel edit state (Nexus v0.10.5 MessageQueue)
    let editing_idx = use_signal(|| Option::<usize>::None);
    let edit_draft = use_signal(String::new);

    let s = state.read();
    let lang = lang(&state);
    let busy = s.snapshot.busy;
    // Stop feedback (Nexus v0.10.5): session/cancel in flight — the
    // stop button turns into a disabled spinner.
    let stopping = s
        .snapshot
        .current
        .and_then(|i| s.snapshot.sessions.get(i))
        .map(|m| m.stopping)
        .unwrap_or(false);
    let wd = s.snapshot.working_directory.clone();
    let cfg_wd = s.snapshot.config.working_directory.clone();
    let status = s.snapshot.status.clone();
    drop(s);

    let cwd = if wd.is_empty() { cfg_wd } else { wd };
    let project_label = short_dir(&cwd);

    // Working directory of the current session — where project agents
    // live (Nexus loads them on `projectCwd` change).
    let session_cwd = {
        let s = state.read();
        s.snapshot
            .current
            .and_then(|i| s.snapshot.sessions.get(i))
            .map(|m| m.working_directory.clone())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| cwd.clone())
    };

    // Reload the project agents whenever the session working directory
    // changes (Nexus useEffect on projectCwd → vibe:get-agents("project")).
    {
        let mut project_agents = project_agents.clone();
        let mut project_agents_cwd = project_agents_cwd.clone();
        let state = state.clone();
        use_effect(move || {
            // Reads happen synchronously so the effect subscribes to the
            // snapshot and re-runs on every poll update.
            let cwd = {
                let s = state.read();
                s.snapshot
                    .current
                    .and_then(|i| s.snapshot.sessions.get(i))
                    .map(|m| m.working_directory.clone())
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| s.snapshot.working_directory.clone())
            };
            if cwd.is_empty() || cwd == project_agents_cwd.peek().clone() {
                return;
            }
            project_agents_cwd.set(cwd.clone());
            project_agents.set(Vec::new());
            spawn(async move {
                match crate::api::api_project_agents(token(&state), cwd).await {
                    Ok(agents) => project_agents.set(agents),
                    Err(_) => {}
                }
            });
        });
    }

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
                        // Round 4 (E10): the file name is the left-aligned
                        // label (stable renders icon + path); only the path
                        // hint rides on the right edge. `.picker-item-kind`
                        // has margin-left:auto and pushed the NAME to the
                        // right.
                        span { class: "picker-item-body",
                            span { class: "picker-item-label", "{name}" }
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
        // Nexus v0.10.5: while session/cancel is in flight the button
        // is disabled, shows a spinner and the stopping title.
        let stop_cls = if stopping {
            "ic-stop-btn stopping"
        } else {
            "ic-stop-btn"
        };
        let stop_title = tr(&lang, if stopping { "stoppingLabel" } else { "stopLabel" }).to_string();
        let stop_icon: Element = if stopping {
            rsx! { span { class: "ic-stop-spinner" } }
        } else {
            rsx! {
                svg {
                    xmlns: "http://www.w3.org/2000/svg",
                    width: "12", height: "12",
                    view_box: "0 0 24 24",
                    fill: "currentColor",
                    rect { x: "4", y: "4", width: "16", height: "16", rx: "2" }
                }
            }
        };
        rsx! {
            button {
                class: "{stop_cls}",
                title: "{stop_title}",
                disabled: stopping,
                onclick: {
                    let state = state.clone();
                    move |_| {
                        let state = state.clone();
                        async move {
                            let _ = crate::api::api_cancel(token(&state)).await;
                        }
                    }
                },
                {stop_icon}
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
                        fit_input_now();
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

    // ----- v0.10.5 Nexus parity: queued-message panel (card top) -----
    let queue: Vec<String> = state.read().queue.clone();
    let queue_panel: Option<Element> = if queue.is_empty() {
        None
    } else {
        let title = if queue.len() == 1 {
            tr(&lang, "messageInQueueOne").to_string()
        } else {
            tf(&lang, "messagesInQueue", &[("count", &queue.len().to_string())])
        };
        let clear_label = tr(&lang, "clearQueueLabel").to_string();
        // Rows are precomputed: the rsx! macro forbids statements in children.
        let rows: Vec<Element> = queue
            .iter()
            .enumerate()
            .map(|(idx, msg)| {
                let editing = editing_idx() == Some(idx);
                let msg = msg.clone();
                rsx! {
                    div { class: "msg-queue-item",
                        span { class: "msg-queue-num", "{idx + 1}" }
                        if editing {
                            input {
                                class: "msg-queue-edit",
                                r#type: "text",
                                value: "{edit_draft.read()}",
                                autofocus: true,
                                oninput: {
                                    let mut edit_draft = edit_draft;
                                    move |e| edit_draft.set(e.value())
                                },
                                onkeydown: {
                                    let state = state.clone();
                                    let mut editing_idx = editing_idx;
                                    let mut edit_draft = edit_draft;
                                    move |e: Event<KeyboardData>| {
                                        match e.key() {
                                            Key::Enter => {
                                                let text = edit_draft.read().trim().to_string();
                                                let mut st = state;
                                                if text.is_empty() {
                                                    if Some(idx) == editing_idx() {
                                                        editing_idx.set(None);
                                                    }
                                                    let mut s = st.write();
                                                    if idx < s.queue.len() {
                                                        s.queue.remove(idx);
                                                    }
                                                } else {
                                                    let mut s = st.write();
                                                    if idx < s.queue.len() {
                                                        s.queue[idx] = text;
                                                    }
                                                    editing_idx.set(None);
                                                }
                                                edit_draft.set(String::new());
                                            }
                                            Key::Escape => {
                                                editing_idx.set(None);
                                                edit_draft.set(String::new());
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        } else {
                            p { class: "msg-queue-text", "{msg}" }
                        }
                        div { class: "msg-queue-actions",
                            if editing {
                                button {
                                    class: "msg-queue-btn",
                                    title: "{tr(&lang, \"mqSave\")}",
                                    onclick: {
                                        let state = state.clone();
                                        let mut editing_idx = editing_idx;
                                        let mut edit_draft = edit_draft;
                                        move |_| {
                                            let text = edit_draft.read().trim().to_string();
                                            let mut st = state;
                                            if text.is_empty() {
                                                editing_idx.set(None);
                                                let mut s = st.write();
                                                if idx < s.queue.len() {
                                                    s.queue.remove(idx);
                                                }
                                            } else {
                                                let mut s = st.write();
                                                if idx < s.queue.len() {
                                                    s.queue[idx] = text;
                                                }
                                                editing_idx.set(None);
                                            }
                                            edit_draft.set(String::new());
                                        }
                                    },
                                    svg {
                                        xmlns: "http://www.w3.org/2000/svg",
                                        width: "14", height: "14",
                                        view_box: "0 0 24 24",
                                        stroke: "currentColor",
                                        stroke_width: "2",
                                        fill: "none",
                                        path { d: "M20 6L9 17L4 12" }
                                    }
                                }
                                button {
                                    class: "msg-queue-btn",
                                    title: "{tr(&lang, \"mqCancel\")}",
                                    onclick: {
                                        let mut editing_idx = editing_idx;
                                        let mut edit_draft = edit_draft;
                                        move |_| {
                                            editing_idx.set(None);
                                            edit_draft.set(String::new());
                                        }
                                    },
                                    svg {
                                        xmlns: "http://www.w3.org/2000/svg",
                                        width: "14", height: "14",
                                        view_box: "0 0 24 24",
                                        stroke: "currentColor",
                                        stroke_width: "2",
                                        fill: "none",
                                        path { d: "M20 4L4 20" }
                                        path { d: "M4 4L20 20" }
                                    }
                                }
                            } else {
                                // Nexus sendQueuedNow: idle → force-send this
                                // prompt immediately; busy → move it to the
                                // front of the queue and break the current
                                // turn (it is delivered by the next drain).
                                button {
                                    class: "msg-queue-btn",
                                    title: "{tr(&lang, \"mqSendNow\")}",
                                    onclick: {
                                        let state = state.clone();
                                        let editing_idx = editing_idx.clone();
                                        move |_| {
                                            let state = state.clone();
                                            let mut editing_idx = editing_idx.clone();
                                            let busy_now = state.read().snapshot.busy;
                                            // None → item vanished, do nothing;
                                            // Some(None) → busy: break the turn;
                                            // Some(Some(text)) → idle: force-send.
                                            let action: Option<Option<String>> = {
                                                let mut st = state.clone();
                                                let mut s = st.write();
                                                if idx >= s.queue.len() {
                                                    None
                                                } else if busy_now {
                                                    let msg = s.queue.remove(idx);
                                                    s.queue.insert(0, msg);
                                                    Some(None)
                                                } else {
                                                    Some(Some(s.queue.remove(idx)))
                                                }
                                            };
                                            editing_idx.set(None);
                                            async move {
                                                match action {
                                                    Some(Some(text)) => {
                                                        let tok = crate::state::token(&state);
                                                        let _ = crate::api::api_send_message(tok, text).await;
                                                    }
                                                    Some(None) => {
                                                        let tok = crate::state::token(&state);
                                                        let _ = crate::api::api_cancel(tok).await;
                                                    }
                                                    None => {}
                                                }
                                            }
                                        }
                                    },
                                    svg {
                                        xmlns: "http://www.w3.org/2000/svg",
                                        width: "14", height: "14",
                                        view_box: "0 0 24 24",
                                        stroke: "currentColor",
                                        stroke_width: "2",
                                        fill: "none",
                                        path { d: "M13 2L4.5 13.5H11L9.5 22L19.5 9.5H13.5L13 2Z" }
                                    }
                                }
                                button {
                                    class: "msg-queue-btn",
                                    title: "{tr(&lang, \"mqEdit\")}",
                                    onclick: {
                                        let mut editing_idx = editing_idx;
                                        let mut edit_draft = edit_draft;
                                        move |_| {
                                            editing_idx.set(Some(idx));
                                            edit_draft.set(msg.clone());
                                        }
                                    },
                                    svg {
                                        xmlns: "http://www.w3.org/2000/svg",
                                        width: "14", height: "14",
                                        view_box: "0 0 24 24",
                                        stroke: "currentColor",
                                        stroke_width: "2",
                                        fill: "none",
                                        path { d: "M8 21L21.5 7.5L16.5 2.5L3 16L3 21L8 21Z" }
                                    }
                                }
                                button {
                                    class: "msg-queue-btn",
                                    title: "{tr(&lang, \"mqRemove\")}",
                                    onclick: {
                                        let state = state.clone();
                                        let mut editing_idx = editing_idx;
                                        move |_| {
                                            if Some(idx) == editing_idx() {
                                                editing_idx.set(None);
                                            }
                                            let mut st = state;
                                            let mut s = st.write();
                                            if idx < s.queue.len() {
                                                s.queue.remove(idx);
                                            }
                                        }
                                    },
                                    svg {
                                        xmlns: "http://www.w3.org/2000/svg",
                                        width: "14", height: "14",
                                        view_box: "0 0 24 24",
                                        stroke: "currentColor",
                                        stroke_width: "2",
                                        fill: "none",
                                        path { d: "M20 4L4 20" }
                                        path { d: "M4 4L20 20" }
                                    }
                                }
                            }
                        }
                    }
                }
            })
            .collect();
        Some(rsx! {
            div { class: "msg-queue",
                div { class: "msg-queue-header",
                    p { class: "msg-queue-title", "{title}" }
                    button {
                        class: "msg-queue-clear",
                        onclick: {
                            let state = state.clone();
                            let mut editing_idx = editing_idx;
                            move |_| {
                                editing_idx.set(None);
                                let mut st = state;
                                st.write().queue.clear();
                            }
                        },
                        "{clear_label}"
                    }
                }
                div { class: "msg-queue-list",
                    {rows.iter().cloned()}
                }
            }
        })
    };

    rsx! {
        div { class: "input-card-wrap",
            // hidden bridge: INPUT_JS clicks it on any pointerdown outside
            // the pickers, closing all dropdowns (chips / attach / @ / /)
            // hidden bridge: DROP_JS parks dropped files on window.__vdDrop
            // and clicks this — the handler uploads them and inserts @path
            button {
                id: "vd-drop-bridge",
                style: "display:none",
                onclick: {
                    let state = state.clone();
                    let mut draft = draft.clone();
                    move |_| {
                        let state = state.clone();
                        let mut draft = draft.clone();
                        spawn(async move {
                            #[cfg(any(feature = "web", feature = "desktop"))]
                            {
                                let files: Vec<serde_json::Value> = dioxus::document::eval("return window.__vdDrop || [];")
                                    .join::<serde_json::Value>()
                                    .await
                                    .ok()
                                    .and_then(|v| v.as_array().cloned())
                                    .unwrap_or_default();
                                let _ = dioxus::document::eval("window.__vdDrop = [];").await;
                                let (root, rid) = crate::state::session_git_ctx(&state);
                                let tok = token(&state);
                                for f in files {
                                    let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("file").to_string();
                                    let b64 = f.get("b64").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    if b64.is_empty() { continue; }
                                    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
                                    let target = format!(".halny-attachments/{}-{}", mistraldeck_core::types::now_ms(), safe);
                                    if crate::fsgit::api_write_binary(tok.clone(), target.clone(), b64, rid.clone(), root.clone()).await.is_ok() {
                                        let d = format!("{}@{}", draft.read(), target);
                                        draft.set(d);
                                        fit_input_now();
                                    }
                                }
                            }
                        });
                    }
                },
            }
            // hidden bridge: PASTE_JS parks pasted images on window.__vdPaste
            // and clicks this — the handler uploads them and inserts @path
            // (same flow as the drop bridge above)
            button {
                id: "vd-paste-bridge",
                style: "display:none",
                onclick: {
                    let state = state.clone();
                    let mut draft = draft.clone();
                    move |_| {
                        let state = state.clone();
                        let mut draft = draft.clone();
                        spawn(async move {
                            #[cfg(any(feature = "web", feature = "desktop"))]
                            {
                                let files: Vec<serde_json::Value> = dioxus::document::eval("return window.__vdPaste || [];")
                                    .join::<serde_json::Value>()
                                    .await
                                    .ok()
                                    .and_then(|v| v.as_array().cloned())
                                    .unwrap_or_default();
                                let _ = dioxus::document::eval("window.__vdPaste = [];").await;
                                let (root, rid) = crate::state::session_git_ctx(&state);
                                let tok = token(&state);
                                for f in files {
                                    let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("paste.png").to_string();
                                    let b64 = f.get("b64").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    if b64.is_empty() { continue; }
                                    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
                                    let target = format!(".halny-attachments/{}-{}", mistraldeck_core::types::now_ms(), safe);
                                    if crate::fsgit::api_write_binary(tok.clone(), target.clone(), b64, rid.clone(), root.clone()).await.is_ok() {
                                        let d = format!("{}@{}", draft.read(), target);
                                        draft.set(d);
                                        fit_input_now();
                                    }
                                }
                            }
                        });
                    }
                },
            }
            button {
                id: "vd-close-pickers",
                style: "display:none",
                onclick: {
                    let mut open_chip = open_chip.clone();
                    let mut attach_open = attach_open.clone();
                    let mut at_open = at_open.clone();
                    let mut slash_open = slash_open.clone();
                    move |_| {
                        open_chip.set(String::new());
                        attach_open.set(false);
                        at_open.set(false);
                        slash_open.set(false);
                    }
                },
            }
            div { class: "input-card",
                {queue_panel.iter().cloned()}
                div { class: "input-card-textarea-wrap",
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
                                    // remote sessions search the remote session
                                    // root over SSH (Nexus remoteSearchFiles)
                                    let (froot, frid) =
                                        crate::state::session_git_ctx(&state);
                                    let res = crate::fsgit::api_search_files(
                                        token(&state),
                                        q,
                                        frid,
                                        froot,
                                    )
                                    .await;
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
                                        // Round 4 (E11): the picker must
                                        // always offer the CLI's built-in
                                        // slash commands (vibe/acp/commands/
                                        // registry.py) — the agent only
                                        // reports them once a slot is live —
                                        // plus the session-scoped skills and
                                        // prompts (global + project for
                                        // local sessions, the remote host for
                                        // SSH sessions).
                                        let (scwd, conn) = {
                                            let s = state.read();
                                            let cur = s.snapshot.current.and_then(|i| s.snapshot.sessions.get(i));
                                            let rid = cur.and_then(|m| m.remote_id.clone());
                                            let conn = rid.and_then(|id| {
                                                s.snapshot
                                                    .config
                                                    .remote_connections
                                                    .iter()
                                                    .find(|c| c.id == id)
                                                    .cloned()
                                            });
                                            (
                                                cur.map(|m| m.working_directory.clone())
                                                    .filter(|d| !d.is_empty())
                                                    .unwrap_or_default(),
                                                conn,
                                            )
                                        };
                                        let mut items: Vec<SlashItem> = Vec::new();
                                        for c in BUILTIN_SLASH_COMMANDS {
                                            items.push(SlashItem {
                                                id: format!("cmd:{}", c.0),
                                                label: c.0.to_string(),
                                                kind: "command",
                                                content: format!("/{}", c.0),
                                                description: c.1.to_string(),
                                                hint: c.2.to_string(),
                                            });
                                        }
                                        for c in state.read().snapshot.commands.clone() {
                                            if !items.iter().any(|i| i.label == c.name) {
                                                items.push(SlashItem {
                                                    id: format!("cmd:{}", c.name),
                                                    label: c.name.clone(),
                                                    kind: "command",
                                                    content: format!("/{}", c.name),
                                                    description: c.description.clone(),
                                                    hint: c.hint.clone(),
                                                });
                                            }
                                        }
                                        // skills + prompts: global scope, plus
                                        // the project scope for local sessions
                                        let mut scopes: Vec<(String, String, Option<mistraldeck_core::snapshot::RemoteConnection>)> =
                                            vec![("global".to_string(), String::new(), conn.clone())];
                                        if conn.is_none() && !scwd.is_empty() {
                                            scopes.push(("project".to_string(), scwd.clone(), None));
                                        }
                                        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                                        for (scope, cwd, c) in &scopes {
                                            for kind in ["skills", "prompts"] {
                                                if let Ok(files) = crate::vibeconfig::api_list_vibe_files(
                                                    token(&state),
                                                    scope.clone(),
                                                    cwd.clone(),
                                                    c.clone(),
                                                    kind.to_string(),
                                                )
                                                .await
                                                {
                                                    for f in files {
                                                        let key = format!("{}:{}", kind, f.display_name);
                                                        if !seen.insert(key) {
                                                            continue;
                                                        }
                                                        items.push(SlashItem {
                                                            id: format!("{}:{}", kind, f.display_name),
                                                            label: f.display_name.clone(),
                                                            kind: if kind == "skills" { "skill" } else { "prompt" },
                                                            content: f.content.clone(),
                                                            description: String::new(),
                                                            hint: String::new(),
                                                        });
                                                    }
                                                }
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
                                fit_input_now();
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
                div { class: "input-card-toolbar",
                    div { class: "input-card-left",
                        // --- attach menu: one ＋ button, file or image only ---
                        div { class: "attach-menu-wrap",
                            button {
                                class: "ic-btn ic-attach-btn",
                                title: "{tr(&lang, \"attachments\")}",
                                onclick: {
                                    let mut attach_open = attach_open.clone();
                                    move |_| {
                                        let cur = *attach_open.read();
                                        attach_open.set(!cur);
                                    }
                                },
                                "＋"
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
                                                crate::components::input::fit_input_now();
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
                                                // image: same @ picker, the
                                                // server inlines image files as
                                                // base64 blocks (image_mime)
                                                let d = format!("{}@", draft.read().clone());
                                                draft.set(d);
                                                crate::components::input::fit_input_now();
                                                at_open.set(true);
                                            }
                                        },
                                        "🖼 {loc(&lang, \"Zdjęcie\", \"Image\")}"
                                    }
                                }
                            }
                        }
                    }
                    div { class: "input-card-right",
                        { config_chip(state.clone(), lang.clone(), "model", tr(&lang, "modelPicker").to_string(), open_chip.clone(), project_agents.clone(), session_cwd.clone()) }
                        { config_chip(state.clone(), lang.clone(), "mode", tr(&lang, "modePicker").to_string(), open_chip.clone(), project_agents.clone(), session_cwd.clone()) }
                        { config_chip(state.clone(), lang.clone(), "thinking", tr(&lang, "thinkingPicker").to_string(), open_chip.clone(), project_agents.clone(), session_cwd.clone()) }
                        {send_btn}
                    }
                }
            }
            div { class: "input-status-bar",
                div { class: "agent-status {status}",
                    span { class: "status-dot" }
                    span { class: "status-label", "{tr(&lang, status_label_key(&status))}" }
                }
                // v0.13.5 parity: the usage bar lives inline in the
                // status bar (single row: status · usage · cwd).
                crate::components::usagebar::UsageBar {}
                if !cwd.is_empty() {
                    span { class: "status-cwd", title: "{cwd}", "{project_label}" }
                }
            }
        }
    }
}

fn short_dir(dir: &str) -> String {
    let name = dir.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("");
    if name.is_empty() { dir.to_string() } else { name.to_string() }
}

/// One model/mode/thinking chip with a dropdown (Nexus ConfigChip).
fn config_chip(
    state: Signal<ClientState>,
    lang: String,
    id: &'static str,
    label: String,
    open: Signal<String>,
    project_agents: Signal<Vec<crate::api::AgentChoiceView>>,
    cwd: String,
) -> Element {
    let s = state.read();
    let busy = s.snapshot.busy;
    let option: Option<ConfigOption> = s.snapshot.config_options.iter().find(|o| o.id == id).cloned();
    drop(s);

    // Only the model chip locks while busy — mode/thinking can be switched
    // mid-session, so their dropdowns must stay clickable (bug: lists did not
    // open after the first message).
    let disabled = option.is_none() || (id == "model" && busy);

    // Choices, Nexus parity:
    // - model: dedupe by NAME, keep-first (v0.12 modelOption dedupe);
    // - mode: append project agents not already offered by the backend as
    //   extra choices — value=name, label=displayName, description =
    //   projectAgentBadge (+ " — " + agent description);
    // - others: raw choices.
    let mut items: Vec<(String, String, String)> = Vec::new();
    if let Some(o) = &option {
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in &o.choices {
            if id == "model" && !seen_names.insert(c.name.clone()) {
                continue;
            }
            items.push((c.value.clone(), c.name.clone(), c.description.clone()));
        }
    }
    // Localized mode/thinking names + descriptions (Nexus terms.ts):
    // CLI values like "accept-edits" or "medium" display in the UI
    // language; custom agent modes keep their own names/descriptions.
    if id == "mode" || id == "thinking" {
        for it in items.iter_mut() {
            if let Some(n) = crate::i18n::option_term(&lang, id, &it.0) {
                it.1 = n.to_string();
            }
            if let Some(d) = crate::i18n::option_term_desc(&lang, id, &it.0) {
                it.2 = d.to_string();
            }
        }
    }
    if id == "mode" {
        let badge = tr(&lang, "projectAgentBadge").to_string();
        let mut values: std::collections::HashSet<String> =
            items.iter().map(|(v, _, _)| v.clone()).collect();
        for a in project_agents.read().iter() {
            if a.name.is_empty() || !values.insert(a.name.clone()) {
                continue;
            }
            let name = a
                .display_name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| a.name.clone());
            let desc = match a.description.as_deref() {
                Some(d) if !d.is_empty() => format!("{badge} — {d}"),
                _ => badge.clone(),
            };
            items.push((a.name.clone(), name, desc));
        }
    }

    let is_open = open.read().clone() == id;
    let current_value = option
        .as_ref()
        .map(|o| o.current_value.clone())
        .unwrap_or_default();
    // Label resolves against the merged item list so a project-agent mode
    // shows its displayName, not the raw agent name.
    let current_label = items
        .iter()
        .find(|(v, _, _)| *v == current_value)
        .map(|(_, n, _)| n.clone())
        .unwrap_or_else(|| {
            option
                .as_ref()
                .map(|o| o.current_label())
                .unwrap_or_else(|| "—".to_string())
        });
    let cls = if disabled { "ic-chip ic-chip-disabled" } else { "ic-chip" };

    // Values the backend itself offered (project agents coming only from
    // the merge need to be exposed to ~/.vibe/agents before activation).
    let cli_values: std::collections::HashSet<String> = option
        .as_ref()
        .map(|o| o.choices.iter().map(|c| c.value.clone()).collect())
        .unwrap_or_default();
    let agent_names: std::collections::HashSet<String> = project_agents
        .read()
        .iter()
        .map(|a| a.name.clone())
        .collect();

    // Precompute the dropdown items (rsx children cannot contain raw
    // statements) — one element per choice.
    let item_els: Vec<Element> = items
        .into_iter()
        .map(|(value, name, desc)| {
            let state = state.clone();
            let open = open.clone();
            let cwd = cwd.clone();
            let needs_expose =
                id == "mode" && agent_names.contains(&value) && !cli_values.contains(&value);
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
                        let cwd = cwd.clone();
                        async move {
                            open.set(String::new());
                            // Nexus handleConfigSelect: a project agent that
                            // the backend did not offer must first be copied
                            // into ~/.vibe/agents (exposeProjectAgent); a
                            // "Unsupported config option" error for a freshly
                            // exposed agent is tolerated — the mode applies
                            // from the next session on.
                            if needs_expose {
                                let _ = crate::api::api_expose_project_agent(
                                    token(&state),
                                    cwd.clone(),
                                    value.clone(),
                                )
                                .await;
                            }
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
