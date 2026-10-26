//! Server functions for the Stats view: full session analytics parsed from
//! the Mistral Vibe CLI logs (`~/.vibe/logs/session/`), token-gated like the
//! rest of the remote API. Pure data - all filtering, bucketing and chart
//! shaping happens client-side. Also exposes subagent transcripts for the
//! chat view's subagent tool-call blocks.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionStatsRow {
    pub session_id: String,
    pub title: String,
    pub model: String,
    /// ISO start time, e.g. `2026-10-24T20:20:43.606000+00:00`
    pub start_time: String,
    pub end_time: String,
    pub working_directory: String,
    pub messages: u64,
    pub prompt_tokens: f64,
    pub completion_tokens: f64,
    pub cached_tokens: f64,
    pub total_tokens: f64,
    pub cost: f64,
    pub tokens_per_second: f64,
    pub tool_succeeded: f64,
    pub tool_failed: f64,
    pub tool_rejected: f64,
    /// All models used across generations (unified sessions).
    #[serde(default)]
    pub models: Vec<String>,
}

/// One message of a subagent transcript (role: `user` | `assistant`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentMsg {
    pub role: String,
    pub content: String,
}

/// A subagent session spawned by a parent session, with its full transcript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubagentSessionView {
    pub session_id: String,
    pub agent_name: String,
    pub start_time: String,
    pub messages: Vec<SubagentMsg>,
}

/// One thinking block of a session preview message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewThinkingView {
    pub id: String,
    pub content: String,
}

/// One tool call line of a session preview message (`done` | `error`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewToolCallView {
    pub title: String,
    pub status: String,
}

/// One message of a session preview (Nexus `SubagentWorkData.messages`) —
/// the subagent "Show work" card transcript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewMessageView {
    pub role: String,
    pub content: String,
    pub thinking: Vec<PreviewThinkingView>,
    pub tool_calls: Vec<PreviewToolCallView>,
}

#[cfg(feature = "server")]
mod imp {
    use super::SessionStatsRow;
    use super::*;
    use vibedeck_engine::Engine;

    pub fn check(token: &str) -> Result<(), ServerFnError> {
        let engine = Engine::global();
        if engine.auth_enabled() && !engine.check_token(token) {
            return Err(ServerFnError::new("unauthorized"));
        }
        Ok(())
    }

    pub fn rows() -> Vec<SessionStatsRow> {
        vibedeck_vibe::stats::load_sessions()
            .into_iter()
            .map(|s| SessionStatsRow {
                session_id: s.session_id.clone(),
                title: s.title.clone().unwrap_or_default(),
                model: s
                    .config
                    .as_ref()
                    .and_then(|c| c.active_model.clone())
                    .unwrap_or_default(),
                start_time: s.start_time.clone(),
                end_time: s.end_time.clone(),
                working_directory: s
                    .environment
                    .as_ref()
                    .and_then(|e| e.working_directory.clone())
                    .unwrap_or_default(),
                messages: s.total_messages.unwrap_or(0),
                prompt_tokens: s.stats.session_prompt_tokens,
                completion_tokens: s.stats.session_completion_tokens,
                cached_tokens: s.stats.cached_tokens,
                total_tokens: s.stats.session_total_llm_tokens,
                cost: s.stats.session_cost,
                tokens_per_second: s.stats.tokens_per_second,
                tool_succeeded: s.stats.tool_calls_succeeded,
                tool_failed: s.stats.tool_calls_failed,
                tool_rejected: s.stats.tool_calls_rejected,
                models: s.models.clone(),
            })
            .collect()
    }

    pub fn subagent_sessions(parent: &str) -> Vec<SubagentSessionView> {
        vibedeck_vibe::stats::load_subagent_sessions(parent)
            .into_iter()
            .map(|s| SubagentSessionView {
                session_id: s.session_id,
                agent_name: s.agent_name,
                start_time: s.start_time,
                messages: s
                    .messages
                    .into_iter()
                    .map(|m| SubagentMsg {
                        role: m.role,
                        content: m.content.unwrap_or_default(),
                    })
                    .collect(),
            })
            .collect()
    }

    pub fn session_preview(session_id: &str) -> Option<Vec<PreviewMessageView>> {
        vibedeck_vibe::stats::load_session_preview(session_id).map(|messages| {
            messages
                .into_iter()
                .map(|m| PreviewMessageView {
                    role: m.role,
                    content: m.content,
                    thinking: m
                        .thinking
                        .into_iter()
                        .map(|t| PreviewThinkingView {
                            id: t.id,
                            content: t.content,
                        })
                        .collect(),
                    tool_calls: m
                        .tool_calls
                        .into_iter()
                        .map(|t| PreviewToolCallView {
                            title: t.title,
                            status: t.status,
                        })
                        .collect(),
                })
                .collect()
        })
    }
}

#[server]
pub async fn api_vibe_stats(token: String) -> Result<Vec<SessionStatsRow>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::rows())
}

/// Subagent transcripts for a parent CLI session (chat subagent blocks).
#[server]
pub async fn api_subagent_logs(
    token: String,
    parent_session_id: String,
) -> Result<Vec<SubagentSessionView>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::subagent_sessions(&parent_session_id))
}

/// Session preview (subagent "Show work" card): the child session's
/// transcript from the unified store. `None` = transcript unavailable.
#[server]
pub async fn api_session_preview(
    token: String,
    session_id: String,
) -> Result<Option<Vec<PreviewMessageView>>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::session_preview(&session_id))
}
