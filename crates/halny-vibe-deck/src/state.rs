//! Client-side reactive state: a local projection of the engine state,
//! kept in sync by the event stream (incremental for streaming chunks,
//! snapshot refetch for structural changes).

use dioxus::prelude::*;
use halny_vibe_deck_core::proto::AgentEvent;
use halny_vibe_deck_core::snapshot::{BusEvent, SessionLoadProgress, Snapshot};
use halny_vibe_deck_core::types::{Message, MessageSender};

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
    /// Round 15: session id (SessionMeta.id) a switch was just requested
    /// for — the chat renders a "loading session" marker immediately
    /// instead of freezing on the old session until the refetch lands.
    pub switching_to: Option<String>,
    pub view: View,
    /// Prompts typed while the agent is busy (sent on completion).
    pub queue: Vec<String>,
    /// Mobile (≤768 px): the sidebar drawer is open (Nexus
    /// mobileSidebarOpen).
    pub mobile_sidebar_open: bool,
    /// True only in the desktop webview. Set once after mount (`window.ipc`
    /// probe) so the SSR render and the hydrated client agree on the layout —
    /// the browser hides the header and shows the floating panel buttons,
    /// the webview keeps the full header (stable v1.0.27 layout parity).
    pub desktop_mode: bool,
    /// agent binary missing → show the first-run SetupView.
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
    /// Element 6: app update check result (banner + Settings Updates).
    pub update_app: Option<halny_vibe_deck_core::snapshot::AppUpdateInfo>,
    /// Element 6: local CLI update check result.
    pub update_cli: Option<halny_vibe_deck_core::snapshot::CliUpdateInfo>,
    /// Element 6: banner dismissed (until the next check).
    pub update_dismissed: bool,
    /// Element 6: an app self-update is running (progress is polled).
    pub update_busy: bool,
    /// Element 6: terminal update error (banner + Settings).
    pub update_error: Option<String>,
    /// Element 6: live self-update progress phase download/install/relaunch.
    pub update_progress: Option<halny_vibe_deck_core::snapshot::UpdateProgress>,
    /// L1: last failed turn (`turn.error` from the wire —
    /// `AgentEvent::TurnFailed`), `(code, message, agent_session_id)`.
    /// Keyed by session so an error from session A never renders in
    /// session B after a switch; cleared on the next submit.
    pub turn_failed: Option<(String, String, String)>,
    /// L1: force the login wizard on top of the Shell ("Sign in
    /// again" in Settings / rate-limit CTA). Consumed by app.rs.
    pub login_requested: bool,
    /// `/rewind` / `/branch` checkpoint picker modal (TUI parity).
    pub rewind_pick: Option<RewindPick>,
    /// Last `session/rewind/read` answer — files a rewind would restore.
    pub rewind_info: Option<RewindInfoView>,
    /// `/teleport` — Vibe Code project picker view (raw JSON) + picker id.
    pub teleport_picker: Option<TeleportUi>,
    /// `vibeCode/teleport/event` progress (raw event JSON).
    pub teleport_progress: Option<String>,
    /// `/loops` modal — raw loops JSON array.
    pub loops_modal: Option<String>,
    /// `/log` modal — (session log JSON, diagnostics logs JSON).
    pub log_modal: Option<(String, String)>,
    /// `workspace/git/worktrees/list` result — (cwd, items JSON).
    pub worktrees: Option<(String, String)>,
    /// Session artifacts (PRs) — raw JSON per artifact.
    pub artifacts: Vec<String>,
}

/// Teleport picker modal state.
#[derive(Clone, Debug, PartialEq)]
pub struct TeleportUi {
    pub picker_id: String,
    pub view_json: String,
}

/// Checkpoint picker opened by `/rewind` or `/branch`.
#[derive(Clone, Debug, PartialEq)]
pub struct RewindPick {
    /// "rewind" | "branch"
    pub mode: String,
    /// (entry_id, label) — one per rewindable history entry.
    pub entries: Vec<(String, String)>,
    /// selected entry + restore-files checkbox (rewind confirm state)
    pub selected: Option<String>,
    pub restore_files: bool,
}

/// `session/rewind/read` answer for the pending picker selection.
#[derive(Clone, Debug, PartialEq)]
pub struct RewindInfoView {
    pub entry_id: String,
    pub has_file_changes: bool,
    pub paths: Vec<String>,
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
            // v1.0.34 parity: a resume that returns an empty history for
            // the session already on screen keeps the visible messages
            // instead of wiping the chat. The comparison is by session ID,
            // not by index: creating a new session inserts it at index 0,
            // so `current` can stay `Some(0)` while pointing at a different
            // session — comparing indices would wrongly keep the previous
            // session's messages on the fresh (empty) one.
            let prev_messages = s.snapshot.current_messages.clone();
            let prev_current_id = s
                .snapshot
                .current
                .and_then(|i| s.snapshot.sessions.get(i))
                .map(|m| m.id.clone());
            s.snapshot = snap;
            let new_current_id = s
                .snapshot
                .current
                .and_then(|i| s.snapshot.sessions.get(i))
                .map(|m| m.id.clone());
            if prev_current_id.is_some()
                && new_current_id.is_some()
                && prev_current_id == new_current_id
                && !prev_messages.is_empty()
                && s.snapshot.current_messages.is_empty()
            {
                s.snapshot.current_messages = prev_messages;
            }
            // Round 15: the switch landed — drop the loading marker.
            if s.switching_to.is_some() && s.switching_to == new_current_id {
                s.switching_to = None;
            }
            s.loaded = true;
            // fetch PR artifacts for the open session (artifacts/list)
            let has_agent = s
                .snapshot
                .current
                .and_then(|i| s.snapshot.sessions.get(i))
                .map(|m| m.agent_session_id.is_some())
                .unwrap_or(false);
            drop(s);
            if has_agent {
                let st = state.clone();
                spawn(async move {
                    let tok = st.read().token.clone().unwrap_or_default();
                    let _ = crate::api::api_artifacts_list(tok).await;
                });
            }
        }
        Err(_e) => { /* snapshot fetch failed; next event will retry */ }
    }
    // One-shot first-run check: show the setup screen when the
    // vibe-app-server binary is missing (any login state).
    if !state.read().setup_checked {
        state.write().setup_checked = true;
        if let Ok(st) = crate::setup::api_setup_status().await {
            state.write().setup_needed = !st.agent_found;
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
                    meta.last_active_ms = halny_vibe_deck_core::types::now_ms();
                }
            }
            false
        }
        BusEvent::StructureChanged => true,
        BusEvent::Agent(e) => apply_agent_event(state, e),
    }
}

fn apply_agent_event(mut state: Signal<ClientState>, e: AgentEvent) -> bool {
    let current_agent_id = {
        let s = state.read();
        s.snapshot
            .current
            .and_then(|i| s.snapshot.sessions.get(i))
            .and_then(|m| m.agent_session_id.clone())
    };
    let is_current = |sid: &str| current_agent_id.as_deref() == Some(sid);
    // Live usage: update counters locally, no refetch - but only for the
    // session the user is looking at (parallel slots stream in parallel).
    if let AgentEvent::UsageUpdate { session_id, .. } = &e {
        if is_current(session_id) {
            let mut s = state.write();
            let _ = s.snapshot.usage.apply(&e);
        }
        return false;
    }
    // Lazy-resume replay progress (Nexus SessionLoadProgress): update the
    // "Resuming session… (N)" overlay count in place, no refetch. The
    // wake finishing (SessionReady) refetches and clears it.
    if let AgentEvent::SessionLoadProgress { session_id, count } = &e {
        if is_current(session_id) {
            let mut s = state.write();
            s.snapshot.session_load_progress = Some(SessionLoadProgress {
                session_id: session_id.clone(),
                count: *count,
            });
        }
        return false;
    }
    match e {
        AgentEvent::MessageChunk {
            session_id,
            text,
            message_id,
        } => {
            if is_current(&session_id) {
                let mut s = state.write();
                // v0.13.7: a new agent messageId starts a new agent
                // message (Nexus `streamingState.agentMsgId` split) -
                // mirror the engine so a snapshot refetch finds the
                // same shape. The "current" fallback never splits.
                if message_id != "current" {
                    if s.snapshot.ai_stream_msg_id.is_some()
                        && s.snapshot.ai_stream_msg_id.as_deref() != Some(message_id.as_str())
                    {
                        finalize_segment_pub(&mut s.snapshot.current_messages);
                    }
                    s.snapshot.ai_stream_msg_id = Some(message_id.clone());
                }
                let msg = ensure_ai_message_pub(&mut s.snapshot.current_messages);
                msg.content.push_str(&text);
            }
            false
        }
        AgentEvent::ThinkingChunk {
            session_id,
            text,
            block_id,
        } => {
            if is_current(&session_id) {
                let mut s = state.write();
                // v0.13.7: reasoning after streamed text opens the NEXT
                // agent message - the thought lands above its own text,
                // not glued under the previous segment.
                if s.snapshot.ai_stream_msg_id.take().is_some() {
                    finalize_segment_pub(&mut s.snapshot.current_messages);
                }
                let msg = ensure_ai_message_pub(&mut s.snapshot.current_messages);
                match msg.thinking.last_mut() {
                    Some(block) if !block.is_complete && block.id == block_id => {
                        block.content.push_str(&text)
                    }
                    _ => msg.thinking.push(halny_vibe_deck_core::types::ThinkingBlock {
                        id: block_id,
                        content: text,
                        is_complete: false,
                        started_at: halny_vibe_deck_core::types::now_ms(),
                        completed_at: 0,
                    }),
                }
            }
            false
        }
        AgentEvent::ThinkingComplete { session_id } => {
            if is_current(&session_id) {
                let mut s = state.write();
                if let Some(msg) = s.snapshot.current_messages.last_mut() {
                    let now = halny_vibe_deck_core::types::now_ms();
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
        AgentEvent::ToolCallStarted {
            session_id,
            tool_call,
        } => {
            if is_current(&session_id) {
                let mut s = state.write();
                // Round 15: dedup by tool-call id — a replayed/resynced
                // entryAdded must not append the same step twice (the chat
                // showed every tool action 10-15x).
                let exists = s
                    .snapshot
                    .current_messages
                    .iter()
                    .any(|m| m.tool_calls.iter().any(|t| t.id == tool_call.id));
                if !exists {
                    ensure_ai_message_pub(&mut s.snapshot.current_messages)
                        .tool_calls
                        .push(tool_call);
                }
            }
            false
        }
        AgentEvent::ToolCallUpdated {
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
        AgentEvent::PlanUpdate {
            session_id,
            entries_json,
        } => {
            if is_current(&session_id) {
                let entries = halny_vibe_deck_core::types::PlanEntry::parse_list(&entries_json);
                state.write().snapshot.current_plan = entries;
            }
            false
        }
        // teleport to Vibe Code Web: picker view + progress events
        AgentEvent::TeleportPicker {
            picker_id,
            view_json,
            ..
        } => {
            let mut s = state.write();
            s.teleport_picker = Some(TeleportUi { picker_id, view_json });
            s.teleport_progress = None;
            false
        }
        AgentEvent::TeleportProgress { event_json, .. } => {
            state.write().teleport_progress = Some(event_json);
            false
        }
        AgentEvent::LoopsListResult { loops_json, .. } => {
            state.write().loops_modal = Some(loops_json);
            false
        }
        AgentEvent::SessionLog { log_json, .. } => {
            let mut s = state.write();
            let e = s.log_modal.take().unwrap_or_else(|| (String::new(), String::new()));
            s.log_modal = Some((log_json, e.1));
            false
        }
        AgentEvent::DiagnosticsLogs { logs_json, .. } => {
            let mut s = state.write();
            let e = s.log_modal.take().unwrap_or_else(|| (String::new(), String::new()));
            s.log_modal = Some((e.0, logs_json));
            false
        }
        AgentEvent::WorktreesResult { cwd, items_json } => {
            state.write().worktrees = Some((cwd, items_json));
            false
        }
        AgentEvent::ArtifactsResult { artifacts_json, .. } => {
            let parsed: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(&artifacts_json)
                .map(|v| v.iter().map(|x| x.to_string()).collect())
                .unwrap_or_default();
            state.write().artifacts = parsed;
            false
        }
        AgentEvent::ArtifactsChanged { .. } => {
            let st = state.clone();
            spawn(async move {
                let _ = crate::api::api_artifacts_list(token(&st)).await;
            });
            false
        }
        // rewind preview answer — the /rewind picker shows affected files
        AgentEvent::RewindInfo {
            entry_id,
            has_file_changes,
            paths,
            ..
        } => {
            state.write().rewind_info = Some(RewindInfoView {
                entry_id,
                has_file_changes,
                paths,
            });
            false
        }
        // L1: turn failed (wire `turn.error`, e.g. rate_limit) —
        // kept client-side (an event, not snapshot state) and
        // rendered by the chat banner until the next submit.
        AgentEvent::TurnFailed {
            session_id,
            code,
            message,
        } => {
            if is_current(&session_id) {
                state.write().turn_failed = Some((code, message, session_id));
            }
            false
        }
        // Everything else (SessionReady, SessionHistory, SessionsList,
        // errors, PromptCompleted...) is structural -> refetch snapshot.
        _ => true,
    }
}

/// v0.13.7: close the streaming AI segment before the next one opens -
/// the message completes and its thinking blocks are marked done (Nexus
/// `finalizeAiMessage`, mirrored engine-side).
fn finalize_segment_pub(messages: &mut Vec<Message>) {
    if let Some(msg) = messages.last_mut() {
        if msg.sender == MessageSender::Ai {
            msg.is_complete = true;
            let now = halny_vibe_deck_core::types::now_ms();
            for block in &mut msg.thinking {
                block.is_complete = true;
                if block.completed_at == 0 {
                    block.completed_at = now;
                }
            }
        }
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
/// Element 6: silent app + CLI update check (Nexus startup check 4s after
/// boot; failures keep the banner hidden). Shared by the Shell auto-check
/// and the Settings "check" button.
pub async fn check_updates(mut state: Signal<ClientState>) {
    let t = token(&state);
    let a = crate::api::api_check_app_update(t.clone()).await;
    let c = crate::api::api_check_local_cli_update(t).await;
    if let Ok(a) = a {
        state.write().update_app = Some(a);
    }
    if let Ok(c) = c {
        state.write().update_cli = Some(c);
    }
}

/// Element 6: run the app self-update (fire-and-forget server fn) and poll
/// the progress until it errors or the app relaunches (the server exits ->
/// the poll fails -> done). No await is held across a state write.
pub async fn start_app_update(mut state: Signal<ClientState>) {
    state.write().update_busy = true;
    state.write().update_error = None;
    let t = token(&state);
    match crate::api::api_update_app(t).await {
        Ok(res) => {
            if !res.ok {
                state.write().update_busy = false;
                if let Some(e) = res.error {
                    state.write().update_error = Some(e);
                }
                return;
            }
            loop {
                match crate::api::api_update_progress(token(&state)).await {
                    Ok((p, e)) => {
                        if let Some(err) = e {
                            state.write().update_error = Some(err);
                            state.write().update_busy = false;
                            return;
                        }
                        state.write().update_progress = p;
                    }
                    Err(_) => {
                        // Server gone (relaunch) or unreachable: stop polling.
                        state.write().update_busy = false;
                        return;
                    }
                }
                sleep_ms(500).await;
            }
        }
        Err(e) => {
            state.write().update_busy = false;
            state.write().update_error = Some(e.to_string());
        }
    }
}

/// Element 6: upgrade the local mistral-vibe CLI (uv), then re-check so the
/// banner + Settings show the fresh version.
pub async fn upgrade_local_cli(mut state: Signal<ClientState>) {
    state.write().update_busy = true;
    state.write().update_error = None;
    let t = token(&state);
    let ok = match crate::api::api_upgrade_local_cli(t.clone()).await {
        Ok(r) => match r.error {
            Some(e) => {
                state.write().update_error = Some(e);
                false
            }
            None => true,
        },
        Err(e) => {
            state.write().update_error = Some(e.to_string());
            false
        }
    };
    if ok {
        if let Ok(c) = crate::api::api_check_local_cli_update(t).await {
            state.write().update_cli = Some(c);
        }
    }
    state.write().update_busy = false;
}

/// Client-safe sleep: `tokio::time::sleep` panics on wasm
/// (`std::time::Instant` is unimplemented there) — that panic used to kill
/// the whole WASM module ~4s after load (update-check timer) and on every
/// event-stream reconnect, leaving the UI permanently dead.
pub async fn sleep_ms(ms: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(ms as u32).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

pub fn token(state: &Signal<ClientState>) -> String {
    state.read().token.clone().unwrap_or_default()
}

/// Git context of the current session: (working directory, remote
/// connection id). Nexus runs git on the *session* cwd — remote SSH
/// sessions pipe the git command through the connection (GitPanel
/// v0.7.0 `remoteId`); local sessions run `git -C <cwd>` in-process.
pub fn session_git_ctx(state: &Signal<ClientState>) -> (String, Option<String>) {
    let s = state.read();
    s.snapshot
        .current
        .and_then(|i| s.snapshot.sessions.get(i))
        .map(|m| (m.working_directory.clone(), m.remote_id.clone()))
        .unwrap_or_else(|| (s.snapshot.working_directory.clone(), None))
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

/// True when the UI runs inside the desktop webview window.
///
/// The desktop launcher opens the webview at `/?desktop=1`; a browser (local
/// or remote access) never carries that flag, so the web layout — no title
/// bar — is guaranteed there (Z23). The old `window.ipc` probe was unreliable
/// in both directions.
pub fn is_desktop() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.location().search().ok())
            .map(|q| q.contains("desktop=1"))
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
    let tok = token(&state);
    // TUI parity (cli-rust): `!cmd` runs an inline shell command via
    // `session/shellCommand` — it is NOT sent to the model as a prompt.
    if let Some(cmd) = text.strip_prefix('!') {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            let _ = crate::api::api_shell_command(tok, cmd.to_string()).await;
            return;
        }
    }
    // TUI parity (cli-rust commands/*): a leading `/command` is executed
    // CLIENT-side and translated to JSON-RPC methods — only unknown `/name`
    // goes to the server as a prompt (skill invocation, server-side).
    if text.starts_with('/') {
        let mut parts = text[1..].splitn(2, |c: char| c.is_whitespace());
        let cmd = parts.next().unwrap_or("").to_string();
        let args = parts.next().unwrap_or("").trim().to_string();
        if !cmd.is_empty() && !cmd.contains(['/', '@']) {
            let lower = cmd.to_ascii_lowercase();
            match lower.as_str() {
                "model" | "thinking" | "agent" | "mode" => {
                    let key = if lower == "agent" { "mode" } else { lower.as_str() };
                    if args.is_empty() {
                        let usage = match key {
                            "model" => "Usage: /model <name> — pick a model with the chip below, or type /model mistral-large-latest",
                            "thinking" => "Usage: /thinking <off|low|medium|high|xhigh>",
                            _ => "Usage: /agent <name> — pick an agent with the chip below",
                        };
                        let _ = crate::api::api_local_notice(tok, usage.to_string()).await;
                    } else {
                        let _ = crate::api::api_set_config_option(tok, key.to_string(), args).await;
                    }
                    return;
                }
                "rewind" | "branch" | "fork" => {
                    // TUI parity: pick a checkpoint entry, then rewind/fork
                    let entries: Vec<(String, String)> = state
                        .read()
                        .snapshot
                        .current_messages
                        .iter()
                        .filter(|m| !m.id.is_empty() && m.id != "current")
                        .map(|m| {
                            let label = m
                                .content
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(60)
                                .collect::<String>();
                            let role = match m.sender {
                                halny_vibe_deck_core::types::MessageSender::User => "you",
                                halny_vibe_deck_core::types::MessageSender::Ai => "agent",
                                _ => "sys",
                            };
                            (m.id.clone(), format!("{role}: {label}"))
                        })
                        .collect();
                    if entries.is_empty() {
                        let _ = crate::api::api_local_notice(
                            tok,
                            "No rewindable entries in this session yet.".to_string(),
                        )
                        .await;
                    } else {
                        let mode = if lower == "fork" { "branch".to_string() } else { lower };
                        state.write().rewind_pick = Some(RewindPick {
                            mode,
                            entries,
                            selected: None,
                            restore_files: true,
                        });
                    }
                    return;
                }
                "update" => {
                    match crate::api::api_check_app_update(tok.clone()).await {
                        Ok(info) if info.update_available => {
                            let _ = crate::api::api_local_notice(
                                tok,
                                format!(
                                    "Update available: {} → {} — use the update button in the header.",
                                    info.current_version,
                                    info.latest_version.unwrap_or_default()
                                ),
                            )
                            .await;
                        }
                        Ok(_) => {
                            let _ = crate::api::api_local_notice(tok, "App is up to date.".to_string()).await;
                        }
                        Err(e) => {
                            let _ = crate::api::api_local_notice(tok, format!("Update check failed: {e}")).await;
                        }
                    }
                    return;
                }
                "diff" => {
                    // open the dock on the Git tab
                    let (l, t, lim) = {
                        let s = state.read();
                        (
                            s.snapshot.config.language.clone(),
                            s.snapshot.config.theme.clone(),
                            s.snapshot.config.sidebar_sessions_limit.unwrap_or(0),
                        )
                    };
                    save_prefs(state.clone(), l, t, true, lim).await;
                    let _ = crate::api::api_local_notice(tok, "Opened the Git tab in the dock.".to_string()).await;
                    return;
                }
                "theme" => {
                    // cycle light → dark → auto
                    let cur = state.read().snapshot.config.theme.clone();
                    let next = match cur.as_str() {
                        "dark" => "auto",
                        "auto" => "light",
                        _ => "dark",
                    }
                    .to_string();
                    let (l, lim) = {
                        let s = state.read();
                        (
                            s.snapshot.config.language.clone(),
                            s.snapshot.config.sidebar_sessions_limit.unwrap_or(0),
                        )
                    };
                    let dock = state.read().snapshot.config.dock_open;
                    save_prefs(state.clone(), l, next.clone(), dock, lim).await;
                    let _ = crate::api::api_local_notice(tok, format!("Theme: {next}")).await;
                    return;
                }
                "config" | "permissions" | "connect" | "plugins" | "mcp" | "agents" => {
                    state.write().view = View::Settings;
                    let _ = crate::api::api_local_notice(
                        tok,
                        "Opened Settings — see the corresponding tab there.".to_string(),
                    )
                    .await;
                    return;
                }
                "resume" => {
                    let _ = crate::api::api_local_notice(
                        tok,
                        "Pick a session in the sidebar to resume it.".to_string(),
                    )
                    .await;
                    return;
                }
                "quit" | "exit" => {
                    #[cfg(any(feature = "web", feature = "desktop"))]
                    {
                        dioxus::document::eval("window.close();").await;
                    }
                    return;
                }
                "retry" => {
                    // TUI /retry: resend the last user prompt (optionally
                    // with extra instructions appended).
                    let last_user = state
                        .read()
                        .snapshot
                        .current_messages
                        .iter()
                        .rev()
                        .find(|m| m.sender == halny_vibe_deck_core::types::MessageSender::User)
                        .map(|m| m.content.clone());
                    if let Some(last) = last_user {
                        let text = if args.is_empty() {
                            last
                        } else {
                            format!("{last}\n\n{args}")
                        };
                        let _ = crate::api::api_send_message(tok, text).await;
                    } else {
                        let _ = crate::api::api_local_notice(
                            tok,
                            "Nothing to retry — no previous prompt.".to_string(),
                        )
                        .await;
                    }
                    return;
                }
                _ => {
                    match crate::api::api_slash_command(tok.clone(), lower, args).await {
                        Ok(true) => return,
                        _ => {} // unknown /name → skill invocation as a prompt
                    }
                }
            }
        }
    }
    if state.read().snapshot.busy {
        state.write().queue.push(text);
        return;
    }
    // v0.13.1: a new send attempt clears the stale error banner
    // (Nexus InputBar handleSend).
    state.write().snapshot.agent_error.clear();
    // L1: a new turn also clears the previous turn failure
    // (rate-limit banner) and any login-wizard request.
    state.write().turn_failed = None;
    state.write().login_requested = false;
    // v0.13.7: a new user turn resets the segment split mirror.
    state.write().snapshot.ai_stream_msg_id = None;
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
            // round 8: Unicode letters too (Ścieżka/plik.md), not ASCII only
            .map(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
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
    // remote sessions resolve and list paths against the remote session
    // root over SSH (Nexus fs.stat/listDir with remoteId)
    let (sess_wd, sess_rid) = session_git_ctx(&state);
    // round 8: resolve against the SESSION's own cwd for local sessions too —
    // the global engine workdir made agent links like
    // "Dokumentacja/QUICK_START.md" resolve to the wrong root whenever the
    // open project differed from the configured workdir
    let cwd = if !sess_wd.is_empty() {
        sess_wd
    } else {
        state.read().snapshot.working_directory.clone()
    };
    let mut path = resolve_session_path(&cwd, raw);
    if path.is_empty() {
        return;
    }
    // Dock paths are working-directory relative (file rows, breadcrumbs,
    // preview-tab dedupe); absolute paths stay only outside the root.
    // round 11: normalize separators + drive-letter case before comparing —
    // a tool-call path like `C:\…\proj/README.md` against cwd `C:/…/proj`
    // used to fail the strip, leak an absolute path into the confined read
    // and end as os error 2.
    if !cwd.is_empty() {
        let base = cwd.trim_end_matches(['/', '\\']);
        let base_n = base.replace('\\', "/");
        let path_n = path.replace('\\', "/");
        let ci = cfg!(windows);
        let eq = if ci { path_n.eq_ignore_ascii_case(&base_n) } else { path_n == base_n };
        let strip = if ci {
            path_n.len() > base_n.len()
                && path_n[..base_n.len() + 1].eq_ignore_ascii_case(&format!("{base_n}/"))
        } else {
            path_n.starts_with(&format!("{base_n}/"))
        };
        if eq {
            path = String::new();
        } else if strip {
            // dock paths are '/'-relative — take the rest from the
            // normalized form so mixed separators can't leak through
            path = path_n[base_n.len() + 1..].to_string();
        }
    }
    // A successful listing means the path is a directory inside the root
    // → reveal in the Files tab. api_list_dir confines paths to the root
    // and errors on plain files, mirroring the Nexus isDir check.
    let reveal = crate::fsgit::api_list_dir(
        token(&state),
        path.clone(),
        sess_rid.clone(),
        cwd.clone(),
    )
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
