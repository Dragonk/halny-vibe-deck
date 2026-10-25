//! SSH remote connections (Nexus parity): run commands over the user's
//! `ssh` command, test/install vibe-acp on remote hosts and list remote
//! CLI sessions by speaking the same JSON-RPC protocol vibe-acp uses
//! locally - only over the ssh pipe.
//!
//! All functions are blocking (they spawn subprocesses); call them from
//! `tokio::task::spawn_blocking` in server contexts.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use vibedeck_core::snapshot::{RemoteConnection, RemoteInstallResult, RemoteSessionInfo, RemoteStatus, RemoteTestResult};

/// Split an ssh command string into argv tokens (respects quotes) -
/// port of Nexus `tokenizeCmd`.
pub fn tokenize_cmd(cmd: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in cmd.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else {
                cur.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

/// Quote a string for the remote POSIX shell (single quotes, escape).
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Result of one `ssh <conn> <remote_cmd>` run.
#[derive(Clone, Debug)]
pub struct SshRunResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl SshRunResult {
    fn err(stderr: String) -> Self {
        Self { code: -1, stdout: String::new(), stderr }
    }
}

/// Run `remote_cmd` on the connection's host via its ssh command.
/// The remote command is passed as a single argument (like Nexus
/// `sshRun`); stdout/stderr are drained on reader threads so a chatty
/// remote never deadlocks the pipe, and the child is killed on timeout.
pub fn ssh_run(conn: &RemoteConnection, remote_cmd: &str, timeout_ms: u64, stdin_data: Option<&str>) -> SshRunResult {
    let tokens = tokenize_cmd(&conn.ssh_cmd);
    if tokens.is_empty() {
        return SshRunResult::err("Empty ssh command".into());
    }
    let mut cmd = Command::new(&tokens[0]);
    cmd.args(&tokens[1..]).arg(remote_cmd);
    cmd.stdin(if stdin_data.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Never flash a console window on Windows.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return SshRunResult::err(e.to_string()),
    };
    if let Some(data) = stdin_data {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(data.as_bytes());
            let _ = si.flush();
        }
    }
    let out_handle = child.stdout.take();
    let err_handle = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut h) = out_handle {
            let _ = h.read_to_string(&mut s);
        }
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut h) = err_handle {
            let _ = h.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let stdout = t_out.join().unwrap_or_default();
                    let stderr = t_err.join().unwrap_or_default();
                    let mut res = SshRunResult::err(format!("timeout after {timeout_ms}ms"));
                    res.stdout = stdout;
                    res.stderr = if stderr.is_empty() { res.stderr } else { stderr };
                    return res;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return SshRunResult::err(e.to_string()),
        }
    };
    SshRunResult {
        code,
        stdout: t_out.join().unwrap_or_default(),
        stderr: t_err.join().unwrap_or_default(),
    }
}

/// Test an (unsaved) connection draft: reachability + vibe-acp presence.
pub fn test_remote(conn: &RemoteConnection) -> RemoteTestResult {
    let ping = ssh_run(conn, "echo __VIBEDECK_OK__", 12_000, None);
    if ping.code != 0 || !ping.stdout.contains("__VIBEDECK_OK__") {
        let err = {
            let e = ping.stderr.trim();
            if !e.is_empty() {
                e
            } else {
                ping.stdout.trim()
            }
            .chars()
            .take(300)
            .collect::<String>()
        };
        return RemoteTestResult {
            reachable: false,
            acp_installed: false,
            acp_path: None,
            version: None,
            error: Some(if err.is_empty() { "SSH connection failed".into() } else { err }),
        };
    }
    // Login shell: non-interactive ssh sessions do not include ~/.local/bin
    // in PATH (uv tool installs live there).
    let which = ssh_run(conn, "bash -lc \"command -v vibe-acp\"", 10_000, None);
    let acp_path = if which.code == 0 { which.stdout.trim().to_string() } else { String::new() };
    if acp_path.is_empty() {
        return RemoteTestResult {
            reachable: true,
            acp_installed: false,
            acp_path: None,
            version: None,
            error: None,
        };
    }
    let ver = ssh_run(conn, "bash -lc \"vibe-acp --version\" 2>/dev/null", 10_000, None);
    RemoteTestResult {
        reachable: true,
        acp_installed: true,
        acp_path: Some(acp_path),
        version: Some(ver.stdout.trim().to_string()).filter(|v| !v.is_empty()),
        error: None,
    }
}

const DEFAULT_INSTALL_CMD: &str = "sh -c 'if command -v uv >/dev/null 2>&1; then uv tool install mistral-vibe 2>/dev/null || uv tool upgrade mistral-vibe; else curl -LsSf https://astral.sh/uv/install.sh | sh && $HOME/.local/bin/uv tool install mistral-vibe; fi'";

/// Install (or upgrade) vibe-acp on the remote host.
pub fn install_remote_acp(conn: &RemoteConnection) -> RemoteInstallResult {
    let cmd = conn.install_command.trim();
    let cmd = if cmd.is_empty() { DEFAULT_INSTALL_CMD } else { cmd };
    let res = ssh_run(conn, cmd, 300_000, None);
    let ok = res.code == 0;
    let log = format!("{}{}", res.stdout, res.stderr);
    RemoteInstallResult {
        ok,
        error: if ok {
            None
        } else {
            let e = res.stderr.trim();
            Some(if e.is_empty() { "install failed".into() } else { e.chars().take(400).collect() })
        },
        log: log.chars().take(2000).collect(),
    }
}

/// Quick liveness check for the sidebar dot.
pub fn check_remote_status(conn: &RemoteConnection) -> RemoteStatus {
    let t0 = Instant::now();
    let res = ssh_run(conn, "true", 8_000, None);
    RemoteStatus {
        id: conn.id.clone(),
        ok: res.code == 0,
        checked_at_ms: vibedeck_core::types::now_ms(),
        latency_ms: if res.code == 0 { Some(t0.elapsed().as_millis() as u64) } else { None },
    }
}

/// The agent argv for a remote slot: tokenized ssh command plus the
/// remote command that starts vibe-acp in a login shell (so uv-installed
/// binaries are on PATH). The ACP client then speaks JSON-RPC over the
/// ssh pipe exactly like a local process.
pub fn remote_argv(conn: &RemoteConnection) -> Vec<String> {
    let mut argv = tokenize_cmd(&conn.ssh_cmd);
    argv.push("bash -lc \"exec vibe-acp\"".to_string());
    argv
}

/// List CLI sessions on the remote host by speaking JSON-RPC to a
/// vibe-acp spawned over ssh (initialize -> session/list with cursor
/// pagination, port of Nexus `listRemoteSessions`).
pub fn list_remote_sessions(conn: &RemoteConnection) -> Vec<RemoteSessionInfo> {
    let tokens = tokenize_cmd(&conn.ssh_cmd);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut cmd = Command::new(&tokens[0]);
    cmd.args(&tokens[1..]).arg("bash -lc \"exec vibe-acp\"");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    // stdout reader forwards JSON lines over a channel; the main loop
    // drives the handshake with a hard deadline.
    let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
    let out_handle = child.stdout.take();
    std::thread::spawn(move || {
        if let Some(mut h) = out_handle {
            let mut buf = Vec::new();
            loop {
                let mut byte = [0u8; 1];
                match h.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if byte[0] == b'\n' {
                            let line = String::from_utf8_lossy(&buf).trim().to_string();
                            buf.clear();
                            if !line.is_empty() && line_tx.send(line).is_err() {
                                break;
                            }
                        } else {
                            buf.push(byte[0]);
                            if buf.len() > 1_000_000 {
                                buf.clear(); // runaway line: drop it
                            }
                        }
                    }
                }
            }
        }
    });
    let mut stdin = child.stdin.take();

    fn write_line(line: &str, stdin: &mut Option<std::process::ChildStdin>) -> bool {
        if let Some(si) = stdin.as_mut() {
            si.write_all(line.as_bytes()).and_then(|_| si.write_all(b"\n")).is_ok()
        } else {
            false
        }
    }

    fn request_list(stdin: &mut Option<std::process::ChildStdin>, pages: u32, cursor: &str) -> String {
        let list_id = format!("list-{pages}");
        let mut params = serde_json::Map::new();
        if !cursor.is_empty() {
            params.insert("cursor".into(), serde_json::json!(cursor));
        }
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": list_id,
            "method": "session/list",
            "params": params,
        });
        let _ = write_line(&req.to_string(), stdin);
        list_id
    }

    let deadline = Instant::now() + Duration::from_millis(25_000);
    let mut sessions: Vec<RemoteSessionInfo> = Vec::new();
    let mut list_id = String::new();
    let mut cursor = String::new();
    let mut pages = 0u32;

    // Kick off the handshake.
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "init",
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientInfo": { "name": "VibeDeck", "version": "0.8.3" }
        },
    });
    if !write_line(&init_req.to_string(), &mut stdin) {
        let _ = child.kill();
        let _ = child.wait();
        return sessions;
    }

    loop {
        if Instant::now() >= deadline {
            break;
        }
        match line_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
                let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                let result = msg.get("result").cloned().unwrap_or(serde_json::Value::Null);
                if id == "init" {
                    list_id = request_list(&mut stdin, pages, &cursor);
                } else if !list_id.is_empty() && id == list_id {
                    let raw = result
                        .get("sessions")
                        .or_else(|| result.get("items"))
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for s in raw {
                        let session_id = s
                            .get("sessionId")
                            .or_else(|| s.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if session_id.is_empty() {
                            continue;
                        }
                        sessions.push(RemoteSessionInfo {
                            session_id,
                            title: s
                                .get("title")
                                .or_else(|| s.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            cwd: s
                                .get("cwd")
                                .or_else(|| s.get("workingDirectory"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            updated_at: s
                                .get("updatedAt")
                                .or_else(|| s.get("updated_at"))
                                .or_else(|| s.get("startTime"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        });
                    }
                    let next_cursor = result
                        .get("next_cursor")
                        .or_else(|| result.get("nextCursor"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if !next_cursor.is_empty() && pages < 20 {
                        pages += 1;
                        cursor = next_cursor.to_string();
                        list_id = request_list(&mut stdin, pages, &cursor);
                    } else {
                        break;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // If the handshake stalled before session/list was sent
                // (e.g. initialize answered slowly), nudge it once.
                if list_id.is_empty() {
                    list_id = request_list(&mut stdin, pages, &cursor);
                }
                if child.try_wait().map(|s| s.is_some()).unwrap_or(true) {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        assert_eq!(tokenize_cmd("ssh user@host -p 2222"), vec!["ssh", "user@host", "-p", "2222"]);
    }

    #[test]
    fn tokenize_quotes() {
        assert_eq!(
            tokenize_cmd("ssh host \"echo 'a b'\" x"),
            vec!["ssh", "host", "echo 'a b'", "x"]
        );
    }

    #[test]
    fn shell_quote() {
        assert_eq!(shq("it's"), "'it'\\''s'");
    }
}
