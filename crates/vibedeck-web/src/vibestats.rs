//! Server functions for the Stats view: full session analytics parsed from
//! the Mistral Vibe CLI logs (`~/.vibe/logs/session/`), token-gated like the
//! rest of the remote API. Pure data - all filtering, bucketing and chart
//! shaping happens client-side.

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
            })
            .collect()
    }
}

#[server]
pub async fn api_vibe_stats(token: String) -> Result<Vec<SessionStatsRow>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::rows())
}
