//! Internal engine state (single source of truth) - multi-slot edition:
//! every session that talks to an agent owns a dedicated vibe-app-server
//! process (ApsHandle), so prompts in different sessions run in
//! parallel.

use std::collections::HashMap;

use mistraldeck_appsrv::ApsHandle;
use mistraldeck_core::proto::{AgentSessionInfo, AvailableCommand, ConfigOption, ModeInfo};
use mistraldeck_core::snapshot::{
    AgentStatus, PendingApproval, PendingQuestion, SessionLoadProgress, UsageStats,
};
use mistraldeck_core::types::ChatSession;

/// Why a slot's process exists - decides what happens once the agent
/// handshake (`Initialized`) completes.
#[derive(Clone, Debug)]
pub enum SlotPurpose {
    /// Fresh local session -> run `session/new`.
    NewLocal { cwd: String },
    /// Mirror of a CLI session -> run `session/load` and replay history.
    LoadBackend { session_id: String, cwd: String },
    /// Fresh session on an SSH remote -> run `session/new` there.
    NewRemote { conn_id: String, cwd: String },
    /// Mirror of a remote CLI session -> `session/load` over ssh.
    LoadRemote {
        conn_id: String,
        session_id: String,
        cwd: String,
    },
}

/// One agent worker (a vibe-app-server subprocess) bound to a local session.
pub struct SlotState {
    /// Command channel straight to the worker thread (the pump thread
    /// owns the event receiver).
    pub cmd: std::sync::mpsc::Sender<mistraldeck_core::proto::AgentCommand>,
    pub purpose: SlotPurpose,
    pub status: AgentStatus,
    pub error: String,
    pub busy: bool,
    /// Last user-visible activity (prompt sent / turn finished) - the
    /// idle reaper closes slots idle longer than the configured window
    /// (Nexus v0.9.0 autoCloseIdleMinutes).
    pub last_activity: std::time::Instant,
    /// Prompt to send as soon as `session/new` completes.
    pub pending_prompt: Option<String>,
    /// Images belonging to `pending_prompt` (v1.0.8) — kept beside
    /// it so a lazy wake does not drop attachments.
    pub pending_images: Vec<mistraldeck_core::proto::PromptImage>,
    /// Server-side prompt queue (round 4, E6/E7): prompts that arrived
    /// while the slot was busy wait here and are sent one at a time when
    /// the running turn completes — so the queue is guarded even in
    /// headless mode with no UI client attached.
    pub queue: std::collections::VecDeque<(String, Vec<mistraldeck_core::proto::PromptImage>)>,
    /// Config option to apply as soon as `session/new` completes.
    pub pending_option: Option<(String, String)>,
    pub approval: Option<PendingApproval>,
    pub question: Option<PendingQuestion>,
    pub usage: UsageStats,
    /// `session/load` replay progress while a lazy-resumed session
    /// wakes up (set to `count: 0` when the wake starts, cleared on
    /// SessionReady / error) — drives the resuming overlay.
    pub load_progress: Option<SessionLoadProgress>,
    pub commands: Vec<AvailableCommand>,
    pub config_options: Vec<ConfigOption>,
    pub modes: Vec<ModeInfo>,
    /// Already-sent stream text per `session|kind|entry` (Nexus v0.10.2
    /// `streamSent`) — guards against full-entry resends from the CLI.
    pub stream_sent: std::collections::HashMap<String, String>,
}

pub struct EngineState {
    pub sessions: Vec<ChatSession>,
    pub current: Option<usize>,
    pub backend_sessions: Vec<AgentSessionInfo>,
    pub next_slot: u64,
    pub slots: HashMap<u64, SlotState>,
    /// ChatSession.id -> slot id
    pub slot_of: HashMap<String, u64>,
    /// Config options chosen on a session whose agent process does not
    /// exist yet — applied (in order) once `SessionReady` arrives.
    pub pending_options: HashMap<String, Vec<(String, String)>>,
    /// Session keys with a `session/cancel` in flight (Nexus v0.10.5
    /// `stoppingSessions`). Set by `Engine::cancel`, cleared when the
    /// turn really ends (PromptCompleted / SessionError / Disconnected)
    /// or a new turn starts.
    pub stopping: std::collections::HashSet<String>,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            sessions: Vec::new(),
            current: None,
            backend_sessions: Vec::new(),
            next_slot: 1,
            slots: HashMap::new(),
            slot_of: HashMap::new(),
            pending_options: HashMap::new(),
            stopping: std::collections::HashSet::new(),
        }
    }
}

impl EngineState {
    pub fn current_session(&self) -> Option<&ChatSession> {
        self.current.and_then(|i| self.sessions.get(i))
    }

    pub fn current_session_mut(&mut self) -> Option<&mut ChatSession> {
        match self.current {
            Some(i) => self.sessions.get_mut(i),
            None => None,
        }
    }

    pub fn session_by_agent_id_mut(&mut self, agent_session_id: &str) -> Option<&mut ChatSession> {
        self.sessions
            .iter_mut()
            .find(|s| s.agent_session_id.as_deref() == Some(agent_session_id))
    }

    /// The local session bound to a slot (if any).
    pub fn session_of_slot(&self, slot_id: u64) -> Option<usize> {
        let key = self
            .slot_of
            .iter()
            .find(|(_, s)| **s == slot_id)
            .map(|(k, _)| k.clone())?;
        self.sessions.iter().position(|s| s.id == key)
    }

    pub fn slot_of_session(&self, session_key: &str) -> Option<u64> {
        self.slot_of.get(session_key).copied()
    }
}

/// Spawn a worker + its pump thread; returns (slot id, command sender).
/// The pump forwards every event to `Engine::apply_slot_event`.
pub fn spawn_slot(slot_id: u64, purpose: SlotPurpose) -> SlotState {
    let handle = ApsHandle::spawn();
    let cmd = handle.cmd_sender();
    std::thread::Builder::new()
        .name(format!("mistraldeck-slot-{slot_id}"))
        .spawn(move || {
            let rx = handle.events();
            loop {
                match rx.recv() {
                    Ok(ev) => {
                        if let Some(engine) = crate::Engine::try_global() {
                            engine.apply_slot_event(slot_id, ev);
                        }
                    }
                    Err(_) => return,
                }
            }
        })
        .expect("failed to spawn slot pump");
    SlotState {
        cmd,
        purpose,
        status: AgentStatus::Disconnected,
        error: String::new(),
        busy: false,
        last_activity: std::time::Instant::now(),
        pending_prompt: None,
        pending_images: Vec::new(),
        queue: std::collections::VecDeque::new(),
        pending_option: None,
        approval: None,
        question: None,
        usage: UsageStats::default(),
        load_progress: None,
        commands: Vec::new(),
        config_options: Vec::new(),
        modes: Vec::new(),
        stream_sent: std::collections::HashMap::new(),
    }
}
