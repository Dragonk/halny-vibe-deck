//! The ACP worker: owns the `vibe-acp` subprocess, executes commands
//! from the UI, and streams events back. Mirrors the architecture of
//! the old Electron main process (`acp.ts`) with none of the Electron.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use vibedeck_core::proto::{AcpCommand, AcpEvent, AcpSessionInfo, ModeInfo};

pub const CLIENT_INFO_NAME: &str = "VibeDeck";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Handle held by the UI. Clone the sender at will; keep the receiver unique.
pub struct AcpHandle {
    cmd_tx: Sender<AcpCommand>,
    event_rx: Receiver<AcpEvent>,
}

impl AcpHandle {
    /// Spawn the worker thread. Cheap: no subprocess is started until
    /// [`AcpCommand::Start`] arrives.
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<AcpCommand>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AcpEvent>();
        std::thread::Builder::new()
            .name("vibedeck-acp".into())
            .spawn(move || Worker::new(event_tx, cmd_rx).run())
            .expect("failed to spawn ACP worker thread");
        Self { cmd_tx, event_rx }
    }

    pub fn send(&self, cmd: AcpCommand) {
        // The worker always drains commands; ignore send errors only if
        // the worker is gone (app shutting down).
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn events(&self) -> &Receiver<AcpEvent> {
        &self.event_rx
    }
}

enum RawMsg {
    /// `session/update` notification params.
    Update(Value),
    /// A request from the agent to the client that needs user interaction.
    AgentRequest {
        id: Value,
        method: String,
        params: Value,
    },
    /// stderr line.
    Stderr(String),
    /// stdout closed — the subprocess is gone.
    StdoutClosed,
    /// Unparseable stdout line.
    BadLine(String),
}

type PendingMap = Arc<Mutex<HashMap<String, Sender<Result<Value>>>>>;

/// Commands that arrive while a prompt is streaming. The worker stashes
/// everything except `Cancel` (which must go out immediately) and handles
/// the stash when the turn finishes.
#[allow(dead_code)]
pub(crate) type CmdStash = Arc<Mutex<Vec<AcpCommand>>>;

struct Client {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingMap,
    raw_rx: Receiver<RawMsg>,
    event_tx: Sender<AcpEvent>,
    /// Sessions being loaded from disk — replay notifications suppressed.
    suppressed: HashSet<String>,
    stdout_closed: bool,
    next_id: u64,
}

impl Client {
    fn spawn_bin(
        acp_path: &str,
        raw_tx: Sender<RawMsg>,
        raw_rx: Receiver<RawMsg>,
        event_tx: Sender<AcpEvent>,
    ) -> Result<Self> {
        let mut cmd = Command::new(acp_path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Never flash a console window for the child process on Windows.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn `{acp_path}`"))?;

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout: ChildStdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = pending.clone();
        let stdin_shared = Arc::new(Mutex::new(stdin));
        let stdin_writer = stdin_shared.clone();
        let raw_tx_stderr = raw_tx.clone();

        // stdout reader: routes responses to pending requests, forwards
        // notifications and interactive agent requests to the worker.
        std::thread::Builder::new()
            .name("vibedeck-acp-stdout".into())
            .spawn(move || {
                let raw_tx = raw_tx;
                let stdin_shared = stdin_writer;
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(line) else {
                        let _ = raw_tx.send(RawMsg::BadLine(line.to_string()));
                        continue;
                    };
                    let id = msg.get("id").cloned();
                    let method = msg.get("method").and_then(Value::as_str);
                    let has_result = msg.get("result").is_some() || msg.get("error").is_some();

                    if let (Some(id), Some(method), false) = (id.clone(), method, has_result) {
                        // A request from the agent to us.
                        let params = msg.get("params").cloned().unwrap_or(Value::Null);
                        if needs_user_interaction(method) {
                            let _ = raw_tx.send(RawMsg::AgentRequest {
                                id,
                                method: method.to_string(),
                                params,
                            });
                        } else {
                            // Protocol-level requests (fs/*, terminal/*, …): auto-approve.
                            let resp =
                                json!({ "jsonrpc": "2.0", "id": id, "result": {"approved": true} });
                            if let Ok(mut w) = stdin_shared.lock() {
                                let _ = w.write_all(format!("{resp}\n").as_bytes());
                                let _ = w.flush();
                            }
                        }
                        continue;
                    }
                    if let Some(id) = id {
                        // Response to one of our requests.
                        let key = id.to_string();
                        let sender = pending_reader.lock().ok().and_then(|mut m| m.remove(&key));
                        if let Some(sender) = sender {
                            if let Some(err) = msg.get("error") {
                                let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
                                let message = err
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("unknown error")
                                    .to_string();
                                let _ = sender.send(Err(anyhow!("[{code}] {message}")));
                            } else {
                                let result = msg.get("result").cloned().unwrap_or(Value::Null);
                                let _ = sender.send(Ok(result));
                            }
                        }
                        continue;
                    }
                    if let Some(method) = method {
                        if method == "session/update" {
                            let params = msg.get("params").cloned().unwrap_or(Value::Null);
                            let _ = raw_tx.send(RawMsg::Update(params));
                        }
                        continue;
                    }
                    let _ = raw_tx.send(RawMsg::BadLine(line.to_string()));
                }
                let _ = raw_tx.send(RawMsg::StdoutClosed);
            })
            .expect("failed to spawn stdout reader");

        // stderr reader: forward to the debug log.
        std::thread::Builder::new()
            .name("vibedeck-acp-stderr".into())
            .spawn(move || {
                let raw_tx = raw_tx_stderr;
                let mut reader = BufReader::new(stderr);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let _ = raw_tx.send(RawMsg::Stderr(line.trim().to_string()));
                }
            })
            .expect("failed to spawn stderr reader");

        Ok(Self {
            child,
            stdin: stdin_shared,
            pending,
            raw_rx,
            event_tx,
            suppressed: HashSet::new(),
            stdout_closed: false,
            next_id: 0,
        })
    }

    fn write_msg(stdin: &Arc<Mutex<ChildStdin>>, msg: &Value) -> Result<()> {
        let mut stdin = stdin.lock().map_err(|_| anyhow!("stdin mutex poisoned"))?;
        stdin.write_all(format!("{msg}\n").as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    fn exited(&mut self) -> bool {
        self.stdout_closed || matches!(self.child.try_wait(), Ok(Some(_)))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Send a JSON-RPC request and wait for the response while pumping
    /// notifications (session/update, agent requests) through `pump`.
    fn request(&mut self, method: &str, params: &Value, timeout: Duration) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = std::sync::mpsc::channel::<Result<Value>>();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id.to_string(), tx);
        }
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = Self::write_msg(&self.stdin, &msg) {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id.to_string());
            }
            return Err(e.context("failed to write to vibe-acp stdin"));
        }

        let deadline = Instant::now() + timeout;
        loop {
            self.drain_raw();
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id.to_string());
                }
                bail!("ACP request timeout: {method}");
            }
            match rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
                Ok(Ok(v)) => return Ok(v),
                Ok(Err(e)) => return Err(e),
                Err(RecvTimeoutError::Timeout) => {
                    if self.exited() {
                        if let Ok(mut pending) = self.pending.lock() {
                            pending.remove(&id.to_string());
                        }
                        bail!("vibe-acp exited while handling {method}");
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if let Ok(mut pending) = self.pending.lock() {
                        pending.remove(&id.to_string());
                    }
                    bail!("ACP response channel closed ({method})");
                }
            }
        }
    }

    /// Consume pending raw messages and emit converted events.
    fn drain_raw(&mut self) {
        let mut events: Vec<AcpEvent> = Vec::new();
        while let Ok(raw) = self.raw_rx.try_recv() {
            match raw {
                RawMsg::Update(params) => {
                    let session_id = params
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if self.suppressed.contains(&session_id) {
                        continue; // session/load replay — skip duplicates
                    }
                    events.extend(crate::convert::convert_update(&params));
                }
                RawMsg::AgentRequest { id, method, params } => {
                    events.extend(crate::convert::convert_agent_request(id, &method, &params));
                }
                RawMsg::Stderr(line) => events.push(AcpEvent::DebugLog(format!("stderr: {line}"))),
                RawMsg::StdoutClosed => self.stdout_closed = true,
                RawMsg::BadLine(line) => {
                    events.push(AcpEvent::DebugLog(format!(
                        "parse error: {}",
                        truncate(&line, 300)
                    )));
                }
            }
        }
        for ev in events {
            let _ = self.event_tx.send(ev);
        }
    }
}

fn needs_user_interaction(method: &str) -> bool {
    method == "ask_user_question"
        || method.contains("ask_user")
        || method == "user/question"
        || method.contains("approv")
        || method.contains("permission")
        || method == "tool/call"
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

struct Worker {
    event_tx: Sender<AcpEvent>,
    cmd_rx: Receiver<AcpCommand>,
    client: Option<Client>,
    resume_not_supported: bool,
}

impl Worker {
    fn new(event_tx: Sender<AcpEvent>, cmd_rx: Receiver<AcpCommand>) -> Self {
        Self {
            event_tx,
            cmd_rx,
            client: None,
            resume_not_supported: false,
        }
    }

    fn emit(&self, ev: AcpEvent) {
        let _ = self.event_tx.send(ev);
    }

    fn log(&self, msg: impl Into<String>) {
        let _ = self.event_tx.send(AcpEvent::DebugLog(msg.into()));
    }

    fn run(mut self) {
        loop {
            match self.cmd_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(client) = self.client.as_mut() {
                        client.drain_raw();
                        if client.exited() {
                            self.client = None;
                            let _ = self.event_tx.send(AcpEvent::Disconnected { code: None });
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        if let Some(mut client) = self.client.take() {
            client.kill();
        }
    }

    fn handle(&mut self, cmd: AcpCommand) {
        match cmd {
            AcpCommand::Start { acp_path } => self.cmd_start(&acp_path),
            AcpCommand::Stop => {
                if let Some(mut c) = self.client.take() {
                    c.kill();
                }
                self.resume_not_supported = false;
                self.emit(AcpEvent::Disconnected { code: None });
            }
            AcpCommand::ListSessions => self.cmd_list_sessions(),
            AcpCommand::NewSession { cwd, model } => self.cmd_new_session(&cwd, model),
            AcpCommand::LoadSession {
                session_id,
                cwd,
                model,
            } => self.cmd_load_session(&session_id, &cwd, model),
            AcpCommand::ResumeSession {
                session_id,
                cwd,
                model,
            } => self.cmd_resume_session(&session_id, &cwd, model),
            AcpCommand::Prompt { session_id, text } => self.cmd_prompt(&session_id, &text),
            AcpCommand::Cancel { session_id } => {
                let note = json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": session_id }
                });
                if let Some(client) = self.client.as_ref() {
                    let _ = Client::write_msg(&client.stdin, &note);
                }
            }
            AcpCommand::SetConfigOption {
                session_id,
                key,
                value,
            } => {
                let Some(client) = self.client.as_mut() else {
                    return;
                };
                let params = json!({ "sessionId": session_id, "configId": key, "value": value });
                if let Err(e) = client.request(
                    "session/set_config_option",
                    &params,
                    Duration::from_secs(30),
                ) {
                    self.emit(AcpEvent::SessionError {
                        context: "set_config_option".into(),
                        message: e.to_string(),
                    });
                }
            }
            AcpCommand::Approve {
                request_id,
                option_id,
            } => {
                self.respond_to_agent(
                    &request_id,
                    json!({
                        "outcome": { "outcome": "selected", "option_id": option_id }
                    }),
                );
            }
            AcpCommand::AnswerQuestion { request_id, answer } => {
                self.respond_to_agent(&request_id, json!({ "answer": answer }));
            }
        }
    }

    /// Write a JSON-RPC response to an agent-initiated request.
    /// `request_id` is the serialized JSON id previously handed to the UI.
    fn respond_to_agent(&mut self, request_id: &str, result: Value) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let Ok(id) = serde_json::from_str::<Value>(request_id) else {
            return;
        };
        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = Client::write_msg(&client.stdin, &resp);
    }

    fn cmd_start(&mut self, acp_path: &str) {
        match self.start_client(acp_path) {
            Ok(()) => {}
            Err(e) => {
                self.client = None;
                self.emit(AcpEvent::SessionError {
                    context: "start".into(),
                    message: e.to_string(),
                });
            }
        }
    }

    fn start_client(&mut self, acp_path: &str) -> Result<()> {
        // Stop any previous instance first.
        if let Some(mut c) = self.client.take() {
            c.kill();
        }
        self.resume_not_supported = false;

        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<RawMsg>();
        let client = Client::spawn_bin(acp_path, raw_tx, raw_rx, self.event_tx.clone())?;
        self.client = Some(client);

        self.initialize()
    }

    fn initialize(&mut self) -> Result<()> {
        let client = self.client.as_mut().unwrap();
        // NOTE: vibe-acp rejects unexpected fields with -32603, so send
        // exactly protocolVersion + clientInfo.
        let params = json!({
            "protocolVersion": 1,
            "clientInfo": { "name": CLIENT_INFO_NAME, "version": env!("CARGO_PKG_VERSION") }
        });
        let result = client
            .request("initialize", &params, Duration::from_secs(30))
            .context("initialize handshake failed")?;
        let protocol_version = result
            .get("protocolVersion")
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32;
        self.emit(AcpEvent::Initialized { protocol_version });
        self.cmd_list_sessions();
        Ok(())
    }

    fn cmd_list_sessions(&mut self) {
        let Some(client) = self.client.as_mut() else {
            self.emit(AcpEvent::SessionsList(Vec::new()));
            return;
        };
        let sessions = match client.request("session/list", &json!({}), Duration::from_secs(30)) {
            Ok(v) => parse_session_list(&v),
            Err(_) => Vec::new(), // session/list unsupported → empty, silently
        };
        self.emit(AcpEvent::SessionsList(sessions));
    }

    fn cmd_new_session(&mut self, cwd: &str, model: Option<String>) {
        let Some(client) = self.client.as_mut() else {
            self.emit(AcpEvent::SessionError {
                context: "new_session".into(),
                message: "ACP not running".into(),
            });
            return;
        };
        let params = json!({ "cwd": cwd, "mcpServers": [] });
        match client.request("session/new", &params, REQUEST_TIMEOUT) {
            Ok(result) => {
                let (session_id, modes, models) = parse_new_session(&result);
                if session_id.is_empty() {
                    self.emit(AcpEvent::SessionError {
                        context: "new_session".into(),
                        message: "server returned no sessionId".into(),
                    });
                    return;
                }
                // Best effort: fetch a human-readable session name.
                let title = client
                    .request(
                        "session/name",
                        &json!({ "sessionId": session_id }),
                        Duration::from_secs(15),
                    )
                    .ok()
                    .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string));
                self.finish_session_ready(session_id, modes, models, title, model);
            }
            Err(e) => self.emit(AcpEvent::SessionError {
                context: "new_session".into(),
                message: e.to_string(),
            }),
        }
    }

    fn cmd_load_session(&mut self, session_id: &str, cwd: &str, model: Option<String>) {
        let Some(client) = self.client.as_mut() else {
            self.emit(AcpEvent::SessionError {
                context: "load_session".into(),
                message: "ACP not running".into(),
            });
            return;
        };
        client.suppressed.insert(session_id.to_string());
        let params = json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": [] });
        let outcome = client.request("session/load", &params, REQUEST_TIMEOUT);
        if let Some(client) = self.client.as_mut() {
            client.suppressed.remove(session_id);
        }
        match outcome {
            Ok(result) => {
                let (sid, modes, models) = parse_new_session(&result);
                self.finish_session_ready(
                    if sid.is_empty() {
                        session_id.to_string()
                    } else {
                        sid
                    },
                    modes,
                    models,
                    None,
                    model,
                );
            }
            Err(e) => self.emit(AcpEvent::SessionError {
                context: "load_session".into(),
                message: e.to_string(),
            }),
        }
    }

    fn cmd_resume_session(&mut self, session_id: &str, cwd: &str, model: Option<String>) {
        if self.resume_not_supported {
            self.cmd_new_session(cwd, model);
            return;
        }
        let Some(client) = self.client.as_mut() else {
            self.emit(AcpEvent::SessionError {
                context: "resume_session".into(),
                message: "ACP not running".into(),
            });
            return;
        };
        let params = json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": [] });
        match client.request("session/resume", &params, REQUEST_TIMEOUT) {
            Ok(result) => {
                let (sid, modes, models) = parse_new_session(&result);
                self.finish_session_ready(
                    if sid.is_empty() {
                        session_id.to_string()
                    } else {
                        sid
                    },
                    modes,
                    models,
                    None,
                    model,
                );
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("Method not implemented") || msg.contains("Method not found") {
                    self.resume_not_supported = true;
                }
                self.log(format!(
                    "session/resume failed ({msg}); starting a new session"
                ));
                self.cmd_new_session(cwd, model);
            }
        }
    }

    fn finish_session_ready(
        &mut self,
        session_id: String,
        modes: Vec<ModeInfo>,
        models: Vec<String>,
        title: Option<String>,
        model: Option<String>,
    ) {
        // Apply the requested model if the server advertises models.
        if let Some(model) = model.clone() {
            if !models.is_empty() {
                if let Some(client) = self.client.as_mut() {
                    let target = if models.contains(&model) {
                        model
                    } else {
                        models[0].clone()
                    };
                    let params =
                        json!({ "sessionId": session_id, "configId": "model", "value": target });
                    let _ = client.request(
                        "session/set_config_option",
                        &params,
                        Duration::from_secs(20),
                    );
                }
            }
        }
        self.emit(AcpEvent::SessionReady {
            session_id,
            modes,
            models,
            title,
        });
    }

    fn cmd_prompt(&mut self, session_id: &str, text: &str) {
        let Some(client) = self.client.as_mut() else {
            self.emit(AcpEvent::SessionError {
                context: "prompt".into(),
                message: "ACP not running".into(),
            });
            return;
        };
        let params =
            json!({ "sessionId": session_id, "prompt": [ { "type": "text", "text": text } ] });
        match client.request("session/prompt", &params, REQUEST_TIMEOUT) {
            Ok(result) => {
                let stop_reason = result
                    .get("stopReason")
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn")
                    .to_string();
                self.emit(AcpEvent::PromptCompleted {
                    session_id: session_id.to_string(),
                    stop_reason,
                });
            }
            Err(e) => self.emit(AcpEvent::SessionError {
                context: "prompt".into(),
                message: e.to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Response parsing helpers
// ---------------------------------------------------------------------------

fn parse_session_list(result: &Value) -> Vec<AcpSessionInfo> {
    let empty = Vec::new();
    let raw = result
        .get("sessions")
        .and_then(Value::as_array)
        .or_else(|| result.get("items").and_then(Value::as_array))
        .unwrap_or(&empty);
    raw.iter()
        .filter_map(|s| {
            let session_id = s
                .get("sessionId")
                .or_else(|| s.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if session_id.is_empty() {
                return None;
            }
            Some(AcpSessionInfo {
                session_id,
                name: s
                    .get("name")
                    .or_else(|| s.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                cwd: s
                    .get("cwd")
                    .or_else(|| s.get("workingDirectory"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                start_time: s
                    .get("startTime")
                    .or_else(|| s.get("createdAt"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// Parse `session/new` / `session/load` / `session/resume` responses.
fn parse_new_session(result: &Value) -> (String, Vec<ModeInfo>, Vec<String>) {
    let session_id = result
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    // modes: { availableModes: [...] } or a bare array
    let modes_value = result
        .pointer("/modes/availableModes")
        .cloned()
        .or_else(|| result.get("modes").filter(|v| v.is_array()).cloned())
        .unwrap_or(Value::Array(Vec::new()));
    let modes = modes_value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(ModeInfo {
                        id: m.get("id").and_then(Value::as_str)?.to_string(),
                        name: m
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // models: { availableModels: [...] } or a bare array of { modelId }
    let models_value = result
        .pointer("/models/availableModels")
        .cloned()
        .or_else(|| result.get("models").filter(|v| v.is_array()).cloned())
        .unwrap_or(Value::Array(Vec::new()));
    let models = models_value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    m.get("modelId")
                        .or_else(|| m.get("id"))
                        .or_else(|| m.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();

    (session_id, modes, models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_session_response() {
        let result = serde_json::json!({
            "sessionId": "abc",
            "modes": { "availableModes": [ { "id": "ask", "name": "Ask" } ] },
            "models": { "availableModels": [ { "modelId": "mistral-medium-3.5" } ] }
        });
        let (sid, modes, models) = parse_new_session(&result);
        assert_eq!(sid, "abc");
        assert_eq!(modes.len(), 1);
        assert_eq!(models, vec!["mistral-medium-3.5".to_string()]);
    }

    #[test]
    fn parses_new_session_response_bare_arrays() {
        let result = serde_json::json!({
            "sessionId": "abc",
            "modes": [ { "id": "ask", "name": "Ask" } ],
            "models": [ { "modelId": "m1" }, { "modelId": "m2" } ]
        });
        let (_, modes, models) = parse_new_session(&result);
        assert_eq!(modes.len(), 1);
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn parses_session_list() {
        let result = serde_json::json!({
            "sessions": [ { "sessionId": "s1", "name": "Session 1", "cwd": "/tmp", "startTime": "2026-01-01" } ]
        });
        let sessions = parse_session_list(&result);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "s1");
    }
}
