//! VibeDeck engine - the headless core between ACP workers and any UI
//! (desktop webview, remote browser, mobile). Every session that talks
//! to an agent owns a **dedicated vibe-acp process** (a "slot"), so any
//! number of sessions can stream in parallel. A separate lightweight
//! worker (the "lister") only runs `session/list` for the CLI history.
//! Every change is published on a broadcast bus so any number of UI
//! clients stays in sync.

pub mod config;
mod types;

pub use config::{random_hex, AuthEntry, EngineConfig};
pub use types::EngineState;
pub use vibedeck_core::snapshot::{
    AcpStatus, BusEvent, PendingApproval, PendingQuestion, SessionMeta, Snapshot, UiConfig,
    UsageStats,
};

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};
use tokio::sync::broadcast;

use vibedeck_acp::AcpHandle;
use vibedeck_core::proto::{AcpCommand, AcpEvent};
use vibedeck_core::types::{now_ms, ChatSession, Message, MessageSender};

use crate::types::{spawn_slot, SlotPurpose};

static ENGINE: OnceLock<Engine> = OnceLock::new();

pub struct Engine {
    /// Lightweight worker for `session/list` (CLI history) + the global
    /// status shown when no session is open.
    lister: AcpHandle,
    global_status: Mutex<AcpStatus>,
    global_error: Mutex<String>,
    state: Mutex<EngineState>,
    tx: broadcast::Sender<BusEvent>,
    config: RwLock<EngineConfig>,
    /// auth token -> username (in-memory sessions)
    tokens: Mutex<HashMap<String, String>>,
}

impl Engine {
    /// Initialize the global engine and start the event pumps.
    pub fn init(cfg: EngineConfig) -> &'static Engine {
        let (tx, _rx) = broadcast::channel(8192);
        let engine = Engine {
            lister: AcpHandle::spawn(),
            global_status: Mutex::new(AcpStatus::Disconnected),
            global_error: Mutex::new(String::new()),
            state: Mutex::new(EngineState::default()),
            tx,
            config: RwLock::new(cfg),
            tokens: Mutex::new(HashMap::new()),
        };
        let e: &'static Engine = ENGINE.get_or_init(|| engine);
        e.start_lister_pump();
        e
    }

    pub fn global() -> &'static Engine {
        ENGINE.get().expect("engine not initialized")
    }

    pub fn try_global() -> Option<&'static Engine> {
        ENGINE.get()
    }

    /// The lister pump: only global status + backend session listings.
    fn start_lister_pump(&'static self) {
        std::thread::Builder::new()
            .name("vibedeck-lister-pump".into())
            .spawn(move || {
                let rx = self.lister.events();
                loop {
                    match rx.recv() {
                        Ok(ev) => {
                            let structural = match &ev {
                                AcpEvent::Initialized { .. } => {
                                    *self.global_status.lock().unwrap() = AcpStatus::Connected;
                                    self.global_error.lock().unwrap().clear();
                                    true
                                }
                                AcpEvent::Disconnected { .. } => {
                                    *self.global_status.lock().unwrap() =
                                        AcpStatus::Disconnected;
                                    true
                                }
                                AcpEvent::SessionsList(sessions) => {
                                    self.state.lock().unwrap().backend_sessions =
                                        sessions.clone();
                                    true
                                }
                                AcpEvent::SessionError { context, message } => {
                                    *self.global_status.lock().unwrap() = AcpStatus::Error;
                                    *self.global_error.lock().unwrap() =
                                        format!("{context}: {message}");
                                    true
                                }
                                _ => false,
                            };
                            let _ = self.tx.send(BusEvent::Acp(ev));
                            if structural {
                                let _ = self.tx.send(BusEvent::StructureChanged);
                            }
                        }
                        Err(_) => return,
                    }
                }
            })
            .expect("failed to spawn lister pump");
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

    /// Spawn the lister worker (when configured) and connect.
    pub fn start_agent(&self) {
        *self.global_status.lock().unwrap() = AcpStatus::Connecting;
        let acp_path = self.config().acp_path;
        if !acp_path.is_empty() {
            self.lister.send(AcpCommand::Start { acp_path });
        }
    }

    pub fn refresh_sessions(&self) {
        self.lister.send(AcpCommand::ListSessions);
    }

    // ---------------- slots ----------------

    /// Spawn a slot for the given local session and start its process.
    fn ensure_slot(&self, session_key: &str, purpose: SlotPurpose) -> Option<u64> {
        let acp_path = self.config().acp_path;
        if acp_path.is_empty() {
            return None;
        }
        let mut st = self.state.lock().unwrap();
        if let Some(slot_id) = st.slot_of_session(session_key) {
            return Some(slot_id);
        }
        let slot_id = st.next_slot;
        st.next_slot += 1;
        let mut slot = spawn_slot(slot_id, purpose);
        slot.status = AcpStatus::Connecting;
        let _ = slot.cmd.send(AcpCommand::Start { acp_path });
        st.slots.insert(slot_id, slot);
        st.slot_of.insert(session_key.to_string(), slot_id);
        Some(slot_id)
    }

    // ---------------- snapshot ----------------

    pub fn snapshot(&self) -> Snapshot {
        let st = self.state.lock().unwrap();
        let cfg = self.config();
        let current_slot = st
            .current_session()
            .and_then(|s| st.slot_of_session(&s.id))
            .and_then(|id| st.slots.get(&id));
        let slot_status = current_slot.map(|s| s.status);
        let slot_busy = current_slot.map(|s| s.busy).unwrap_or(false);
        let global = *self.global_status.lock().unwrap();
        Snapshot {
            status: slot_status.unwrap_or(global).as_str().to_string(),
            acp_error: current_slot
                .map(|s| s.error.clone())
                .unwrap_or_else(|| self.global_error.lock().unwrap().clone()),
            busy: slot_busy,
            current: st.current,
            sessions: st
                .sessions
                .iter()
                .map(|s| {
                    let (busy, status) = st
                        .slot_of_session(&s.id)
                        .and_then(|id| st.slots.get(&id))
                        .map(|slot| (slot.busy, slot.status.as_str().to_string()))
                        .unwrap_or((false, String::new()));
                    SessionMeta::from_session(s, busy, &status)
                })
                .collect(),
            current_messages: st
                .current_session()
                .map(|s| s.messages.clone())
                .unwrap_or_default(),
            modes: current_slot.map(|s| s.modes.clone()).unwrap_or_default(),
            config_options: current_slot
                .map(|s| s.config_options.clone())
                .unwrap_or_default(),
            commands: current_slot.map(|s| s.commands.clone()).unwrap_or_default(),
            backend_sessions: st.backend_sessions.clone(),
            usage: current_slot.map(|s| s.usage.clone()).unwrap_or_default(),
            approval: current_slot.and_then(|s| s.approval.clone()),
            question: current_slot.and_then(|s| s.question.clone()),
            working_directory: cfg.working_directory.clone(),
            preferred_model: cfg.preferred_model.clone(),
            config: UiConfig {
                language: cfg.language.clone(),
                theme: cfg.theme.clone(),
                acp_path: cfg.acp_path.clone(),
                working_directory: cfg.working_directory.clone(),
                remote_enabled: cfg.remote_enabled,
                remote_port: cfg.remote_port,
                auth_configured: cfg.auth.is_some(),
                dock_open: cfg.dock_open,
                sidebar_sessions_limit: cfg.sidebar_sessions_limit,
                hidden_sessions: cfg.hidden_sessions.clone(),
                hidden_projects: cfg.hidden_projects.clone(),
            },
        }
    }

    // ---------------- commands ----------------

    /// New local session: a cheap shell entry - the agent process spawns
    /// lazily with the first prompt (no process spam from "+" clicks).
    pub fn create_local_session(&self) {
        let cwd = {
            let st = self.state.lock().unwrap();
            st.current_session()
                .map(|s| s.working_directory.clone())
                .unwrap_or_else(|| self.config().working_directory.clone())
        };
        // Always create a fresh session: with parallel slots every local
        // session owns its agent process, so '+' must never silently reuse
        // (and switch away from) an existing empty session.
        {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new("New session", &cwd);
            session.working_directory = cwd;
            st.sessions.insert(0, session);
            st.current = Some(0);
        }
        self.publish(BusEvent::StructureChanged);
    }

    pub fn switch_session(&self, index: usize) {
        {
            let mut st = self.state.lock().unwrap();
            if index >= st.sessions.len() {
                return;
            }
            st.current = Some(index);
        }
        self.publish(BusEvent::StructureChanged);
    }

    /// Remove a session and stop its agent process.
    pub fn close_session(&self, index: usize) {
        {
            let mut st = self.state.lock().unwrap();
            if index >= st.sessions.len() {
                return;
            }
            let key = st.sessions[index].id.clone();
            if let Some(slot_id) = st.slot_of.remove(&key) {
                if let Some(slot) = st.slots.remove(&slot_id) {
                    // Stop the subprocess; the pump thread exits when
                    // the event channel closes.
                    let _ = slot.cmd.send(AcpCommand::Stop);
                }
            }
            st.sessions.remove(index);
            st.current = match st.current {
                Some(c) if c == index => Some(0usize.min(st.sessions.len().saturating_sub(1))),
                Some(c) if c > index => Some(c - 1),
                other => other,
            };
        }
        self.publish(BusEvent::StructureChanged);
    }

    /// Open (or switch to) a session from session/list - creates the local
    /// mirror with its own agent process and replays the CLI history.
    pub fn open_backend_session(&self, session_id: &str) {
        let info = {
            let mut st = self.state.lock().unwrap();
            if let Some(i) = st
                .sessions
                .iter()
                .position(|s| s.acp_session_id.as_deref() == Some(session_id))
            {
                st.current = Some(i);
                drop(st);
                self.publish(BusEvent::StructureChanged);
                return;
            }
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
        let key = {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new(
                if info.name.is_empty() { "CLI session" } else { &info.name },
                &cwd,
            );
            session.working_directory = cwd.clone();
            let key = session.id.clone();
            st.sessions.insert(0, session);
            st.current = Some(0);
            key
        };
        self.ensure_slot(
            &key,
            SlotPurpose::LoadBackend {
                session_id: info.session_id.clone(),
                cwd,
            },
        );
        self.publish(BusEvent::StructureChanged);
    }

    pub fn send_message(&self, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.config().acp_path.is_empty() {
            // No agent configured - tell the user instead of hanging.
            let mut st = self.state.lock().unwrap();
            if st.current.is_none() {
                let cwd = self.workdir();
                let mut session = ChatSession::new("New session", &cwd);
                session.working_directory = cwd;
                st.sessions.insert(0, session);
                st.current = Some(0);
            }
            if let Some(session) = st.current_session_mut() {
                session
                    .messages
                    .push(Message::new_system("Configure the ACP binary in Settings to talk to an agent."));
            }
            drop(st);
            self.publish(BusEvent::StructureChanged);
            return;
        }
        if self.state.lock().unwrap().current.is_none() {
            self.create_local_session();
        }
        let (key, acp_id, user_msg) = {
            let mut st = self.state.lock().unwrap();
            let Some(session) = st.current_session_mut() else { return };
            session.messages.push(Message::new_user(&text));
            session.last_active_ms = now_ms();
            (
                session.id.clone(),
                session.acp_session_id.clone(),
                session.messages.last().expect("just pushed").clone(),
            )
        };
        self.publish(BusEvent::UserMessageAdded {
            local_session_id: key.clone(),
            message: user_msg,
        });
        let mut st = self.state.lock().unwrap();
        match acp_id {
            Some(acp_session_id) => {
                if let Some(slot_id) = st.slot_of_session(&key) {
                    if let Some(slot) = st.slots.get_mut(&slot_id) {
                        slot.busy = true;
                        let _ = slot.cmd.send(AcpCommand::Prompt {
                            session_id: acp_session_id,
                            text,
                        });
                    }
                }
            }
            None => match st.slot_of_session(&key) {
                Some(slot_id) => {
                    if let Some(slot) = st.slots.get_mut(&slot_id) {
                        slot.busy = true;
                        slot.pending_prompt = Some(text);
                    }
                }
                None => {
                    // Lazily spawn this session's own agent process.
                    let cwd = st
                        .current_session()
                        .map(|s| s.working_directory.clone())
                        .unwrap_or_else(|| self.workdir());
                    drop(st);
                    self.ensure_slot(&key, SlotPurpose::NewLocal { cwd });
                    let mut st = self.state.lock().unwrap();
                    if let Some(slot_id) = st.slot_of_session(&key) {
                        if let Some(slot) = st.slots.get_mut(&slot_id) {
                            slot.busy = true;
                            slot.pending_prompt = Some(text);
                        }
                    }
                }
            },
        }
    }

    pub fn cancel(&self) {
        let st = self.state.lock().unwrap();
        let (key, acp) = {
            let Some(session) = st.current_session() else { return };
            (session.id.clone(), session.acp_session_id.clone())
        };
        let Some(acp_session_id) = acp else { return };
        let Some(slot_id) = st.slot_of_session(&key) else { return };
        if let Some(slot) = st.slots.get(&slot_id) {
            let _ = slot.cmd.send(AcpCommand::Cancel { session_id: acp_session_id });
        }
    }

    pub fn approve(&self, request_id: String, option_id: String) {
        let mut st = self.state.lock().unwrap();
        for (_, slot) in st.slots.iter_mut() {
            if slot.approval.as_ref().map(|a| &a.request_id) == Some(&request_id) {
                let _ = slot.cmd.send(AcpCommand::Approve {
                    request_id,
                    option_id,
                });
                slot.approval = None;
                break;
            }
        }
    }

    pub fn answer_question(&self, request_id: String, answer: String) {
        let mut st = self.state.lock().unwrap();
        for (_, slot) in st.slots.iter_mut() {
            if slot.question.as_ref().map(|q| &q.request_id) == Some(&request_id) {
                let _ = slot.cmd.send(AcpCommand::AnswerQuestion {
                    request_id: request_id.clone(),
                    answer,
                });
                slot.question = None;
                break;
            }
        }
    }

    /// Select a config option locally + on the session's agent.
    pub fn set_config_option(&self, id: &str, value: String) {
        if id == "model" {
            self.update_config(|c| c.preferred_model = Some(value.clone()));
        }
        let mut st = self.state.lock().unwrap();
        let Some(session) = st.current_session() else { return };
        let key = session.id.clone();
        let acp_id = session.acp_session_id.clone();
        let Some(slot_id) = st.slot_of_session(&key) else { return };
        let Some(slot) = st.slots.get_mut(&slot_id) else { return };
        if let Some(opt) = slot.config_options.iter_mut().find(|o| o.id == id) {
            opt.current_value = value.clone();
        }
        if let Some(session_id) = acp_id {
            let _ = slot.cmd.send(AcpCommand::SetConfigOption {
                session_id,
                key: id.to_string(),
                value,
            });
        }
    }

    // ---------------- auth ----------------

    pub fn auth_enabled(&self) -> bool {
        self.config().auth.is_some()
    }

    /// Notify every connected UI client that sessions/config changed.
    pub fn publish_structure(&self) {
        let _ = self.tx.send(BusEvent::StructureChanged);
    }

    /// Persist UI preferences (language / theme / dock / sidebar limit).
    pub fn set_prefs(&self, language: String, theme: String, dock_open: bool, sidebar_limit: u32) {
        self.update_config(|c| {
            c.language = language;
            c.theme = theme;
            c.dock_open = dock_open;
            c.sidebar_sessions_limit = if sidebar_limit > 0 {
                Some(sidebar_limit)
            } else {
                None
            };
        });
        self.publish_structure();
    }

    /// Configure remote-control access: login credentials + port + enabled.
    /// Credentials are stored as sha256(salt + password) - never in plain
    /// text, never sent back to any client.
    pub fn set_remote(&self, username: &str, password: &str, port: u16, enable: bool) {
        self.update_config(|c| {
            c.auth = Some(AuthEntry::new(username, password));
            c.remote_enabled = enable;
            c.remote_port = port;
        });
        self.publish_structure();
    }

    pub fn hide_session(&self, key: &str) {
        self.update_config(|c| {
            if !c.hidden_sessions.iter().any(|s| s == key) {
                c.hidden_sessions.push(key.to_string());
            }
        });
        // If this key matches a live local session, stop its agent process
        // so hidden sessions do not leak running vibe-acp workers.
        {
            let mut st = self.state.lock().unwrap();
            let matched = st
                .sessions
                .iter()
                .find(|s| s.id == key || s.acp_session_id.as_deref() == Some(key))
                .map(|s| s.id.clone());
            if let Some(sid) = matched {
                if let Some(slot_id) = st.slot_of.remove(&sid) {
                    if let Some(slot) = st.slots.remove(&slot_id) {
                        let _ = slot.cmd.send(AcpCommand::Stop);
                    }
                }
            }
        }
        self.publish_structure();
    }

    pub fn hide_project(&self, wd: &str) {
        self.update_config(|c| {
            if !c.hidden_projects.iter().any(|s| s == wd) {
                c.hidden_projects.push(wd.to_string());
            }
        });
        self.publish_structure();
    }

    pub fn restore_session(&self, key: &str) {
        self.update_config(|c| {
            c.hidden_sessions.retain(|s| s != key);
        });
        self.publish_structure();
    }

    pub fn restore_project(&self, wd: &str) {
        self.update_config(|c| {
            c.hidden_projects.retain(|s| s != wd);
        });
        self.publish_structure();
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

    // ---------------- state machine (per-slot) ----------------

    /// Apply one ACP worker event to the owning slot + session.
    pub(crate) fn apply_slot_event(&'static self, slot_id: u64, event: AcpEvent) {
        let event_out = event.clone();
        let mut guard = self.state.lock().unwrap();
        let st = &mut *guard;
        if !st.slots.contains_key(&slot_id) {
            return; // slot closed while events were in flight
        }
        let slot_session = st.session_of_slot(slot_id);
        let global_model = self.config().preferred_model.clone();
        {
            let Some(slot) = st.slots.get_mut(&slot_id) else { return };
            let _ = slot.usage.apply(&event);
            match event {
                AcpEvent::Initialized { .. } => {
                    slot.status = AcpStatus::Connected;
                    slot.error.clear();
                    match slot.purpose.clone() {
                        SlotPurpose::NewLocal { cwd } => {
                            let _ = slot.cmd.send(AcpCommand::NewSession {
                                cwd,
                                model: global_model,
                            });
                        }
                        SlotPurpose::LoadBackend { session_id, cwd } => {
                            slot.busy = true;
                            let _ = slot.cmd.send(AcpCommand::LoadSession {
                                session_id,
                                cwd,
                                model: global_model,
                            });
                        }
                    }
                }
                AcpEvent::Disconnected { .. } => {
                    slot.status = AcpStatus::Disconnected;
                    slot.busy = false;
                    slot.pending_prompt = None;
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get_mut(i) {
                            for msg in &mut session.messages {
                                if msg.sender == MessageSender::Ai {
                                    msg.is_complete = true;
                                }
                            }
                        }
                    }
                }
                AcpEvent::SessionsList(_) => { /* the lister owns session/list */ }
                AcpEvent::SessionReady {
                    session_id,
                    modes,
                    config_options,
                    title,
                } => {
                    slot.status = AcpStatus::Connected;
                    slot.error.clear();
                    slot.modes = modes;
                    slot.config_options = config_options;
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.acp_session_id = Some(session_id.clone());
                            if let Some(title) = title {
                                if !title.is_empty() {
                                    session.name = title;
                                }
                            }
                            session.last_active_ms = now_ms();
                        }
                    }
                    if let Some((key, value)) = slot.pending_option.take() {
                        let _ = slot.cmd.send(AcpCommand::SetConfigOption {
                            session_id: session_id.clone(),
                            key,
                            value,
                        });
                    }
                    match slot.pending_prompt.take() {
                        Some(text) => {
                            slot.busy = true;
                            let _ = slot
                                .cmd
                                .send(AcpCommand::Prompt { session_id, text });
                        }
                        None => {
                            slot.busy = false;
                        }
                    }
                }
                AcpEvent::SessionHistory {
                    session_id,
                    messages,
                } => {
                    let matches_purpose = matches!(
                        &slot.purpose,
                        SlotPurpose::LoadBackend { session_id: sid, .. } if *sid == session_id
                    );
                    if matches_purpose {
                        if let Some(i) = slot_session {
                            if let Some(session) = st.sessions.get_mut(i) {
                                session.messages = messages;
                                session.last_active_ms = now_ms();
                            }
                        }
                        slot.busy = false;
                    }
                }
                AcpEvent::UserMessageChunk { .. } => {}
                AcpEvent::SessionError { context, message } => {
                    slot.status = AcpStatus::Error;
                    slot.error = format!("{context}: {message}");
                    slot.busy = false;
                    slot.pending_prompt = None;
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get_mut(i) {
                            for msg in &mut session.messages {
                                if msg.sender == MessageSender::Ai {
                                    msg.is_complete = true;
                                }
                            }
                            session.messages.push(Message::new_system(&format!(
                                "WARNING {context}: {message}"
                            )));
                        }
                    }
                }
                AcpEvent::MessageChunk { session_id, text } => {
                    if let Some(session) = st
                        .sessions
                        .iter_mut()
                        .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                    {
                        let msg = ensure_ai_message(session);
                        msg.content.push_str(&text);
                    }
                }
                AcpEvent::ThinkingChunk { session_id, text } => {
                    if let Some(session) = st
                        .sessions
                        .iter_mut()
                        .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                    {
                        let msg = ensure_ai_message(session);
                        match msg.thinking.last_mut() {
                            Some(block) if !block.is_complete => block.content.push_str(&text),
                            _ => msg.thinking.push(vibedeck_core::types::ThinkingBlock {
                                id: "thinking".into(),
                                content: text,
                                is_complete: false,
                            }),
                        }
                    }
                }
                AcpEvent::ThinkingComplete { session_id } => {
                    if let Some(session) = st
                        .sessions
                        .iter_mut()
                        .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                    {
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
                    if let Some(session) = st
                        .sessions
                        .iter_mut()
                        .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                    {
                        ensure_ai_message(session).tool_calls.push(tool_call);
                    }
                }
                AcpEvent::ToolCallUpdated {
                    session_id,
                    tool_call_id,
                    status,
                    output,
                } => {
                    if let Some(session) = st
                        .sessions
                        .iter_mut()
                        .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                    {
                        for msg in session.messages.iter_mut().rev() {
                            for tc in &mut msg.tool_calls {
                                if tc.id == tool_call_id {
                                    tc.status = status;
                                    if let Some(ref o) = output {
                                        tc.output = Some(o.to_owned());
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
                    slot.approval = Some(PendingApproval {
                        request_id,
                        method,
                        params_json,
                        tool_name,
                    });
                }
                AcpEvent::AskUserQuestion {
                    request_id,
                    question,
                    choices,
                } => {
                    slot.question = Some(PendingQuestion {
                        request_id,
                        question,
                        choices,
                    });
                }
                AcpEvent::PlanUpdate {
                    session_id,
                    entries_json,
                } => {
                    let _ = (session_id, entries_json);
                }
                AcpEvent::AvailableCommands { commands, .. } => {
                    slot.commands = commands;
                }
                AcpEvent::PromptCompleted { session_id, .. } => {
                    slot.busy = false;
                    if let Some(session) = st
                        .sessions
                        .iter_mut()
                        .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                    {
                        for msg in &mut session.messages {
                            if msg.sender == MessageSender::Ai {
                                msg.is_complete = true;
                            }
                        }
                    }
                }
                AcpEvent::UsageUpdate { .. } => { /* handled above */ }
                AcpEvent::DebugLog(line) => {
                    let _ = line;
                }
            }
        }
        drop(guard);
        // Forward the raw event so streaming clients update in place.
        self.publish(BusEvent::Acp(event_out));
        // Anything structural (status, approvals, ready) -> refetch hint.
        self.publish(BusEvent::StructureChanged);
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
