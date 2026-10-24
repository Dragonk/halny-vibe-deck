//! VibeDeck engine - the headless core between the ACP worker and any UI
//! (desktop webview, remote browser, mobile). Owns the session state
//! machine ported from the egui app (priming, replay attach, pending
//! prompts) and publishes every change on a broadcast bus so any number
//! of UI clients can stay in sync.

pub mod config;
mod types;

pub use config::{random_hex, AuthEntry, EngineConfig};
pub use types::EngineState;
pub use vibedeck_core::snapshot::{
    AcpStatus, BusEvent, PendingApproval, PendingQuestion, SessionMeta, Snapshot, UsageStats,
};

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};
use tokio::sync::broadcast;

use vibedeck_acp::AcpHandle;
use vibedeck_core::proto::{AcpCommand, AcpEvent};
use vibedeck_core::types::{now_ms, ChatSession, Message, MessageSender, ThinkingBlock};

static ENGINE: OnceLock<Engine> = OnceLock::new();

pub struct Engine {
    acp: AcpHandle,
    state: Mutex<EngineState>,
    tx: broadcast::Sender<BusEvent>,
    config: RwLock<EngineConfig>,
    /// auth token -> username (in-memory sessions)
    tokens: Mutex<HashMap<String, String>>,
}

impl Engine {
    /// Initialize the global engine and start the event pump.
    pub fn init(cfg: EngineConfig) -> &'static Engine {
        let (tx, _rx) = broadcast::channel(4096);
        let engine = Engine {
            acp: AcpHandle::spawn(),
            state: Mutex::new(EngineState::default()),
            tx,
            config: RwLock::new(cfg),
            tokens: Mutex::new(HashMap::new()),
        };
        let e: &'static Engine = ENGINE.get_or_init(|| engine);
        e.start_pump();
        e
    }

    pub fn global() -> &'static Engine {
        ENGINE.get().expect("engine not initialized")
    }

    pub fn try_global() -> Option<&'static Engine> {
        ENGINE.get()
    }

    /// Background thread: drain ACP worker events -> apply to state ->
    /// re-broadcast to every subscribed UI client.
    fn start_pump(&'static self) {
        std::thread::Builder::new()
            .name("vibedeck-engine-pump".into())
            .spawn(move || {
                let rx = self.acp.events();
                loop {
                    match rx.recv() {
                        Ok(ev) => {
                            self.apply_event(&ev);
                            self.publish(BusEvent::Acp(ev));
                        }
                        Err(_) => return,
                    }
                }
            })
            .expect("failed to spawn engine pump");
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }

    fn publish(&self, ev: BusEvent) {
        let _ = self.tx.send(ev);
    }

    // ---------------- config ----------------

    pub fn config(&self) -> EngineConfig {
        self.config.read().unwrap().clone()
    }

    pub fn update_config(&self, f: impl FnOnce(&mut EngineConfig)) {
        let mut c = self.config.write().unwrap();
        f(&mut c);
        c.save();
    }

    pub fn workdir(&self) -> String {
        let wd = self.config().working_directory;
        if wd.is_empty() {
            dirs::home_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| ".".to_string())
        } else {
            wd
        }
    }

    // ---------------- lifecycle ----------------

    /// Spawn vibe-acp (when configured) and connect.
    pub fn start_agent(&self) {
        {
            let mut st = self.state.lock().unwrap();
            st.status = Some(AcpStatus::Connecting);
            st.acp_error.clear();
        }
        let acp_path = self.config().acp_path;
        if !acp_path.is_empty() {
            self.acp.send(AcpCommand::Start { acp_path });
        }
    }

    pub fn refresh_sessions(&self) {
        self.acp.send(AcpCommand::ListSessions);
    }

    // ---------------- snapshot ----------------

    pub fn snapshot(&self) -> Snapshot {
        let st = self.state.lock().unwrap();
        let cfg = self.config();
        Snapshot {
            status: st.status().as_str().to_string(),
            acp_error: st.acp_error.clone(),
            busy: st.busy,
            current: st.current,
            sessions: st.sessions.iter().map(SessionMeta::from).collect(),
            current_messages: st
                .current_session()
                .map(|s| s.messages.clone())
                .unwrap_or_default(),
            modes: st.modes.clone(),
            config_options: st.config_options.clone(),
            commands: st.commands.clone(),
            backend_sessions: st.backend_sessions.clone(),
            usage: st.usage.clone(),
            approval: st.approval.clone(),
            question: st.question.clone(),
            working_directory: cfg.working_directory.clone(),
            preferred_model: cfg.preferred_model.clone(),
        }
    }

    // ---------------- commands ----------------

    /// New local (not yet backend-backed) session, at the top of the list.
    pub fn create_local_session(&self) {
        let cwd = {
            let st = self.state.lock().unwrap();
            st.current_session()
                .map(|s| s.working_directory.clone())
                .unwrap_or_else(|| self.config().working_directory.clone())
        };
        let mut session = ChatSession::new("New session", &cwd);
        session.working_directory = cwd;
        let mut st = self.state.lock().unwrap();
        st.sessions.insert(0, session);
        st.current = Some(0);
        st.usage = UsageStats::default();
        drop(st);
        self.publish(BusEvent::StructureChanged);
    }

    pub fn switch_session(&self, index: usize) {
        {
            let mut st = self.state.lock().unwrap();
            if index >= st.sessions.len() {
                return;
            }
            st.current = Some(index);
            st.usage = UsageStats::default();
        }
        self.publish(BusEvent::StructureChanged);
    }

    /// Open (or switch to) a session from session/list - creates the local
    /// mirror and issues session/load when not yet loaded.
    pub fn open_backend_session(&self, session_id: &str) {
        let info = {
            let st = self.state.lock().unwrap();
            st.backend_sessions
                .iter()
                .find(|s| s.session_id == session_id)
                .cloned()
        };
        let Some(info) = info else { return };
        let cwd = if info.cwd.is_empty() {
            self.workdir()
        } else {
            info.cwd.clone()
        };
        let model = self.config().preferred_model.clone();
        {
            let mut st = self.state.lock().unwrap();
            if let Some(i) = st
                .sessions
                .iter()
                .position(|s| s.acp_session_id.as_deref() == Some(info.session_id.as_str()))
            {
                st.current = Some(i);
                drop(st);
                self.publish(BusEvent::StructureChanged);
                return;
            }
            let mut session = ChatSession::new(
                if info.name.is_empty() { "CLI session" } else { &info.name },
                &cwd,
            );
            session.working_directory = cwd.clone();
            st.sessions.insert(0, session);
            st.current = Some(0);
            st.busy = true;
            st.loading_session = Some(info.session_id.clone());
        }
        self.acp.send(AcpCommand::LoadSession {
            session_id: info.session_id.clone(),
            cwd,
            model,
        });
        self.publish(BusEvent::StructureChanged);
    }

    /// Create the backend (vibe-acp) session for the current local one.
    fn start_backend_session(&self) {
        let cwd = self.workdir();
        let model = self.config().preferred_model.clone();
        {
            let mut st = self.state.lock().unwrap();
            st.busy = true;
        }
        self.acp.send(AcpCommand::NewSession { cwd, model });
    }

    pub fn send_message(&self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.state.lock().unwrap().current.is_none() {
            self.create_local_session();
            self.start_backend_session();
        }
        let (acp_id, user_msg, local_id) = {
            let mut st = self.state.lock().unwrap();
            let Some(session) = st.current_session_mut() else { return };
            session.messages.push(Message::new_user(&text));
            session.last_active_ms = now_ms();
            (
                session.acp_session_id.clone(),
                session.messages.last().expect("just pushed").clone(),
                session.id.clone(),
            )
        };
        self.publish(BusEvent::UserMessageAdded {
            local_session_id: local_id,
            message: user_msg,
        });
        match acp_id {
            Some(acp_session_id) => {
                self.set_busy(true);
                self.acp.send(AcpCommand::Prompt {
                    session_id: acp_session_id,
                    text,
                });
            }
            None => {
                let mut st = self.state.lock().unwrap();
                if let Some(primed) = st.primed_session.take() {
                    if let Some(session) = st.current_session_mut() {
                        session.acp_session_id = Some(primed.clone());
                    }
                    st.busy = true;
                    drop(st);
                    self.acp.send(AcpCommand::Prompt {
                        session_id: primed,
                        text,
                    });
                } else if st.priming {
                    st.busy = true;
                    st.pending_prompt = Some(text);
                } else {
                    st.busy = true;
                    st.pending_prompt = Some(text);
                    drop(st);
                    self.start_backend_session();
                }
            }
        }
    }

    pub fn cancel(&self) {
        let sid = {
            let st = self.state.lock().unwrap();
            st.current_session()
                .and_then(|s| s.acp_session_id.clone())
        };
        if let Some(session_id) = sid {
            self.acp.send(AcpCommand::Cancel { session_id });
        }
    }

    pub fn approve(&self, request_id: String, option_id: String) {
        self.acp.send(AcpCommand::Approve { request_id, option_id });
        let mut st = self.state.lock().unwrap();
        st.approval = None;
    }

    pub fn answer_question(&self, request_id: String, answer: String) {
        self.acp.send(AcpCommand::AnswerQuestion { request_id, answer });
        let mut st = self.state.lock().unwrap();
        st.question = None;
    }

    /// Select a config option locally + on the agent.
    pub fn set_config_option(&self, id: &str, value: String) {
        {
            let mut st = self.state.lock().unwrap();
            if let Some(opt) = st.config_options.iter_mut().find(|o| o.id == id) {
                opt.current_value = value.clone();
            }
        }
        if id == "model" {
            self.update_config(|c| c.preferred_model = Some(value.clone()));
        }
        let sid = {
            let st = self.state.lock().unwrap();
            st.current_session()
                .and_then(|s| s.acp_session_id.clone())
                .or_else(|| st.primed_session.clone())
        };
        if let Some(session_id) = sid {
            self.acp.send(AcpCommand::SetConfigOption {
                session_id,
                key: id.to_string(),
                value,
            });
        }
    }

    fn set_busy(&self, busy: bool) {
        self.state.lock().unwrap().busy = busy;
    }

    // ---------------- auth ----------------

    pub fn auth_enabled(&self) -> bool {
        self.config().auth.is_some()
    }

    /// Verify credentials and mint a session token (None on bad login).
    pub fn login(&self, username: &str, password: &str) -> Option<String> {
        let ok = self
            .config()
            .auth
            .as_ref()
            .map(|a| a.verify(username, password))
            .unwrap_or(false);
        if !ok {
            return None;
        }
        let token = random_hex(32);
        self.tokens
            .lock()
            .unwrap()
            .insert(token.clone(), username.to_string());
        Some(token)
    }

    pub fn check_token(&self, token: &str) -> bool {
        !token.is_empty() && self.tokens.lock().unwrap().contains_key(token)
    }

    pub fn logout(&self, token: &str) {
        self.tokens.lock().unwrap().remove(token);
    }

    // ---------------- state machine (port of the egui app) ----------------

    fn prime_session_if_needed(&self) {
        let (active, primed, priming, status) = {
            let st = self.state.lock().unwrap();
            (
                st.current_session().and_then(|s| s.acp_session_id.clone()),
                st.primed_session.clone(),
                st.priming,
                st.status(),
            )
        };
        if status == AcpStatus::Connected && active.is_none() && primed.is_none() && !priming {
            self.state.lock().unwrap().priming = true;
            let cwd = self.workdir();
            let model = self.config().preferred_model.clone();
            self.acp.send(AcpCommand::NewSession { cwd, model });
        }
    }

    fn finalize_open_messages(st: &mut EngineState) {
        if let Some(session) = st.current_session_mut() {
            for msg in &mut session.messages {
                if msg.sender == MessageSender::Ai {
                    msg.is_complete = true;
                }
            }
        }
    }

    fn push_system_message(st: &mut EngineState, text: &str) {
        if let Some(session) = st.current_session_mut() {
            session.messages.push(Message::new_system(text));
        }
    }

    /// Port of VibedeckApp::apply_event (minus egui-specific bits).
    fn apply_event(&self, event: &AcpEvent) {
        let mut st = self.state.lock().unwrap();
        if st.usage.apply(event) {
            return; // live usage counters updated
        }
        match event {
            AcpEvent::Initialized { protocol_version } => {
                st.status = Some(AcpStatus::Connected);
                st.acp_error.clear();
                let _ = protocol_version;
                // Prime a background session so the chips and the
                // slash-command list have real values before the first
                // message.
                drop(st);
                self.prime_session_if_needed();
            }
            AcpEvent::Disconnected { .. } => {
                st.status = Some(AcpStatus::Disconnected);
                st.busy = false;
                st.primed_session = None;
                st.priming = false;
                Self::finalize_open_messages(&mut st);
            }
            AcpEvent::SessionsList(sessions) => st.backend_sessions = sessions.clone(),
            AcpEvent::SessionReady {
                session_id,
                modes,
                config_options,
                title,
            } => {
                st.status = Some(AcpStatus::Connected);
                st.modes = modes.clone();
                st.config_options = config_options.clone();
                if st.priming {
                    // Background prime of a fresh session: stash it for the
                    // first prompt instead of claiming whatever session is
                    // currently open - that may be a CLI mirror whose
                    // session/load replay is still in flight.
                    st.priming = false;
                    st.primed_session = Some(session_id.clone());
                } else {
                    st.busy = false;
                    // Only a ready for the session being loaded may clear
                    // the pending load - otherwise an unrelated session
                    // (e.g. the primed one) would cancel the replay attach.
                    if st.loading_session.as_deref() == Some(session_id.as_str()) {
                        st.loading_session = None;
                    }
                    if let Some(session) = st.current_session_mut() {
                        session.acp_session_id = Some(session_id.clone());
                        if let Some(title) = title {
                            if !title.is_empty() {
                                session.name = title.clone();
                            }
                        }
                    } else {
                        // No local session yet - background session; the
                        // first prompt will attach to it.
                        st.primed_session = Some(session_id.clone());
                    }
                }
                let text = st.pending_prompt.take();
                drop(st);
                if let Some(text) = text {
                    self.send_message(text);
                }
            }
            AcpEvent::UsageUpdate { .. } => { /* handled above */ }
            AcpEvent::SessionHistory {
                session_id,
                messages,
            } => {
                if st.loading_session.as_deref() == Some(session_id.as_str()) {
                    if let Some(session) = st.current_session_mut() {
                        session.messages = messages.clone();
                        session.last_active_ms = now_ms();
                    }
                }
            }
            AcpEvent::UserMessageChunk { .. } => {}
            AcpEvent::SessionError { context, message } => {
                st.busy = false;
                st.priming = false;
                st.loading_session = None;
                st.status = Some(AcpStatus::Error);
                st.acp_error = format!("{context}: {message}");
                Self::finalize_open_messages(&mut st);
                Self::push_system_message(&mut st, &format!("WARNING {context}: {message}"));
            }
            AcpEvent::MessageChunk { session_id, text } => {
                if let Some(session) = st.session_by_acp_id_mut(session_id) {
                    let msg = ensure_ai_message(session);
                    msg.content.push_str(text);
                }
            }
            AcpEvent::ThinkingChunk { session_id, text } => {
                if let Some(session) = st.session_by_acp_id_mut(session_id) {
                    let msg = ensure_ai_message(session);
                    match msg.thinking.last_mut() {
                        Some(block) if !block.is_complete => block.content.push_str(text),
                        _ => msg.thinking.push(ThinkingBlock {
                            id: "thinking".into(),
                            content: text.clone(),
                            is_complete: false,
                        }),
                    }
                }
            }
            AcpEvent::ThinkingComplete { session_id } => {
                if let Some(session) = st.session_by_acp_id_mut(session_id) {
                    if let Some(msg) = session.messages.last_mut() {
                        for block in &mut msg.thinking {
                            block.is_complete = true;
                        }
                    }
                }
            }
            AcpEvent::ToolCallStarted {
                session_id,
                tool_call,
            } => {
                if let Some(session) = st.session_by_acp_id_mut(session_id) {
                    ensure_ai_message(session).tool_calls.push(tool_call.clone());
                }
            }
            AcpEvent::ToolCallUpdated {
                session_id,
                tool_call_id,
                status,
                output,
            } => {
                if let Some(session) = st.session_by_acp_id_mut(session_id) {
                    for msg in session.messages.iter_mut().rev() {
                        for tc in &mut msg.tool_calls {
                            if &tc.id == tool_call_id {
                                tc.status = *status;
                                if let Some(output) = output {
                                    tc.output = Some(output.clone());
                                }
                            }
                        }
                    }
                }
            }
            AcpEvent::ToolApprovalRequest {
                request_id,
                method,
                params_json,
                tool_name,
            } => {
                st.approval = Some(PendingApproval {
                    request_id: request_id.clone(),
                    method: method.clone(),
                    params_json: params_json.clone(),
                    tool_name: tool_name.clone(),
                });
            }
            AcpEvent::AskUserQuestion {
                request_id,
                question,
                choices,
            } => {
                st.question = Some(PendingQuestion {
                    request_id: request_id.clone(),
                    question: question.clone(),
                    choices: choices.clone(),
                });
            }
            AcpEvent::PlanUpdate {
                session_id,
                entries_json,
            } => {
                let _ = (session_id, entries_json);
            }
            AcpEvent::AvailableCommands { commands, .. } => {
                st.commands = commands.clone();
            }
            AcpEvent::PromptCompleted { .. } => {
                st.busy = false;
                Self::finalize_open_messages(&mut st);
            }
            AcpEvent::DebugLog(line) => {
                let _ = line;
            }
        }
    }
}

/// Ensure the session ends with an open AI message and return it.
pub fn ensure_ai_message(session: &mut ChatSession) -> &mut Message {
    let needs_new = !matches!(
        session.messages.last(),
        Some(msg) if msg.sender == MessageSender::Ai && !msg.is_complete
    );
    if needs_new {
        session.messages.push(Message::new_ai());
    }
    session.messages.last_mut().expect("just pushed")
}
