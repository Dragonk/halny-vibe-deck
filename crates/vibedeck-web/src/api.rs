//! Server functions: the only bridge between the UI client and the engine.
//! Every mutating call carries the auth token (checked server-side).

use dioxus::prelude::*;
use vibedeck_core::snapshot::Snapshot;

#[cfg(feature = "server")]
fn check(token: &str) -> Result<(), ServerFnError> {
    let engine = vibedeck_engine::Engine::global();
    if engine.auth_enabled() && !engine.check_token(token) {
        return Err(ServerFnError::new("unauthorized"));
    }
    Ok(())
}

/// Does the server require login? (safe to call without a token)
#[server]
pub async fn api_auth_state() -> Result<bool, ServerFnError> {
    Ok(vibedeck_engine::Engine::global().auth_enabled())
}

#[server]
pub async fn api_login(username: String, password: String) -> Result<Option<String>, ServerFnError> {
    Ok(vibedeck_engine::Engine::global().login(&username, &password))
}

#[server]
pub async fn api_logout(token: String) -> Result<(), ServerFnError> {
    vibedeck_engine::Engine::global().logout(&token);
    Ok(())
}

#[server]
pub async fn api_snapshot(token: String) -> Result<Snapshot, ServerFnError> {
    check(&token)?;
    Ok(vibedeck_engine::Engine::global().snapshot())
}

#[server]
pub async fn api_create_session(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().create_local_session();
    Ok(())
}

#[server]
pub async fn api_switch_session(token: String, index: usize) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().switch_session(index);
    Ok(())
}

#[server]
pub async fn api_open_backend_session(token: String, session_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().open_backend_session(&session_id);
    Ok(())
}

#[server]
pub async fn api_send_message(token: String, text: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().send_message(text);
    Ok(())
}

#[server]
pub async fn api_cancel(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().cancel();
    Ok(())
}

#[server]
pub async fn api_approve(token: String, request_id: String, option_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().approve(request_id, option_id);
    Ok(())
}

#[server]
pub async fn api_answer_question(token: String, request_id: String, answer: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().answer_question(request_id, answer);
    Ok(())
}

#[server]
pub async fn api_set_config_option(token: String, key: String, value: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().set_config_option(&key, value);
    Ok(())
}

#[server]
pub async fn api_refresh_sessions(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().refresh_sessions();
    Ok(())
}

/// Persist UI preferences: language, theme, dock open, sidebar limit.
#[server]
pub async fn api_set_prefs(
    token: String,
    language: String,
    theme: String,
    dock_open: bool,
    sidebar_sessions_limit: u32,
) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().set_prefs(language, theme, dock_open, sidebar_sessions_limit);
    Ok(())
}

/// Configure remote-control access (login + password + port + enabled).
/// The password is hashed (sha256 with salt) server-side; nothing is
/// ever displayed back to the client.
#[server]
pub async fn api_set_remote(
    token: String,
    username: String,
    password: String,
    port: u16,
    enable: bool,
) -> Result<(), ServerFnError> {
    check(&token)?;
    if username.trim().is_empty() || password.is_empty() {
        return Err(ServerFnError::new("username and password required"));
    }
    vibedeck_engine::Engine::global()
        .set_remote(username.trim(), &password, port, enable);
    Ok(())
}

/// Hide a session from the sidebar (nothing is deleted from disk).
#[server]
pub async fn api_hide_session(token: String, key: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().hide_session(&key);
    Ok(())
}

#[server]
pub async fn api_hide_project(token: String, wd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().hide_project(&wd);
    Ok(())
}

#[server]
pub async fn api_restore_session(token: String, key: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().restore_session(&key);
    Ok(())
}

#[server]
pub async fn api_restore_project(token: String, wd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    vibedeck_engine::Engine::global().restore_project(&wd);
    Ok(())
}
