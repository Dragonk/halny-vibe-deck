//! Server functions: the only bridge between the UI client and the engine.
//!
//! Auth model (Nexus parity): the local server binds 127.0.0.1 and is
//! trusted without a login; remote browser access goes through the HTTPS
//! listener (serve.rs) whose middleware authenticates every request with a
//! session cookie before it can reach any of these functions. The token
//! parameters are kept for protocol compatibility and are not enforced.

use dioxus::prelude::*;
use mistraldeck_core::snapshot::Snapshot;

#[cfg(feature = "server")]
fn check(_token: &str) -> Result<(), ServerFnError> {
    // No token enforcement: requests arrive either from the local
    // 127.0.0.1 listener (trusted) or through the remote HTTPS listener,
    // whose middleware already validated the session cookie.
    Ok(())
}

/// Local-only guard: reject requests that were routed through the remote
/// HTTPS listener (marked `x-vd-origin: remote` by its middleware). Used
/// for credential editing — login/password/port can only be changed from
/// the local interface.
#[cfg(feature = "server")]
fn require_local() -> Result<(), ServerFnError> {
    if let Some(ctx) = dioxus_fullstack::FullstackContext::current() {
        let headers = ctx.parts_mut().headers.clone();
        let origin = headers
            .get("x-vd-origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        if origin == "remote" {
            return Err(ServerFnError::new(
                "credentials can only be edited from the local interface",
            ));
        }
    }
    Ok(())
}

#[server]
pub async fn api_snapshot(token: String) -> Result<Snapshot, ServerFnError> {
    check(&token)?;
    Ok(mistraldeck_engine::Engine::global().snapshot())
}

/// Re-read the current session from the agent before the client refreshes.
///
/// Called on visibility-resync (hidden tab, sleeping laptop, dropped
/// websocket). Without it a client-side snapshot refetch only sees what the
/// server projection already knew — messages the agent emitted during the
/// gap would stay missing until the user reloaded the page (the stable
/// edition's bug). The engine answers immediately; the authoritative state
/// arrives as bus events and the client's refresh picks it up.
#[server]
pub async fn api_resync_current(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().resync_current_session();
    Ok(())
}

// ---------------------------------------------------------------------------
// L1: login & account (setup/* + account/read + identity/read)
// ---------------------------------------------------------------------------

/// Result of `api_login_start`: the URL to open in the user's
/// browser and the poll deadline (epoch ms) from the server.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoginStartView {
    pub sign_in_url: String,
    pub expires_at_ms: i64,
}

/// Trigger the L1 preflow: `setup/status` + `identity/read` +
/// `account/read`. Results arrive through the snapshot (`setup`,
/// `identity`, `account` fields) — the UI just refreshes.
#[server]
pub async fn api_setup_account_status(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().setup_refresh();
    Ok(())
}

/// Start the browser sign-in flow (PKCE begin — blocking curl on
/// the engine side, so off the async runtime).
#[server]
pub async fn api_login_start(token: String) -> Result<LoginStartView, ServerFnError> {
    check(&token)?;
    let (sign_in_url, expires_at_ms) =
        tokio::task::spawn_blocking(|| mistraldeck_engine::Engine::global().login_browser_start())
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .map_err(ServerFnError::new)?;
    Ok(LoginStartView {
        sign_in_url,
        expires_at_ms,
    })
}

/// One poll step of the browser flow (client calls every 3 s).
/// Returns "pending" | "completed" | "expired" | "denied"; network
/// / protocol failures come back as Err with the detail.
#[server]
pub async fn api_login_poll(token: String) -> Result<String, ServerFnError> {
    check(&token)?;
    let outcome = tokio::task::spawn_blocking(|| {
        mistraldeck_engine::Engine::global().login_browser_poll()
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    match outcome {
        mistraldeck_engine::PollOutcome::Pending => Ok("pending".to_string()),
        mistraldeck_engine::PollOutcome::Completed { .. } => Ok("completed".to_string()),
        mistraldeck_engine::PollOutcome::Expired => Ok("expired".to_string()),
        mistraldeck_engine::PollOutcome::Denied => Ok("denied".to_string()),
        mistraldeck_engine::PollOutcome::Failed(msg) => Err(ServerFnError::new(msg)),
    }
}

/// Login with a pasted api key: `setup/store-credential` (+ the
/// pump-chained `setup/submit-choices`). Err carries the outcome
/// detail (`env_var_error` / `save_error`). Blocking wait — off
/// the async runtime.
#[server]
pub async fn api_login_with_api_key(
    token: String,
    api_key: String,
    custom_domain: Option<String>,
) -> Result<(), ServerFnError> {
    check(&token)?;
    if api_key.trim().is_empty() {
        return Err(ServerFnError::new("api key is empty"));
    }
    let res = tokio::task::spawn_blocking(move || {
        mistraldeck_engine::Engine::global().login_with_api_key(api_key.trim(), custom_domain.as_deref())
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    let (outcome, detail) = res.map_err(ServerFnError::new)?;
    match outcome.as_str() {
        "completed" => Ok(()),
        // "unsupported" = method_not_found (old CLI) — degrade.
        other => Err(ServerFnError::new(match detail {
            Some(d) => format!("{other}: {d}"),
            None => other.to_string(),
        })),
    }
}

/// New session in a project (D1: a session always has a working directory).
#[server]
pub async fn api_create_session(token: String, cwd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().create_local_session_in(&cwd);
    Ok(())
}

#[server]
pub async fn api_switch_session(token: String, index: usize) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().switch_session(index);
    Ok(())
}

#[server]
pub async fn api_open_backend_session(token: String, session_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().open_backend_session(&session_id);
    Ok(())
}

/// Send a prompt. `@path` markers in the text are resolved server-side
/// (Nexus parity): every readable file/directory under the working
/// directory is inlined as a `<vibe_attach_*>` block and recorded on the
/// user message so the UI can render attachment chips. Unresolvable
/// markers stay as typed.
#[server]
pub async fn api_send_message(token: String, text: String) -> Result<(), ServerFnError> {
    check(&token)?;
    let (expanded, attachments) = resolve_attachments(&text);
    mistraldeck_engine::Engine::global().send_message_with(expanded, attachments);
    Ok(())
}

#[server]
pub async fn api_cancel(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().cancel();
    Ok(())
}

#[server]
pub async fn api_approve(token: String, request_id: String, option_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().approve(request_id, option_id);
    Ok(())
}

#[server]
pub async fn api_answer_question(token: String, request_id: String, answer: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().answer_question(request_id, answer);
    Ok(())
}

/// Execute a TUI-style slash command client-side (compact/clear/rename/
/// reload/help). Returns true when the command was recognized and must NOT
/// be forwarded as a prompt (unknown `/name` = skill invocation).
#[server]
pub async fn api_slash_command(token: String, command: String, args: String) -> Result<bool, ServerFnError> {
    check(&token)?;
    Ok(mistraldeck_engine::Engine::global().slash_command(&command, &args))
}

/// `!<command>` → `session/shellCommand` on the current session's agent.
#[server]
pub async fn api_shell_command(token: String, command: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().shell_command(command);
    Ok(())
}

/// Local system notice in the current transcript (e.g. `/help` output).
#[server]
pub async fn api_local_notice(token: String, text: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().local_notice(&text);
    Ok(())
}

/// `/branch [entryId]` → `session/fork` of the current session.
#[server]
pub async fn api_fork_session(token: String, entry_id: Option<String>) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().fork_session(entry_id);
    Ok(())
}

/// `/rewind <entryId>` → `session/rewind` on the current session.
#[server]
pub async fn api_rewind_session(
    token: String,
    entry_id: String,
    restore_files: bool,
) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().rewind_session(entry_id, restore_files);
    Ok(())
}

/// `session/rewind/read` — which files a rewind to `entry_id` would restore.
#[server]
pub async fn api_rewind_read(token: String, entry_id: String) -> Result<bool, ServerFnError> {
    check(&token)?;
    Ok(mistraldeck_engine::Engine::global().rewind_read(entry_id))
}

/// `session/pin` — pin/unpin a backend session (GUI extra).
#[server]
pub async fn api_pin_session(token: String, session_id: String, pinned: bool) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().pin_session(&session_id, pinned);
    Ok(())
}

/// `session/archive` — archive/unarchive (GUI extra).
#[server]
pub async fn api_archive_session(token: String, session_id: String, archived: bool) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().archive_session(&session_id, archived);
    Ok(())
}

/// `session/markAsSeen` — clear the unread dot.
#[server]
pub async fn api_mark_session_seen(token: String, session_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().mark_session_seen(&session_id);
    Ok(())
}

/// `session/delete` — delete a backend session from disk.
#[server]
pub async fn api_delete_backend_session(token: String, session_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().delete_backend_session(&session_id);
    Ok(())
}

// --- teleport to Vibe Code Web ---

#[server]
pub async fn api_teleport_open(token: String, prompt: Option<String>) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_open(prompt);
    Ok(())
}

#[server]
pub async fn api_teleport_select(token: String, picker_id: String, project_id: String, prompt: Option<String>) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_select(&picker_id, &project_id, prompt);
    Ok(())
}

#[server]
pub async fn api_teleport_create(token: String, picker_id: String, name: String, default_branch: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_create(&picker_id, &name, &default_branch);
    Ok(())
}

#[server]
pub async fn api_teleport_load_more(token: String, picker_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_load_more(&picker_id);
    Ok(())
}

#[server]
pub async fn api_teleport_picker_cancel(token: String, picker_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_picker_cancel(&picker_id);
    Ok(())
}

#[server]
pub async fn api_teleport_push_respond(token: String, operation_id: String, approved: bool) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_push_respond(&operation_id, approved);
    Ok(())
}

#[server]
pub async fn api_teleport_abort(token: String, operation_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().teleport_abort(&operation_id);
    Ok(())
}

// --- loops admin / logs / worktrees / artifacts ---

#[server]
pub async fn api_loops_list(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().loops_list();
    Ok(())
}

#[server]
pub async fn api_loops_delete(token: String, loop_id: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().loops_delete(&loop_id);
    Ok(())
}

#[server]
pub async fn api_loops_clear(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().loops_clear();
    Ok(())
}

#[server]
pub async fn api_session_log_read(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().session_log_read();
    Ok(())
}

#[server]
pub async fn api_diagnostics_logs_read(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().diagnostics_logs_read();
    Ok(())
}

#[server]
pub async fn api_worktrees_list(token: String, cwd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().worktrees_list(&cwd);
    Ok(())
}

#[server]
pub async fn api_artifacts_list(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().artifacts_list();
    Ok(())
}

#[server]
pub async fn api_set_config_option(token: String, key: String, value: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().set_config_option(&key, value);
    Ok(())
}

/// Re-index the history list from the local session store (no agent
/// process — Z5).
#[server]
pub async fn api_refresh_sessions(token: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().refresh_sessions_from_disk();
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
    mistraldeck_engine::Engine::global().set_prefs(language, theme, dock_open, sidebar_sessions_limit);
    Ok(())
}

/// General settings (SettingsView general tab): idle timeout.
#[server]
pub async fn api_set_general(
    token: String,
    auto_close_idle_minutes: u32,
) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().set_general(auto_close_idle_minutes);
    Ok(())
}

/// Configure remote-access credentials (login + password + port + enabled).
/// The password is hashed (sha256 with salt) server-side; nothing is
/// ever displayed back to the client. LOCAL-ONLY: requests routed through
/// the remote HTTPS listener are rejected (credentials must be edited
/// from the local interface).
#[server]
pub async fn api_set_remote(
    token: String,
    username: String,
    password: String,
    port: u16,
    enable: bool,
) -> Result<(), ServerFnError> {
    check(&token)?;
    require_local()?;
    if username.trim().is_empty() || password.is_empty() {
        return Err(ServerFnError::new("username and password required"));
    }
    mistraldeck_engine::Engine::global()
        .set_remote(username.trim(), &password, port, enable);
    Ok(())
}

/// Access hint for the Settings "Server" tab: local + LAN HTTPS URLs,
/// remote listener state and the certificate in use (device IPs included).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ServeInfoView {
    pub remote_enabled: bool,
    pub remote_port: u16,
    pub auth_configured: bool,
    pub custom_cert: bool,
    pub cert_path: String,
    pub key_path: String,
    pub urls: Vec<String>,
}

#[server]
pub async fn api_serve_info(token: String) -> Result<ServeInfoView, ServerFnError> {
    check(&token)?;
    Ok(crate::serve::serve_info())
}

/// Set the custom TLS certificate paths (empty = generated self-signed).
/// LOCAL-ONLY, like credential editing.
#[server]
pub async fn api_set_remote_certs(
    token: String,
    cert_path: String,
    key_path: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    require_local()?;
    mistraldeck_engine::Engine::global().set_remote_certs(&cert_path, &key_path);
    Ok(())
}

/// Hide a session from the sidebar (nothing is deleted from disk).
#[server]
pub async fn api_hide_session(token: String, key: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().hide_session(&key);
    Ok(())
}

#[server]
pub async fn api_hide_project(token: String, wd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().hide_project(&wd);
    Ok(())
}

#[server]
pub async fn api_restore_session(token: String, key: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().restore_session(&key);
    Ok(())
}

#[server]
pub async fn api_restore_project(token: String, wd: String) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().restore_project(&wd);
    Ok(())
}

// ---------------------------------------------------------------------------
// @-attachment resolution (server-side, Nexus parity)
// ---------------------------------------------------------------------------

/// Expand `@path` markers in a prompt into inline `<vibe_attach_*>`
/// blocks (like the Nexus InputBar) and collect display attachments.
/// Only paths inside the current working directory are readable; the
/// content is capped so a huge file cannot blow up the prompt.
#[cfg(feature = "server")]
fn resolve_attachments(text: &str) -> (String, Vec<mistraldeck_core::types::Attachment>) {
    const MAX_FILE_BYTES: usize = 256 * 1024;
    const MAX_DIR_ENTRIES: usize = 400;
    // v1.0.8 image contract (CLI parity): base64 images for the LLM.
    const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
    const MAX_IMAGES: usize = 8;

    let engine = mistraldeck_engine::Engine::global();
    // round 8: resolve against the CURRENT SESSION's working directory (and
    // over SSH for remote sessions) — the engine's global workdir made
    // `@file` markers in a remote (or non-default project) chat silently
    // resolve against the wrong local root
    let (cwd, remote) = engine.current_session_ctx();
    let mut expanded = text.to_string();
    let mut attachments: Vec<mistraldeck_core::types::Attachment> = Vec::new();
    let mut image_count = 0usize;

    if cwd.is_empty() || !text.contains('@') {
        return (expanded, attachments);
    }

    // Tokens starting with `@` (Nexus regex: /@([^\s]+)/g) — collect unique
    // markers first so the same path referenced twice is only read once.
    let mut markers: Vec<String> = Vec::new();
    for tok in text.split_whitespace() {
        if let Some(raw) = tok.strip_prefix('@') {
            if !raw.is_empty() && !markers.iter().any(|m| m == tok) {
                markers.push(tok.to_string());
            }
        }
    }

    // Remote session: read the markers on the host over SSH.
    if let Some(rid) = &remote {
        if let Some(conn) = engine.remote_connection(rid) {
            let base = cwd.trim_end_matches('/');
            for marker in &markers {
                let raw = &marker[1..];
                let is_dir = raw.ends_with('/');
                let rel = raw.trim_end_matches('/');
                if rel.is_empty() || rel.contains("..") || rel.starts_with('/') {
                    continue;
                }
                let full = format!("{base}/{rel}");
                let q = mistraldeck_engine::remote::shq(&full);
                let probe = mistraldeck_engine::remote::ssh_run(
                    &conn,
                    &format!("if [ -d {q} ]; then echo DIR; elif [ -f {q} ]; then echo FILE; else echo MISSING; fi"),
                    8000,
                    None,
                );
                let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
                let (block, content, mime, data_base64) = match probe.stdout.trim() {
                    "DIR" => {
                        let ls = mistraldeck_engine::remote::ssh_run(
                            &conn,
                            &format!("ls -1 {q} 2>/dev/null | head -n {MAX_DIR_ENTRIES}"),
                            8000,
                            None,
                        );
                        let listing = ls.stdout.trim_end_matches('\n').to_string();
                        (
                            format!(
                                "<vibe_attach_directory><name>{name}</name><location>{raw}</location><content>{listing}</content></vibe_attach_directory>"
                            ),
                            listing,
                            String::new(),
                            String::new(),
                        )
                    }
                    "FILE" => {
                        let meta = mistraldeck_engine::remote::ssh_run(
                            &conn,
                            &format!("stat -c %s {q} 2>/dev/null || stat -f %z {q}"),
                            8000,
                            None,
                        );
                        let size: u64 = meta.stdout.trim().parse().unwrap_or(0);
                        let path = std::path::PathBuf::from(&full);
                        if let Some(mime) = image_mime(&path) {
                            if image_count >= MAX_IMAGES || size > MAX_IMAGE_BYTES as u64 || size == 0 {
                                continue;
                            }
                            image_count += 1;
                            let b64r = mistraldeck_engine::remote::ssh_run(
                                &conn,
                                &format!("base64 {q} 2>/dev/null || base64 -i {q}"),
                                15000,
                                None,
                            );
                            (
                                format!(
                                    "<vibe_attach_image><name>{name}</name><location>{raw}</location></vibe_attach_image>"
                                ),
                                String::new(),
                                mime.to_string(),
                                b64r.stdout.trim().to_string(),
                            )
                        } else {
                            if size == 0 || size > MAX_FILE_BYTES as u64 {
                                continue;
                            }
                            let cat = mistraldeck_engine::remote::ssh_run(
                                &conn,
                                &format!("cat {q}"),
                                15000,
                                None,
                            );
                            let content = cat.stdout;
                            (
                                format!(
                                    "<vibe_attach_file><name>{name}</name><location>{raw}</location><content>{content}</content></vibe_attach_file>"
                                ),
                                content,
                                String::new(),
                                String::new(),
                            )
                        }
                    }
                    _ => continue,
                };
                if let Some(pos) = expanded.find(marker.as_str()) {
                    expanded.replace_range(pos..pos + marker.len(), &block);
                }
                attachments.push(mistraldeck_core::types::Attachment {
                    path: raw.to_string(),
                    is_dir,
                    content,
                    mime,
                    data_base64,
                });
            }
            return (expanded, attachments);
        }
    }

    let root = std::path::PathBuf::from(&cwd);
    for marker in markers {
        let raw = &marker[1..]; // strip '@'
        let is_dir = raw.ends_with('/');
        let rel = raw.trim_end_matches('/');
        if rel.is_empty() || rel.contains("..") || rel.starts_with('/') {
            continue; // no traversal outside the working directory
        }
        let full = root.join(rel);
        let (block, content, mime, data_base64) = if is_dir {
            let listing = match std::fs::read_dir(&full) {
                Ok(entries) => {
                    let mut names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .take(MAX_DIR_ENTRIES)
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            if e.path().is_dir() {
                                format!("{}/", name)
                            } else {
                                name
                            }
                        })
                        .collect();
                    names.sort();
                    names.join("\n")
                }
                Err(_) => continue, // not a readable directory — leave marker as typed
            };
            let name = full
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| rel.to_string());
            (
                format!(
                    "<vibe_attach_directory><name>{name}</name><location>{raw}</location><content>{listing}</content></vibe_attach_directory>"
                ),
                listing,
                String::new(),
                String::new(),
            )
        } else if let Some(mime) = image_mime(&full) {
            // v1.0.8: images must reach the LLM as base64 content
            // blocks — inlining the bytes as text is useless. The
            // marker stays in the text as a location note only.
            if image_count >= MAX_IMAGES {
                continue;
            }
            let bytes = match std::fs::metadata(&full).map(|m| m.len()) {
                Ok(len) if len <= MAX_IMAGE_BYTES as u64 => std::fs::read(&full),
                _ => continue, // missing or too large — leave marker as typed
            };
            let bytes = match bytes {
                Ok(b) => b,
                Err(_) => continue,
            };
            image_count += 1;
            let name = full
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| rel.to_string());
            use base64::Engine as _;
            (
                format!(
                    "<vibe_attach_image><name>{name}</name><location>{raw}</location></vibe_attach_image>"
                ),
                String::new(),
                mime.to_string(),
                base64::engine::general_purpose::STANDARD.encode(&bytes),
            )
        } else {
            let bytes = match std::fs::metadata(&full).map(|m| m.len()) {
                Ok(len) if len <= MAX_FILE_BYTES as u64 => std::fs::read_to_string(&full),
                _ => continue, // missing or too large — leave marker as typed
            };
            let content = match bytes {
                Ok(c) => c,
                Err(_) => continue, // binary / unreadable — leave marker as typed
            };
            let name = full
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| rel.to_string());
            (
                format!(
                    "<vibe_attach_file><name>{name}</name><location>{raw}</location><content>{content}</content></vibe_attach_file>"
                ),
                content,
                String::new(),
                String::new(),
            )
        };
        attachments.push(mistraldeck_core::types::Attachment {
            path: raw.to_string(),
            is_dir,
            content,
            mime,
            data_base64,
        });
        // Replace only the first occurrence of this marker (Nexus does the
        // same via String.replace).
        if let Some(pos) = expanded.find(marker.as_str()) {
            expanded.replace_range(pos..pos + marker.len(), &block);
        }
    }

    (expanded, attachments)
}

/// v1.0.8: image extensions forwarded to the LLM as base64 blocks
/// (PNG/JPEG/GIF/WebP — the CLI contract).
#[cfg(feature = "server")]
fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// SSH remote connections (Nexus parity)
// ---------------------------------------------------------------------------

/// Saved SSH remote connections (Settings editor + sidebar groups).
#[server]
pub async fn api_remote_connections(
    token: String,
) -> Result<Vec<mistraldeck_core::snapshot::RemoteConnection>, ServerFnError> {
    check(&token)?;
    Ok(mistraldeck_engine::Engine::global().config().remote_connections)
}

/// Persist the full remote-connection list. Allowed through the remote
/// HTTPS listener too (Nexus only protects the serve/credentials section).
#[server]
pub async fn api_save_remote_connections(
    token: String,
    conns: Vec<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    check(&token)?;
    for c in &conns {
        if c.name.trim().is_empty() || c.ssh_cmd.trim().is_empty() {
            return Err(ServerFnError::new("name and ssh command required"));
        }
    }
    mistraldeck_engine::Engine::global().set_remote_connections(conns);
    Ok(())
}

/// Test an (unsaved) connection draft: reachability + agent presence.
#[server]
pub async fn api_test_remote(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
) -> Result<mistraldeck_core::snapshot::RemoteTestResult, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote::test_remote(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Install (or upgrade) the Mistral Vibe CLI on the remote host.
/// LOCAL-ONLY (Nexus WEB_BLOCKED remote install).
#[server]
pub async fn api_install_remote_cli(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
) -> Result<mistraldeck_core::snapshot::RemoteInstallResult, ServerFnError> {
    check(&token)?;
    require_local()?;
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote::install_remote_agent(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// CLI update check for a remote connection (vibe --version + PyPI latest).
#[server]
pub async fn api_check_remote_cli_update(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
) -> Result<mistraldeck_core::snapshot::CliUpdateInfo, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote::check_remote_cli_update(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Upgrade the CLI on a remote connection (uv preferred, pip3 fallback).
#[server]
pub async fn api_upgrade_remote_cli(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
) -> Result<mistraldeck_core::snapshot::CliUpgradeResult, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote::upgrade_remote_cli(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// App self-update check (GitHub releases/latest, flavor-aware).
#[server]
pub async fn api_check_app_update(
    token: String,
) -> Result<mistraldeck_core::snapshot::AppUpdateInfo, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(mistraldeck_engine::updates::check_app_update)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Local mistral-vibe CLI update check (PyPI latest with cache).
#[server]
pub async fn api_check_local_cli_update(
    token: String,
) -> Result<mistraldeck_core::snapshot::CliUpdateInfo, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(mistraldeck_engine::updates::check_local_cli_update)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Upgrade the local mistral-vibe CLI with uv (up to 5 minutes).
#[server]
pub async fn api_upgrade_local_cli(
    token: String,
) -> Result<mistraldeck_core::snapshot::CliUpgradeResult, ServerFnError> {
    check(&token)?;
    // v1.0.5: upgrading restarts the CLI — refused while a turn runs.
    if mistraldeck_engine::Engine::global().any_turn_in_flight() {
        return Ok(mistraldeck_core::snapshot::CliUpgradeResult {
            version: None,
            error: Some("A turn is running — update refused".into()),
        });
    }
    tokio::task::spawn_blocking(mistraldeck_engine::updates::upgrade_local_cli)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// App self-update: download + install on a detached engine thread
/// (fire-and-forget; poll `api_update_progress` for phases/errors - on
/// success the app relaunches itself).
#[server]
pub async fn api_update_app(
    token: String,
) -> Result<mistraldeck_core::snapshot::AppUpdateResult, ServerFnError> {
    check(&token)?;
    // v1.0.5: the relaunch kills live agent processes — refuse while
    // any session's turn is in flight.
    if mistraldeck_engine::Engine::global().any_turn_in_flight() {
        return Ok(mistraldeck_core::snapshot::AppUpdateResult {
            ok: false,
            error: Some("A turn is running — update refused".into()),
        });
    }
    tokio::task::spawn_blocking(mistraldeck_engine::updates::update_app)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Current app self-update progress + terminal error (phase
/// download/install/relaunch).
#[server]
pub async fn api_update_progress(
    token: String,
) -> Result<
    (
        Option<mistraldeck_core::snapshot::UpdateProgress>,
        Option<String>,
    ),
    ServerFnError,
> {
    check(&token)?;
    Ok(mistraldeck_engine::updates::update_progress())
}

/// Raw remote config.toml content ("" when missing), global or project scope.
#[server]
pub async fn api_get_remote_vibe_config(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
) -> Result<String, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::get_remote_vibe_config(&conn, &dir))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Validate + save raw remote config.toml; `None` on success.
#[server]
pub async fn api_save_remote_raw_config(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    content: String,
) -> Result<Option<String>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::save_remote_raw_config(&conn, &dir, &content))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// The remote `~/.vibe` dir.
#[server]
pub async fn api_get_remote_vibe_dir(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
) -> Result<String, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::get_remote_vibe_dir(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Global agents of a remote host.
#[server]
pub async fn api_get_remote_agents(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
) -> Result<Vec<mistraldeck_core::snapshot::RemoteAgentFile>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::get_remote_agents(&conn, &dir))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Project agents (`<cwd>/.vibe/agents`) on a remote host.
#[server]
pub async fn api_list_remote_project_agents(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    cwd: String,
) -> Result<Vec<mistraldeck_core::snapshot::RemoteAgentFile>, ServerFnError> {
    check(&token)?;
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::list_remote_project_agents(&conn, &cwd))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Expose one remote project agent globally on the remote host.
#[server]
pub async fn api_expose_remote_project_agent(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    cwd: String,
    name: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    let res = tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::expose_remote_project_agent(&conn, &cwd, &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    res.map_err(|e| ServerFnError::new(e))
}

/// Expose all remote project agents globally on the remote host.
#[server]
pub async fn api_expose_remote_project_agents(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    cwd: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    let res = tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::expose_remote_project_agents(&conn, &cwd))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    res.map_err(|e| ServerFnError::new(e))
}

/// Save an agent profile remotely.
#[server]
pub async fn api_save_remote_agent(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
    agent: mistraldeck_core::snapshot::RemoteAgentFile,
) -> Result<(), ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    let res = tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::save_remote_agent(&conn, &dir, &name, &agent))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    res.map_err(|e| ServerFnError::new(e))
}

/// Delete an agent profile remotely.
#[server]
pub async fn api_delete_remote_agent(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::delete_remote_agent(&conn, &dir, &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Global prompts of a remote host.
#[server]
pub async fn api_get_remote_prompts(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
) -> Result<Vec<mistraldeck_core::snapshot::RemotePromptFile>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::get_remote_prompts(&conn, &dir))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Save a prompt remotely.
#[server]
pub async fn api_save_remote_prompt(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
    content: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    let res = tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::save_remote_prompt(&conn, &dir, &name, &content))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    res.map_err(|e| ServerFnError::new(e))
}

/// Delete a prompt remotely.
#[server]
pub async fn api_delete_remote_prompt(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::delete_remote_prompt(&conn, &dir, &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Remote config file tree for the raw tab.
#[server]
pub async fn api_list_remote_config_files(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
) -> Result<Vec<mistraldeck_core::snapshot::RemoteConfigFileEntry>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::list_remote_config_files(&conn, &dir))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Remote skills of a scope (tree + SKILL.md contents).
#[server]
pub async fn api_get_remote_skills(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
) -> Result<Vec<mistraldeck_core::snapshot::RemoteSkill>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::get_remote_skills(&conn, &dir))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Create a remote skill; `None` on success.
#[server]
pub async fn api_create_remote_skill(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
) -> Result<Option<String>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::create_remote_skill(&conn, &dir, &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Delete a remote skill; `None` on success.
#[server]
pub async fn api_delete_remote_skill(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
) -> Result<Option<String>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::delete_remote_skill(&conn, &dir, &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Write a file inside a remote skill; `None` on success.
#[server]
pub async fn api_write_remote_skill_file(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
    rel_path: String,
    content: String,
) -> Result<Option<String>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::write_remote_skill_file(&conn, &dir, &name, &rel_path, &content))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Delete a file inside a remote skill; `None` on success.
#[server]
pub async fn api_delete_remote_skill_file(
    token: String,
    conn: mistraldeck_core::snapshot::RemoteConnection,
    dir: Option<String>,
    name: String,
    rel_path: String,
) -> Result<Option<String>, ServerFnError> {
    check(&token)?;
    let dir = mistraldeck_engine::remote_vibe::scope_dir(dir.as_deref());
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote_vibe::delete_remote_skill_file(&conn, &dir, &name, &rel_path))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}


/// CLI sessions discovered on a remote host (JSON-RPC over ssh).
#[server]
pub async fn api_remote_sessions(
    token: String,
    conn_id: String,
) -> Result<Vec<mistraldeck_core::snapshot::RemoteSessionInfo>, ServerFnError> {
    check(&token)?;
    let Some(conn) = mistraldeck_engine::Engine::global().remote_connection(&conn_id) else {
        return Ok(Vec::new());
    };
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote::list_remote_sessions(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Quick liveness check for the sidebar status dot.
#[server]
pub async fn api_check_remote_status(
    token: String,
    conn_id: String,
) -> Result<mistraldeck_core::snapshot::RemoteStatus, ServerFnError> {
    check(&token)?;
    let Some(conn) = mistraldeck_engine::Engine::global().remote_connection(&conn_id) else {
        return Ok(mistraldeck_core::snapshot::RemoteStatus::default());
    };
    tokio::task::spawn_blocking(move || mistraldeck_engine::remote::check_remote_status(&conn))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// New session on an SSH remote (sidebar "+" per location).
#[server]
pub async fn api_start_remote_session(
    token: String,
    conn_id: String,
    location: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    mistraldeck_engine::Engine::global().create_remote_session(&conn_id, &location);
    Ok(())
}

/// Open a CLI session discovered on an SSH remote (replays history).
#[server]
pub async fn api_open_remote_session(
    token: String,
    conn_id: String,
    session_id: String,
    title: String,
    cwd: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    // The lazy preview reads the remote unified store over SSH (blocking,
    // up to ~85 s) - keep it off the async runtime.
    tokio::task::spawn_blocking(move || {
        mistraldeck_engine::Engine::global().open_remote_session(&conn_id, &session_id, &title, &cwd);
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// A project agent (`<cwd>/.vibe/agents/<name>.toml`) surfaced in the
/// mode chip dropdown — Nexus `vibe:get-agents("project")` view.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentChoiceView {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

/// Project-level agents for the mode chip (top-level only, no subagents).
#[server]
pub async fn api_project_agents(
    token: String,
    cwd: String,
) -> Result<Vec<AgentChoiceView>, ServerFnError> {
    check(&token)?;
    let dir = std::path::Path::new(&cwd).join(".vibe").join("agents");
    let agents = mistraldeck_vibe::AgentFile::load_all_in(&dir);
    Ok(agents
        .into_iter()
        .filter(|a| a.agent_type.as_deref() != Some("subagent") && !a.name.is_empty())
        .map(|a| AgentChoiceView {
            name: a.name,
            display_name: a.display_name,
            description: a.description,
        })
        .collect())
}

/// Copy a project agent into `~/.vibe/agents/` so the CLI backend can
/// activate it as a mode (Nexus `vibe:expose-project-agent`).
#[server]
pub async fn api_expose_project_agent(
    token: String,
    cwd: String,
    name: String,
) -> Result<(), ServerFnError> {
    check(&token)?;
    // Sanitize: the name selects a file directly under the agents dir.
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name == "."
    {
        return Err(ServerFnError::new("invalid agent name"));
    }
    let src = std::path::Path::new(&cwd)
        .join(".vibe")
        .join("agents")
        .join(format!("{name}.toml"));
    if !src.is_file() {
        return Err(ServerFnError::new("project agent file not found"));
    }
    let dest_dir = mistraldeck_vibe::vibe_home().join("agents");
    std::fs::create_dir_all(&dest_dir).map_err(|e| ServerFnError::new(e.to_string()))?;
    let dest = dest_dir.join(format!("{name}.toml"));
    std::fs::copy(&src, &dest).map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(())
}
