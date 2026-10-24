//! Client-side reactive state: a local projection of the engine state,
//! kept in sync by the event stream (incremental for streaming chunks,
//! snapshot refetch for structural changes).

use dioxus::prelude::*;
use vibedeck_core::proto::AcpEvent;
use vibedeck_core::snapshot::{BusEvent, Snapshot};
use vibedeck_core::types::{Message, MessageSender};

#[derive(Clone, Default)]
pub struct ClientState {
    pub auth_required: bool,
    pub token: Option<String>,
    pub snapshot: Snapshot,
    pub loaded: bool,
    pub sidebar_open: bool,
}

/// Refetch the whole snapshot (structural change or first load).
pub async fn refresh(mut state: Signal<ClientState>) {
    let token = state.read().token.clone().unwrap_or_default();
    match crate::api::api_snapshot(token).await {
        Ok(snap) => {
            state.write().snapshot = snap;
            state.write().loaded = true;
        }
        Err(_e) => { /* snapshot fetch failed; next event will retry */ }
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
    // Live usage: update counters locally, no refetch.
    {
        let mut s = state.write();
        if s.snapshot.usage.apply(&e) {
            return false;
        }
    }
    let current_acp_id = {
        let s = state.read();
        s.snapshot
            .current
            .and_then(|i| s.snapshot.sessions.get(i))
            .and_then(|m| m.acp_session_id.clone())
    };
    let is_current = |sid: &str| current_acp_id.as_deref() == Some(sid);
    match e {
        // Streaming events: apply incrementally when they belong to the
        // currently-open session.
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
