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

/// One content-search hit: the session id plus a snippet around the first
/// match (History view content search, Nexus `searchSessionContent`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentMatchView {
    pub session_id: String,
    pub snippet: String,
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
    #[serde(default)]
    pub tool_name: String,
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

/// Context window + auto-compact threshold of the current model
/// (UsageBar gauge, Nexus v0.13.5). `window = None` = no gauge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UsageModelInfoView {
    pub window: Option<f64>,
    pub threshold: Option<f64>,
}

#[cfg(feature = "server")]
mod imp {
    use super::SessionStatsRow;
    use super::*;

    pub fn check(_token: &str) -> Result<(), ServerFnError> {
        // Same auth model as api.rs: no token enforcement inside the
        // server functions — the remote HTTPS listener's middleware has
        // already gated the request, and the loopback listener is trusted.
        Ok(())
    }

    pub fn rows() -> Vec<SessionStatsRow> {
        mistraldeck_vibe::stats::load_sessions()
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
        mistraldeck_vibe::stats::load_subagent_sessions(parent)
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
        mistraldeck_vibe::stats::load_session_preview(session_id).map(|messages| {
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
                            tool_name: t.tool_name,
                        })
                        .collect(),
                })
                .collect()
        })
    }

    /// Full tool-result content for one tool call of a session, read from
    /// the on-disk message log (Nexus `stats:vibe-session-messages` +
    /// BashBlock's "Ran X" enrichment).
    pub fn tool_result(session_id: &str, tool_call_id: &str) -> Option<String> {
        mistraldeck_vibe::stats::load_session_messages(session_id)
            .into_iter()
            .find(|m| {
                m.role == "tool" && m.tool_call_id.as_deref() == Some(tool_call_id)
            })
            .and_then(|m| m.content)
    }

    pub fn content_matches(query: &str) -> Vec<ContentMatchView> {
        mistraldeck_vibe::stats::search_session_content(query)
            .into_iter()
            .map(|m| ContentMatchView {
                session_id: m.session_id,
                snippet: m.snippet,
            })
            .collect()
    }

    pub fn usage_model_info(model: &str) -> UsageModelInfoView {
        let (window, threshold) = mistraldeck_vibe::config::usage_model_info(model);
        UsageModelInfoView {
            window: window.map(|v| v as f64),
            threshold: threshold.map(|v| v as f64),
        }
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

/// Full tool-result content for one tool call of a session (chat BashBlock
/// enrichment when the wire output is only a "Ran X" summary).
#[server]
pub async fn api_session_tool_result(
    token: String,
    session_id: String,
    tool_call_id: String,
) -> Result<Option<String>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::tool_result(&session_id, &tool_call_id))
}

/// Content search across stored session transcripts (History view).
#[server]
pub async fn api_search_session_content(
    token: String,
    query: String,
) -> Result<Vec<ContentMatchView>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::content_matches(&query))
}

/// Context window + auto-compact threshold of a model (UsageBar gauge).
#[server]
pub async fn api_usage_model_info(
    token: String,
    model: String,
) -> Result<UsageModelInfoView, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::usage_model_info(&model))
}
