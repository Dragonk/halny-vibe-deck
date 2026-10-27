//! The ACP worker: owns the `vibe-acp` subprocess, executes commands
//! from the UI, and streams events back. Mirrors the architecture of
//! the old Electron main process (`acp.ts`) with none of the Electron.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use mistraldeck_core::proto::{
    AcpCommand, AcpEvent, AcpSessionInfo, ConfigOption, ConfigOptionChoice, ModeInfo,
};
use mistraldeck_core::types::{Message, MessageSender, PlanEntry, ThinkingBlock};

pub const CLIENT_INFO_NAME: &str = "Halny Vibe Deck";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Handle held by the UI. Clone the sender at will; keep the receiver unique.
pub struct AcpHandle {
    cmd_tx: Sender<AcpCommand>,
    // Mutex so the handle is Sync (shared statics, server threads).
    event_rx: std::sync::Mutex<Receiver<AcpEvent>>,
}

impl AcpHandle {
    /// Spawn the worker thread. Cheap: no subprocess is started until
    /// [`AcpCommand::Start`] arrives.
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<AcpCommand>();
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AcpEvent>();
        std::thread::Builder::new()
            .name("mistraldeck-acp".into())
            .spawn(move || Worker::new(event_tx, cmd_rx).run())
            .expect("failed to spawn ACP worker thread");
        Self {
            cmd_tx,
            event_rx: std::sync::Mutex::new(event_rx),
        }
    }

    pub fn send(&self, cmd: AcpCommand) {
        // The worker always drains commands; ignore send errors only if
        // the worker is gone (app shutting down).
        let _ = self.cmd_tx.send(cmd);
    }

    /// Clone of the command channel (for a dedicated pump thread that
    /// owns the event receiver).
    pub fn cmd_sender(&self) -> std::sync::mpsc::Sender<AcpCommand> {
        self.cmd_tx.clone()
    }

    /// Lock the event receiver (single consumer).
    pub fn events(&self) -> std::sync::MutexGuard<'_, Receiver<AcpEvent>> {
        self.event_rx.lock().expect("ACP event receiver poisoned")
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
    /// Sessions being loaded from disk — their `session/update` replay is
    /// collected here instead of being forwarded live, then emitted once
    /// as a structured [`AcpEvent::SessionHistory`].
    replaying: HashMap<String, Vec<Value>>,
    stdout_closed: bool,
    next_id: u64,
}

impl Client {
    fn spawn_argv(
        argv: &[String],
        raw_tx: Sender<RawMsg>,
        raw_rx: Receiver<RawMsg>,
        event_tx: Sender<AcpEvent>,
    ) -> Result<Self> {
        if argv.is_empty() {
            anyhow::bail!("empty agent command");
        }
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
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
            .with_context(|| format!("failed to spawn `{}`", argv.join(" ")))?;

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
            .name("mistraldeck-acp-stdout".into())
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
            .name("mistraldeck-acp-stderr".into())
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
            replaying: HashMap::new(),
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
                    if let Some(replay) = self.replaying.get_mut(&session_id) {
                        // session/load replay — collected, emitted as
                        // SessionHistory once the request finishes. Every
                        // 25 buffered updates a progress event goes out
                        // (Nexus `SessionLoadProgress`) for the
                        // "Resuming session… (N)" overlay.
                        replay.push(params);
                        if replay.len() % 25 == 0 {
                            events.push(AcpEvent::SessionLoadProgress {
                                session_id: session_id.clone(),
                                count: replay.len(),
                            });
                        }
                        continue;
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
            AcpCommand::Start { argv } => self.cmd_start(&argv),
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
            AcpCommand::Prompt {
                session_id,
                text,
                images,
            } => self.cmd_prompt(&session_id, &text, images),
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
            // L1: setup/* + account methods are app-server-only
            // (global methods); the legacy ACP transport ignores them.
            _ => {}
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

    fn cmd_start(&mut self, argv: &[String]) {
        match self.start_client(argv) {
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

    fn start_client(&mut self, argv: &[String]) -> Result<()> {
        // Stop any previous instance first.
        if let Some(mut c) = self.client.take() {
            c.kill();
        }
        self.resume_not_supported = false;

        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<RawMsg>();
        let client = Client::spawn_argv(argv, raw_tx, raw_rx, self.event_tx.clone())?;
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
                let (session_id, modes, config_options, current_mode_id) =
                    parse_new_session(&result);
                if session_id.is_empty() {
                    self.emit(AcpEvent::SessionError {
                        context: "new_session".into(),
                        message: "server returned no sessionId".into(),
                    });
                    return;
                }
                // NOTE: `session/name` was removed — vibe-acp 3.0.2
                // answers it with -32601 and titles come from the UI anyway.
                self.finish_session_ready(
                    session_id,
                    modes,
                    config_options,
                    None,
                    model,
                    current_mode_id,
                );
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
        client.replaying.insert(session_id.to_string(), Vec::new());
        let params = json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": [] });
        let outcome = client.request("session/load", &params, REQUEST_TIMEOUT);
        // vibe-acp flushes the tail of the session/load replay right after
        // the response; grace-drain it into the replay buffer instead of
        // letting it leak into the live event stream.
        if outcome.is_ok() {
            for _ in 0..40 {
                client.drain_raw();
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let replay = client
            .replaying
            .remove(session_id)
            .unwrap_or_default();
        match outcome {
            Ok(result) => {
                let (sid, modes, config_options, current_mode_id) = parse_new_session(&result);
                let (history, plan) = replay_to_messages(&replay);
                if !history.is_empty() || !plan.is_empty() {
                    self.emit(AcpEvent::SessionHistory {
                        session_id: session_id.to_string(),
                        messages: history,
                        plan,
                    });
                }
                self.finish_session_ready(
                    if sid.is_empty() {
                        session_id.to_string()
                    } else {
                        sid
                    },
                    modes,
                    config_options,
                    None,
                    model,
                    current_mode_id,
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
                let (sid, modes, config_options, current_mode_id) = parse_new_session(&result);
                self.finish_session_ready(
                    if sid.is_empty() {
                        session_id.to_string()
                    } else {
                        sid
                    },
                    modes,
                    config_options,
                    None,
                    model,
                    current_mode_id,
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
        config_options: Vec<ConfigOption>,
        title: Option<String>,
        model: Option<String>,
        current_mode_id: Option<String>,
    ) {
        // Apply the requested model if the server advertises a model option.
        if let Some(model) = model.clone() {
            if !model.is_empty() {
                if let Some(opt) = config_options.iter().find(|o| o.id == "model") {
                    if opt.choices.iter().any(|c| c.value == model) {
                        if let Some(client) = self.client.as_mut() {
                            let params = json!({
                                "sessionId": session_id,
                                "configId": "model",
                                "value": model
                            });
                            let _ = client.request(
                                "session/set_config_option",
                                &params,
                                Duration::from_secs(20),
                            );
                        }
                    }
                }
            }
        }
        self.emit(AcpEvent::SessionReady {
            session_id,
            modes,
            config_options,
            title,
            current_mode_id,
        });
    }

    fn cmd_prompt(
        &mut self,
        session_id: &str,
        text: &str,
        images: Vec<mistraldeck_core::proto::PromptImage>,
    ) {
        let Some(client) = self.client.as_mut() else {
            self.emit(AcpEvent::SessionError {
                context: "prompt".into(),
                message: "ACP not running".into(),
            });
            return;
        };
        // v1.0.8: image attachments go out as ACP image content
        // blocks alongside the text block.
        let mut prompt = vec![json!({ "type": "text", "text": text })];
        for img in &images {
            prompt.push(json!({
                "type": "image",
                "data": img.data_base64,
                "mimeType": img.mime,
            }));
        }
        let params = json!({ "sessionId": session_id, "prompt": prompt });
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
/// vibe-acp ≥ 3.0 reports `configOptions` (model / mode / thinking…);
/// older servers report `models.availableModels` — folded into a
/// synthesized "model" option so the UI always has one shape to render.
fn parse_new_session(
    result: &Value,
) -> (String, Vec<ModeInfo>, Vec<ConfigOption>, Option<String>) {
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
    let modes: Vec<ModeInfo> = modes_value
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

    // configOptions: [{id, name, currentValue, options: [{value, name, description}]}]
    let mut config_options: Vec<ConfigOption> = result
        .get("configOptions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    let id = o.get("id").and_then(Value::as_str)?.to_string();
                    let choices = o
                        .get("options")
                        .and_then(Value::as_array)
                        .map(|opts| {
                            opts.iter()
                                .filter_map(|c| {
                                    Some(ConfigOptionChoice {
                                        value: c.get("value").and_then(Value::as_str)?.to_string(),
                                        name: c
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                        description: c
                                            .get("description")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(ConfigOption {
                        id,
                        name: o
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        current_value: o
                            .get("currentValue")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        choices,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Legacy `models` shape → synthesize a "model" config option.
    if !config_options.iter().any(|o| o.id == "model") {
        let models_value = result
            .pointer("/models/availableModels")
            .cloned()
            .or_else(|| result.get("models").filter(|v| v.is_array()).cloned())
            .unwrap_or(Value::Array(Vec::new()));
        let models: Vec<String> = models_value
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
        if !models.is_empty() {
            let current = result
                .pointer("/models/currentModel")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            config_options.push(ConfigOption {
                id: "model".into(),
                name: "Model".into(),
                current_value: if current.is_empty() {
                    models[0].clone()
                } else {
                    current
                },
                choices: models
                    .into_iter()
                    .map(|m| ConfigOptionChoice {
                        value: m.clone(),
                        name: m,
                        description: String::new(),
                    })
                    .collect(),
            });
        }
    }

    // `modes.currentModeId` (Nexus `parseModes`): falls back to the first
    // mode when the server does not report one.
    let current_mode_id = result
        .pointer("/modes/currentModeId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| modes.first().map(|m| m.id.clone()));

    (session_id, modes, config_options, current_mode_id)
}

/// Fold collected `session/load` replay updates into a message list the
/// UI can show as conversation history, plus the last plan snapshot.
///
/// The replay arrives as `user_message_chunk` / `agent_message_chunk` /
/// `agent_thought_chunk` / `tool_call` notifications; user chunks start a
/// new turn, everything else appends to the currently open assistant
/// message. Everything is marked complete — the replay is a finished
/// transcript, not a live stream. The plan is cumulative state: only the
/// last replayed snapshot matters (Nexus v0.11.1 `assembleHistory`).
fn replay_to_messages(updates: &[Value]) -> (Vec<Message>, Vec<PlanEntry>) {
    let mut messages: Vec<Message> = Vec::new();
    let mut plan: Vec<PlanEntry> = Vec::new();
    // v0.13.7 (Nexus `assembleHistory` currentAiMid): messageId of the
    // text inside the current AI message. Replay splits agent segments
    // on a messageId change exactly like the live stream; the "current"
    // fallback (CLI without messageIds) never changes, so old replays
    // keep the merged single-message shape.
    let mut current_ai_mid: Option<String> = None;
    for params in updates {
        for ev in crate::convert::convert_update(params) {
            match ev {
                AcpEvent::UserMessageChunk { text, .. } => {
                    messages.push(Message::new_user(&text));
                    current_ai_mid = None;
                }
                AcpEvent::MessageChunk {
                    text,
                    message_id,
                    ..
                } => {
                    // v0.13.7: a new messageId starts a new agent message,
                    // matching the live stream split.
                    if current_ai_mid.is_some()
                        && current_ai_mid.as_deref() != Some(message_id.as_str())
                    {
                        if let Some(last) = messages.last_mut() {
                            last.is_complete = true;
                        }
                    }
                    current_ai_mid = Some(message_id.clone());
                    ensure_replay_ai_message(&mut messages).content.push_str(&text);
                }
                AcpEvent::ThinkingChunk { text, block_id, .. } => {
                    // v0.13.7: reasoning that arrives after a streamed text
                    // belongs to the next segment - close the current one
                    // before attaching it (Nexus resets currentAiMid).
                    if current_ai_mid.is_some() {
                        if let Some(last) = messages.last_mut() {
                            last.is_complete = true;
                        }
                        current_ai_mid = None;
                    }
                    let msg = ensure_replay_ai_message(&mut messages);
                    match msg.thinking.last_mut() {
                        Some(block) if !block.is_complete && block.id == block_id => {
                            block.content.push_str(&text)
                        }
                        _ => msg.thinking.push(ThinkingBlock {
                            id: block_id,
                            content: text,
                            is_complete: false,
                            started_at: mistraldeck_core::types::now_ms(),
                            completed_at: 0,
                        }),
                    }
                }
                AcpEvent::ThinkingComplete { .. } => {
                    if let Some(msg) = messages.last_mut() {
                        let now = mistraldeck_core::types::now_ms();
                        for block in &mut msg.thinking {
                            block.is_complete = true;
                            if block.completed_at == 0 {
                                block.completed_at = now;
                            }
                        }
                    }
                }
                AcpEvent::ToolCallStarted { tool_call, .. } => {
                    let msg = ensure_replay_ai_message(&mut messages);
                    msg.tool_calls.push(tool_call);
                }
                AcpEvent::ToolCallUpdated {
                    tool_call_id,
                    status,
                    output,
                    ..
                } => {
                    for msg in messages.iter_mut().rev() {
                        for tc in &mut msg.tool_calls {
                            if tc.id == tool_call_id {
                                tc.status = status;
                                if output.is_some() {
                                    tc.output = output.clone();
                                }
                            }
                        }
                    }
                }
                AcpEvent::PlanUpdate { entries_json, .. } => {
                    plan = PlanEntry::parse_list(&entries_json);
                }
                _ => {} // usage / commands are not part of the transcript
            }
        }
    }
    for msg in &mut messages {
        msg.is_complete = true;
        for block in &mut msg.thinking {
            block.is_complete = true;
        }
    }
    (messages, plan)
}

/// Find or open the assistant message replay events append to.
fn ensure_replay_ai_message(messages: &mut Vec<Message>) -> &mut Message {
    let needs_new = !matches!(
        messages.last(),
        Some(msg) if msg.sender == MessageSender::Ai && !msg.is_complete
    );
    if needs_new {
        messages.push(Message::new_ai());
    }
    messages.last_mut().expect("just pushed")
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
        let (sid, modes, config_options, current_mode_id) = parse_new_session(&result);
        assert_eq!(sid, "abc");
        assert_eq!(modes.len(), 1);
        assert_eq!(current_mode_id.as_deref(), Some("ask"));
        let model_opt = config_options.iter().find(|o| o.id == "model").unwrap();
        assert_eq!(model_opt.current_value, "mistral-medium-3.5");
        assert_eq!(model_opt.choices.len(), 1);
    }

    #[test]
    fn parses_new_session_current_mode_id() {
        // Explicit `modes.currentModeId` wins; without it the first
        // available mode is the fallback (Nexus currentModeId).
        let result = serde_json::json!({
            "sessionId": "abc",
            "modes": {
                "currentModeId": "smart-approve",
                "availableModes": [
                    { "id": "ask", "name": "Ask" },
                    { "id": "smart-approve", "name": "Smart Approve" }
                ]
            }
        });
        let (sid, modes, _config_options, current_mode_id) = parse_new_session(&result);
        assert_eq!(sid, "abc");
        assert_eq!(modes.len(), 2);
        assert_eq!(current_mode_id.as_deref(), Some("smart-approve"));
    }

    #[test]
    fn replay_splits_agent_segments_on_message_id() {
        // v0.13.7: agent_message_chunk entries with different messageIds
        // become separate AI messages (Nexus `currentAiMid` split).
        let updates = vec![
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "hello" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": { "type": "text", "text": "seg one" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "messageId": "m2", "content": { "type": "text", "text": "seg two" } }
            }),
        ];
        let (messages, _) = replay_to_messages(&updates);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].sender, MessageSender::User);
        assert_eq!(messages[1].sender, MessageSender::Ai);
        assert_eq!(messages[1].content, "seg one");
        assert_eq!(messages[2].sender, MessageSender::Ai);
        assert_eq!(messages[2].content, "seg two");
    }

    #[test]
    fn replay_thought_after_text_opens_next_segment() {
        // v0.13.7: reasoning streamed after a text segment belongs to the
        // NEXT message - the thought lands above its own text.
        let updates = vec![
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "hello" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": { "type": "text", "text": "seg one" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_thought_chunk", "messageId": "b1", "content": { "type": "text", "text": "hmm" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "messageId": "m2", "content": { "type": "text", "text": "seg two" } }
            }),
        ];
        let (messages, _) = replay_to_messages(&updates);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].content, "seg one");
        assert!(messages[1].thinking.is_empty());
        assert_eq!(messages[2].content, "seg two");
        assert_eq!(messages[2].thinking.len(), 1);
        assert_eq!(messages[2].thinking[0].id, "b1");
        assert_eq!(messages[2].thinking[0].content, "hmm");
    }

    #[test]
    fn replay_builds_conversation_history() {
        // session/load replays the transcript as session/update chunks:
        // user_message_chunk → agent_message_chunk (×2, one message)
        // → user_message_chunk → agent_message_chunk + tool_call.
        let updates = vec![
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "hello" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "hi " } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "there" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "list files" } }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "t1",
                    "title": "read_file",
                    "kind": "read",
                    "rawInput": { "path": "a.txt" }
                }
            }),
            serde_json::json!({
                "sessionId": "s1",
                "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "done" } }
            }),
        ];
        let (messages, plan) = replay_to_messages(&updates);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].sender, MessageSender::User);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].sender, MessageSender::Ai);
        assert_eq!(messages[1].content, "hi there");
        assert!(messages[1].is_complete);
        assert_eq!(messages[3].sender, MessageSender::Ai);
        assert_eq!(messages[3].content, "done");
        assert_eq!(messages[3].tool_calls.len(), 1);
        assert_eq!(messages[3].tool_calls[0].title, "read_file");
        assert!(messages.iter().all(|m| m.is_complete));
        assert!(plan.is_empty());
    }

    #[test]
    fn parses_config_options_modern_shape() {
        // vibe-acp 3.0.2: configOptions with model / mode / thinking.
        let result = serde_json::json!({
            "sessionId": "abc",
            "modes": {
                "currentModeId": "accept-edits",
                "availableModes": [ { "id": "accept-edits", "name": "Accept Edits" } ]
            },
            "configOptions": [
                {
                    "id": "mode",
                    "name": "Session Mode",
                    "currentValue": "accept-edits",
                    "options": [ { "value": "ask", "name": "Ask" } ],
                    "type": "select",
                    "category": "mode"
                },
                {
                    "id": "model",
                    "name": "Model",
                    "currentValue": "glm-5-3-flash",
                    "options": [
                        { "value": "mistral-medium-3.5", "name": "Mistral Medium 3.5" },
                        { "value": "glm-5-3-flash", "name": "GLM 5.3 Flash" }
                    ]
                },
                {
                    "id": "thinking",
                    "name": "Thinking",
                    "currentValue": "max",
                    "options": [
                        { "value": "low", "name": "Low" },
                        { "value": "high", "name": "High" },
                        { "value": "max", "name": "Max" }
                    ]
                }
            ]
        });
        let (sid, _modes, config_options, _current_mode) = parse_new_session(&result);
        assert_eq!(sid, "abc");
        assert_eq!(config_options.len(), 3);
        let model = config_options.iter().find(|o| o.id == "model").unwrap();
        assert_eq!(model.current_value, "glm-5-3-flash");
        assert_eq!(model.current_label(), "GLM 5.3 Flash");
        assert_eq!(model.choices.len(), 2);
        let thinking = config_options.iter().find(|o| o.id == "thinking").unwrap();
        assert_eq!(thinking.choices.len(), 3);
    }

    #[test]
    fn parses_new_session_response_bare_arrays() {
        let result = serde_json::json!({
            "sessionId": "abc",
            "modes": [ { "id": "ask", "name": "Ask" } ],
            "models": [ { "modelId": "m1" }, { "modelId": "m2" } ]
        });
        let (_, modes, options, _current_mode) = parse_new_session(&result);
        assert_eq!(modes.len(), 1);
        // legacy bare `models` array becomes a synthetic "model" option
        let model_opt = options
            .iter()
            .find(|o| o.id == "model")
            .expect("synthetic model option");
        assert_eq!(model_opt.choices.len(), 2);
        assert_eq!(model_opt.choices[0].name, "m1");
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
