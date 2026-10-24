//! Internal engine state (single source of truth).

use crate::AcpStatus;
use vibedeck_core::proto::{AcpSessionInfo, AvailableCommand, ConfigOption, ModeInfo};
use vibedeck_core::types::ChatSession;

pub struct EngineState {
    pub status: Option<AcpStatus>,
    pub acp_error: String,
    pub sessions: Vec<ChatSession>,
    pub current: Option<usize>,
    pub backend_sessions: Vec<AcpSessionInfo>,
    pub primed_session: Option<String>,
    pub priming: bool,
    pub loading_session: Option<String>,
    pub pending_prompt: Option<String>,
    pub busy: bool,
    pub modes: Vec<ModeInfo>,
    pub config_options: Vec<ConfigOption>,
    pub approval: Option<vibedeck_core::snapshot::PendingApproval>,
    pub question: Option<vibedeck_core::snapshot::PendingQuestion>,
    pub commands: Vec<AvailableCommand>,
    pub usage: vibedeck_core::snapshot::UsageStats,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            status: None,
            acp_error: String::new(),
            sessions: Vec::new(),
            current: None,
            backend_sessions: Vec::new(),
            primed_session: None,
            priming: false,
            loading_session: None,
            pending_prompt: None,
            busy: false,
            modes: Vec::new(),
            config_options: Vec::new(),
            approval: None,
            question: None,
            commands: Vec::new(),
            usage: Default::default(),
        }
    }
}

impl EngineState {
    pub fn status(&self) -> AcpStatus {
        self.status.unwrap_or(AcpStatus::Disconnected)
    }

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
}
