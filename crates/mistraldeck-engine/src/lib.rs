//! Mistral Deck engine - the headless core between ACP workers and any UI
//! (desktop webview, remote browser, mobile). Every session that talks
//! to an agent owns a **dedicated vibe-acp process** (a "slot"), so any
//! number of sessions can stream in parallel. A separate lightweight
//! worker (the "lister") only runs `session/list` for the CLI history.
//! Every change is published on a broadcast bus so any number of UI
//! clients stays in sync.

pub mod config;
pub mod remote;
pub mod remote_vibe;
pub mod updates;
mod types;

pub use config::{random_hex, AuthEntry, EngineConfig};
pub use types::EngineState;
pub use mistraldeck_core::snapshot::{
    AcpStatus, BusEvent, PendingApproval, PendingQuestion, SessionLoadProgress, SessionMeta,
    Snapshot, UiConfig, UsageStats,
};

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, RwLock};
use tokio::sync::broadcast;

use mistraldeck_acp::AcpHandle;
use mistraldeck_core::proto::{AcpCommand, AcpEvent};
use mistraldeck_core::types::{
    now_ms, new_id, Attachment, ChatSession, Message, MessageSender, ThinkingBlock, ToolCall,
    ToolStatus,
};

use crate::types::{spawn_slot, SlotPurpose};

/// Sliding expiry of remote HTTPS session tokens (30 days, like Nexus).
const TOKEN_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

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
    /// auth token -> (username, expires) — remote HTTPS sessions
    /// minted by serve.rs; 30-day sliding expiry.
    tokens: Mutex<HashMap<String, (String, std::time::Instant)>>,
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
            .name("mistraldeck-lister-pump".into())
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
            self.lister.send(AcpCommand::Start { argv: vec![acp_path] });
        }
    }

    pub fn refresh_sessions(&self) {
        self.lister.send(AcpCommand::ListSessions);
    }

    /// Warm start: connect the agent and index CLI sessions up front, so
    /// the first snapshot request does not pay the discovery cost.
    pub fn prime(&self) {
        if !self.config().acp_path.is_empty() {
            self.start_agent();
        }
        self.refresh_sessions();
    }

    // ---------------- slots ----------------

    /// Spawn a slot for the given session and start its process. Local
    /// slots run `acp_path`; remote slots run the connection's ssh
    /// command with `bash -lc "exec vibe-acp"` (same JSON-RPC protocol).
    fn ensure_slot(&self, session_key: &str, purpose: SlotPurpose) -> Option<u64> {
        let argv = match &purpose {
            SlotPurpose::NewLocal { .. } | SlotPurpose::LoadBackend { .. } => {
                let acp_path = self.config().acp_path;
                if acp_path.is_empty() {
                    return None;
                }
                vec![acp_path]
            }
            SlotPurpose::NewRemote { conn_id, .. } | SlotPurpose::LoadRemote { conn_id, .. } => {
                let cfg = self.config();
                let conn = cfg.remote_connections.iter().find(|c| c.id == *conn_id)?;
                crate::remote::remote_argv(conn)
            }
        };
        let mut st = self.state.lock().unwrap();
        if let Some(slot_id) = st.slot_of_session(session_key) {
            return Some(slot_id);
        }
        let slot_id = st.next_slot;
        st.next_slot += 1;
        let mut slot = spawn_slot(slot_id, purpose);
        slot.status = AcpStatus::Connecting;
        let _ = slot.cmd.send(AcpCommand::Start { argv });
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
                    let mut meta = SessionMeta::from_session(s, busy, &status);
                    meta.stopping = st.stopping.contains(&s.id);
                    meta
                })
                .collect(),
            current_messages: st
                .current_session()
                .map(|s| s.messages.clone())
                .unwrap_or_default(),
            current_plan: st
                .current_session()
                .map(|s| s.plan.clone())
                .unwrap_or_default(),
            modes: current_slot.map(|s| s.modes.clone()).unwrap_or_default(),
            // Per-session config wins (survives switching without a slot);
            // the live slot fills in for sessions that never went ready.
            config_options: st
                .current_session()
                .map(|s| s.config_options.clone())
                .filter(|o| !o.is_empty())
                .or_else(|| current_slot.map(|s| s.config_options.clone()))
                .unwrap_or_default(),
            commands: current_slot.map(|s| s.commands.clone()).unwrap_or_default(),
            backend_sessions: st.backend_sessions.clone(),
            // The live slot wins; a lazy-resumed session (no slot yet,
            // preview read from disk) or one whose slot died keeps its
            // mirrored per-session usage (Nexus `session.usage`).
            usage: current_slot
                .map(|s| s.usage.clone())
                .or_else(|| st.current_session().map(|s| s.usage.clone()))
                .unwrap_or_default(),
            session_load_progress: current_slot.and_then(|s| s.load_progress.clone()),
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
                auto_close_idle_minutes: cfg.auto_close_idle_minutes,
                recent_folders: cfg.recent_folders.clone(),
                hidden_sessions: cfg.hidden_sessions.clone(),
                hidden_projects: cfg.hidden_projects.clone(),
                remote_connections: cfg.remote_connections.clone(),
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
        self.note_recent_folder(&cwd);
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
            // Nexus `restoreSessionChips`: a session without recorded
            // option lists (historical / never spawned this run) gets
            // display-only lists built from the vibe config of its
            // working directory + the recorded last model/mode/thinking.
            restore_session_chips(&mut st);
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
            st.stopping.remove(&key);
            st.current = match st.current {
                Some(c) if c == index => Some(0usize.min(st.sessions.len().saturating_sub(1))),
                Some(c) if c > index => Some(c - 1),
                other => other,
            };
        }
        self.publish(BusEvent::StructureChanged);
    }

    /// Open (or switch to) a session from session/list — lazy resume
    /// (Nexus §2.2): the transcript, plan, usage and last config are
    /// read straight from the unified store on disk; **no agent process
    /// is spawned**. The first prompt wakes the session via
    /// `session/load` (with the recorded config re-applied), which then
    /// replaces the preview with the full replayed history.
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
        self.note_recent_folder(&cwd);
        {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new(
                if info.name.is_empty() { "CLI session" } else { &info.name },
                &cwd,
            );
            session.working_directory = cwd.clone();
            // Lazy: the ACP id is known upfront, the agent process is not
            // spawned until a prompt (or config change) needs it.
            session.acp_session_id = Some(info.session_id.clone());
            // Disk preview (Nexus `previewSession`): transcript + plan +
            // usage + last config from the unified store — zero ACP.
            if let Some(preview) = mistraldeck_vibe::load_full_session_preview(&info.session_id) {
                session.messages = preview_to_messages(&preview.messages);
                session.plan = preview
                    .plan
                    .into_iter()
                    .map(|e| mistraldeck_core::types::PlanEntry {
                        content: e.content,
                        status: mistraldeck_core::types::PlanEntry::normalize_status(&e.status),
                        priority: e.priority,
                    })
                    .collect();
                if let Some(title) = preview.title.filter(|t| !t.is_empty()) {
                    session.name = title;
                }
                if let Some(cfg) = preview.config {
                    session.last_model = cfg.model;
                    session.last_mode = cfg.mode;
                    session.last_thinking = cfg.thinking;
                }
                session.usage = mistraldeck_core::snapshot::UsageStats {
                    prompt_tokens: preview.usage.prompt_tokens,
                    completion_tokens: preview.usage.completion_tokens,
                    cached_tokens: preview.usage.cached_tokens,
                    total_tokens: preview.usage.total_tokens,
                    context_used: preview.usage.context_used,
                    // The preview has no context window size (Nexus sets 0);
                    // a live usage_update fills it in once the agent runs.
                    context_size: 0.0,
                    // v0.13.5: the preview carries no wire cost - compute
                    // it from the local price table (Nexus loadSessionPreview)
                    // so the UsageBar shows $ before the session resumes.
                    cost: {
                        let prices = mistraldeck_vibe::config::model_prices();
                        if prices.is_empty() {
                            0.0
                        } else {
                            let model = session.last_model.as_deref();
                            mistraldeck_vibe::stats::session_cost_from_prices(
                                &prices,
                                model,
                                preview.usage.prompt_tokens,
                                preview.usage.cached_tokens,
                                preview.usage.completion_tokens,
                            )
                        }
                    },
                    ..Default::default()
                };
            }
            st.sessions.insert(0, session);
            st.current = Some(0);
            // Per-session chips for a session that never went live this
            // run (Nexus restoreSessionChips after previewSession).
            restore_session_chips(&mut st);
        }
        self.publish(BusEvent::StructureChanged);
    }

    // ---------------- SSH remote sessions (Nexus parity) ----------------

    /// Look up a saved remote connection by id.
    pub fn remote_connection(&self, conn_id: &str) -> Option<mistraldeck_core::snapshot::RemoteConnection> {
        self.config().remote_connections.into_iter().find(|c| c.id == conn_id)
    }

    /// Persist the full remote-connection list (Settings editor).
    pub fn set_remote_connections(
        &self,
        conns: Vec<mistraldeck_core::snapshot::RemoteConnection>,
    ) {
        self.update_config(|c| c.remote_connections = conns);
        self.publish_structure();
    }

    /// New session on an SSH remote: creates the local mirror bound to
    /// `conn_id` and spawns its agent process over ssh.
    pub fn create_remote_session(&self, conn_id: &str, location: &str) {
        if self.remote_connection(conn_id).is_none() {
            return;
        }
        let cwd = location.to_string();
        let label = location
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or("Remote session");
        let key = {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new(label, &cwd);
            session.working_directory = cwd.clone();
            session.remote_id = Some(conn_id.to_string());
            let key = session.id.clone();
            st.sessions.insert(0, session);
            st.current = Some(0);
            key
        };
        self.ensure_slot(
            &key,
            SlotPurpose::NewRemote {
                conn_id: conn_id.to_string(),
                cwd,
            },
        );
        self.publish(BusEvent::StructureChanged);
    }

    /// Open (or switch to) a CLI session discovered on an SSH remote:
    /// creates the local mirror and replays the remote history.
    pub fn open_remote_session(
        &self,
        conn_id: &str,
        session_id: &str,
        title: &str,
        cwd: &str,
    ) {
        if self.remote_connection(conn_id).is_none() {
            return;
        }
        {
            let mut st = self.state.lock().unwrap();
            if let Some(i) = st.sessions.iter().position(|s| {
                s.remote_id.as_deref() == Some(conn_id)
                    && s.acp_session_id.as_deref() == Some(session_id)
            }) {
                st.current = Some(i);
                drop(st);
                self.publish(BusEvent::StructureChanged);
                return;
            }
        }
        let cwd = if cwd.is_empty() {
            self.workdir()
        } else {
            cwd.to_string()
        };
        let name = if title.is_empty() { "Remote session" } else { title };
        // v0.9.0 lazy remote resume (Nexus `loadRemoteSession` ->
        // `previewSession`): the remote unified store is read over SSH and
        // NO vibe-acp is started on the remote host; the agent wakes on the
        // first prompt (send_message_with -> LoadRemote).
        let preview = self
            .remote_connection(conn_id)
            .and_then(|conn| crate::remote::load_remote_session_preview(&conn, session_id));
        {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new(name, &cwd);
            session.working_directory = cwd.clone();
            session.remote_id = Some(conn_id.to_string());
            // Lazy: the ACP id is known upfront, the remote agent process is
            // not spawned until a prompt (or config change) needs it.
            session.acp_session_id = Some(session_id.to_string());
            if let Some(preview) = preview {
                session.messages = preview_to_messages(&preview.messages);
                session.plan = preview
                    .plan
                    .into_iter()
                    .map(|e| mistraldeck_core::types::PlanEntry {
                        content: e.content,
                        status: mistraldeck_core::types::PlanEntry::normalize_status(&e.status),
                        priority: e.priority,
                    })
                    .collect();
                if let Some(t) = preview.title.filter(|t| !t.is_empty()) {
                    session.name = t;
                }
                if let Some(cfg) = preview.config {
                    session.last_model = cfg.model;
                    session.last_mode = cfg.mode;
                    session.last_thinking = cfg.thinking;
                }
                session.usage = mistraldeck_core::snapshot::UsageStats {
                    prompt_tokens: preview.usage.prompt_tokens,
                    completion_tokens: preview.usage.completion_tokens,
                    cached_tokens: preview.usage.cached_tokens,
                    total_tokens: preview.usage.total_tokens,
                    context_used: preview.usage.context_used,
                    // The preview has no context window size (Nexus sets 0);
                    // a live usage_update fills it in once the agent runs.
                    context_size: 0.0,
                    // v0.13.5 pre-resume cost: computed from the local
                    // price table (Nexus `loadSessionPreview`) so the
                    // UsageBar shows $ before the session resumes.
                    cost: {
                        let prices = mistraldeck_vibe::config::model_prices();
                        if prices.is_empty() {
                            0.0
                        } else {
                            let model = session.last_model.as_deref();
                            mistraldeck_vibe::stats::session_cost_from_prices(
                                &prices,
                                model,
                                preview.usage.prompt_tokens,
                                preview.usage.cached_tokens,
                                preview.usage.completion_tokens,
                            )
                        }
                    },
                    ..Default::default()
                };
            }
            st.sessions.insert(0, session);
            st.current = Some(0);
            // Per-session chips for a session that never went live this
            // run (Nexus restoreSessionChips after previewSession).
            restore_session_chips(&mut st);
        }
        self.publish(BusEvent::StructureChanged);
    }

    pub fn send_message(&self, text: String) {
        self.send_message_with(text, Vec::new());
    }

    /// Like [`send_message`], but stores `attachments` on the user
    /// message (the web `@marker` resolver expands file references into
    /// attachments before sending).
    pub fn send_message_with(&self, text: String, attachments: Vec<Attachment>) {
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
            if let Some(m) = session.messages.last_mut() {
                m.attachments = attachments;
            }
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
        // A new turn clears any stale stop request for the session
        // (Nexus markSessionBusy).
        st.stopping.remove(&key);
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
                } else {
                    // Lazy-resumed CLI session (Nexus §2.2): the first
                    // prompt wakes the agent — `session/load` replays
                    // the history, then the prompt goes. The recorded
                    // last config is re-applied inside the wake (Nexus
                    // `applyConfig`), before the prompt.
                    let cur = st.current_session().cloned();
                    let cwd = cur
                        .as_ref()
                        .map(|s| s.working_directory.clone())
                        .unwrap_or_else(|| self.workdir());
                    // A lazy remote mirror wakes its agent over SSH
                    // (LoadRemote); a local CLI session over the local
                    // binary (LoadBackend) - the same session/load wake
                    // either way, with the recorded config re-applied
                    // inside it.
                    let purpose = match cur.as_ref().and_then(|s| s.remote_id.clone()) {
                        Some(conn_id) => SlotPurpose::LoadRemote {
                            conn_id,
                            session_id: acp_session_id.clone(),
                            cwd,
                        },
                        None => SlotPurpose::LoadBackend {
                            session_id: acp_session_id.clone(),
                            cwd,
                        },
                    };
                    drop(st);
                    self.ensure_slot(&key, purpose);
                    let mut st = self.state.lock().unwrap();
                    if let Some(slot_id) = st.slot_of_session(&key) {
                        if let Some(slot) = st.slots.get_mut(&slot_id) {
                            slot.busy = true;
                            slot.pending_prompt = Some(text);
                            slot.load_progress = Some(mistraldeck_core::snapshot::SessionLoadProgress {
                                session_id: acp_session_id.clone(),
                                count: 0,
                            });
                        }
                    }
                    // lastModel/lastMode/lastThinking re-apply (Nexus
                    // `loadSession(…, applyConfig)` — v0.10.2 "config
                    // applied INSIDE session/load, before the prompt").
                    let pending: Vec<(String, String)> = st
                        .sessions
                        .iter()
                        .find(|s| s.id == key)
                        .map(|session| {
                            [
                                ("model", session.last_model.clone()),
                                ("mode", session.last_mode.clone()),
                                ("thinking", session.last_thinking.clone()),
                            ]
                            .into_iter()
                            .filter_map(|(k, v)| v.map(|v| (k.to_string(), v)))
                            .collect()
                        })
                        .unwrap_or_default();
                    if !pending.is_empty() {
                        st.pending_options.insert(key.clone(), pending);
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
        let mut st = self.state.lock().unwrap();
        let (key, acp) = {
            let Some(session) = st.current_session() else { return };
            (session.id.clone(), session.acp_session_id.clone())
        };
        let Some(acp_session_id) = acp else { return };
        let Some(slot_id) = st.slot_of_session(&key) else { return };
        let mut marked = false;
        if let Some(slot) = st.slots.get(&slot_id) {
            if slot
                .cmd
                .send(AcpCommand::Cancel { session_id: acp_session_id })
                .is_ok()
            {
                // Nexus v0.10.5 markSessionStopping: explicit stopping
                // feedback while the agent acknowledges session/cancel;
                // cleared when the turn really ends (or a new turn
                // starts — send_message_with).
                st.stopping.insert(key);
                marked = true;
            }
        }
        drop(st);
        if marked {
            self.publish_structure();
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

    /// Select a config option locally + on the session's agent. The value
    /// is mirrored on the ChatSession (chips survive slot-less switching
    /// and lazy respawns); sessions without a running agent queue the
    /// change until `SessionReady`.
    pub fn set_config_option(&self, id: &str, value: String) {
        if id == "model" {
            self.update_config(|c| c.preferred_model = Some(value.clone()));
        }
        let mut st = self.state.lock().unwrap();
        let Some(session) = st.current_session() else { return };
        let key = session.id.clone();
        let acp_id = session.acp_session_id.clone();
        // Mirror on the session first (works with no agent process).
        if let Some(session) = st.current_session_mut() {
            if let Some(opt) = session.config_options.iter_mut().find(|o| o.id == id) {
                opt.current_value = value.clone();
            }
            // Nexus handleConfigSelect: record the choice on the session
            // so switching back (and the next session/load) restores it.
            match id {
                "model" => session.last_model = Some(value.clone()),
                "mode" => session.last_mode = Some(value.clone()),
                "thinking" => session.last_thinking = Some(value.clone()),
                _ => {}
            }
        }
        match st.slot_of_session(&key) {
            Some(slot_id) => {
                if let Some(slot) = st.slots.get_mut(&slot_id) {
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
            }
            None => {
                // No agent process yet — apply on SessionReady. The wire
                // never notifies for set_config_option, so the local
                // mirror stays the source of truth for the UI.
                st.pending_options
                    .entry(key)
                    .or_default()
                    .push((id.to_string(), value));
            }
        }
        drop(st);
        // Chips in every connected client must see the new value.
        self.publish_structure();
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

    /// General settings (SettingsView general tab): acp path + the idle
    /// auto-close timeout in minutes (the idle reaper itself is element 8).
    pub fn set_general(&self, acp_path: String, auto_close_idle_minutes: u32) {
        self.update_config(|c| {
            c.acp_path = acp_path;
            c.auto_close_idle_minutes = if auto_close_idle_minutes > 0 {
                Some(auto_close_idle_minutes)
            } else {
                None
            };
        });
        self.publish_structure();
    }

    /// Disconnect the agent (Nexus acp.stop): stop the lister and every
    /// live slot. Sessions survive as lazy previews and wake again on
    /// the next prompt (lazy resume, Nexus 2.2).
    pub fn stop_agent(&self) {
        self.lister.send(AcpCommand::Stop);
        {
            let mut st = self.state.lock().unwrap();
            let slot_ids: Vec<u64> = st.slot_of.values().cloned().collect();
            for slot_id in slot_ids {
                if let Some(slot) = st.slots.remove(&slot_id) {
                    let _ = slot.cmd.send(AcpCommand::Stop);
                }
            }
            st.slot_of.clear();
        }
        *self.global_status.lock().unwrap() = AcpStatus::Disconnected;
        self.publish(BusEvent::StructureChanged);
    }

    /// Record a folder in the recent list (Nexus recentFolders, cap 10).
    pub fn note_recent_folder(&self, cwd: &str) {
        let cwd = cwd.trim();
        if cwd.is_empty() {
            return;
        }
        self.update_config(|c| {
            c.recent_folders.retain(|f| f != cwd);
            c.recent_folders.insert(0, cwd.to_string());
            c.recent_folders.truncate(10);
        });
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

    /// Verify credentials and mint a remote session token (None on a bad
    /// login). Tokens live in memory only and expire after 30 days of
    /// inactivity.
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
        self.tokens.lock().unwrap().insert(
            token.clone(),
            (username.to_string(), std::time::Instant::now() + TOKEN_TTL),
        );
        Some(token)
    }

    /// Valid, non-expired token? Refreshes the sliding expiry window.
    pub fn check_token(&self, token: &str) -> bool {
        let mut tokens = self.tokens.lock().unwrap();
        match tokens.get_mut(token) {
            Some(entry) if entry.1 > std::time::Instant::now() => {
                entry.1 = std::time::Instant::now() + TOKEN_TTL;
                true
            }
            Some(_) => {
                tokens.remove(token);
                false
            }
            None => false,
        }
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
        // The owning session's key — used to clear the stopping marker
        // (Nexus v0.10.5 stoppingSessions) when the turn really ends.
        let slot_key = slot_session
            .and_then(|i| st.sessions.get(i))
            .map(|s| s.id.clone());
        let global_model = self.config().preferred_model.clone();
        // Nexus §2.2 fallback: session/load failed — the pending prompt
        // moves to a fresh session in the same cwd (newSession + resend).
        let mut respawn: Option<(String, String)> = None;
        {
            let Some(slot) = st.slots.get_mut(&slot_id) else { return };
            let _ = slot.usage.apply(&event);
            // Mirror live usage onto the session (direct field access:
            // disjoint from the `slot` borrow of `st.slots`; runs before
            // the match partially moves `event`). The mirror survives
            // slot death and feeds the UsageBar of lazy-resumed sessions
            // whose preview usage came from disk (Nexus `session.usage`).
            if let AcpEvent::UsageUpdate { session_id, .. } = &event {
                if let Some(session) = st
                    .sessions
                    .iter_mut()
                    .find(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                {
                    let _ = session.usage.apply(&event);
                }
            }
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
                        SlotPurpose::NewRemote { cwd, .. } => {
                            let _ = slot.cmd.send(AcpCommand::NewSession {
                                cwd,
                                model: global_model,
                            });
                        }
                        SlotPurpose::LoadRemote { session_id, cwd, .. } => {
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
                    slot.load_progress = None;
                    // The turn will never end on its own — clear the
                    // stopping marker (Nexus markSessionIdle).
                    if let Some(k) = slot_key.as_deref() {
                        st.stopping.remove(k);
                    }
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
                    current_mode_id,
                } => {
                    slot.status = AcpStatus::Connected;
                    slot.error.clear();
                    // The wake finished: the resuming overlay disappears.
                    slot.load_progress = None;
                    slot.modes = modes;
                    slot.config_options = config_options.clone();
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.acp_session_id = Some(session_id.clone());
                            if let Some(title) = title {
                                if !title.is_empty() {
                                    session.name = title;
                                }
                            }
                            // Per-session config mirror: the chips keep their
                            // values across slot-less switching + respawns.
                            session.config_options = config_options.clone();
                            session.last_active_ms = now_ms();
                            // Nexus records lastModel/lastMode/lastThinking on
                            // every confirmed config-options payload
                            // (SessionInitialized / ConfigOptionsUpdate); the
                            // mode chip also honours `currentModeId`.
                            for opt in &config_options {
                                match opt.id.as_str() {
                                    "model" => {
                                        session.last_model = Some(opt.current_value.clone())
                                    }
                                    "mode" => session.last_mode =
                                        Some(opt.current_value.clone()),
                                    "thinking" => {
                                        session.last_thinking =
                                            Some(opt.current_value.clone())
                                    }
                                    _ => {}
                                }
                            }
                            if let Some(cm) = &current_mode_id {
                                if session.last_mode.is_none() {
                                    session.last_mode = Some(cm.clone());
                                }
                                if let Some(opt) = session
                                    .config_options
                                    .iter_mut()
                                    .find(|o| o.id == "mode" && o.current_value.is_empty())
                                {
                                    opt.current_value = cm.clone();
                                }
                            }
                        }
                    }
                    // Config options chosen before the agent was ready:
                    // apply them (in order) to the fresh session.
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get(i) {
                            let key = session.id.clone();
                            if let Some(pending) = st.pending_options.remove(&key) {
                                for (opt_id, opt_value) in pending {
                                    if let Some(opt) =
                                        slot.config_options.iter_mut().find(|o| o.id == opt_id)
                                    {
                                        opt.current_value = opt_value.clone();
                                    }
                                    let _ = slot.cmd.send(AcpCommand::SetConfigOption {
                                        session_id: session_id.clone(),
                                        key: opt_id,
                                        value: opt_value,
                                    });
                                }
                            }
                        }
                    }
                    if let Some((key, value)) = slot.pending_option.take() {
                        let _ = slot.cmd.send(AcpCommand::SetConfigOption {
                            session_id: session_id.clone(),
                            key,
                            value,
                        });
                    }
                    // The wake finished: a new turn may go out with the
                    // pending prompt (markSessionBusy) or the session is
                    // idle — either way any stale stopping marker goes.
                    if let Some(k) = slot_key.as_deref() {
                        st.stopping.remove(k);
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
                    plan,
                } => {
                    let matches_purpose = matches!(
                        &slot.purpose,
                        SlotPurpose::LoadBackend { session_id: sid, .. } if *sid == session_id
                    ) || matches!(
                        &slot.purpose,
                        SlotPurpose::LoadRemote { session_id: sid, .. } if *sid == session_id
                    );
                    if matches_purpose {
                        if let Some(i) = slot_session {
                            if let Some(session) = st.sessions.get_mut(i) {
                                session.messages = messages;
                                session.plan = plan;
                                session.last_active_ms = now_ms();
                            }
                        }
                        slot.busy = false;
                        if let Some(k) = slot_key.as_deref() {
                            st.stopping.remove(k);
                        }
                    }
                }
                AcpEvent::UserMessageChunk { .. } => {}
                // Replay progress while a lazy session wakes (Nexus
                // `SessionLoadProgress`, every 25 buffered updates).
                AcpEvent::SessionLoadProgress { session_id, count } => {
                    slot.load_progress = Some(SessionLoadProgress {
                        session_id,
                        count,
                    });
                }
                AcpEvent::SessionError { context, message } => {
                    slot.status = AcpStatus::Error;
                    slot.error = format!("{context}: {message}");
                    slot.busy = false;
                    slot.load_progress = None;
                    // The turn died — clear the stopping marker.
                    if let Some(k) = slot_key.as_deref() {
                        st.stopping.remove(k);
                    }
                    // Nexus §2.2: session/load failed (CLI session gone
                    // from disk) — silently fall back to a fresh session
                    // in the same cwd; the pending prompt resends there.
                    if context == "load_session" {
                        if let Some(prompt) = slot.pending_prompt.take() {
                            match &slot.purpose {
                                SlotPurpose::LoadBackend { cwd, .. }
                                | SlotPurpose::LoadRemote { cwd, .. } => {
                                    respawn = Some((prompt, cwd.clone()));
                                }
                                _ => {}
                            }
                        }
                    } else {
                        slot.pending_prompt = None;
                    }
                    if respawn.is_none() {
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
                            _ => msg.thinking.push(mistraldeck_core::types::ThinkingBlock {
                                id: "thinking".into(),
                                content: text,
                                is_complete: false,
                                started_at: mistraldeck_core::types::now_ms(),
                                completed_at: 0,
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
                            let now = mistraldeck_core::types::now_ms();
                            for block in &mut msg.thinking {
                                block.is_complete = true;
                                if block.completed_at == 0 {
                                    block.completed_at = now;
                                }
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
                    subagent,
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
                                    // v0.13: subagent meta merges.
                                    if let Some(sa) = &subagent {
                                        match &mut tc.subagent {
                                            Some(cur) => {
                                                if sa.agent.is_some() {
                                                    cur.agent = sa.agent.clone();
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
                    // Cumulative checklist state: the last update wins.
                    // Stored on the session so it survives switching.
                    let entries = mistraldeck_core::types::PlanEntry::parse_list(&entries_json);
                    let target = st
                        .sessions
                        .iter()
                        .position(|s| s.acp_session_id.as_deref() == Some(session_id.as_str()))
                        .or(slot_session);
                    if let Some(i) = target {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.plan = entries;
                            session.last_active_ms = now_ms();
                        }
                    }
                }
                AcpEvent::AvailableCommands { commands, .. } => {
                    slot.commands = commands;
                }
                AcpEvent::PromptCompleted { session_id, .. } => {
                    slot.busy = false;
                    // The turn really ended — clear the stopping marker
                    // (Nexus markSessionIdle).
                    if let Some(k) = slot_key.as_deref() {
                        st.stopping.remove(k);
                    }
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
                // Wildcard: must not move out of `event` (the usage
                // mirror borrows it after the match).
                AcpEvent::DebugLog(_) => {}
            }
        }
        drop(guard);
        // Nexus §2.2 fallback: the lazy wake failed — start a fresh
        // session in the same cwd and send the pending prompt there.
        if let Some((prompt, cwd)) = respawn {
            let key = {
                let mut st = self.state.lock().unwrap();
                let mut session = ChatSession::new("CLI session", &cwd);
                session.working_directory = cwd.clone();
                let key = session.id.clone();
                st.sessions.insert(0, session);
                st.current = Some(0);
                key
            };
            self.ensure_slot(&key, SlotPurpose::NewLocal { cwd });
            let mut st = self.state.lock().unwrap();
            if let Some(slot_id) = st.slot_of_session(&key) {
                if let Some(slot) = st.slots.get_mut(&slot_id) {
                    slot.busy = true;
                    slot.pending_prompt = Some(prompt);
                }
            }
        }
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

/// Convert a lazy-resume disk preview into chat messages (Nexus
/// `applySessionHistory`): user/ai text, thinking blocks and tool-call
/// lines — everything complete, nothing streaming.
fn preview_to_messages(messages: &[mistraldeck_vibe::PreviewMessage]) -> Vec<Message> {
    messages
        .iter()
        .map(|m| {
            let mut msg = if m.role == "user" {
                Message::new_user(&m.content)
            } else {
                let mut ai = Message::new_ai();
                ai.content = m.content.clone();
                ai
            };
            msg.is_complete = true;
            msg.thinking = m
                .thinking
                .iter()
                .map(|t| ThinkingBlock {
                    id: t.id.clone(),
                    content: t.content.clone(),
                    is_complete: true,
                    started_at: 0,
                    completed_at: 0,
                })
                .collect();
            msg.tool_calls = m
                .tool_calls
                .iter()
                .map(|tc| ToolCall {
                    id: new_id(),
                    title: tc.title.clone(),
                    kind: "tool".to_string(),
                    status: if tc.status == "error" {
                        ToolStatus::Error
                    } else {
                        ToolStatus::Done
                    },
                    input_json: None,
                    output: None,
                    file_changes: Vec::new(),
                    diffs: Vec::new(),
                    tool_name: String::new(),
                    previous_content: None,
                    raw_output_json: None,
                    subagent: None,
                    started_at: 0,
                })
                .collect();
            msg
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Per-session chips (Nexus restoreSessionChips / syncSessionChips)
// ---------------------------------------------------------------------------

fn capitalize(s: &str) -> String {
    let mut cs = s.chars();
    match cs.next() {
        Some(c) => c.to_uppercase().collect::<String>() + cs.as_str(),
        None => String::new(),
    }
}

/// Port of Nexus `restoreSessionChips` (+ the disk half of
/// `syncSessionChips`): a session without recorded option lists — a
/// historical session never spawned this run — gets display-only
/// model/mode/thinking lists built from the vibe config of its working
/// directory, with the recorded last values (falling back to the unified
/// store on disk, `generations/*/runtime-state.json`) as the current
/// values. A session that never went live never inherits the option
/// lists of the previously viewed session.
fn restore_session_chips(st: &mut EngineState) {
    use mistraldeck_core::proto::{ConfigOption, ConfigOptionChoice};
    let Some(i) = st.current else { return };
    let Some(session) = st.sessions.get(i) else { return };
    if !session.config_options.is_empty() {
        return; // live (or already restored) — keep the recorded lists
    }
    let acp_id = session.acp_session_id.clone();
    let wd = session.working_directory.clone();
    let mut model = session.last_model.clone();
    let mut mode = session.last_mode.clone();
    let mut thinking = session.last_thinking.clone();
    // Missing last* values come from the unified store on disk (Nexus
    // `vibeSessionLastConfig`: newest generation with an active_model).
    if model.is_none() || mode.is_none() || thinking.is_none() {
        if let Some(last) = acp_id
            .as_deref()
            .and_then(mistraldeck_vibe::load_session_last_config)
        {
            model = model.or(last.model);
            mode = mode.or(last.mode);
            thinking = thinking.or(last.thinking);
        }
    }

    // Model list: global config + project config of the session cwd,
    // merged by key (alias||name) — project entries override.
    let key_of = |m: &mistraldeck_vibe::ModelConfig| {
        m.alias.clone().unwrap_or_else(|| m.name.clone())
    };
    let mut models = mistraldeck_vibe::VibeConfigFile::load().models();
    if !wd.is_empty() {
        if let Some(p) = mistraldeck_vibe::find_project_config(std::path::Path::new(&wd)) {
            if let Ok(pcfg) = mistraldeck_vibe::VibeConfigFile::load_at(&p) {
                for pm in pcfg.models() {
                    match models.iter().position(|m| key_of(m) == key_of(&pm)) {
                        Some(idx) => models[idx] = pm,
                        None => models.push(pm),
                    }
                }
            }
        }
    }
    let cur_model = model
        .as_deref()
        .and_then(|lm| models.iter().find(|m| key_of(m) == lm || m.name == lm));

    // CLI defaults: the five levels; an explicitly empty set means "off".
    let levels: Vec<String> = cur_model
        .and_then(|m| m.thinking_levels.clone())
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| {
            ["off", "low", "medium", "high", "max"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    let mut thinking_current = thinking.clone().unwrap_or_default();
    if !levels.iter().any(|l| *l == thinking_current) {
        let dl = cur_model.and_then(|m| m.thinking.clone());
        thinking_current = dl
            .filter(|d| levels.iter().any(|l| *l == *d))
            .or_else(|| levels.iter().find(|l| *l == "high").cloned())
            .unwrap_or_else(|| levels.last().cloned().unwrap_or_default());
    }

    // Agents join the mode list (global + project, top-level only,
    // deduped by name — Nexus `agentNames` set).
    let mut agents = mistraldeck_vibe::AgentFile::load_all();
    if !wd.is_empty() {
        agents.extend(mistraldeck_vibe::AgentFile::load_all_in(
            &std::path::Path::new(&wd).join(".vibe").join("agents"),
        ));
    }
    let mut mode_choices: Vec<ConfigOptionChoice> = [
        ("ask", "Ask"),
        ("accept-edits", "Accept Edits"),
        ("smart-approve", "Smart Approve"),
        ("auto-approve", "Auto Approve"),
    ]
    .iter()
    .map(|(v, n)| ConfigOptionChoice {
        value: v.to_string(),
        name: n.to_string(),
        description: String::new(),
    })
    .collect();
    let mut seen = std::collections::HashSet::new();
    for a in &agents {
        if a.agent_type.as_deref() == Some("subagent") {
            continue;
        }
        if a.name.is_empty() || !seen.insert(a.name.clone()) {
            continue;
        }
        mode_choices.push(ConfigOptionChoice {
            value: a.name.clone(),
            name: a.display_name.clone().unwrap_or_else(|| a.name.clone()),
            description: String::new(),
        });
    }

    let options = vec![
        ConfigOption {
            id: "mode".to_string(),
            name: "Mode".to_string(),
            current_value: mode.clone().unwrap_or_else(|| "accept-edits".to_string()),
            choices: mode_choices,
        },
        ConfigOption {
            id: "model".to_string(),
            name: "Model".to_string(),
            current_value: model.clone().unwrap_or_default(),
            choices: models
                .iter()
                .map(|m| ConfigOptionChoice {
                    value: key_of(m),
                    name: m.display_name.clone().unwrap_or_else(|| m.name.clone()),
                    description: m.name.clone(),
                })
                .collect(),
        },
        ConfigOption {
            id: "thinking".to_string(),
            name: "Thinking".to_string(),
            current_value: thinking_current.clone(),
            choices: levels
                .iter()
                .map(|l| ConfigOptionChoice {
                    value: l.clone(),
                    name: capitalize(l),
                    description: String::new(),
                })
                .collect(),
        },
    ];

    let Some(session) = st.sessions.get_mut(i) else { return };
    if session.config_options.is_empty() {
        session.config_options = options;
        session.last_model = model;
        session.last_mode = mode;
        session.last_thinking = if thinking_current.is_empty() {
            thinking
        } else {
            Some(thinking_current)
        };
    }
}
