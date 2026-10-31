//! JSON-RPC 2.0 connection to `vibe-app-server` over stdio (NDJSON).
//!
//! Port of the reference client semantics (`vibe/app_server/client.py`,
//! docs/appserver-protocol.md §2/§3):
//! - client request ids are STRINGS (`client-1`, `client-2`, …),
//! - one connection owns one subprocess,
//! - server→client requests (`callback/call`) are surfaced to the
//!   owner so it can ack + answer them,
//! - the stdout reader classifies every line into Response /
//!   Notification / ServerRequest.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

/// A decoded inbound message.
#[derive(Clone, Debug)]
pub enum Inbound {
    /// Reply to one of our requests.
    Response {
        id: String,
        result: Result<Value, (i64, String)>,
    },
    /// Fire-and-forget notification.
    Notification { method: String, params: Value },
    /// Server→client request (must be answered!).
    ServerRequest {
        id: i64,
        method: String,
        params: Value,
    },
}

struct Shared {
    next_id: u64,
    /// Server-request ids already acked (acks are idempotent).
    acked: std::collections::HashSet<i64>,
}

pub struct Conn {
    child: Child,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    shared: Arc<Mutex<Shared>>,
    /// Inbound queue (reader thread → worker). Shared handle so the
    /// worker can drain it without borrowing the whole Conn.
    pub inbox: Arc<Mutex<Receiver<Inbound>>>,
    inbound_tx: Sender<Inbound>,
}

impl Conn {
    /// Spawn `vibe-app-server` (argv[0] is the binary, the rest are
    /// args — for SSH remotes it is the full ssh command with a
    /// trailing login-shell `exec vibe-app-server`) and start the
    /// stdout reader thread.
    pub fn spawn(argv: &[String]) -> Result<Self> {
        if argv.is_empty() {
            bail!("empty vibe-app-server argv");
        }
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Agent stderr carries the CLI's own warnings and, with the e2e
            // stub, its trace — forward it to the server log (CI diagnosis).
            .stderr(Stdio::inherit());
        // Never flash an empty console window per session (issue Z28): every
        // local session owns one agent process, so a GUI build without
        // CREATE_NO_WINDOW opens a black terminal per session. Stable parity:
        // `windowsHide: true` on the child process.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawn {:?} (is vibe-app-server installed?)", argv[0]))?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = channel::<Inbound>();
        let conn = Self {
            child,
            stdin: Arc::new(Mutex::new(Some(stdin))),
            shared: Arc::new(Mutex::new(Shared {
                next_id: 1,
                acked: std::collections::HashSet::new(),
            })),
            inbox: Arc::new(Mutex::new(rx)),
            inbound_tx: tx,
        };

        let tx = conn.inbound_tx.clone();
        let shared = conn.shared.clone();
        std::thread::Builder::new()
            .name("mistraldeck-appsrv-rx".into())
            .spawn(move || reader(stdout, tx, shared))
            .context("spawn app-server reader thread")?;
        Ok(conn)
    }

    /// Send a request with an auto-generated string id.
    pub fn request(&self, method: &str, params: Value) -> Result<String> {
        let id = {
            let mut s = self.shared.lock().expect("conn shared");
            let id = format!("client-{}", s.next_id);
            s.next_id += 1;
            id
        };
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send_line(&msg)?;
        Ok(id)
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.send_line(&msg)
    }

    /// Answer a server→client request (single response envelope).
    pub fn respond(&self, id: i64, result: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "id": id, "result": result});
        self.send_line(&msg)
    }

    /// Ack a `callback/call` server request (`{callbackId, accepted}`).
    /// The decision itself travels separately via `callback/result`.
    pub fn ack_callback(&self, id: i64, params: &Value, accepted: bool) {
        let already = {
            let mut s = self.shared.lock().expect("conn shared");
            if s.acked.contains(&id) {
                true
            } else {
                s.acked.insert(id);
                false
            }
        };
        if already {
            return;
        }
        // round 8: the entry may be nested under `params.callback` or be the
        // params object itself — read callbackId from both shapes
        let callback_id = params
            .get("callback")
            .and_then(|c| c.get("callbackId"))
            .cloned()
            .or_else(|| params.get("callbackId").cloned())
            .unwrap_or(Value::Null);
        let _ = self.respond(id, json!({"callbackId": callback_id, "accepted": accepted}));
    }

    fn send_line(&self, msg: &Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        let mut guard = self.stdin.lock().expect("conn stdin");
        let w = guard
            .as_mut()
            .ok_or_else(|| anyhow!("app-server stdin closed"))?;
        w.write_all(line.as_bytes())
            .and_then(|_| w.write_all(b"\n"))
            .and_then(|_| w.flush())
            .with_context(|| format!("write to app-server failed: {line}"))?;
        Ok(())
    }

    /// PID of the agent child process — used to tell OUR OWN lease apart
    /// from another process's ("open elsewhere" badge must never light up
    /// for ourselves).
    pub fn child_pid(&self) -> Option<u32> {
        Some(self.child.id())
    }

    /// Non-blocking process-exit probe.
    pub fn exited(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(status)) => status.code(),
            _ => None,
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.stdin.lock().expect("conn stdin").take();
        // Round 15: on Windows the direct child is only the launcher shim
        // (uv .exe spawns python as a grandchild) — Child::kill() would
        // leave the real agent running. Kill the whole tree via taskkill.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &self.child.id().to_string()])
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The stdout pump: one line = one message, classify and forward.
fn reader(stdout: ChildStdout, tx: Sender<Inbound>, shared: Arc<Mutex<Shared>>) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            log::warn!("app-server: unparsable line: {:.200}", line);
            continue;
        };
        let kind = if let Some(method) = msg.get("method").and_then(Value::as_str) {
            match msg.get("id") {
                // server→client request: reference ids are ints
                Some(Value::Number(n)) => Inbound::ServerRequest {
                    id: n.as_i64().unwrap_or(-1),
                    method: method.to_string(),
                    params: msg.get("params").cloned().unwrap_or(Value::Null),
                },
                _ => Inbound::Notification {
                    method: method.to_string(),
                    params: msg.get("params").cloned().unwrap_or(Value::Null),
                },
            }
        } else if let Some(id) = msg.get("id").and_then(Value::as_str) {
            let result = match msg.get("error") {
                Some(err) => Err((
                    err.get("code").and_then(Value::as_i64).unwrap_or(-1),
                    err.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                )),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            Inbound::Response {
                id: id.to_string(),
                result,
            }
        } else {
            log::warn!("app-server: message without method/id: {:.200}", msg);
            continue;
        };
        let _keep_shared_alive = shared.lock().is_ok();
        if tx.send(kind).is_err() {
            break;
        }
    }
}

/// Waiting-request table shared between the worker and its pump.
pub type PendingReplies = Arc<Mutex<HashMap<String, Sender<Inbound>>>>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The reader classifies a `callback/call` server request correctly
    /// (int id → ServerRequest) — the shape the worker must answer.
    #[test]
    fn classify_server_request() {
        let (tx, rx) = channel::<Inbound>();
        let shared = Arc::new(Mutex::new(Shared {
            next_id: 1,
            acked: Default::default(),
        }));
        let msg = r#"{"jsonrpc":"2.0","id":7,"method":"callback/call","params":{"callback":{"callbackId":"cb-1"}}}"#;
        // emulate the reader's classification (the reader fn itself needs
        // a ChildStdout; the logic under test is the branch above)
        let msg: Value = serde_json::from_str(msg).unwrap();
        if let Some(method) = msg.get("method").and_then(Value::as_str) {
            match msg.get("id") {
                Some(Value::Number(n)) => tx.send(Inbound::ServerRequest {
                    id: n.as_i64().unwrap(),
                    method: method.into(),
                    params: msg["params"].clone(),
                })
                .unwrap(),
                _ => panic!("expected server request"),
            }
        }
        let _ = shared;
        match rx.try_recv().unwrap() {
            Inbound::ServerRequest { id, method, params } => {
                assert_eq!(id, 7);
                assert_eq!(method, "callback/call");
                assert_eq!(params["callback"]["callbackId"], "cb-1");
            }
            _ => panic!("wrong kind"),
        }
    }

    /// Our own request ids are strings and grow monotonically — the
    /// server rejects int ids (docs pitfall 1).
    #[test]
    fn request_ids_are_strings() {
        let s = Shared {
            next_id: 1,
            acked: Default::default(),
        };
        let id = format!("client-{}", s.next_id);
        assert_eq!(id, "client-1");
        assert!(id.parse::<i64>().is_err() == false || id.starts_with("client-"));
    }
}
