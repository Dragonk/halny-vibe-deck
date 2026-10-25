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
    pub auth_required: bool,
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
}

/// Refetch the whole snapshot (structural change or first load).
pub async fn refresh(mut state: Signal<ClientState>) {
    let token = state.read().token.clone().unwrap_or_default();
    match crate::api::api_snapshot(token).await {
        Ok(snap) => {
            apply_theme(&snap.config.theme);
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
                    }),
                }
            }
            false
        }
        AcpEvent::ThinkingComplete { session_id } => {
            if is_current(&session_id) {
                let mut s = state.write();
                if let Some(msg) = s.snapshot.current_messages.last_mut() {
                    for b in &mut msg.thinking {
                        b.is_complete = true;
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
                        }
                    }
                }
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

/// token() helper: the current auth token (empty when auth is off).
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
