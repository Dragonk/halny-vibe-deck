//! Internal engine state (single source of truth) - multi-slot edition:
//! every session that talks to an agent owns a dedicated vibe-acp
//! process (AcpHandle), so prompts in different sessions run in
//! parallel.

use std::collections::HashMap;

use vibedeck_acp::AcpHandle;
use vibedeck_core::proto::{
    AcpSessionInfo, AvailableCommand, ConfigOption, ModeInfo,
};
use vibedeck_core::snapshot::{AcpStatus, PendingApproval, PendingQuestion, UsageStats};
use vibedeck_core::types::ChatSession;

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

/// One agent worker (a vibe-acp subprocess) bound to a local session.
pub struct SlotState {
    /// Command channel straight to the worker thread (the pump thread
    /// owns the event receiver).
    pub cmd: std::sync::mpsc::Sender<vibedeck_core::proto::AcpCommand>,
    pub purpose: SlotPurpose,
    pub status: AcpStatus,
    pub error: String,
    pub busy: bool,
    /// Prompt to send as soon as `session/new` completes.
    pub pending_prompt: Option<String>,
    /// Config option to apply as soon as `session/new` completes.
    pub pending_option: Option<(String, String)>,
    pub approval: Option<PendingApproval>,
    pub question: Option<PendingQuestion>,
    pub usage: UsageStats,
    pub commands: Vec<AvailableCommand>,
    pub config_options: Vec<ConfigOption>,
    pub modes: Vec<ModeInfo>,
}

pub struct EngineState {
    pub sessions: Vec<ChatSession>,
    pub current: Option<usize>,
    pub backend_sessions: Vec<AcpSessionInfo>,
    pub next_slot: u64,
    pub slots: HashMap<u64, SlotState>,
    /// ChatSession.id -> slot id
    pub slot_of: HashMap<String, u64>,
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

    pub fn session_by_acp_id_mut(&mut self, acp_session_id: &str) -> Option<&mut ChatSession> {
        self.sessions
            .iter_mut()
            .find(|s| s.acp_session_id.as_deref() == Some(acp_session_id))
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
    let handle = AcpHandle::spawn();
    let cmd = handle.cmd_sender();
    std::thread::Builder::new()
        .name(format!("vibedeck-slot-{slot_id}"))
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
        status: AcpStatus::Disconnected,
        error: String::new(),
        busy: false,
        pending_prompt: None,
        pending_option: None,
        approval: None,
        question: None,
        usage: UsageStats::default(),
        commands: Vec::new(),
        config_options: Vec::new(),
        modes: Vec::new(),
    }
}
