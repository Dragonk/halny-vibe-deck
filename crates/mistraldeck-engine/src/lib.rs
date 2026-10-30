//! Halny Vibe Deck engine - the headless core between agent workers and
//! any UI (desktop webview, remote browser, mobile). Every session that
//! talks to an agent owns a **dedicated vibe-app-server process** (a
//! "slot"), so any number of sessions can stream in parallel. A separate
//! lightweight worker (the "lister") only runs `session/list` for the
//! CLI history. Every change is published on a broadcast bus so any
//! number of UI clients stays in sync.

pub mod auth_browser;
pub mod config;
pub mod remote;
pub mod remote_vibe;
pub mod session_token;
pub mod systemd;
pub mod updates;

/// `/help` output (TUI parity): the slash commands this GUI executes
/// client-side, exactly like the official cli-rust does.
pub const SLASH_HELP: &str = "\
**Available commands** (executed locally, like the terminal UI):

- `/compact [instructions]` — summarize and compact the history
- `/clear`, `/new` — clear the conversation context
- `/rename <title>` — rename this session
- `/reload` — reload config, agents and skills from disk
- `/model <name>` — switch the model for this session
- `/thinking <level>` — set the thinking level (off/low/medium/high/xhigh)
- `/agent <name>` — switch the agent/mode for this session
- `/retry [instructions]` — continue the last interrupted response
- `/rewind <entry>` — rewind the session to a checkpoint (restores files)
- `/branch [entry]` — fork the session at a checkpoint into a new session
- `/status` — session statistics
- `/version` — app + CLI versions
- `/delete` — delete the current session
- `/help` — show this list
- `!<command>` — run a shell command in the session and show its output
- `/<skill>` — invoke a skill (resolved by the agent)
- `@<path>` — attach a file or folder to the prompt
";
mod types;

pub use auth_browser::{BrowserSignInFlow, PollOutcome};

pub use config::{random_hex, AuthEntry, EngineConfig};
pub use types::EngineState;
pub use mistraldeck_core::snapshot::{
    AgentStatus, BusEvent, PendingApproval, PendingQuestion, SessionLoadProgress, SessionMeta,
    Snapshot, UiConfig, UsageStats,
};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};
use tokio::sync::broadcast;

use mistraldeck_appsrv::ApsHandle;
use mistraldeck_core::proto::{AgentCommand, AgentEvent};
use mistraldeck_core::snapshot::SetupInstallState;
use mistraldeck_core::types::{
    now_ms, new_id, Attachment, ChatSession, Message, MessageSender, ThinkingBlock, ToolCall,
    ToolStatus,
};

use crate::types::{spawn_slot, SlotPurpose};

/// Hide the console window of a spawned helper on Windows
/// (CREATE_NO_WINDOW). The GUI build has `windows_subsystem = "windows"`,
/// so every child console process otherwise flashes a black terminal
/// window (update checks, curl, ssh, systemctl…). No-op elsewhere.
pub fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Sliding expiry of remote HTTPS session tokens (30 days, like Nexus).

static ENGINE: OnceLock<Engine> = OnceLock::new();

pub struct Engine {
    /// Lightweight worker for `session/list` (CLI history) + the global
    /// status shown when no session is open.
    lister: ApsHandle,
    global_status: Mutex<AgentStatus>,
    global_error: Mutex<String>,
    /// Detected agent binary (None = not found / not checked yet) and
    /// the install instructions surfaced while it is missing.
    detected_agent: Mutex<Option<PathBuf>>,
    agent_missing: Mutex<Option<String>>,
    /// Local dependency-install state (`AgentCommand::SetupInstall`).
    install: Mutex<SetupInstallState>,
    install_started: AtomicBool,
    state: Mutex<EngineState>,
    tx: broadcast::Sender<BusEvent>,
    config: RwLock<EngineConfig>,
    /// auth token -> (username, expires) — remote HTTPS sessions
    /// minted by serve.rs; 30-day sliding expiry.
    // --- L1: login & account (app-server global methods) ---
    /// Browser sign-in flow in progress (PKCE verifier lives here).
    signin_flow: Mutex<Option<auth_browser::BrowserSignInFlow>>,
    /// Last `setup/status` seed (drives the login gate + wizard).
    last_setup: Mutex<Option<mistraldeck_core::snapshot::SetupStatusView>>,
    /// Last `account/read` view (banner, rate limit, settings).
    last_account: Mutex<Option<mistraldeck_core::snapshot::AccountView>>,
    /// Last `identity/read` (whoami, settings).
    last_identity: Mutex<Option<mistraldeck_core::snapshot::IdentityView>>,
    /// Last `setup/store-credential` outcome (apikey login server-fn).
    last_store_outcome: Mutex<Option<(String, Option<String>)>>,
}

impl Engine {
    /// Initialize the global engine and start the event pumps.
    pub fn init(cfg: EngineConfig) -> &'static Engine {
        let (tx, _rx) = broadcast::channel(8192);
        let engine = Engine {
            lister: ApsHandle::spawn(),
            global_status: Mutex::new(AgentStatus::Disconnected),
            global_error: Mutex::new(String::new()),
            detected_agent: Mutex::new(None),
            agent_missing: Mutex::new(None),
            install: Mutex::new(SetupInstallState::default()),
            install_started: AtomicBool::new(false),
            state: Mutex::new(EngineState::default()),
            tx,
            config: RwLock::new(cfg),
            signin_flow: Mutex::new(None),
            last_setup: Mutex::new(None),
            last_account: Mutex::new(None),
            last_identity: Mutex::new(None),
            last_store_outcome: Mutex::new(None),
        };
        let e: &'static Engine = ENGINE.get_or_init(|| engine);
        e.start_lister_pump();
        e.start_idle_reaper();
        e
    }

    pub fn global() -> &'static Engine {
        ENGINE.get().expect("engine not initialized")
    }

    pub fn try_global() -> Option<&'static Engine> {
        ENGINE.get()
    }

    /// Background idle reaper: runs every 60 s (Nexus v0.9.0 interval).
    fn start_idle_reaper(&'static self) {
        std::thread::Builder::new()
            .name("mistraldeck-idle-reaper".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                self.reap_idle_sessions();
            })
            .ok();
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
                                AgentEvent::Initialized { .. } => {
                                    *self.global_status.lock().unwrap() = AgentStatus::Connected;
                                    self.global_error.lock().unwrap().clear();
                                    // L1: seed login/account state right
                                    // after the handshake (methods are
                                    // global — no session needed).
                                    self.setup_refresh();
                                    true
                                }
                                AgentEvent::Disconnected { .. } => {
                                    *self.global_status.lock().unwrap() =
                                        AgentStatus::Disconnected;
                                    true
                                }
                                AgentEvent::SessionsList(sessions) => {
                                    let mut st = self.state.lock().unwrap();
                                    // Live sessions pick up the CLI-generated
                                    // title here (session/list carries it) —
                                    // otherwise a fresh session keeps the
                                    // placeholder name until it is reopened.
                                    for info in sessions.iter() {
                                        if info.session_id.is_empty() || info.name.is_empty() {
                                            continue;
                                        }
                                        if let Some(s) = st.sessions.iter_mut().find(|s| {
                                            s.agent_session_id.as_deref()
                                                == Some(info.session_id.as_str())
                                        }) {
                                            s.name = info.name.clone();
                                            s.last_active_ms = now_ms();
                                        }
                                    }
                                    st.backend_sessions = sessions.clone();
                                    true
                                }
                                AgentEvent::SessionError { context, message } => {
                                    *self.global_status.lock().unwrap() = AgentStatus::Error;
                                    *self.global_error.lock().unwrap() =
                                        format!("{context}: {message}");
                                    true
                                }
                                // --- L1: setup / konto ---
                                AgentEvent::SetupStatusReady { status } => {
                                    *self.last_setup.lock().unwrap() = Some(status.clone());
                                    true
                                }
                                AgentEvent::SetupStoreOutcome { outcome, detail } => {
                                    *self.last_store_outcome.lock().unwrap() =
                                        Some((outcome.clone(), detail.clone()));
                                    // Chain: store-credential completed →
                                    // submit-choices (domknięcie onboarding).
                                    if outcome == "completed" {
                                        self.lister.send(AgentCommand::SetupSubmitChoices);
                                    }
                                    false
                                }
                                AgentEvent::SetupChoicesOutcome { outcome, .. } => {
                                    // Onboarding domknięty — odśwież status
                                    // (hasApiKey) + konto.
                                    if outcome == "completed" {
                                        self.setup_refresh();
                                    }
                                    true
                                }
                                AgentEvent::IdentityReady { identity } => {
                                    *self.last_identity.lock().unwrap() = Some(identity.clone());
                                    false
                                }
                                AgentEvent::AccountReady { account } => {
                                    *self.last_account.lock().unwrap() = Some(account.clone());
                                    true
                                }
                                _ => false,
                            };
                            let _ = self.tx.send(BusEvent::Agent(ev));
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

    /// The current session's (working_directory, remote_id) — server-side
    /// resolution of `@marker`s and chat path links must use the SESSION's
    /// root (remote sessions resolve on the host), not the global workdir.
    pub fn current_session_ctx(&self) -> (String, Option<String>) {
        let st = self.state.lock().unwrap();
        match st.current_session() {
            Some(s) if !s.working_directory.is_empty() => {
                (s.working_directory.clone(), s.remote_id.clone())
            }
            _ => (self.workdir(), None),
        }
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

    /// Resolve the agent binary: the explicit config override first,
    /// then auto-detection (cached so prompts and snapshots stay cheap).
    fn resolve_agent_path(&self) -> Option<PathBuf> {
        let explicit = self.config().agent_path;
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
        {
            let cached = self.detected_agent.lock().unwrap();
            if cached.is_some() {
                return cached.clone();
            }
        }
        let detected = mistraldeck_setup::detect_appserver();
        *self.detected_agent.lock().unwrap() = detected.clone();
        detected
    }

    /// Spawn the lister worker and connect. When `vibe-app-server` is
    /// not found, no Start goes out — the missing-agent state (with
    /// install instructions) is published instead.
    pub fn start_agent(&self) {
        match self.resolve_agent_path() {
            Some(path) => {
                *self.agent_missing.lock().unwrap() = None;
                *self.global_status.lock().unwrap() = AgentStatus::Connecting;
                self.lister.send(AgentCommand::Start {
                    argv: vec![path.display().to_string()],
                });
            }
            None => {
                let instructions = mistraldeck_setup::install_instructions().join("\n");
                *self.global_status.lock().unwrap() = AgentStatus::Error;
                *self.agent_missing.lock().unwrap() = Some(instructions);
            }
        }
        self.publish_structure();
    }

    /// Index CLI sessions through the agent (`session/list`). Explicit user
    /// actions only: this is the single place that starts the sessionless
    /// lister worker, so a cold start never spawns an agent process (Z5).
    pub fn refresh_sessions(&self) {
        self.ensure_lister();
        self.lister.send(AgentCommand::ListSessions);
    }

    /// Boot warm-up without the agent: the sidebar history comes from the
    /// on-disk unified store (`session/list` would need a live handshake).
    pub fn prime(&self) {
        self.refresh_sessions_from_disk();
    }

    /// Populate `backend_sessions` from the local session store. No agent
    /// process, no handshake — the history list is readable before the user
    /// sends anything.
    pub fn refresh_sessions_from_disk(&self) {
        let mut infos: Vec<mistraldeck_core::proto::AgentSessionInfo> =
            mistraldeck_vibe::stats::load_sessions()
                .into_iter()
                .filter(|s| !s.session_id.is_empty())
                .map(|s| {
                    // Round 5 (R2): a session the CLI has not titled yet
                    // still gets a real name on the list — index title,
                    // else the first user prompt — instead of "untitled".
                    let mut name = s.title.clone().unwrap_or_default();
                    if name.trim().is_empty() {
                        name = mistraldeck_vibe::stats::index_title(&s.session_id)
                            .unwrap_or_default();
                    }
                    if name.trim().is_empty() {
                        name = mistraldeck_vibe::stats::index_preview(&s.session_id)
                            .unwrap_or_default();
                    }
                    if name.len() > 80 {
                        name = format!("{}…", &name[..name.len().min(80)]);
                    }
                    mistraldeck_core::proto::AgentSessionInfo {
                        session_id: s.session_id.clone(),
                        name,
                        cwd: s
                            .environment
                            .as_ref()
                            .and_then(|e| e.working_directory.clone())
                            .unwrap_or_default(),
                        start_time: s.start_time.clone(),
                        pinned_at: None,
                        archived_at: None,
                        is_unseen: false,
                        worktree: None,
                    }
                })
                .collect();
        // Newest first (ISO-8601 timestamps sort lexicographically).
        infos.sort_by(|a, b| b.start_time.cmp(&a.start_time));
        if let Ok(mut st) = self.state.lock() {
            st.backend_sessions = infos;
        }
        self.publish_structure();
    }

    /// Start the sessionless lister worker once (late, on demand).
    fn ensure_lister(&self) {
        let running = matches!(
            *self.global_status.lock().unwrap(),
            AgentStatus::Connected | AgentStatus::Connecting
        );
        if !running {
            self.start_agent();
        }
    }

    // ---------------- slots ----------------

    /// Spawn a slot for the given session and start its process. Local
    /// slots run the detected `vibe-app-server`; remote slots run the
    /// connection's ssh command with `bash -lc "exec vibe-app-server"`
    /// (same JSON-RPC protocol).
    fn ensure_slot(&self, session_key: &str, purpose: SlotPurpose) -> Option<u64> {
        mistraldeck_appsrv::trace(format!("ensure_slot key={session_key} purpose={purpose:?}"));
        let argv = match &purpose {
            SlotPurpose::NewLocal { .. } | SlotPurpose::LoadBackend { .. } => {
                // Missing agent: no Start, publish the install state instead.
                let Some(path) = self.resolve_agent_path() else {
                    *self.agent_missing.lock().unwrap() =
                        Some(mistraldeck_setup::install_instructions().join("\n"));
                    *self.global_status.lock().unwrap() = AgentStatus::Error;
                    self.publish_structure();
                    return None;
                };
                vec![path.display().to_string()]
            }
            SlotPurpose::NewRemote { conn_id, .. } | SlotPurpose::LoadRemote { conn_id, .. } => {
                let cfg = self.config();
                let conn = cfg.remote_connections.iter().find(|c| c.id == *conn_id)?;
                crate::remote::remote_argv(conn)
            }
        };
        let mut st = self.state.lock().unwrap();
        if let Some(slot_id) = reuse_slot_or_cleanup(&mut st, session_key) {
            mistraldeck_appsrv::trace(format!("ensure_slot reused slot={slot_id} for {session_key}"));
            return Some(slot_id);
        }
        let slot_id = st.next_slot;
        st.next_slot += 1;
        let mut slot = spawn_slot(slot_id, purpose);
        slot.status = AgentStatus::Connecting;
        let _ = slot.cmd.send(AgentCommand::Start { argv });
        st.slots.insert(slot_id, slot);
        st.slot_of.insert(session_key.to_string(), slot_id);
        mistraldeck_appsrv::trace(format!("ensure_slot spawned slot={slot_id} for {session_key}"));
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
            agent_error: current_slot
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
            // Round 7 (R13): a brand-new session (no agent process yet) must
            // still show enabled chips with the agent defaults — derive them
            // from the config files so model/mode/thinking can be changed
            // BEFORE the first prompt.
            config_options: st
                .current_session()
                .map(|s| s.config_options.clone())
                .filter(|o| !o.is_empty())
                .or_else(|| current_slot.map(|s| s.config_options.clone()))
                .filter(|o| !o.is_empty())
                .unwrap_or_else(|| {
                    let (wd, model, mode, thinking) = match st.current_session() {
                        Some(s) => (
                            s.working_directory.clone(),
                            s.last_model.clone().or_else(|| cfg.preferred_model.clone()),
                            s.last_mode.clone(),
                            s.last_thinking.clone(),
                        ),
                        None => (cfg.working_directory.clone(), cfg.preferred_model.clone(), None, None),
                    };
                    build_chip_options(&wd, model, mode, thinking).0
                }),
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
            ai_stream_msg_id: st.current_session().and_then(|s| s.stream_msg_id.clone()),
            approval: current_slot.and_then(|s| s.approval.clone()),
            question: current_slot.and_then(|s| s.question.clone()),
            working_directory: cfg.working_directory.clone(),
            preferred_model: cfg.preferred_model.clone(),
            config: UiConfig {
                language: cfg.language.clone(),
                theme: cfg.theme.clone(),
                agent_path: cfg.agent_path.clone(),
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
            account: self.last_account.lock().unwrap().clone(),
            setup: self.last_setup.lock().unwrap().clone(),
            identity: self.last_identity.lock().unwrap().clone(),
            agent_missing: self.agent_missing.lock().unwrap().clone(),
            agent_detected_path: self
                .detected_agent
                .lock()
                .unwrap()
                .as_ref()
                .map(|p| p.display().to_string()),
            setup_install: self.install.lock().unwrap().clone(),
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
                .filter(|wd| !wd.is_empty())
                .unwrap_or_else(|| self.workdir())
        };
        self.create_local_session_in(&cwd);
    }

    /// New local session **in a project** (D1/D2): the working directory is
    /// mandatory, so a session can never exist without a project. Still a
    /// cheap shell entry - the agent process spawns with the first prompt.
    pub fn create_local_session_in(&self, cwd: &str) {
        let cwd = cwd.trim();
        if cwd.is_empty() {
            return;
        }
        let cwd = cwd.to_string();
        self.note_recent_folder(&cwd);
        // Always create a fresh session: with parallel slots every local
        // session owns its agent process, so '+' must never silently reuse
        // (and switch away from) an existing empty session.
        {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new("New session", &cwd);
            session.working_directory = cwd;
            // Round 7 (R13): config chosen on a fresh chat (no session yet)
            // applies to this new session once its agent is ready.
            if let Some(pending) = st.pending_options.remove("") {
                let key = session.id.clone();
                for (k, v) in &pending {
                    match k.as_str() {
                        "model" => session.last_model = Some(v.clone()),
                        "mode" => session.last_mode = Some(v.clone()),
                        "thinking" => session.last_thinking = Some(v.clone()),
                        _ => {}
                    }
                }
                st.pending_options.insert(key, pending);
            }
            st.sessions.insert(0, session);
            st.current = Some(0);
            // A fresh session must offer the model/mode/thinking pickers
            // immediately (no agent process yet): build the display-only
            // chip lists from the project's vibe config, like the lazy
            // previews do (Nexus restoreSessionChips on a new session).
            restore_session_chips(&mut st);
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
                    let _ = slot.cmd.send(AgentCommand::Stop);
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
                .position(|s| s.agent_session_id.as_deref() == Some(session_id))
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
            // Lazy: the agent id is known upfront, the agent process is not
            // spawned until a prompt (or config change) needs it.
            session.agent_session_id = Some(info.session_id.clone());
            // Disk preview (Nexus `previewSession`): transcript + plan +
            // usage + last config from the unified store — no transport calls.
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
                } else if session.name == "CLI session" {
                    // No stored title: derive one from the first user prompt
                    // (stable shows real names, never a placeholder).
                    let first_user = preview
                        .messages
                        .iter()
                        .find(|m| m.role == "user")
                        .and_then(|m| m.content.trim().lines().next().map(|l| l.trim().to_string()))
                        .filter(|l| !l.is_empty());
                    if let Some(l) = first_user {
                        let snippet: String = l.chars().take(48).collect();
                        session.name = if l.chars().count() > 48 {
                            format!("{snippet}…")
                        } else {
                            snippet
                        };
                    }
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
                // v1.99.9: the CLI excludes subagent child-session tokens
                // from the parent's usage — roll them in so the UsageBar
                // shows the full cache in/out of the session tree.
                let sub = mistraldeck_vibe::stats::subagent_usage_totals(&info.session_id);
                session.usage.prompt_tokens += sub.usage.prompt_tokens;
                session.usage.completion_tokens += sub.usage.completion_tokens;
                session.usage.cached_tokens += sub.usage.cached_tokens;
                session.usage.total_tokens += sub.usage.total_tokens;
                session.usage.cost += sub.cost;
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
                    && s.agent_session_id.as_deref() == Some(session_id)
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
        // NO agent process is started on the remote host; it wakes on the
        // first prompt (send_message_with -> LoadRemote).
        let preview = self
            .remote_connection(conn_id)
            .and_then(|conn| crate::remote::load_remote_session_preview(&conn, session_id));
        {
            let mut st = self.state.lock().unwrap();
            let mut session = ChatSession::new(name, &cwd);
            session.working_directory = cwd.clone();
            session.remote_id = Some(conn_id.to_string());
            // Lazy: the agent id is known upfront, the remote agent process is
            // not spawned until a prompt (or config change) needs it.
            session.agent_session_id = Some(session_id.to_string());
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
        // round 8: a remote (SSH) session spawns the agent on the HOST —
        // requiring a local vibe binary here made pure-SSH usage fail with
        // "Mistral Vibe CLI not found" before the spawn branch was reached
        let current_is_remote = {
            let st = self.state.lock().unwrap();
            st.current
                .and_then(|i| st.sessions.get(i))
                .map(|s| s.remote_id.is_some())
                .unwrap_or(false)
        };
        if !current_is_remote && self.resolve_agent_path().is_none() {
            // No agent binary - tell the user instead of hanging.
            let instructions = mistraldeck_setup::install_instructions().join("\n");
            *self.agent_missing.lock().unwrap() = Some(instructions.clone());
            *self.global_status.lock().unwrap() = AgentStatus::Error;
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
                    .push(Message::new_system(&format!(
                        "Mistral Vibe CLI not found - install it to talk to an agent.\n{instructions}"
                    )));
            }
            drop(st);
            self.publish(BusEvent::StructureChanged);
            return;
        }
        if self.state.lock().unwrap().current.is_none() {
            self.create_local_session();
        }
        mistraldeck_appsrv::trace("send_message_with start");
        let (key, agent_id, user_msg) = {
            let mut st = self.state.lock().unwrap();
            let Some(session) = st.current_session_mut() else { return };
            session.messages.push(Message::new_user(&text));
            // v0.13.7: a new user turn resets the segment split state
            // (Nexus clears `streamingState` per turn).
            session.stream_msg_id = None;
            if let Some(m) = session.messages.last_mut() {
                m.attachments = attachments;
            }
            session.last_active_ms = now_ms();
            (
                session.id.clone(),
                session.agent_session_id.clone(),
                session.messages.last().expect("just pushed").clone(),
            )
        };
        // v1.0.8: image attachments travel as agent-transport image content blocks.
        let images: Vec<mistraldeck_core::proto::PromptImage> = user_msg
            .attachments
            .iter()
            .filter(|a| a.mime.starts_with("image/") && !a.data_base64.is_empty())
            .map(|a| mistraldeck_core::proto::PromptImage {
                data_base64: a.data_base64.clone(),
                mime: a.mime.clone(),
            })
            .collect();
        self.publish(BusEvent::UserMessageAdded {
            local_session_id: key.clone(),
            message: user_msg,
        });
        let mut st = self.state.lock().unwrap();
        // A new turn clears any stale stop request for the session
        // (Nexus markSessionBusy).
        st.stopping.remove(&key);
        match agent_id {
            Some(agent_session_id) => {
                if let Some(slot_id) = st.slot_of_session(&key) {
                    if let Some(slot) = st.slots.get_mut(&slot_id) {
                        if slot.busy {
                            // Round 4 (E6/E7): the agent is mid-turn — queue
                            // the prompt server-side instead of firing it
                            // now; it goes out when the turn completes
                            // (pump_slot_queue on PromptCompleted).
                            slot.queue.push_back((text.clone(), images.clone()));
                            slot.last_activity = std::time::Instant::now();
                        } else {
                            slot.busy = true;
                            slot.last_activity = std::time::Instant::now();
                            // v0.13.1: a failed prompt send unblocks the chat.
                            if slot
                                .cmd
                                .send(AgentCommand::Prompt {
                                    session_id: agent_session_id,
                                    text,
                                    images,
                                })
                                .is_err()
                            {
                                mark_prompt_failed(slot);
                            }
                        }
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
                            session_id: agent_session_id.clone(),
                            cwd,
                        },
                        None => SlotPurpose::LoadBackend {
                            session_id: agent_session_id.clone(),
                            cwd,
                        },
                    };
                    drop(st);
                    self.ensure_slot(&key, purpose);
                    let mut st = self.state.lock().unwrap();
                    if let Some(slot_id) = st.slot_of_session(&key) {
                        if let Some(slot) = st.slots.get_mut(&slot_id) {
                            slot.busy = true;
                            slot.last_activity = std::time::Instant::now();
                            // v0.10.2: fresh stream after a reload — drop
                            // stale dedup prefixes for this session (Nexus
                            // `loadSession` cleanup).
                            slot.stream_sent.retain(|k, _| {
                                !k.starts_with(&format!("{agent_session_id}|"))
                            });
                            slot.pending_prompt = Some(text);
                            slot.pending_images = images;
                            slot.load_progress = Some(mistraldeck_core::snapshot::SessionLoadProgress {
                                session_id: agent_session_id.clone(),
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
                            .filter(|(_, v)| !v.trim().is_empty())
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
                        slot.last_activity = std::time::Instant::now();
                        slot.pending_prompt = Some(text);
                        slot.pending_images = images.clone();
                    }
                }
                None => {
                    // Lazily spawn this session's own agent process.
                    let cur = st.current_session().cloned();
                    let cwd = cur
                        .as_ref()
                        .map(|s| s.working_directory.clone())
                        .unwrap_or_else(|| self.workdir());
                    // Round 7 (R14): a remote session mirror (SSH) must wake
                    // its agent OVER SSH — never spawn a local process in a
                    // remote path.
                    let purpose = match cur.as_ref().and_then(|s| s.remote_id.clone()) {
                        Some(conn_id) => SlotPurpose::NewRemote { conn_id, cwd: cwd.clone() },
                        None => SlotPurpose::NewLocal { cwd: cwd.clone() },
                    };
                    mistraldeck_appsrv::trace(format!(
                        "send_message_with: no slot for {key} (cwd={cwd}) -> spawn {purpose:?}"
                    ));
                    drop(st);
                    self.ensure_slot(&key, purpose);
                    let mut st = self.state.lock().unwrap();
                    if let Some(slot_id) = st.slot_of_session(&key) {
                        if let Some(slot) = st.slots.get_mut(&slot_id) {
                            if slot.busy {
                                // Round 4 (E6/E7): busy wake — queue behind it.
                                slot.queue.push_back((text, images));
                            } else {
                                slot.busy = true;
                                slot.last_activity = std::time::Instant::now();
                                slot.pending_prompt = Some(text);
                                slot.pending_images = images;
                            }
                        }
                    }
                }
            },
        }
        // Round 4 (E8): publish the busy flip so every client refetches the
        // snapshot immediately — the send button must not stay active while
        // the agent is already streaming.
        self.publish(BusEvent::StructureChanged);
    }

    pub fn cancel(&self) {
        let mut st = self.state.lock().unwrap();
        let (key, agent_id) = {
            let Some(session) = st.current_session() else { return };
            (session.id.clone(), session.agent_session_id.clone())
        };
        let Some(agent_session_id) = agent_id else { return };
        let Some(slot_id) = st.slot_of_session(&key) else { return };
        let mut marked = false;
        if let Some(slot) = st.slots.get(&slot_id) {
            if slot
                .cmd
                .send(AgentCommand::Cancel { session_id: agent_session_id })
                .is_ok()
            {
                // Nexus v0.10.5 markSessionStopping: explicit stopping
                // feedback while the agent acknowledges session/cancel;
                // cleared when the turn really ends (or a new turn
                // starts — send_message_with).
                st.stopping.insert(key.clone());
                marked = true;
            }
        }
        drop(st);
        if marked {
            self.publish_structure();
            // Nexus 2.6: when the agent never acknowledges the cancel
            // within 4 s, force-release the session and stop the hung
            // worker (it revives as a lazy preview on the next prompt).
            std::thread::Builder::new()
                .name("mistraldeck-cancel-watchdog".into())
                .spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(4));
                    if let Some(e) = crate::Engine::try_global() {
                        e.force_release_if_stopping(key);
                    }
                })
                .ok();
        }
    }

    /// v1.0.5: true while any session's turn is in flight — app/CLI
    /// self-updates must be refused, because the relaunch would kill
    /// live worker processes.
    pub fn any_turn_in_flight(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.slots.values().any(|s| s.busy)
    }

    pub fn approve(&self, request_id: String, option_id: String) {
        let mut st = self.state.lock().unwrap();
        for (_, slot) in st.slots.iter_mut() {
            if slot.approval.as_ref().map(|a| &a.request_id) == Some(&request_id) {
                let _ = slot.cmd.send(AgentCommand::Approve {
                    request_id,
                    option_id,
                });
                slot.approval = None;
                // round 8: publish so the overlay disappears immediately
                // instead of waiting for the next agent event
                drop(st);
                self.publish(BusEvent::StructureChanged);
                return;
            }
        }
    }

    pub fn answer_question(&self, request_id: String, answer: String) {
        let mut st = self.state.lock().unwrap();
        for (_, slot) in st.slots.iter_mut() {
            if slot.question.as_ref().map(|q| &q.request_id) == Some(&request_id) {
                let _ = slot.cmd.send(AgentCommand::AnswerQuestion {
                    request_id: request_id.clone(),
                    answer,
                });
                slot.question = None;
                // round 8: publish so the overlay disappears immediately
                drop(st);
                self.publish(BusEvent::StructureChanged);
                return;
            }
        }
    }

    /// Send a session-scoped command to the current session's agent slot.
    /// Returns false when no agent process is live for the session.
    fn agent_cmd_slot(&self, f: impl FnOnce(&str) -> AgentCommand) -> bool {
        let st = self.state.lock().unwrap();
        let Some(session) = st.current_session() else {
            return false;
        };
        let key = session.id.clone();
        let Some(agent_id) = session.agent_session_id.clone() else {
            return false;
        };
        let Some(slot_id) = st.slot_of_session(&key) else {
            return false;
        };
        let Some(slot) = st.slots.get(&slot_id) else {
            return false;
        };
        let _ = slot.cmd.send(f(&agent_id));
        true
    }

    /// TUI `/compact [instructions]` → `session/compact`.
    pub fn session_compact(&self, extra: String) {
        if self.agent_cmd_slot(|sid| AgentCommand::SessionCompact {
            session_id: sid.to_string(),
            extra_instructions: extra,
        }) {
            self.local_notice("Compacting conversation…");
        }
    }

    /// TUI `/clear`, `/new` → `session/history/clear`.
    pub fn history_clear(&self) {
        if self.agent_cmd_slot(|sid| AgentCommand::HistoryClear {
            session_id: sid.to_string(),
        }) {
            self.local_notice("Clearing conversation history…");
        }
    }

    /// TUI `/rename <title>` → `session/rename`.
    pub fn rename_session(&self, title: String) {
        if self.agent_cmd_slot(|sid| AgentCommand::SessionRename {
            session_id: sid.to_string(),
            title: title.clone(),
        }) {
            self.local_notice(&format!("Renaming session to “{title}”…"));
        }
    }

    /// TUI `!cmd` → `session/shellCommand` (the output streams back as
    /// history entries).
    pub fn shell_command(&self, command: String) {
        if self.agent_cmd_slot(|sid| AgentCommand::ShellCommand {
            session_id: sid.to_string(),
            command: command.clone(),
        }) {
            self.local_notice(&format!("Running `!{command}`…"));
        }
    }

    /// TUI `/reload` → `config/reload` (global method — any live slot).
    pub fn config_reload(&self) {
        let st = self.state.lock().unwrap();
        let sent = if let Some(session) = st.current_session() {
            let key = session.id.clone();
            st.slot_of_session(&key)
                .and_then(|slot_id| st.slots.get(&slot_id))
                .map(|slot| slot.cmd.send(AgentCommand::ConfigReload))
                .is_some()
        } else {
            false
        };
        let sent = sent
            || st
                .slots
                .values()
                .next()
                .map(|slot| slot.cmd.send(AgentCommand::ConfigReload).is_ok())
                .unwrap_or(false);
        drop(st);
        if sent {
            self.local_notice("Reloading configuration from disk…");
        }
    }

    /// Execute a TUI-style slash command against the current session.
    /// Returns true when the command was recognized (and must NOT be sent
    /// as a prompt — unknown `/name` stays a prompt so server-side skill
    /// invocation keeps working).
    pub fn slash_command(&self, command: &str, args: &str) -> bool {
        let args = args.trim();
        match command.to_ascii_lowercase().as_str() {
            "compact" => {
                self.session_compact(args.to_string());
                true
            }
            "clear" | "new" => {
                self.history_clear();
                true
            }
            "rename" => {
                if args.is_empty() {
                    self.local_notice("Usage: /rename <new title>");
                } else {
                    self.rename_session(args.to_string());
                }
                true
            }
            "reload" => {
                self.config_reload();
                true
            }
            "help" => {
                self.local_notice(crate::SLASH_HELP);
                true
            }
            "status" => {
                self.local_notice(&self.status_text());
                true
            }
            "version" => {
                let cli = crate::updates::local_cli_version()
                    .unwrap_or_else(|| "unknown".to_string());
                self.local_notice(&format!(
                    "Halny Vibe Deck {} · Mistral Vibe CLI {cli}",
                    crate::updates::app_version()
                ));
                true
            }
            "delete" => {
                // TUI /delete: delete the CURRENT backend session
                let st = self.state.lock().unwrap();
                let agent = st
                    .current_session()
                    .and_then(|s| s.agent_session_id.clone());
                drop(st);
                if let Some(sid) = agent {
                    self.delete_backend_session(&sid);
                    self.local_notice("Session deleted.");
                } else {
                    self.local_notice("No backend session to delete.");
                }
                true
            }
            _ => false,
        }
    }

    /// `/status` — TUI renders AgentStatsSnapshot locally; we have the same
    /// data on the session usage mirror.
    fn status_text(&self) -> String {
        let st = self.state.lock().unwrap();
        let Some(s) = st.current_session() else {
            return "No active session.".to_string();
        };
        let u = &s.usage;
        let msgs = s.messages.len();
        let turns = s.messages.iter().filter(|m| m.sender == MessageSender::User).count();
        format!(
            "**Session status**\n\n- messages: {msgs} ({turns} turns)\n- prompt tokens: {:.0}\n- completion tokens: {:.0}\n- total tokens: {:.0} (cached {:.0})\n- cost: ${:.4}\n- context: {:.0} / {:.0} tokens",
            u.prompt_tokens,
            u.completion_tokens,
            u.total_tokens,
            u.cached_tokens,
            u.cost,
            u.context_used,
            u.context_size
        )
    }

    /// TUI `/branch` → `session/fork` at `entry_id` (None = end of history).
    pub fn fork_session(&self, entry_id: Option<String>) {
        let Some((key, agent)) = self.current_agent_pair() else { return };
        let sent = {
            let st = self.state.lock().unwrap();
            st.slot_of_session(&key)
                .and_then(|id| st.slots.get(&id))
                .map(|slot| {
                    slot.cmd.send(AgentCommand::SessionFork {
                        source_session_id: agent,
                        entry_id,
                    })
                })
                .is_some()
        };
        if sent {
            self.local_notice("Forking session…");
        }
    }

    /// TUI `/rewind` → `session/rewind` to `entry_id`.
    pub fn rewind_session(&self, entry_id: String, restore_files: bool) {
        if self.agent_cmd_slot(|sid| AgentCommand::SessionRewind {
            session_id: sid.to_string(),
            entry_id,
            restore_files,
        }) {
            self.local_notice("Rewinding session…");
        }
    }

    /// `session/rewind/read` — which files would a rewind restore.
    pub fn rewind_read(&self, entry_id: String) -> bool {
        self.agent_cmd_slot(|sid| AgentCommand::RewindRead {
            session_id: sid.to_string(),
            entry_id,
        })
    }

    /// `session/pin` (GUI extra — the TUI has no pin UI).
    pub fn pin_session(&self, session_id: &str, pinned: bool) {
        self.any_slot_cmd(|slot| {
            let _ = slot.cmd.send(AgentCommand::PinSession {
                session_id: session_id.to_string(),
                pinned,
            });
        });
    }

    /// `session/archive` (GUI extra).
    pub fn archive_session(&self, session_id: &str, archived: bool) {
        self.any_slot_cmd(|slot| {
            let _ = slot.cmd.send(AgentCommand::ArchiveSession {
                session_id: session_id.to_string(),
                archived,
            });
        });
    }

    pub fn mark_session_seen(&self, session_id: &str) {
        self.any_slot_cmd(|slot| {
            let _ = slot.cmd.send(AgentCommand::MarkSessionSeen {
                session_id: session_id.to_string(),
            });
        });
    }

    /// `session/delete` — remove a backend session from disk.
    pub fn delete_backend_session(&self, session_id: &str) {
        self.any_slot_cmd(|slot| {
            let _ = slot.cmd.send(AgentCommand::DeleteBackendSession {
                session_id: session_id.to_string(),
            });
        });
    }

    fn current_agent_pair(&self) -> Option<(String, String)> {
        let st = self.state.lock().unwrap();
        let s = st.current_session()?;
        Some((s.id.clone(), s.agent_session_id.clone()?))
    }

    /// Send a command through any live slot (global/list-level ops).
    fn any_slot_cmd(&self, f: impl FnOnce(&crate::types::SlotState)) {
        let st = self.state.lock().unwrap();
        if let Some((_, slot)) = st.slots.iter().next() {
            f(slot);
        }
    }

    /// Push a local system notice into the current session's transcript
    /// (survives resyncs — the merge keeps System messages at the tail).
    pub fn local_notice(&self, text: &str) {
        let mut st = self.state.lock().unwrap();
        if let Some(s) = st.current_session_mut() {
            s.messages.push(Message::new_system(text));
            s.last_active_ms = now_ms();
        }
        drop(st);
        self.publish_structure();
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
        let Some(session) = st.current_session() else {
            // Round 7 (R13): no session yet (fresh chat before the first
            // prompt) — remember the choice globally and apply it to the
            // session that the first prompt creates.
            if !value.trim().is_empty() {
                st.pending_options
                    .entry(String::new())
                    .or_default()
                    .push((id.to_string(), value));
            }
            drop(st);
            self.publish_structure();
            return;
        };
        let key = session.id.clone();
        let agent_id = session.agent_session_id.clone();
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
            // Z26: the thinking levels belong to the model — after a model
            // change the whole chip set is re-derived, otherwise the picker
            // keeps offering the previous model's levels.
            if id == "model" {
                let wd = session.working_directory.clone();
                let (options, model, mode, thinking) = build_chip_options(
                    &wd,
                    session.last_model.clone(),
                    session.last_mode.clone(),
                    session.last_thinking.clone(),
                );
                session.config_options = options;
                session.last_model = model;
                session.last_mode = mode;
                session.last_thinking = thinking;
            }
            // The user gets a visible confirmation in the chat (Nexus shows
            // the same CLI notice); translate_cli_notice localizes it.
            let notice = match id {
                "model" => format!("Model changed to {}", value),
                "mode" => format!("Agent changed to {}", value),
                "thinking" => format!("Thinking changed to {}", value),
                _ => format!("{id} changed to {value}"),
            };
            session.messages.push(Message::new_system(&notice));
            session.last_active_ms = now_ms();
        }
        match st.slot_of_session(&key) {
            Some(slot_id) => {
                if let Some(slot) = st.slots.get_mut(&slot_id) {
                    if let Some(opt) = slot.config_options.iter_mut().find(|o| o.id == id) {
                        opt.current_value = value.clone();
                    }
                    if let Some(session_id) = agent_id {
                        let _ = slot.cmd.send(AgentCommand::SetConfigOption {
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

    /// General settings (SettingsView general tab): the idle auto-close
    /// timeout in minutes (the idle reaper itself is element 8). The
    /// agent binary is auto-detected; the explicit `agent_path` lives
    /// in the config file only.
    pub fn set_general(&self, auto_close_idle_minutes: u32) {
        self.update_config(|c| {
            c.auto_close_idle_minutes = if auto_close_idle_minutes > 0 {
                Some(auto_close_idle_minutes)
            } else {
                None
            };
        });
        self.publish_structure();
    }

    /// Disconnect the agent (Nexus agent.stop): stop the lister and every
    /// live slot. Sessions survive as lazy previews and wake again on
    /// the next prompt (lazy resume, Nexus 2.2).
    pub fn stop_agent(&self) {
        self.lister.send(AgentCommand::Stop);
        {
            let mut st = self.state.lock().unwrap();
            let slot_ids: Vec<u64> = st.slot_of.values().cloned().collect();
            for slot_id in slot_ids {
                if let Some(slot) = st.slots.remove(&slot_id) {
                    let _ = slot.cmd.send(AgentCommand::Stop);
                }
            }
            st.slot_of.clear();
        }
        *self.global_status.lock().unwrap() = AgentStatus::Disconnected;
        self.publish(BusEvent::StructureChanged);
    }

    // ---------------- L1: login & account ----------------

    /// Login/account methods only exist on the app-server transport
    /// (global methods, no session) — the engine refuses clearly until
    /// the lister is connected instead of hanging.
    fn aps_not_connected() -> String {
        "app-server transport not connected (setup/* unavailable)".to_string()
    }

    /// `setup/status` — the L1 preflow, sessionless-safe on the
    /// lister. `identity/read` + `account/read` REQUIRE a session
    /// (`AccountReadParams.session_id`) — they go through a slot
    /// with an attached session (`refresh_account`).
    pub fn setup_refresh(&self) {
        self.lister.send(AgentCommand::SetupStatus);
    }

    /// Provider name for `setup/store-credential` (None = server default).
    fn provider_name(&self) -> Option<String> {
        self.last_setup
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.provider.as_ref().map(|p| p.name.clone()))
    }

    /// Start the browser sign-in flow: PKCE begin (blocking curl).
    /// Returns `(sign_in_url, expires_at_ms)` for the UI; the
    /// verifier never leaves the engine.
    pub fn login_browser_start(&self) -> Result<(String, i64), String> {
        if *self.global_status.lock().unwrap() != AgentStatus::Connected {
            return Err(Self::aps_not_connected());
        }
        let setup = self.last_setup.lock().unwrap().clone();
        let Some(setup) = setup else {
            // Seed not loaded yet — trigger it; the UI retries.
            self.lister.send(AgentCommand::SetupStatus);
            return Err("setup status not loaded yet — retry".to_string());
        };
        if setup.unsupported {
            return Err("CLI does not support setup/* (method_not_found)".to_string());
        }
        if setup.pinned_console_url.is_some() {
            // Login domain is pinned — the flow still runs, but the
            // UI must not offer a custom domain. Kept as info only.
        }
        let base = setup
            .provider
            .map(|p| p.browser_auth_api_base_url)
            .filter(|b| !b.is_empty())
            .ok_or_else(|| "browser sign-in not supported by this provider".to_string())?;
        let flow = auth_browser::browser_sign_in_begin(&base)?;
        let url = flow.sign_in_url.clone();
        let expires = flow.expires_at_ms;
        *self.signin_flow.lock().unwrap() = Some(flow);
        Ok((url, expires))
    }

    /// One poll step of the browser flow (blocking curl). On
    /// `Completed` the api key is immediately sent to the transport
    /// (`setup/store-credential`; `setup/submit-choices` is chained
    /// by the event pump when the store outcome arrives).
    pub fn login_browser_poll(&self) -> PollOutcome {
        let mut guard = self.signin_flow.lock().unwrap();
        let Some(flow) = guard.as_mut() else {
            return PollOutcome::Failed("no sign-in flow in progress".to_string());
        };
        let outcome = auth_browser::browser_sign_in_poll(flow);
        if let PollOutcome::Completed { api_key } = &outcome {
            self.send_store_credential(&api_key, None);
        }
        // The flow is one-shot: terminal states clear it (a retry
        // starts a fresh POST /vibe/sign-in).
        if !matches!(outcome, PollOutcome::Pending) {
            *guard = None;
        }
        outcome
    }

    /// Login with a pasted api key: `setup/store-credential`, then
    /// (on `completed`, via the pump) `setup/submit-choices`. Blocks
    /// briefly waiting for the store outcome so the server-fn can
    /// return the exact error (env_var_error / save_error).
    pub fn login_with_api_key(&self, api_key: &str, custom_domain: Option<&str>) -> Result<(String, Option<String>), String> {
        if *self.global_status.lock().unwrap() != AgentStatus::Connected {
            return Err(Self::aps_not_connected());
        }
        self.send_store_credential(api_key, custom_domain);
        // Wait up to 5 s for the store outcome (worker round-trip).
        for _ in 0..25 {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if let Some((outcome, detail)) = self.last_store_outcome.lock().unwrap().clone() {
                return Ok((outcome, detail));
            }
        }
        Err("store-credential timed out".to_string())
    }

    fn send_store_credential(&self, api_key: &str, custom_domain: Option<&str>) {
        // Reset so the waiter sees only the fresh outcome.
        *self.last_store_outcome.lock().unwrap() = None;
        self.lister.send(AgentCommand::SetupStoreCredential {
            provider: self.provider_name(),
            api_key: api_key.to_string(),
            custom_domain: custom_domain.map(str::to_string),
        });
    }

    /// Re-read account state (`account/read` + `identity/read`) on the
    /// current session's slot (or any connected slot — never the
    /// sessionless lister: both methods are session-scoped).
    ///
    /// Callers must NOT already hold the engine state lock: `state` is a
    /// plain `Mutex`, so taking it twice on one thread hangs the slot pump
    /// (that was Z29). Code paths that run inside `apply_slot_event` send
    /// the commands directly to their slot instead.
    pub fn refresh_account(&self) {
        let cmd = {
            let st = self.state.lock().unwrap();
            st.current_session()
                .and_then(|s| st.slot_of_session(&s.id))
                .or_else(|| st.slots.iter().next().map(|(id, _)| *id))
                .and_then(|id| st.slots.get(&id).map(|slot| slot.cmd.clone()))
        };
        if let Some(cmd) = cmd {
            let _ = cmd.send(AgentCommand::IdentityRead);
            let _ = cmd.send(AgentCommand::AccountRead);
        }
    }

    /// Re-read the current session from the agent (`session/resume`).
    ///
    /// The browser view can miss events (hidden tab, sleeping laptop, a
    /// dropped websocket). A client-side snapshot refetch alone only sees
    /// this server's projection, so anything the agent emitted while the
    /// stream was broken stays missing until the user reloads the page —
    /// exactly the stable-edition bug we must not repeat. Asking the agent
    /// for the authoritative session state closes that hole: the projection
    /// is replaced, the UI receives the backfilled history on its refresh.
    pub fn resync_current_session(&self) {
        let st = self.state.lock().unwrap();
        // Round 6 (R6, session isolation): resync ONLY the current session's
        // own slot. The old fallback picked an arbitrary slot, whose worker
        // then emitted another session's history into this one.
        let target = st
            .current_session()
            .and_then(|s| st.slot_of_session(&s.id))
            .and_then(|id| st.slots.get(&id).map(|slot| slot.cmd.clone()));
        let agent_session_id = st
            .current_session()
            .and_then(|s| s.agent_session_id.clone());
        drop(st);
        if let (Some(cmd), Some(session_id)) = (target, agent_session_id) {
            let _ = cmd.send(AgentCommand::ResumeSession {
                session_id,
                cwd: String::new(),
                model: None,
            });
        }
    }

    // ---------------- setup install (local dependencies) ----------------

    /// Current install state (progress lines + running/finished flags).
    pub fn setup_install_state(&self) -> SetupInstallState {
        self.install.lock().unwrap().clone()
    }

    /// Kick off the local dependency install (uv → mistral-vibe → verify
    /// `vibe-app-server`) on a background thread. Idempotent while a run
    /// is in flight; progress streams as `AgentEvent::SetupInstallLine`,
    /// the terminal outcome as `AgentEvent::SetupInstallDone` (and is kept
    /// in the snapshot for late joiners). On success the agent restarts
    /// immediately — no app restart needed.
    pub fn setup_install(&self) {
        if self.install_started.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.install.lock().unwrap() = SetupInstallState {
            lines: Vec::new(),
            running: true,
            finished: None,
        };
        self.publish_structure();
        std::thread::Builder::new()
            .name("mistraldeck-setup-install".into())
            .spawn(move || {
                let push_line = |message: String, kind: &str| {
                    if let Some(e) = Engine::try_global() {
                        let mut g = e.install.lock().unwrap();
                        g.lines.push(mistraldeck_core::snapshot::SetupInstallLine {
                            message: message.clone(),
                            kind: kind.to_string(),
                        });
                        let _ = e.tx.send(BusEvent::Agent(AgentEvent::SetupInstallLine {
                            message,
                            kind: kind.to_string(),
                        }));
                    }
                };
                let ok = mistraldeck_setup::install_dependencies(|p| match p {
                    mistraldeck_setup::SetupProgress::Info(m) => push_line(m, "info"),
                    mistraldeck_setup::SetupProgress::Success(m) => push_line(m, "success"),
                    mistraldeck_setup::SetupProgress::Error(m) => push_line(m, "error"),
                    mistraldeck_setup::SetupProgress::Done(_) => {}
                })
                .map(|_| true)
                .unwrap_or_else(|e| {
                    push_line(format!("install failed: {e:#}"), "error");
                    false
                });
                if let Some(e) = Engine::try_global() {
                    {
                        let mut g = e.install.lock().unwrap();
                        g.running = false;
                        g.finished = Some(ok);
                    }
                    let _ = e.tx.send(BusEvent::Agent(AgentEvent::SetupInstallDone { ok }));
                    e.publish_structure();
                    if ok {
                        // Verified vibe-app-server: (re)connect right away.
                        *e.detected_agent.lock().unwrap() = None;
                        e.start_agent();
                    }
                    e.install_started.store(false, Ordering::SeqCst);
                }
            })
            .ok();
    }

    /// Auto-close idle live sessions (Nexus v0.9.0): stop every slot whose
    /// session sat idle longer than the configured window. Sessions
    /// survive as lazy previews and revive on the next prompt.
    pub fn reap_idle_sessions(&'static self) {
        let minutes = self.config().auto_close_idle_minutes.unwrap_or(60);
        if minutes == 0 {
            return;
        }
        let cutoff = std::time::Duration::from_secs(u64::from(minutes) * 60);
        let stopped = {
            let mut st = self.state.lock().unwrap();
            reap_idle_slots(&mut st, cutoff)
        };
        if !stopped.is_empty() {
            self.publish(BusEvent::StructureChanged);
        }
    }

    /// Watchdog callback for a stuck cancel (Nexus 2.6 Stop-after-timeout
    /// force-release): if the stopping marker is still there, the agent
    /// never acknowledged the cancel - kill the worker, drop the slot and
    /// surface a visible error.
    pub fn force_release_if_stopping(&'static self, key: String) {
        let released = {
            let mut st = self.state.lock().unwrap();
            force_release_stopping(&mut st, &key)
        };
        if released {
            *self.global_error.lock().unwrap() = "Cancel timed out - the agent was stopped; the session revives on the next prompt."
                .to_string();
            self.publish_structure();
        }
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

    /// Custom TLS certificate for the HTTPS listeners (empty = generated
    /// self-signed). Takes effect on the next server restart.
    pub fn set_remote_certs(&self, cert_path: &str, key_path: &str) {
        self.update_config(|c| {
            c.cert_path = cert_path.trim().to_string();
            c.key_path = key_path.trim().to_string();
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
        // so hidden sessions do not leak running agent workers.
        {
            let mut st = self.state.lock().unwrap();
            let matched = st
                .sessions
                .iter()
                .find(|s| s.id == key || s.agent_session_id.as_deref() == Some(key))
                .map(|s| s.id.clone());
            if let Some(sid) = matched {
                if let Some(slot_id) = st.slot_of.remove(&sid) {
                    if let Some(slot) = st.slots.remove(&slot_id) {
                        let _ = slot.cmd.send(AgentCommand::Stop);
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
    /// Verify credentials and issue a stateless session token (stable
    /// v1.0.36: tokens survive restarts instead of logging every browser
    /// out on a self-update or a watchdog restart).
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
        Some(crate::session_token::issue(username))
    }

    /// Valid, non-expired token? Refreshes the sliding expiry window.
    pub fn check_token(&self, token: &str) -> bool {
        self.token_user(token).is_some()
    }

    /// Username behind a valid token (None when invalid/expired/revoked).
    /// The sliding window needs no server state: the login handler
    /// re-issues the cookie when it drops below half of its lifetime.
    pub fn token_user(&self, token: &str) -> Option<String> {
        crate::session_token::verify(token).map(|(user, _)| user)
    }

    /// Re-issue a token for an already authenticated user (sliding window).
    pub fn login_token_for(&self, username: &str) -> Option<String> {
        Some(crate::session_token::issue(username))
    }

    /// True when a valid token is past half of its sliding window and the
    /// caller should re-issue it (stable v1.0.36 sliding 180-day window).
    pub fn token_refresh_due(&self, token: &str) -> bool {
        crate::session_token::verify(token)
            .map(|(_, refresh)| refresh)
            .unwrap_or(false)
    }

    pub fn logout(&self, token: &str) {
        crate::session_token::revoke(token);
    }

    // ---------------- state machine (per-slot) ----------------

    /// Apply one agent worker event to the owning slot + session.
/// Index of the session an agent event belongs to. Round 6 (R6, session
/// isolation): when duplicate mirrors share one agent id, prefer the
/// CURRENT session so events never land in a different open tab's session.
fn agent_session_idx(
    sessions: &[mistraldeck_core::types::ChatSession],
    current: Option<usize>,
    agent_id: &str,
) -> Option<usize> {
    if let Some(i) = current {
        if sessions
            .get(i)
            .map(|s| s.agent_session_id.as_deref() == Some(agent_id))
            .unwrap_or(false)
        {
            return Some(i);
        }
    }
    sessions
        .iter()
        .position(|s| s.agent_session_id.as_deref() == Some(agent_id))
}

    pub(crate) fn apply_slot_event(&'static self, slot_id: u64, event: AgentEvent) {
        mistraldeck_appsrv::trace(format!("slot {slot_id} event {event:?}"));
        let event_out = event.clone();
        // Deduped replacement for streamed text events (None = unchanged);
        // a fully deduped (empty) chunk is suppressed entirely.
        let mut publish_override: Option<AgentEvent> = None;
        let mut suppress = false;
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
            if let AgentEvent::UsageUpdate { session_id, .. } = &event {
                if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
                {
                    let _ = session.usage.apply(&event);
                }
            }
            match event {
                AgentEvent::Initialized { .. } => {
                    slot.status = AgentStatus::Connected;
                    slot.error.clear();
                    match slot.purpose.clone() {
                        SlotPurpose::NewLocal { cwd } => {
                            let _ = slot.cmd.send(AgentCommand::NewSession {
                                cwd,
                                model: global_model,
                            });
                        }
                        SlotPurpose::LoadBackend { session_id, cwd } => {
                            slot.busy = true;
                            slot.last_activity = std::time::Instant::now();
                            let _ = slot.cmd.send(AgentCommand::LoadSession {
                                session_id,
                                cwd,
                                model: global_model,
                            });
                        }
                        SlotPurpose::NewRemote { cwd, .. } => {
                            let _ = slot.cmd.send(AgentCommand::NewSession {
                                cwd,
                                model: global_model,
                            });
                        }
                        SlotPurpose::LoadRemote { session_id, cwd, .. } => {
                            slot.busy = true;
                            slot.last_activity = std::time::Instant::now();
                            let _ = slot.cmd.send(AgentCommand::LoadSession {
                                session_id,
                                cwd,
                                model: global_model,
                            });
                        }
                    }
                }
                AgentEvent::Disconnected { .. } => {
                    slot.status = AgentStatus::Disconnected;
                    slot.busy = false;
                    slot.last_activity = std::time::Instant::now();
                    slot.pending_prompt = None;
                    slot.pending_images.clear();
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
                AgentEvent::SessionsList(_) => { /* the lister owns session/list */ }
                AgentEvent::SessionReady {
                    session_id,
                    modes,
                    config_options,
                    title,
                    current_mode_id,
                    child_sessions,
                } => {
                    slot.status = AgentStatus::Connected;
                    slot.error.clear();
                    // The wake finished: the resuming overlay disappears.
                    slot.load_progress = None;
                    // subagent shelf from the snapshot + "open elsewhere" lease
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.subagents = child_sessions
                                .iter()
                                .filter_map(|c| parse_subagent(c))
                                .collect();
                        }
                    }
                    {
                        let sid = session_id.clone();
                        let _ = slot.cmd.send(AgentCommand::LeaseSubscribe { session_id: sid });
                    }
                    // L1: session up → re-read account state (the
                    // sessionId-scoped `account/read` is fresher now).
                    // Sent straight to this slot: `apply_slot_event` holds the
                    // engine state lock here, so a helper that takes it again
                    // would self-deadlock (Z29 — "Agent is working…" forever).
                    let _ = slot.cmd.send(AgentCommand::IdentityRead);
                    let _ = slot.cmd.send(AgentCommand::AccountRead);
                    slot.modes = modes;
                    // Z26: only an agent-provided list replaces the chips; an
                    // empty payload keeps the config-derived lists (a snapshot
                    // start never carries configOptions).
                    if !config_options.is_empty() {
                        slot.config_options = config_options.clone();
                    }
                    if let Some(i) = slot_session {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.agent_session_id = Some(session_id.clone());
                            if let Some(title) = title {
                                if !title.is_empty() {
                                    session.name = title;
                                }
                            }
                            // Per-session config mirror: the chips keep their
                            // values across slot-less switching + respawns.
                            // An empty agent payload keeps the config-derived
                            // list (Z26) instead of blanking the pickers.
                            if !config_options.is_empty() {
                                session.config_options = config_options.clone();
                            }
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
                        if let Some(session) = st.sessions.get_mut(i) {
                            let key = session.id.clone();
                            if let Some(pending) = st.pending_options.remove(&key) {
                                for (opt_id, opt_value) in pending {
                                    // An empty value is never sent — the
                                    // server rejects it (INVALID_PARAMS) and
                                    // the old code turned that into a fatal
                                    // turn error.
                                    if opt_value.trim().is_empty() {
                                        continue;
                                    }
                                    if let Some(opt) =
                                        slot.config_options.iter_mut().find(|o| o.id == opt_id)
                                    {
                                        opt.current_value = opt_value.clone();
                                    }
                                    // round 8: mirror the pending choice into the
                                    // session itself — otherwise a later agent
                                    // config-options payload clobbers the
                                    // pre-prompt selection in the snapshot
                                    if let Some(opt) = session
                                        .config_options
                                        .iter_mut()
                                        .find(|o| o.id == opt_id)
                                    {
                                        opt.current_value = opt_value.clone();
                                    }
                                    match opt_id.as_str() {
                                        "model" => session.last_model = Some(opt_value.clone()),
                                        "mode" => session.last_mode = Some(opt_value.clone()),
                                        "thinking" => {
                                            session.last_thinking = Some(opt_value.clone())
                                        }
                                        _ => {}
                                    }
                                    let _ = slot.cmd.send(AgentCommand::SetConfigOption {
                                        session_id: session_id.clone(),
                                        key: opt_id,
                                        value: opt_value,
                                    });
                                }
                            }
                        }
                    }
                    if let Some((key, value)) = slot.pending_option.take() {
                        let _ = slot.cmd.send(AgentCommand::SetConfigOption {
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
                            slot.last_activity = std::time::Instant::now();
                            // v0.13.1: unblock the chat when the prompt
                            // send fails (dead worker channel).
                            if slot
                                .cmd
                                .send(AgentCommand::Prompt {
                                    session_id,
                                    text,
                                    images: std::mem::take(&mut slot.pending_images),
                                })
                                .is_err()
                            {
                                mark_prompt_failed(slot);
                            }
                        }
                        None => {
                            slot.busy = false;
                            slot.last_activity = std::time::Instant::now();
                        }
                    }
                }
                AgentEvent::SessionHistory {
                    session_id,
                    messages,
                    plan,
                } => {
                    // Apply for a lazy wake (Load*) AND for a resync of a
                    // live session (New*): the authoritative history from
                    // session/resume must repaint the open chat in both
                    // cases (the hidden-tab backfill resyncs a live slot).
                    let matches_purpose = matches!(
                        &slot.purpose,
                        SlotPurpose::LoadBackend { session_id: sid, .. } if *sid == session_id
                    ) || matches!(
                        &slot.purpose,
                        SlotPurpose::LoadRemote { session_id: sid, .. } if *sid == session_id
                    );
                    let slot_agent_id = slot_session
                        .and_then(|i| st.sessions.get(i))
                        .and_then(|s| s.agent_session_id.clone());
                    let matches = matches_purpose
                        || slot_agent_id.as_deref() == Some(session_id.as_str());
                    if matches {
                        if let Some(i) = slot_session {
                            if let Some(session) = st.sessions.get_mut(i) {
                                // Round 4 (E3): MERGE the authoritative
                                // history with local-only messages instead
                                // of replacing. The server history may lag
                                // the just-sent prompt (it is persisted with
                                // the turn) and never carries local system
                                // notices (WARNING …, config changes) — a
                                // blind replace made both vanish mid-turn.
                                let new_user_texts: std::collections::HashSet<String> =
                                    messages
                                        .iter()
                                        .filter(|m| m.sender == MessageSender::User)
                                        .map(|m| m.content.clone())
                                        .collect();
                                // Round 7 (R10): mid-turn resyncs used to
                                // reorder the chat — unpersisted user prompts
                                // were re-inserted after the last SERVER user
                                // message (i.e. before the in-flight AI
                                // answer), and in-flight AI messages (thinking,
                                // tool calls) were dropped entirely. Now:
                                // server messages win, live state merges into
                                // entries with the same id, and local-only
                                // messages keep their order at the tail.
                                let server_msg_ids: std::collections::HashSet<String> =
                                    messages
                                        .iter()
                                        .filter(|m| m.sender == MessageSender::Ai)
                                        .filter(|m| !m.id.is_empty())
                                        .map(|m| m.id.clone())
                                        .collect();
                                let local_ai: Vec<Message> = session
                                    .messages
                                    .iter()
                                    .filter(|m| m.sender == MessageSender::Ai)
                                    .cloned()
                                    .collect();
                                let mut merged = messages;
                                for m in merged.iter_mut() {
                                    if m.sender == MessageSender::Ai && !m.id.is_empty() {
                                        if let Some(local) =
                                            local_ai.iter().find(|l| l.id == m.id)
                                        {
                                            if local.content.len() > m.content.len() {
                                                m.content = local.content.clone();
                                            }
                                            for tc in &local.tool_calls {
                                                match m
                                                    .tool_calls
                                                    .iter_mut()
                                                    .find(|t| t.id == tc.id)
                                                {
                                                    Some(ex) => {
                                                        if tc.status == ToolStatus::Running
                                                            || ex.status != ToolStatus::Running
                                                        {
                                                            *ex = tc.clone();
                                                        }
                                                    }
                                                    None => m.tool_calls.push(tc.clone()),
                                                }
                                            }
                                            if local.thinking.len() > m.thinking.len() {
                                                m.thinking = local.thinking.clone();
                                            }
                                        }
                                    }
                                }
                                let mut tail: Vec<Message> = Vec::new();
                                for old in &session.messages {
                                    match old.sender {
                                        MessageSender::System => tail.push(old.clone()),
                                        MessageSender::User
                                            if !new_user_texts.contains(old.content.as_str()) =>
                                        {
                                            tail.push(old.clone())
                                        }
                                        MessageSender::Ai
                                            if old.id.is_empty()
                                                || !server_msg_ids.contains(&old.id) =>
                                        {
                                            tail.push(old.clone())
                                        }
                                        _ => {}
                                    }
                                }
                                merged.extend(tail);
                                session.messages = merged;
                                // round 8: the snapshot's SessionHistory carries an
                                // empty plan — overwriting unconditionally wiped a
                                // live todo list on every resync
                                if !plan.is_empty() {
                                    session.plan = plan;
                                }
                                session.last_active_ms = now_ms();
                                session.stream_msg_id = None;
                            }
                        }
                        // round 8: SessionHistory must NOT clear busy — right
                        // after a wake the SessionReady already sent the pending
                        // prompt with busy=true, and this resync used to flip
                        // it back to false for the whole first turn (no
                        // "agent is working" marker, queued prompts could
                        // fire mid-turn). busy is owned by PromptCompleted /
                        // SessionError / the wake path only.
                        slot.last_activity = std::time::Instant::now();
                        if let Some(k) = slot_key.as_deref() {
                            st.stopping.remove(k);
                        }
                    }
                }
                AgentEvent::UserMessageChunk { .. } => {}
                // Replay progress while a lazy session wakes (Nexus
                // `SessionLoadProgress`, every 25 buffered updates).
                AgentEvent::SessionLoadProgress { session_id, count } => {
                    slot.load_progress = Some(SessionLoadProgress {
                        session_id,
                        count,
                    });
                }
                AgentEvent::SessionError { context, message } => {
                    slot.status = AgentStatus::Error;
                    slot.error = format!("{context}: {message}");
                    slot.busy = false;
                    slot.last_activity = std::time::Instant::now();
                    slot.load_progress = None;
                    // The turn died — clear the stopping marker.
                    if let Some(k) = slot_key.as_deref() {
                        st.stopping.remove(k);
                    }
                    // Nexus §2.2: the lazy wake failed (CLI session gone
                    // from disk) — silently fall back to a fresh session
                    // in the same cwd; the pending prompt resends there.
                    // Round 6 (R7): the worker reports wake failures with
                    // context "session/resume" (LoadSession maps to
                    // resume_session), not only "load_session" — without
                    // this the pending prompt was dropped and the chat
                    // showed no busy marker at all.
                    if context == "load_session" || context == "session/resume" {
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
                        slot.pending_images.clear();
                    }
                    if respawn.is_none() {
                        if let Some(i) = slot_session {
                            if let Some(session) = st.sessions.get_mut(i) {
                                for msg in &mut session.messages {
                                    if msg.sender == MessageSender::Ai {
                                        msg.is_complete = true;
                                    }
                                }
                                // v0.10.2: close open thinking blocks and
                                // stuck tool calls (Nexus finalize on
                                // Error).
                                finalize_session_thinking(session);
                                complete_session_tool_calls(session);
                                session.messages.push(Message::new_system(&format!(
                                    "WARNING {context}: {message}"
                                )));
                            }
                        }
                    }
                }
                AgentEvent::MessageChunk {
                    session_id,
                    message_id,
                    text,
                } => {
                    let mid = message_id.clone();
                    // v0.10.2 dedup guard (Nexus `dedupChunk`): a delta chunk
                    // followed by the full accumulated text must not
                    // duplicate content. Clients receive the deduped text;
                    // fully deduped chunks are not published at all.
                    let out_text =
                        dedup_chunk(&mut slot.stream_sent, &session_id, "m", &message_id, &text);
                    if out_text.is_empty() {
                        suppress = true;
                    } else {
                        publish_override = Some(AgentEvent::MessageChunk {
                            session_id: session_id.clone(),
                            message_id,
                            text: out_text.clone(),
                        });
                        if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
                        {
                            // v0.13.7: a new agent messageId starts a new
                            // agent message (Nexus `streamingState.agentMsgId`
                            // split) - intermediate texts of one turn no
                            // longer merge into a single giant bubble.
                            session.split_on_message(&mid);
                            {
                                let msg = ensure_ai_message(session);
                                // round 8: stamp the wire messageId onto the live
                                // bubble — without it the SessionHistory merge
                                // can never match this message by id and appends
                                // a duplicate on every mid-turn resync
                                // (reordering / thinking flicker)
                                if mid != "current" {
                                    msg.id = mid.clone();
                                }
                                msg.content.push_str(&out_text);
                            }
                            // v0.10.2: the CLI never emits agent_thought_end —
                            // once the answer text streams, all thinking
                            // blocks of this message are done (Nexus
                            // `finalizeAllThinking` on MessageChunk).
                            finalize_session_thinking(session);
                        }
                    }
                }
                AgentEvent::ThinkingChunk {
                    session_id,
                    block_id,
                    text,
                } => {
                    let bid = block_id.clone();
                    // Same dedup guard for reasoning streams (kind "t", per
                    // thought block — Nexus keys thoughts by messageId).
                    let out_text =
                        dedup_chunk(&mut slot.stream_sent, &session_id, "t", &block_id, &text);
                    if out_text.is_empty() {
                        suppress = true;
                    } else {
                        publish_override = Some(AgentEvent::ThinkingChunk {
                            session_id: session_id.clone(),
                            block_id,
                            text: out_text.clone(),
                        });
                        if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
                        {
                            // v0.13.7: reasoning that arrives after a
                            // streamed text opens the NEXT agent message -
                            // the thought lands above its own text, not
                            // glued under the previous segment.
                            session.split_on_thought();
                            let msg = ensure_ai_message(session);
                            match msg.thinking.last_mut() {
                                Some(block) if !block.is_complete && block.id == bid => {
                                    block.content.push_str(&out_text)
                                }
                                _ => msg.thinking.push(mistraldeck_core::types::ThinkingBlock {
                                    id: bid,
                                    content: out_text,
                                    is_complete: false,
                                    started_at: mistraldeck_core::types::now_ms(),
                                    completed_at: 0,
                                }),
                            }
                        }
                    }
                }
                AgentEvent::ThinkingComplete { session_id } => {
                    if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
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
                AgentEvent::ToolCallStarted {
                    session_id,
                    tool_call,
                } => {
                    if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
                    {
                        ensure_ai_message(session).tool_calls.push(tool_call);
                        // v0.10.2: tool activity means thinking is over
                        // (Nexus `finalizeAllThinking` on ToolCall).
                        finalize_session_thinking(session);
                    }
                }
                AgentEvent::ToolCallUpdated {
                    session_id,
                    tool_call_id,
                    status,
                    output,
                    subagent,
                } => {
                    if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
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
                AgentEvent::ToolApprovalRequest {
                    request_id,
                    method,
                    params_json,
                    tool_name,
                } => {
                    // v1.0.6: in the built-in auto-approve mode ("yolo")
                    // approve ourselves instead of surfacing the dialog.
                    // Custom agents keep the manual approval flow.
                    let auto = slot_session
                        .and_then(|i| st.sessions.get(i))
                        .map(|s| {
                            s.last_mode.as_deref() == Some("auto-approve")
                                || s.config_options
                                    .iter()
                                    .any(|o| o.id == "mode" && o.current_value == "auto-approve")
                        })
                        .unwrap_or(false);
                    if auto {
                        let _ = slot.cmd.send(AgentCommand::Approve {
                            request_id,
                            option_id: "allow_once".into(),
                        });
                    } else {
                        slot.approval = Some(PendingApproval {
                            request_id,
                            method,
                            params_json,
                            tool_name,
                        });
                    }
                }
                AgentEvent::AskUserQuestion {
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
                AgentEvent::PlanUpdate {
                    session_id,
                    entries_json,
                } => {
                    // Cumulative checklist state: the last update wins.
                    // Stored on the session so it survives switching.
                    let entries = mistraldeck_core::types::PlanEntry::parse_list(&entries_json);
                    let target = st
                        .sessions
                        .iter()
                        .position(|s| s.agent_session_id.as_deref() == Some(session_id.as_str()))
                        .or(slot_session);
                    if let Some(i) = target {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.plan = entries;
                            session.last_active_ms = now_ms();
                        }
                    }
                }
                AgentEvent::AvailableCommands { commands, .. } => {
                    slot.commands = commands;
                }
                AgentEvent::ChildSessionUpdated {
                    session_id,
                    child_json,
                } => {
                    // subagent shelf: upsert the child in the session's list
                    if let Some(child) = parse_subagent(&child_json) {
                        let target = Self::agent_session_idx(&st.sessions, st.current, &session_id)
                            .or(slot_session);
                        if let Some(i) = target {
                            if let Some(session) = st.sessions.get_mut(i) {
                                match session
                                    .subagents
                                    .iter_mut()
                                    .find(|s| s.id == child.id)
                                {
                                    Some(ex) => *ex = child,
                                    None => session.subagents.push(child),
                                }
                                session.last_active_ms = now_ms();
                            }
                        }
                    }
                }
                AgentEvent::LeaseUpdated {
                    session_id,
                    held_elsewhere,
                } => {
                    let target = Self::agent_session_idx(&st.sessions, st.current, &session_id)
                        .or(slot_session);
                    if let Some(i) = target {
                        if let Some(session) = st.sessions.get_mut(i) {
                            session.leased_elsewhere = held_elsewhere;
                        }
                    }
                }
                AgentEvent::RewindInfo { .. } => {
                    // forwarded to clients as-is (the rewind picker reads it)
                }
                AgentEvent::PromptCompleted { session_id, .. } => {
                    slot.busy = false;
                    slot.last_activity = std::time::Instant::now();
                    // The turn really ended — clear the stopping marker
                    // (Nexus markSessionIdle).
                    if let Some(k) = slot_key.as_deref() {
                        st.stopping.remove(k);
                    }
                    if let Some(session) = Self::agent_session_idx(&st.sessions, st.current, &session_id).and_then(|i| st.sessions.get_mut(i))
                    {
                        for msg in &mut session.messages {
                            if msg.sender == MessageSender::Ai {
                                msg.is_complete = true;
                            }
                        }
                        // v0.10.2: the CLI never finalizes thinking blocks
                        // and a turn can end with a tool call still
                        // "running" — close both (Nexus
                        // `finalizeAllThinking` + `completeToolCalls`).
                        finalize_session_thinking(session);
                        complete_session_tool_calls(session);
                    }
                }
                AgentEvent::UsageUpdate { .. } => { /* handled above */ }
                // Wildcard: must not move out of `event` (the usage
                // mirror borrows it after the match).
                AgentEvent::DebugLog(_) => {}
                // L1: setup/account events are lister-scoped —
                // slots never emit them.
                _ => {}
            }
        }
        drop(guard);
        // Round 4 (E6/E7): the turn ended (PromptCompleted / SessionReady)
        // — release the next queued prompt, one at a time. This is the
        // headless-safe queue guard: it lives in the engine, so it works
        // with no UI client attached. On errors the queue stays put (the
        // respawn fallback owns the pending prompt there).
        if matches!(
            event_out,
            AgentEvent::PromptCompleted { .. } | AgentEvent::SessionReady { .. }
        ) {
            let mut st2 = self.state.lock().unwrap();
            pump_slot_queue(&mut st2, slot_id);
        }
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
                    slot.last_activity = std::time::Instant::now();
                    slot.pending_prompt = Some(prompt);
                    // Respawn fallback re-sends text only (the
                    // images were already rendered on the message).
                    slot.pending_images.clear();
                }
            }
        }
        // Forward the event so streaming clients update in place — the
        // deduped variant for streamed text; fully deduped (empty) chunks
        // are dropped (nothing new to show).
        if !suppress {
            self.publish(BusEvent::Agent(publish_override.unwrap_or(event_out)));
        }
        // Anything structural (status, approvals, ready) -> refetch hint.
        self.publish(BusEvent::StructureChanged);
    }
}

/// v0.13.1: a failed prompt send must not leave the chat stuck busy
/// (Nexus `markSessionIdle` on every sendPrompt error path) — the worker
/// process is gone, so the turn can never complete; surface the error and
/// free the slot. Its Disconnected status makes the next prompt respawn a
/// fresh worker via `reuse_slot_or_cleanup`.
/// Round 4 (E6/E7): send the slot's next queued prompt when the slot is
/// idle. Call with the engine state lock held, after `busy` flipped false
/// (PromptCompleted / SessionReady). No-op while busy or queue empty.
fn pump_slot_queue(st: &mut crate::types::EngineState, slot_id: u64) {
    let agent_id = st
        .session_of_slot(slot_id)
        .and_then(|i| st.sessions.get(i))
        .and_then(|s| s.agent_session_id.clone());
    let Some(agent_id) = agent_id else { return };
    let Some(slot) = st.slots.get_mut(&slot_id) else { return };
    if slot.busy {
        return;
    }
    let Some((text, images)) = slot.queue.pop_front() else {
        return;
    };
    slot.busy = true;
    slot.last_activity = std::time::Instant::now();
    if slot
        .cmd
        .send(AgentCommand::Prompt {
            session_id: agent_id,
            text,
            images,
        })
        .is_err()
    {
        mark_prompt_failed(slot);
    }
}

pub fn mark_prompt_failed(slot: &mut crate::types::SlotState) {
    slot.busy = false;
    slot.status = AgentStatus::Disconnected;
    slot.error = "Worker unavailable (process gone)".into();
}

/// v0.10.2 guard against full-entry resends from the CLI — a delta chunk
/// followed by the full accumulated text would otherwise duplicate content
/// (Nexus `dedupChunk`): when the already-sent prefix is long enough (>= 40
/// chars) and the incoming text starts with it, only the new suffix is kept.
pub fn dedup_chunk(
    sent: &mut std::collections::HashMap<String, String>,
    session_id: &str,
    kind: &str,
    entry_id: &str,
    text: &str,
) -> String {
    if text.is_empty() {
        return String::new();
    }
    let key = format!("{session_id}|{kind}|{entry_id}");
    let prev = sent.get(&key).cloned().unwrap_or_default();
    let mut out = text.to_string();
    if prev.len() >= 40 && text.starts_with(&prev) {
        out = text[prev.len()..].to_string();
    }
    sent.insert(key, format!("{prev}{out}"));
    out
}

/// The CLI never finalizes thinking blocks (no `agent_thought_end` on the
/// wire) — close every open block of the session's AI messages (Nexus
/// `finalizeAllThinking`, called when answer text / tool calls / turn end
/// arrive).
pub fn finalize_session_thinking(session: &mut ChatSession) {
    let now = now_ms();
    for msg in &mut session.messages {
        if msg.sender == MessageSender::Ai {
            for block in &mut msg.thinking {
                block.is_complete = true;
                if block.completed_at == 0 {
                    block.completed_at = now;
                }
            }
        }
    }
}

/// A turn can end while a tool call is still `Running` (lost update,
/// cancel) — close them so the steps timeline has no spinners (Nexus
/// `completeToolCalls`).
pub fn complete_session_tool_calls(session: &mut ChatSession) {
    for msg in &mut session.messages {
        if msg.sender == MessageSender::Ai {
            for tc in &mut msg.tool_calls {
                // v1.0.37 parity: a subagent invocation (agents.spawn / task
                // with a child session) stays `running` while the child works
                // — the CLI closes the call itself when the child finishes,
                // so PromptCompleted must not force it to done.
                let subagent_live = tc
                    .subagent
                    .as_ref()
                    .and_then(|s| s.child_session_id.as_deref())
                    .is_some();
                if tc.status == ToolStatus::Running && !subagent_live {
                    tc.status = ToolStatus::Done;
                }
            }
        }
    }
}

/// Ensure the session ends with an open AI message and return it.
/// Parse one `PublicChildSession` wire object into a SubagentView.
pub fn parse_subagent(json: &str) -> Option<mistraldeck_core::types::SubagentView> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    Some(mistraldeck_core::types::SubagentView {
        id: v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        name: v
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("subagent")
            .to_string(),
        agent_type: v
            .get("agentType")
            .or_else(|| v.get("agent_type"))
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        status: status_of(v.get("status")),
        updated_at: v
            .get("updatedAt")
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
    })
}

fn status_of(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Object(m)) => m
            .get("type")
            .or_else(|| m.get("status"))
            .and_then(|x| x.as_str())
            .unwrap_or("idle")
            .to_string(),
        _ => "idle".to_string(),
    }
}

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
            // round 8: keep the history entry id so the SessionHistory merge
            // matches disk-preview messages with live ones by id (no dupes)
            if !m.id.is_empty() {
                msg.id = m.id.clone();
            }
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
                .map(|tc| {
                    // v1.99.9: restore the subagent meta from disk so the
                    // historical card keeps its prompt + "Show work"; a
                    // restored subagent is finished, never stuck running.
                    let subagent = tc.subagent.as_ref().map(|s| {
                        mistraldeck_core::types::SubagentInfo {
                            agent: s.agent.clone(),
                            agent_name: s.agent_name.clone(),
                            task: Some(s.task.clone()),
                            child_session_id: s.child_session_id.clone(),
                        }
                    });
                    let status = match tc.status.as_str() {
                        "error" => ToolStatus::Error,
                        "running" | "in_progress" if subagent.is_none() => ToolStatus::Running,
                        _ => ToolStatus::Done,
                    };
                    ToolCall {
                        id: new_id(),
                        title: if tc.title.trim().is_empty() {
                            tc.tool_name.clone()
                        } else {
                            tc.title.clone()
                        },
                        kind: "tool".to_string(),
                        status,
                        input_json: None,
                        output: None,
                        file_changes: Vec::new(),
                        diffs: Vec::new(),
                        tool_name: tc.tool_name.clone(),
                        previous_content: None,
                        raw_output_json: None,
                        subagent,
                        started_at: 0,
                    }
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
/// Build the model/mode/thinking chip lists for one session from the CLI
/// configuration **of that project** (Z26/Z27):
/// - models: global `config.toml` merged with the project's own file,
/// - thinking levels: the **selected model's** `thinking_levels`
///   (each model has its own set — a model change must re-derive them),
/// - modes: approval modes plus the global agents (`~/.vibe/agents`) and the
///   project agents (`<cwd>/.vibe/agents`) only — with their descriptions,
///   so a project never shows another project's agents.
///
/// Returns `(options, model, mode, thinking)` with the resolved current
/// values (a value that does not exist in the new list falls back to a
/// sensible default).
pub fn build_chip_options(
    wd: &str,
    model: Option<String>,
    mode: Option<String>,
    thinking: Option<String>,
) -> (
    Vec<mistraldeck_core::proto::ConfigOption>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    use mistraldeck_core::proto::{ConfigOption, ConfigOptionChoice};
    let key_of = |m: &mistraldeck_vibe::ModelConfig| {
        m.alias.clone().unwrap_or_else(|| m.name.clone())
    };

    // Model list: global config + project config of the session cwd,
    // merged by key (alias||name) — project entries override.
    let mut models = mistraldeck_vibe::VibeConfigFile::load().models();
    if !wd.is_empty() {
        if let Some(p) = mistraldeck_vibe::find_project_config(std::path::Path::new(wd)) {
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
    // Round 4 (E2/E4): a session with no recorded model still shows the
    // CLI default — the global `active_model`, else the first configured
    // model — instead of an empty chip. New sessions and sessions reopened
    // from the list both land here.
    let model = model
        .filter(|m| !m.trim().is_empty())
        .or_else(|| {
            mistraldeck_vibe::VibeConfigFile::load()
                .active_model()
                .filter(|a| !a.trim().is_empty())
        })
        .or_else(|| models.first().map(|m| key_of(m)));
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

    // Global agents join the mode list (top-level only, deduped by name —
    // Nexus `agentNames` set); descriptions travel with each choice so the
    // picker explains what the mode does. **Project** agents
    // (`<cwd>/.vibe/agents`) are merged by the client for the session's own
    // project only, and are exposed to `~/.vibe/agents` before activation.
    let agents = mistraldeck_vibe::AgentFile::load_all();
    let mut mode_choices: Vec<ConfigOptionChoice> = [
        ("ask", "Ask", "Ask before every tool call"),
        ("accept-edits", "Accept Edits", "Apply file edits without asking"),
        ("smart-approve", "Smart Approve", "Ask only for risky commands"),
        ("auto-approve", "Auto Approve", "Run everything without asking"),
    ]
    .iter()
    .map(|(v, n, d)| ConfigOptionChoice {
        value: v.to_string(),
        name: n.to_string(),
        description: d.to_string(),
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
            description: a
                .description
                .clone()
                .unwrap_or_else(|| a.active_model.clone().unwrap_or_default()),
        });
    }

    let mut mode_current = mode.clone().unwrap_or_else(|| "accept-edits".to_string());
    if !mode_choices.iter().any(|c| c.value == mode_current) {
        mode_current = "accept-edits".to_string();
    }

    let options = vec![
        ConfigOption {
            id: "mode".to_string(),
            name: "Mode".to_string(),
            current_value: mode_current.clone(),
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
    let thinking_out = if thinking_current.is_empty() {
        thinking
    } else {
        Some(thinking_current)
    };
    (options, model, Some(mode_current), thinking_out)
}

fn restore_session_chips(st: &mut EngineState) {
    let Some(i) = st.current else { return };
    let Some(session) = st.sessions.get(i) else { return };
    if !session.config_options.is_empty() {
        return; // live (or already restored) — keep the recorded lists
    }
    let agent_id = session.agent_session_id.clone();
    let wd = session.working_directory.clone();
    let mut model = session.last_model.clone();
    let mut mode = session.last_mode.clone();
    let mut thinking = session.last_thinking.clone();
    // Missing last* values come from the unified store on disk (Nexus
    // `vibeSessionLastConfig`: newest generation with an active_model).
    if model.is_none() || mode.is_none() || thinking.is_none() {
        if let Some(last) = agent_id
            .as_deref()
            .and_then(mistraldeck_vibe::load_session_last_config)
        {
            model = model.or(last.model);
            mode = mode.or(last.mode);
            thinking = thinking.or(last.thinking);
        }
    }

    let (options, model, mode, thinking) = build_chip_options(&wd, model, mode, thinking);

    let Some(session) = st.sessions.get_mut(i) else { return };
    if session.config_options.is_empty() {
        session.config_options = options;
        session.last_model = model;
        session.last_mode = mode;
        session.last_thinking = thinking;
    }
}

/// Auto-close idle live sessions (Nexus v0.9.0 `autoCloseIdleMinutes`):
/// stop every slot that finished its turn longer than `cutoff` ago and is
/// neither busy nor waiting for an approval/question. Sessions survive
/// as lazy previews (the next prompt wakes the agent again); returns the
/// ids of the stopped slots so the caller can publish one event.
pub fn reap_idle_slots(
    st: &mut crate::types::EngineState,
    cutoff: std::time::Duration,
) -> Vec<u64> {
    let ids: Vec<u64> = st
        .slots
        .iter()
        .filter(|(_, s)| {
            !s.busy
                && s.approval.is_none()
                && s.question.is_none()
                && s.last_activity.elapsed() > cutoff
        })
        .map(|(id, _)| *id)
        .collect();
    for id in &ids {
        if let Some(slot) = st.slots.remove(id) {
            let _ = slot.cmd.send(mistraldeck_core::proto::AgentCommand::Stop);
        }
        st.slot_of.retain(|_, sid| sid != id);
    }
    ids
}

#[cfg(test)]
mod idle_reaper_tests {
    use super::*;
    use crate::types::{SlotPurpose, SlotState, EngineState};
    use mistraldeck_core::proto::AgentCommand;
    use mistraldeck_core::snapshot::AgentStatus;
    use std::sync::mpsc;

    /// Round 4 (E6/E7): the server-side prompt queue — a busy slot keeps
    /// queued prompts; once idle, pump sends exactly the next one.
    #[test]
    fn queue_pumps_one_prompt_when_idle_and_holds_while_busy() {
        let (mut s, rx) = slot(false, std::time::Duration::from_secs(0));
        s.queue.push_back(("p1".to_string(), Vec::new()));
        s.queue.push_back(("p2".to_string(), Vec::new()));
        let mut st = EngineState::default();
        st.sessions.push(ChatSession::new("s", "/tmp"));
        st.sessions[0].agent_session_id = Some("agent-1".to_string());
        st.current = Some(0);
        st.slots.insert(7, s);
        st.slot_of.insert("sess".to_string(), 7);
        st.sessions[0].id = "sess".to_string();

        // busy: nothing is sent
        st.slots.get_mut(&7).unwrap().busy = true;
        pump_slot_queue(&mut st, 7);
        assert_eq!(st.slots[&7].queue.len(), 2);
        assert!(rx.try_recv().is_err());

        // idle: exactly one prompt goes out, the rest stays queued
        st.slots.get_mut(&7).unwrap().busy = false;
        pump_slot_queue(&mut st, 7);
        assert!(st.slots[&7].busy);
        assert_eq!(st.slots[&7].queue.len(), 1);
        match rx.try_recv() {
            Ok(AgentCommand::Prompt { session_id, text, .. }) => {
                assert_eq!(session_id, "agent-1");
                assert_eq!(text, "p1");
            }
            other => panic!("expected Prompt, got {other:?}"),
        }
        // still busy -> the second prompt waits
        pump_slot_queue(&mut st, 7);
        assert_eq!(st.slots[&7].queue.len(), 1);
        assert!(rx.try_recv().is_err());
    }


    fn slot(busy: bool, age: std::time::Duration) -> (SlotState, mpsc::Receiver<AgentCommand>) {
        let (tx, rx) = mpsc::channel();
        let s = SlotState {
            cmd: tx,
            purpose: SlotPurpose::NewLocal { cwd: "/tmp".into() },
            status: AgentStatus::Disconnected,
            error: String::new(),
            busy,
            last_activity: std::time::Instant::now()
                .checked_sub(age)
                .unwrap_or_else(std::time::Instant::now),
            pending_prompt: None,
            pending_images: Vec::new(),
            queue: std::collections::VecDeque::new(),
            pending_option: None,
            approval: None,
            question: None,
            usage: Default::default(),
            load_progress: None,
            commands: Vec::new(),
            config_options: Vec::new(),
            modes: Vec::new(),
            stream_sent: std::collections::HashMap::new(),
        };
        (s, rx)
    }

    #[test]
    fn reap_stops_idle_slots_and_keeps_busy_and_fresh() {
        let mut st = EngineState::default();
        let (idle, idle_rx) = slot(false, std::time::Duration::from_secs(600));
        let (busy, _busy_rx) = slot(true, std::time::Duration::from_secs(600));
        let (fresh, _fresh_rx) = slot(false, std::time::Duration::from_secs(5));
        st.slots.insert(1, idle);
        st.slots.insert(2, busy);
        st.slots.insert(3, fresh);
        st.slot_of.insert("s-idle".into(), 1);
        st.slot_of.insert("s-busy".into(), 2);
        st.slot_of.insert("s-fresh".into(), 3);

        let stopped = reap_idle_slots(&mut st, std::time::Duration::from_secs(60));

        assert_eq!(stopped, vec![1]);
        assert!(!st.slots.contains_key(&1));
        assert!(st.slots.contains_key(&2));
        assert!(st.slots.contains_key(&3));
        // slot_of no longer references the stopped slot
        assert!(!st.slot_of.values().any(|sid| *sid == 1));
        // the stopped worker received the Stop command
        assert!(matches!(idle_rx.try_recv(), Ok(AgentCommand::Stop)));
    }

    #[test]
    fn reap_skips_waiting_approval_and_question() {
        let mut st = EngineState::default();
        let (mut waiting, _rx) = slot(false, std::time::Duration::from_secs(600));
        waiting.approval = Some(mistraldeck_core::snapshot::PendingApproval {
            request_id: "r1".into(),
            method: "tool Approval".into(),
            params_json: "{}".into(),
            tool_name: None,
        });
        st.slots.insert(1, waiting);
        let (mut asked, _rx2) = slot(false, std::time::Duration::from_secs(600));
        asked.question = Some(mistraldeck_core::snapshot::PendingQuestion {
            request_id: "r2".into(),
            question: "Proceed?".into(),
            choices: Vec::new(),
        });
        st.slots.insert(2, asked);

        let stopped = reap_idle_slots(&mut st, std::time::Duration::from_secs(60));
        assert!(stopped.is_empty());
        assert!(st.slots.contains_key(&1));
        assert!(st.slots.contains_key(&2));
    }
}

/// Zombie check before slot reuse (Nexus 2.6 ping-before-reuse): a slot
/// that already reported `Disconnected` is a dead worker - the command
/// channel would happily swallow prompts into the void. Stop it, drop
/// the mapping and let the caller spawn a fresh slot. Returns
/// `Some(slot_id)` when the existing slot can be reused.
pub fn reuse_slot_or_cleanup(
    st: &mut crate::types::EngineState,
    session_key: &str,
) -> Option<u64> {
    let slot_id = st.slot_of_session(session_key)?;
    if let Some(slot) = st.slots.get(&slot_id) {
        if slot.status != mistraldeck_core::snapshot::AgentStatus::Disconnected {
            return Some(slot_id);
        }
    }
    if let Some(slot) = st.slots.remove(&slot_id) {
        let _ = slot.cmd.send(mistraldeck_core::proto::AgentCommand::Stop);
    }
    st.slot_of.remove(session_key);
    None
}

/// Force-release a stuck cancel (Nexus 2.6: Stop after a 4 s timeout ->
/// markSessionIdle + stopWorker). Returns true when a release happened.
pub fn force_release_stopping(st: &mut crate::types::EngineState, key: &str) -> bool {
    if !st.stopping.remove(key) {
        return false;
    }
    if let Some(slot_id) = st.slot_of.remove(key) {
        if let Some(slot) = st.slots.remove(&slot_id) {
            let _ = slot.cmd.send(mistraldeck_core::proto::AgentCommand::Stop);
        }
        let _ = slot_id;
    }
    // Close dangling assistant messages - the turn will never end on its
    // own (same as the Disconnected handler / Nexus markSessionIdle).
    if let Some(session) = st.sessions.iter_mut().find(|s| s.id == key) {
        for msg in &mut session.messages {
            if msg.sender == MessageSender::Ai {
                msg.is_complete = true;
            }
        }
    }
    true
}

#[cfg(test)]
mod zombie_tests {
    use super::*;
    use crate::types::{EngineState, SlotPurpose, SlotState};
    use mistraldeck_core::proto::AgentCommand;
    use mistraldeck_core::snapshot::AgentStatus;
    use std::sync::mpsc;

    fn slot(status: AgentStatus, busy: bool) -> (SlotState, mpsc::Receiver<AgentCommand>) {
        let (tx, rx) = mpsc::channel();
        let s = SlotState {
            cmd: tx,
            purpose: SlotPurpose::NewLocal { cwd: "/tmp".into() },
            status,
            error: String::new(),
            busy,
            last_activity: std::time::Instant::now(),
            pending_prompt: None,
            pending_images: Vec::new(),
            queue: std::collections::VecDeque::new(),
            pending_option: None,
            approval: None,
            question: None,
            usage: Default::default(),
            load_progress: None,
            commands: Vec::new(),
            config_options: Vec::new(),
            modes: Vec::new(),
            stream_sent: std::collections::HashMap::new(),
        };
        (s, rx)
    }

    #[test]
    fn reuse_skips_disconnected_and_keeps_connected() {
        let mut st = EngineState::default();
        let (dead, dead_rx) = slot(AgentStatus::Disconnected, false);
        st.slots.insert(1, dead);
        st.slot_of.insert("s-dead".into(), 1);
        assert_eq!(reuse_slot_or_cleanup(&mut st, "s-dead"), None);
        assert!(st.slots.is_empty());
        assert!(st.slot_of.is_empty());
        assert!(matches!(dead_rx.try_recv(), Ok(AgentCommand::Stop)));

        let (alive, _alive_rx) = slot(AgentStatus::Connected, false);
        st.slots.insert(2, alive);
        st.slot_of.insert("s-alive".into(), 2);
        assert_eq!(reuse_slot_or_cleanup(&mut st, "s-alive"), Some(2));
        assert!(st.slots.contains_key(&2));
    }

    #[test]
    fn force_release_clears_stopping_and_kills_slot() {
        let mut st = EngineState::default();
        let (hung, hung_rx) = slot(AgentStatus::Connected, true);
        st.slots.insert(1, hung);
        st.slot_of.insert("s-key".into(), 1);
        st.stopping.insert("s-key".to_string());

        assert!(force_release_stopping(&mut st, "s-key"));
        assert!(st.slots.is_empty());
        assert!(st.slot_of.is_empty());
        assert!(st.stopping.is_empty());
        assert!(matches!(hung_rx.try_recv(), Ok(AgentCommand::Stop)));
        // A second watchdog (or a turn that already ended) is a no-op.
        assert!(!force_release_stopping(&mut st, "s-key"));
    }
}


#[cfg(test)]
mod segment_tests {
    use super::*;
    use mistraldeck_core::types::MessageSender;

    #[test]
    fn split_on_message_opens_new_bubble() {
        let mut session = ChatSession::new("s", "/tmp");
        session.split_on_message("m1");
        ensure_ai_message(&mut session).content = "one".into();
        // Same id: the chunk stays in the current bubble.
        session.split_on_message("m1");
        assert_eq!(session.messages.len(), 1);
        // A different id finalizes the segment; the next chunk opens a
        // new bubble (ensure_ai_message sees a completed last message).
        session.split_on_message("m2");
        assert_eq!(session.messages.len(), 1);
        assert!(session.messages[0].is_complete);
        ensure_ai_message(&mut session);
        assert_eq!(session.messages.len(), 2);
        assert!(!session.messages[1].is_complete);
        assert_eq!(session.messages[1].sender, MessageSender::Ai);
        assert_eq!(session.stream_msg_id.as_deref(), Some("m2"));
        // The "current" fallback (CLI without messageIds) never splits.
        session.split_on_message("current");
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.stream_msg_id.as_deref(), Some("m2"));
    }

    #[test]
    fn thought_after_text_opens_next_segment() {
        let mut session = ChatSession::new("s", "/tmp");
        session.split_on_message("m1");
        ensure_ai_message(&mut session).content = "one".into();
        // Reasoning after streamed text closes the current segment.
        session.split_on_thought();
        assert_eq!(session.messages.len(), 1);
        assert!(session.messages[0].is_complete);
        assert!(session.stream_msg_id.is_none());
        // A thought BEFORE any streamed text (a tool call opened the
        // bubble) stays in the same message - no split.
        ensure_ai_message(&mut session);
        session.split_on_thought();
        assert_eq!(session.messages.len(), 2);
        assert!(!session.messages[1].is_complete);
    }
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    #[test]
    fn dedup_chunk_slices_full_resends() {
        let mut sent = std::collections::HashMap::new();
        // Short prefixes pass through untouched (threshold 40, Nexus parity)
        // - separate entry id so it does not pollute the accumulation below.
        assert_eq!(dedup_chunk(&mut sent, "s", "m", "short", "hi"), "hi");
        // Long delta accumulates.
        let long = "a".repeat(50);
        assert_eq!(dedup_chunk(&mut sent, "s", "m", "current", &long), long);
        // Full resend: prefix >= 40 and text starts with it -> only the tail.
        let full = format!("{long}tail");
        assert_eq!(dedup_chunk(&mut sent, "s", "m", "current", &full), "tail");
        // Empty chunks emit nothing.
        assert_eq!(dedup_chunk(&mut sent, "s", "m", "current", ""), "");
        // A different entry id dedups independently.
        assert_eq!(dedup_chunk(&mut sent, "s", "m", "other", &full), full);
    }

    #[test]
    fn finalize_and_complete_close_open_blocks() {
        let mut session = ChatSession::new("s", "/tmp");
        let mut msg = Message::new_ai();
        msg.thinking.push(mistraldeck_core::types::ThinkingBlock {
            id: "thinking".into(),
            content: "hmm".into(),
            is_complete: false,
            started_at: 1,
            completed_at: 0,
        });
        msg.tool_calls.push(
            serde_json::from_str(
                r#"{"id":"tc1","title":"read_file","kind":"read","status":"running"}"#,
            )
            .unwrap(),
        );
        session.messages.push(msg);
        finalize_session_thinking(&mut session);
        complete_session_tool_calls(&mut session);
        let m = &session.messages[0];
        assert!(m.thinking[0].is_complete);
        assert!(m.thinking[0].completed_at > 0);
        assert_eq!(m.tool_calls[0].status, ToolStatus::Done);
    }
}


#[cfg(test)]
mod prompt_unblock_tests {
    use super::*;
    use crate::types::{SlotPurpose, SlotState};
    use mistraldeck_core::proto::AgentCommand;
    use mistraldeck_core::snapshot::AgentStatus;
    use std::sync::mpsc;

    #[test]
    fn failed_prompt_send_unblocks_slot() {
        let (tx, rx) = mpsc::channel();
        drop(rx); // channel closed -> send fails
        let mut s = SlotState {
            cmd: tx,
            purpose: SlotPurpose::NewLocal { cwd: "/tmp".into() },
            status: AgentStatus::Connected,
            error: String::new(),
            busy: true,
            last_activity: std::time::Instant::now(),
            pending_prompt: None,
            pending_images: Vec::new(),
            queue: std::collections::VecDeque::new(),
            pending_option: None,
            approval: None,
            question: None,
            usage: Default::default(),
            load_progress: None,
            commands: Vec::new(),
            config_options: Vec::new(),
            modes: Vec::new(),
            stream_sent: std::collections::HashMap::new(),
        };
        let failed = s
            .cmd
            .send(AgentCommand::Prompt {
                session_id: "x".into(),
                text: "hi".into(),
                images: Vec::new(),
            })
            .is_err();
        assert!(failed);
        mark_prompt_failed(&mut s);
        assert!(!s.busy);
        assert_eq!(s.status, AgentStatus::Disconnected);
        assert!(!s.error.is_empty());
    }
}
