//! Client-side reactive state: a local projection of the engine state,
//! kept in sync by the event stream (incremental for streaming chunks,
//! snapshot refetch for structural changes).

use dioxus::prelude::*;
use vibedeck_core::proto::AcpEvent;
use vibedeck_core::snapshot::{BusEvent, Snapshot};
use vibedeck_core::types::{Message, MessageSender};

/// Top-level application views (like Nexus AppView).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Chat,
    Stats,
    History,
    Settings,
}

#[derive(Clone, Default)]
pub struct ClientState {
    /// Vestigial auth token (kept for server-fn protocol compatibility;
    /// always None — see serve.rs for the auth model).
    pub token: Option<String>,
    pub snapshot: Snapshot,
    pub loaded: bool,
    pub view: View,
    /// Prompts typed while the agent is busy (sent on completion).
    pub queue: Vec<String>,
    /// vibe-acp binary missing → show the first-run SetupView.
    pub setup_needed: bool,
    /// User skipped the SetupView.
    pub setup_skipped: bool,
    /// One-shot guard for the api_setup_status bootstrap check.
    pub setup_checked: bool,
    /// Pending dock navigation requested by a clickable chat path
    /// (markdown path links / tool path chips). Consumed by the dock;
    /// `preview_open_seq` bumps on every request.
    pub preview_open: Option<PreviewOpen>,
    pub preview_open_seq: u64,
}

/// A pending dock navigation triggered by a clickable chat path
/// (Nexus `openFilePreview`): a directory revealed in the dock Files
/// tab (`reveal = true`) or a file opened in a preview tab.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewOpen {
    pub path: String,
    pub reveal: bool,
}

/// Refetch the whole snapshot (structural change or first load).
pub async fn refresh(mut state: Signal<ClientState>) {
    let token = state.read().token.clone().unwrap_or_default();
    match crate::api::api_snapshot(token).await {
        Ok(snap) => {
            apply_theme(&snap.config.theme);
            // Remember the UI language for the remote /login page (serve.rs)
            // — the login page renders before the app (and its config) is
            // available, so it reads this hint from localStorage.
            let cfg_lang = snap.config.language.clone();
            if !cfg_lang.is_empty() && cfg_lang != "auto" {
                dioxus::document::eval(&format!(
                    "try{{localStorage.setItem('vdLang','{cfg_lang}')}}catch(e){{}}"
                ));
            }
            let mut s = state.write();
            s.snapshot = snap;
            s.loaded = true;
        }
        Err(_e) => { /* snapshot fetch failed; next event will retry */ }
    }
    // One-shot first-run check: show the setup screen when the
    // vibe-acp binary is missing (any login state).
    if !state.read().setup_checked {
        state.write().setup_checked = true;
        if let Ok(st) = crate::setup::api_setup_status().await {
            state.write().setup_needed = !st.vib_acp_found;
        }
    }
}

/// Apply one bus event to the local projection. Returns true when a
/// refetch is needed (handled by the caller).
pub fn apply_event(mut state: Signal<ClientState>, ev: BusEvent) -> bool {
    match ev {
        BusEvent::UserMessageAdded {
            local_session_id,
            message,
        } => {
            let mut s = state.write();
            let cur_id = s
                .snapshot
                .current
                .and_then(|i| s.snapshot.sessions.get(i))
                .map(|m| m.id.clone());
            if cur_id.as_deref() == Some(local_session_id.as_str()) {
                s.snapshot.current_messages.push(message);
            }
            if let Some(cur) = s.snapshot.current {
                if let Some(meta) = s.snapshot.sessions.get_mut(cur) {
                    meta.message_count += 1;
                    meta.last_active_ms = vibedeck_core::types::now_ms();
                }
            }
            false
        }
        BusEvent::StructureChanged => true,
        BusEvent::Acp(e) => apply_acp_event(state, e),
    }
}

fn apply_acp_event(mut state: Signal<ClientState>, e: AcpEvent) -> bool {
    let current_acp_id = {
        let s = state.read();
        s.snapshot
            .current
            .and_then(|i| s.snapshot.sessions.get(i))
            .and_then(|m| m.acp_session_id.clone())
    };
    let is_current = |sid: &str| current_acp_id.as_deref() == Some(sid);
    // Live usage: update counters locally, no refetch - but only for the
    // session the user is looking at (parallel slots stream in parallel).
    if let AcpEvent::UsageUpdate { session_id, .. } = &e {
        if is_current(session_id) {
            let mut s = state.write();
            let _ = s.snapshot.usage.apply(&e);
        }
        return false;
    }
    match e {
        AcpEvent::MessageChunk { session_id, text } => {
            if is_current(&session_id) {
                let mut s = state.write();
                let msg = ensure_ai_message_pub(&mut s.snapshot.current_messages);
                msg.content.push_str(&text);
            }
            false
        }
        AcpEvent::ThinkingChunk { session_id, text } => {
            if is_current(&session_id) {
                let mut s = state.write();
                let msg = ensure_ai_message_pub(&mut s.snapshot.current_messages);
                match msg.thinking.last_mut() {
                    Some(block) if !block.is_complete => block.content.push_str(&text),
                    _ => msg.thinking.push(vibedeck_core::types::ThinkingBlock {
                        id: "thinking".into(),
                        content: text,
                        is_complete: false,
                        started_at: vibedeck_core::types::now_ms(),
                        completed_at: 0,
                    }),
                }
            }
            false
        }
        AcpEvent::ThinkingComplete { session_id } => {
            if is_current(&session_id) {
                let mut s = state.write();
                if let Some(msg) = s.snapshot.current_messages.last_mut() {
                    let now = vibedeck_core::types::now_ms();
                    for b in &mut msg.thinking {
                        b.is_complete = true;
                        if b.completed_at == 0 {
                            b.completed_at = now;
                        }
                    }
                }
            }
            false
        }
        AcpEvent::ToolCallStarted {
            session_id,
            tool_call,
        } => {
            if is_current(&session_id) {
                let mut s = state.write();
                ensure_ai_message_pub(&mut s.snapshot.current_messages)
                    .tool_calls
                    .push(tool_call);
            }
            false
        }
        AcpEvent::ToolCallUpdated {
            session_id,
            tool_call_id,
            status,
            output,
            subagent,
        } => {
            if is_current(&session_id) {
                let mut s = state.write();
                for msg in s.snapshot.current_messages.iter_mut().rev() {
                    for tc in &mut msg.tool_calls {
                        if tc.id == tool_call_id {
                            tc.status = status;
                            if let Some(output) = &output {
                                tc.output = Some(output.clone());
                            }
                            // v0.13: subagent meta merges (child_session_id
                            // arrives via a later tool_call_update).
                            if let Some(sa) = &subagent {
                                match &mut tc.subagent {
                                    Some(cur) => {
                                        if sa.agent.is_some() {
                                            cur.agent = sa.agent.clone();
                                        }
                                        if sa.agent_name.is_some() {
                                            cur.agent_name = sa.agent_name.clone();
                                        }
                                        if sa.task.is_some() {
                                            cur.task = sa.task.clone();
                                        }
                                        if sa.child_session_id.is_some() {
                                            cur.child_session_id =
                                                sa.child_session_id.clone();
                                        }
                                    }
                                    None => tc.subagent = Some(sa.clone()),
                                }
                            }
                        }
                    }
                }
            }
            false
        }
        // Agent plan / todo checklist: cumulative snapshot, applied in
        // place for the open session (no refetch flicker).
        AcpEvent::PlanUpdate {
            session_id,
            entries_json,
        } => {
            if is_current(&session_id) {
                let entries = vibedeck_core::types::PlanEntry::parse_list(&entries_json);
                state.write().snapshot.current_plan = entries;
            }
            false
        }
        // Everything else (SessionReady, SessionHistory, SessionsList,
        // errors, PromptCompleted...) is structural -> refetch snapshot.
        _ => true,
    }
}

/// Ensure the message list ends with an open AI message.
fn ensure_ai_message_pub(messages: &mut Vec<Message>) -> &mut Message {
    let needs_new = !matches!(
        messages.last(),
        Some(msg) if msg.sender == MessageSender::Ai && !msg.is_complete
    );
    if needs_new {
        messages.push(Message::new_ai());
    }
    messages.last_mut().expect("just pushed")
}

/// token() helper: the current auth token (always empty — the token is
/// vestigial; auth happens at the remote HTTPS listener, not in the API).
pub fn token(state: &Signal<ClientState>) -> String {
    state.read().token.clone().unwrap_or_default()
}

/// The OS/browser language ("pl" out of "pl-PL"), mapped to a supported
/// UI language code. Sync-safe: a tiny JS interop call on wasm only.
pub fn os_lang() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let raw = web_sys::window()
            .map(|w| w.navigator().language())
            .flatten()
            .unwrap_or_default();
        let l = raw.to_lowercase();
        let code: String = l.chars().take(2).collect();
        match code.as_str() {
            "pl" | "de" | "fr" | "es" | "it" | "en" => code,
            _ => "en".to_string(),
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        "en".to_string()
    }
}

/// True when the UI runs inside the desktop webview window (wry injects
/// `window.ipc`); in a plain browser (remote access) it is false.
pub fn is_desktop() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Reflect::has(&js_sys::global(), &wasm_bindgen::JsValue::from_str("ipc"))
            .unwrap_or(false)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// Resolve the UI language code for i18n ("auto"/"system" -> the OS or
/// browser language, like Nexus "Auto (System)").
pub fn lang(state: &Signal<ClientState>) -> String {
    let l = state.read().snapshot.config.language.clone();
    match l.as_str() {
        "" | "auto" | "system" => os_lang(),
        other => other.to_string(),
    }
}

/// Send a prompt now, or queue it while the agent is busy.
pub async fn submit(mut state: Signal<ClientState>, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    if state.read().snapshot.busy {
        state.write().queue.push(text);
        return;
    }
    let tok = token(&state);
    let _ = crate::api::api_send_message(tok, text).await;
}

/// After a turn finished (and the client refetched a non-busy snapshot),
/// send the next queued prompt, one at a time.
pub async fn drain_queue(mut state: Signal<ClientState>) {
    if state.read().snapshot.busy {
        return;
    }
    let next = {
        let mut s = state.write();
        if s.queue.is_empty() {
            None
        } else {
            Some(s.queue.remove(0))
        }
    };
    if let Some(text) = next {
        let tok = token(&state);
        let _ = crate::api::api_send_message(tok, text).await;
    }
}

/// Apply the current theme to <html data-theme=...> (auto = OS preference).
pub fn apply_theme(theme: &str) {
    let script = match theme {
        "dark" => "document.documentElement.dataset.theme='dark';".to_string(),
        "light" => "document.documentElement.dataset.theme='light';".to_string(),
        _ => "var t=document.documentElement;function ap(m){t.dataset.theme=m.matches?'dark':'light'};var mq=matchMedia('(prefers-color-scheme: dark)');ap(mq);mq.addEventListener('change',ap);".to_string(),
    };
    dioxus::document::eval(script.as_str());
}

/// Optimistically persist UI prefs (language / theme / dock / limit).
pub async fn save_prefs(
    mut state: Signal<ClientState>,
    language: String,
    theme: String,
    dock_open: bool,
    sidebar_limit: u32,
) {
    {
        let mut s = state.write();
        s.snapshot.config.language = language.clone();
        s.snapshot.config.theme = theme.clone();
        s.snapshot.config.dock_open = dock_open;
        s.snapshot.config.sidebar_sessions_limit = if sidebar_limit > 0 {
            Some(sidebar_limit)
        } else {
            None
        };
    }
    apply_theme(&theme);
    let tok = token(&state);
    let _ = crate::api::api_set_prefs(tok, language, theme, dock_open, sidebar_limit).await;
}

// ---------------------------------------------------------------------------
// Clickable chat paths (port of Nexus MessageBubble.tsx / store.ts)
// ---------------------------------------------------------------------------

fn is_abs_path(p: &str) -> bool {
    p.starts_with('/') || p.starts_with('\\') || {
        let mut cs = p.chars();
        cs.next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) && cs.next() == Some(':')
    }
}

/// Port of Nexus `looksLikeInlinePath`: heuristic deciding whether an
/// inline markdown code span should render as a clickable path link.
pub fn looks_like_inline_path(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.len() > 512 {
        return false;
    }
    if t.contains(' ') || t.contains('\t') || t.contains('\n') {
        return false;
    }
    let p = t.strip_prefix('@').unwrap_or(t);
    if p.contains("://") {
        return false;
    }
    let abs = is_abs_path(p);
    let rel = (p.contains('/') || p.contains('\\'))
        && p
            .chars()
            .next()
            .map(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .unwrap_or(false);
    abs || rel
}

/// Port of Nexus `resolveSessionPath`: strip a leading `@`, surrounding
/// quotes/backticks and trailing punctuation, then resolve relative
/// paths against the session working directory. (Like Nexus, the
/// trailing-quote strip keeps single quotes — prose paths end with
/// `"…",` or "`…`." far more often than with a stray `'`.)
pub fn resolve_session_path(cwd: &str, raw: &str) -> String {
    let mut path = raw.trim();
    if let Some(stripped) = path.strip_prefix('@') {
        path = stripped;
    }
    // Agent prose wraps paths in quotes/backticks and trailing punctuation
    path = path.trim_start_matches(['"', '\'', '`']);
    path = path.trim_end_matches(['"', '`']);
    path = path.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
    if !is_abs_path(path) && !cwd.is_empty() && !path.is_empty() {
        let sep = if cwd.contains('\\') { '\\' } else { '/' };
        let base = if cwd.ends_with('/') || cwd.ends_with('\\') {
            cwd.to_string()
        } else {
            format!("{cwd}{sep}")
        };
        return format!("{base}{path}");
    }
    path.to_string()
}

/// Port of Nexus `openFilePreview` + `ensureDockOpen`: directories reveal
/// in the dock Files tab, everything else opens a preview tab (missing
/// paths and paths outside the working root fall back to a tab, exactly
/// like the Nexus `fs.stat` → `listDir` → `openFilePreviewTab` chain).
/// The dock picks the request up through `preview_open_seq`.
pub async fn open_path(mut state: Signal<ClientState>, raw: &str) {
    let cwd = state.read().snapshot.working_directory.clone();
    let mut path = resolve_session_path(&cwd, raw);
    if path.is_empty() {
        return;
    }
    // Dock paths are working-directory relative (file rows, breadcrumbs,
    // preview-tab dedupe); absolute paths stay only outside the root.
    if !cwd.is_empty() {
        let base = cwd.trim_end_matches(['/', '\\']);
        if path == cwd {
            path = String::new();
        } else if let Some(rest) = path.strip_prefix(&format!("{base}/")) {
            path = rest.to_string();
        } else if let Some(rest) = path.strip_prefix(&format!("{base}\\")) {
            path = rest.to_string();
        }
    }
    // A successful listing means the path is a directory inside the root
    // → reveal in the Files tab. api_list_dir confines paths to the root
    // and errors on plain files, mirroring the Nexus isDir check.
    let reveal = crate::fsgit::api_list_dir(token(&state), path.clone())
        .await
        .is_ok();
    // Nexus `ensureDockOpen`: open the dock and persist the pref.
    if !state.read().snapshot.config.dock_open {
        let (language, theme, limit) = {
            let s = state.read();
            (
                s.snapshot.config.language.clone(),
                s.snapshot.config.theme.clone(),
                s.snapshot.config.sidebar_sessions_limit.unwrap_or(0),
            )
        };
        save_prefs(state.clone(), language, theme, true, limit).await;
    }
    let mut s = state.write();
    s.preview_open = Some(PreviewOpen { path, reveal });
    s.preview_open_seq += 1;
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn inline_path_heuristic() {
        assert!(looks_like_inline_path("src/main.rs"));
        assert!(looks_like_inline_path("/etc/hosts"));
        assert!(looks_like_inline_path("C:\\Users\\k\\file.txt"));
        assert!(looks_like_inline_path("@src/lib.rs"));
        assert!(looks_like_inline_path("./docs/spec.md"));
        assert!(!looks_like_inline_path("hello world"));
        assert!(!looks_like_inline_path("https://example.com/x"));
        assert!(!looks_like_inline_path("plainword"));
        assert!(!looks_like_inline_path(""));
        assert!(!looks_like_inline_path(&"x".repeat(600)));
    }

    #[test]
    fn resolves_and_strips_noise() {
        assert_eq!(resolve_session_path("/w", "src/a.rs"), "/w/src/a.rs");
        assert_eq!(resolve_session_path("/w", "`src/a.rs`"), "/w/src/a.rs");
        assert_eq!(resolve_session_path("/w", "src/a.rs,"), "/w/src/a.rs");
        assert_eq!(resolve_session_path("/w", "\"rel/b\""), "/w/rel/b");
        assert_eq!(resolve_session_path("/w", "@/abs/x.txt"), "/abs/x.txt");
        assert_eq!(resolve_session_path("", "src/a.rs"), "src/a.rs");
        assert_eq!(resolve_session_path("/w", "C:\\x\\y"), "C:\\x\\y");
        // Nexus quirk preserved 1:1: each strip pass runs once in sequence,
        // so `` `x`. `` keeps the trailing backtick (quotes strip before
        // the trailing dot is removed, leaving the quote exposed).
        assert_eq!(resolve_session_path("/w", "`src/a.rs`."), "/w/src/a.rs`");
        // trailing single quote survives the quote strip (Nexus behaviour)
        assert_eq!(resolve_session_path("/w", "it's"), "/w/it's");
    }
}
