//! The app-server worker: owns the `vibe-app-server` subprocess,
//! executes [`AgentCommand`]s from the engine and streams [`AgentEvent`]s
//! back — the same surface the engine drives, so the UI layer never
//! touches the wire protocol.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use mistraldeck_core::proto::{AgentCommand, AgentEvent, AgentSessionInfo};

use crate::conn::{Conn, Inbound};
use crate::proto::{CallbackEntry, SessionListItem, StateResponse};
use crate::reducer::Projection;

/// Flatten a raw wire value (epoch ms / ISO string) to a display string.
fn value_to_string(v: Option<Value>) -> String {
    match v {
        Some(Value::String(s)) => s,
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// First present string field (camelCase before snake_case fallbacks).
fn str_field(v: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// First present bool field (missing = false — tolerant wire style).
fn bool_field(v: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Opt-in diagnostic trace (`VD_TRACE=1`): the RPC/slot lifecycle log used to
/// debug "the agent never receives session/start" (issue Z29).
pub fn trace(line: impl AsRef<str>) {
    if std::env::var_os("VD_TRACE").is_some() {
        eprintln!("[appsrv] {}", line.as_ref());
    }
}

pub const CLIENT_NAME: &str = "Halny Vibe Deck";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Handle held by the engine (`send` / `cmd_sender` / `events`).
pub struct ApsHandle {
    cmd_tx: Sender<AgentCommand>,
    event_rx: Mutex<Receiver<AgentEvent>>,
}

impl ApsHandle {
    /// Spawn the worker thread. Cheap: no subprocess starts until
    /// [`AgentCommand::Start`] arrives.
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = channel::<AgentCommand>();
        let (event_tx, event_rx) = channel::<AgentEvent>();
        std::thread::Builder::new()
            .name("mistraldeck-appsrv".into())
            .spawn(move || Worker::new(event_tx, cmd_rx).run())
            .expect("failed to spawn app-server worker thread");
        Self {
            cmd_tx,
            event_rx: Mutex::new(event_rx),
        }
    }

    pub fn send(&self, cmd: AgentCommand) {
        // the worker always drains commands; ignore send errors only
        // if the worker is gone (app shutting down)
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn cmd_sender(&self) -> Sender<AgentCommand> {
        self.cmd_tx.clone()
    }

    pub fn events(&self) -> std::sync::MutexGuard<'_, Receiver<AgentEvent>> {
        self.event_rx.lock().expect("app-server receiver poisoned")
    }
}

struct Worker {
    events: Sender<AgentEvent>,
    cmds: Receiver<AgentCommand>,
    conn: Option<Conn>,
    /// Shared inbox handle (survives conn swaps; borrow-free draining).
    inbox: Arc<Mutex<Receiver<Inbound>>>,
    proj: Projection,
    /// Reply channels keyed by OUR request id.
    pending: Arc<Mutex<HashMap<String, Sender<Inbound>>>>,
    /// Argv of the last spawn — a transport hiccup reconnects with it
    /// (stable v1.0.16: resume retries instead of surfacing the error).
    last_argv: Vec<String>,
}

impl Worker {
    fn new(events: Sender<AgentEvent>, cmds: Receiver<AgentCommand>) -> Self {
        // placeholder inbox until a connection exists
        let (_tx, rx) = channel::<Inbound>();
        Self {
            events,
            cmds,
            conn: None,
            inbox: Arc::new(Mutex::new(rx)),
            proj: Projection::new(),
            pending: Arc::new(Mutex::new(HashMap::new())),
            last_argv: Vec::new(),
        }
    }

    fn emit(&self, ev: AgentEvent) {
        let _ = self.events.send(ev);
    }

    fn error(&self, context: &str, message: impl Into<String>) {
        self.emit(AgentEvent::SessionError {
            context: context.to_string(),
            message: message.into(),
        });
    }

    fn run(mut self) {
        trace("worker up");
        loop {
            match self.cmds.recv_timeout(Duration::from_millis(100)) {
                Ok(cmd) => {
                    trace(format!("worker <- cmd {cmd:?}"));
                    self.handle(cmd)
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    trace("worker down (commands closed)");
                    return;
                }
            }
            self.pump_inbound();
        }
    }

    /// Drain the shared inbox; route replies to waiters, handle the
    /// rest (notifications, server requests) through the reducer.
    fn pump_inbound(&mut self) {
        loop {
            let msg = {
                let inbox = self.inbox.lock().expect("inbox");
                match inbox.try_recv() {
                    Ok(m) => m,
                    Err(_) => return,
                }
            };
            match msg {
                Inbound::Response { id, result } => {
                    let waiter =
                        self.pending.lock().expect("pending").remove(&id);
                    if let Some(tx) = waiter {
                        let _ = tx.send(Inbound::Response { id, result });
                    }
                    // unknown response ids (late replies) are dropped
                }
                Inbound::ServerRequest { id, method, params } => {
                    self.server_request(id, &method, &params);
                }
                Inbound::Notification { method, params } => {
                    self.consume(&method, &params);
                }
            }
        }
    }

    fn server_request(&mut self, id: i64, method: &str, params: &Value) {
        if method == "callback/call" {
            // ack immediately, then surface to the UI; the decision
            // itself travels separately via callback/result (F2)
            if let Some(conn) = &self.conn {
                conn.ack_callback(id, params, true);
            }
            if let Ok(cb) = serde_json::from_value::<CallbackEntry>(
                params.get("callback").cloned().unwrap_or(Value::Null),
            ) {
                if let Some(ev) = self.proj.callback_event(&cb) {
                    self.emit(ev);
                }
            }
        } else {
            // unknown server request — ack generically so the server
            // is not left waiting
            if let Some(conn) = &self.conn {
                let _ = conn.respond(id, json!({}));
            }
        }
    }

    fn consume(&mut self, method: &str, params: &Value) {
        for ev in self.proj.consume(method, params) {
            self.emit(ev);
        }
        if self.proj.resync_needed {
            self.resync();
        }
    }

    fn handle(&mut self, cmd: AgentCommand) {
        match cmd {
            AgentCommand::Start { argv } => self.start(argv),
            AgentCommand::Stop => {
                self.conn = None;
                self.last_argv.clear();
                self.proj = Projection::new();
            }
            AgentCommand::ListSessions => self.list_sessions(),
            AgentCommand::NewSession { cwd, model } => {
                self.new_session(&cwd, model.as_deref());
            }
            AgentCommand::LoadSession { session_id, .. }
            | AgentCommand::ResumeSession { session_id, .. } => {
                self.resume_session(&session_id);
            }
            AgentCommand::Prompt {
                session_id,
                text,
                images,
            } => {
                self.enqueue(&session_id, &text, images);
            }
            AgentCommand::Cancel { .. } => self.interrupt(),
            AgentCommand::SetConfigOption {
                session_id,
                key,
                value,
            } => self.set_config_option(&session_id, &key, &value),
            AgentCommand::Approve {
                request_id,
                option_id,
            } => self.approve(&request_id, &option_id),
            AgentCommand::AnswerQuestion {
                request_id,
                answer,
            } => self.answer(&request_id, &answer),
            // --- L1: setup / konto (metody globalne, idą przed sesją) ---
            AgentCommand::SetupStatus => self.setup_status(),
            AgentCommand::SetupStoreCredential {
                provider,
                api_key,
                custom_domain,
            } => self.store_credential(provider.as_deref(), &api_key, custom_domain.as_deref()),
            AgentCommand::SetupSubmitChoices => self.submit_choices(),
            AgentCommand::IdentityRead => self.read_identity(),
            AgentCommand::AccountRead => self.read_account(),
            // Engine-side (local uv install); never reaches a transport.
            AgentCommand::SetupInstall => {}
        }
    }

    /// Spawn + initialize handshake (F1).
    fn start(&mut self, argv: Vec<String>) {
        if self.conn.is_some() {
            return; // already running
        }
        self.last_argv = argv.clone();
        match Conn::spawn(&argv) {
            Ok(conn) => {
                self.inbox = conn.inbox.clone();
                self.conn = Some(conn);
            }
            Err(e) => {
                self.error("start", format!("{e:#}"));
                return;
            }
        }
        let params = json!({
            "clientInfo": {
                "name": CLIENT_NAME,
                "version": CLIENT_VERSION,
                "entrypoint": "unknown",
                "terminalEmulator": "unknown",
            },
            "capabilities": {
                "callbackKinds": ["approval", "user_input"],
                "clientTools": [],
                "sessionSpawn": false,
            },
        });
        match self.call("initialize", params, "initialize") {
            Ok(res) => {
                // LSP-style completion: the server only enters the
                // INITIALIZED state after this notification (server.py
                // _handle_notification — undocumented in the protocol ref)
                let _ = self.conn.as_ref().unwrap().notify("initialized", json!({}));
                let info = res.get("serverInfo").cloned();
                let name = info
                    .as_ref()
                    .and_then(|i| i.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("vibe-app-server")
                    .to_string();
                let ver = info
                    .as_ref()
                    .and_then(|i| i.get("version"))
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                self.emit(AgentEvent::Initialized { protocol_version: 1 });
                self.emit(AgentEvent::DebugLog(format!(
                    "app-server: {name} {ver} connected"
                )));
            }
            Err(e) => {
                // handshake failed — drop the process so the next
                // Start can retry cleanly
                self.conn = None;
                self.error("initialize", format!("{e:#}"));
            }
        }
    }

    /// Request/response with no wall-clock cap, pumping the inbox while
    /// waiting so notifications keep flowing in order (response
    /// boundary — docs/appserver-protocol.md §10). The error carries
    /// the raw RPC `(code, message)` so callers can do feature
    /// detection (`method_not_found` / -32601, L1).
    ///
    /// No wall-clock cap (stable v1.0.34 parity): a legitimate turn can
    /// run for hours, and the CLI only answers some requests (resume)
    /// after long work. Safety comes from the agent-exit probe below —
    /// when the vibe-app-server process dies, every waiter fails fast
    /// instead of hanging.
    fn call_res(&mut self, method: &str, params: Value, context: &str) -> Result<Value, (i64, String)> {
        trace(format!("rpc -> {method}"));
        if self.conn.is_none() {
            trace(format!("rpc !! {method}: not connected"));
            return Err((-32000, format!("{context}: not connected")));
        }
        let id = self
            .conn
            .as_ref()
            .expect("checked")
            .request(method, params)
            .map_err(|e| (-32000, format!("{context}: {e:#}")))?;
        let (tx, rx) = channel();
        self.pending
            .lock()
            .expect("pending")
            .insert(id.clone(), tx);
        loop {
            self.pump_inbound();
            match rx.try_recv() {
                Ok(Inbound::Response { result, .. }) => {
                    return match result {
                        Ok(v) => {
                            if let Ok(r) =
                                serde_json::from_value::<StateResponse>(v.clone())
                            {
                                if let Some(w) = r.last_event_id {
                                    self.proj.note_response_watermark(w);
                                }
                            }
                            Ok(v)
                        }
                        Err((code, message)) => Err((code, format!("{context}: {message}"))),
                    };
                }
                Ok(_) => {} // routed elsewhere
                Err(_) => {}
            }
            if self.conn.is_none() {
                self.pending.lock().expect("pending").remove(&id);
                return Err((-32000, format!("{context}: connection gone")));
            }
            // Fail fast when the agent process died mid-request
            // (stable v1.0.4/v1.0.34: reject pending, never hang).
            if let Some(conn) = self.conn.as_mut() {
                if conn.exited().is_some() {
                    self.pending.lock().expect("pending").remove(&id);
                    self.conn = None;
                    return Err((-32000, format!("{context}: agent exited")));
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn call(&mut self, method: &str, params: Value, context: &str) -> Result<Value> {
        self.call_res(method, params, context)
            .map_err(|(_code, message)| anyhow!("{message}"))
    }

    fn list_sessions(&mut self) {
        let res = match self.call("session/list", json!({}), "session/list") {
            Ok(v) => v,
            Err(e) => {
                self.error("session/list", format!("{e:#}"));
                return;
            }
        };
        // CLI 3.0.x wraps the list in `items` (SessionListResponse); older
        // releases sent a bare array or `sessions` — accept all three.
        let items: Vec<SessionListItem> = ["items", "sessions"]
            .iter()
            .find_map(|k| res.get(k).cloned())
            .or_else(|| Some(res.clone()))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let infos: Vec<AgentSessionInfo> = items
            .into_iter()
            .map(|s| {
                // Round 4 (B3): a session the CLI has not titled yet gets a
                // real name — the index title, else the first user prompt
                // (`preview`) — instead of a bare "(CLI)" placeholder.
                let mut name = s.title.unwrap_or_default();
                if name.trim().is_empty() {
                    name = mistraldeck_vibe::stats::index_title(&s.id).unwrap_or_default();
                }
                if name.trim().is_empty() {
                    name = s.preview.unwrap_or_default();
                }
                if name.trim().is_empty() {
                    // round 5 (R2): last fallback — the on-disk index preview
                    name = mistraldeck_vibe::stats::index_preview(&s.id).unwrap_or_default();
                }
                if name.len() > 80 {
                    name = format!("{}…", &name[..name.len().min(80)]);
                }
                AgentSessionInfo {
                    session_id: s.id,
                    name,
                    cwd: s.cwd.unwrap_or_default(),
                    start_time: value_to_string(s.updated_at),
                }
            })
            .collect();
        self.emit(AgentEvent::SessionsList(infos));
    }

    fn new_session(&mut self, cwd: &str, model: Option<&str>) {
        let mut agent_config = json!({});
        if !cwd.is_empty() {
            // empty string cwd is REJECTED by the server validator —
            // omit the field instead (docs pitfall 2)
            agent_config["cwd"] = json!(cwd);
        }
        let mut params = json!({
            "agentConfig": agent_config,
            "kind": "normal",
        });
        if let Some(m) = model {
            if !m.is_empty() {
                // per-session model without changing the user default
                params["model"] = json!(m);
                params["sessionOnly"] = json!(true);
            }
        }
        match self.call("session/start", params, "session/start") {
            Ok(res) => self.apply_state_response(res, "session/start"),
            Err(e) => self.error("session/start", format!("{e:#}")),
        }
    }

    fn resume_session(&mut self, session_id: &str) {
        let params = json!({ "sessionId": session_id });
        match self.call_res("session/resume", params.clone(), "session/resume") {
            Ok(res) => self.apply_state_response(res, "session/resume"),
            Err((code, msg)) => {
                // v1.0.16 parity: a transport hiccup (-32000 = our internal
                // "connection gone / agent exited" class) gets one retry
                // after a reconnect instead of surfacing the error.
                if code == -32000 && !self.last_argv.is_empty() {
                    self.conn = None;
                    let argv = self.last_argv.clone();
                    self.start(argv);
                    if self.conn.is_some() {
                        std::thread::sleep(Duration::from_millis(300));
                        match self.call_res("session/resume", params, "session/resume") {
                            Ok(res) => {
                                self.apply_state_response(res, "session/resume");
                                return;
                            }
                            Err((_, m2)) => {
                                self.error("session/resume", m2);
                                return;
                            }
                        }
                    }
                }
                self.error("session/resume", msg);
            }
        }
    }

    /// F1: `{state, lastEventId}` — the authoritative snapshot replaces
    /// the projection state (no replay buffering, no races).
    fn apply_state_response(&mut self, res: Value, context: &str) {
        let Ok(r) = serde_json::from_value::<StateResponse>(res) else {
            self.error(context, "invalid response shape");
            return;
        };
        if let Some(w) = r.last_event_id {
            self.proj.note_response_watermark(w);
        }
        if let Some(state) = r.state {
            for ev in self.proj.apply_snapshot(state) {
                self.emit(ev);
            }
        }
    }

    fn enqueue(
        &mut self,
        session_id: &str,
        text: &str,
        images: Vec<mistraldeck_core::proto::PromptImage>,
    ) {
        // Inline images travel as `SessionImageContentBlock`s (models.py):
        // a data URI plus the media type — the server decodes it back into
        // an inline ImageAttachment (app_server/_turn_input.py).
        let mut content = vec![json!({"type": "text", "text": text})];
        for img in images {
            content.push(json!({
                "type": "image",
                "uri": format!("data:{};base64,{}", img.mime, img.data_base64),
                "mediaType": img.mime,
                "altText": "image",
            }));
        }
        let params = json!({
            "sessionId": session_id,
            "entries": [
                {"role": "user", "content": content}
            ],
        });
        if let Err(e) = self.call(
            "session/turn/enqueue",
            params,
            "session/turn/enqueue",
        ) {
            self.error("session/turn/enqueue", format!("{e:#}"));
        }
    }

    /// W9: interrupt with expected_turn validation (STALE_TURN).
    fn interrupt(&mut self) {
        let Some(turn) = self.proj.current_turn.clone() else {
            self.emit(AgentEvent::DebugLog(
                "app-server: cancel without an active turn".into(),
            ));
            return;
        };
        let Some(turn_id) = turn.id else { return };
        let params = json!({
            "sessionId": self.proj.session_id,
            "expectedTurnId": turn_id,
        });
        if let Err(e) = self.call("turn/interrupt", params, "turn/interrupt") {
            self.error("turn/interrupt", format!("{e:#}"));
        }
    }

    /// F4: model/agent/thinking per session. `session/agent/update`
    /// for agent/mode, `config/model/write` for model + thinking —
    /// always sessionOnly so the user default is untouched (W5).
    ///
    /// A rejected write (the server validates model/reasoningEffort per
    /// session model) is NOT a session error: the turn must not die with
    /// "WARNING config/model/write …". It is logged as a debug line and
    /// the session view is refreshed instead.
    fn set_config_option(&mut self, session_id: &str, key: &str, value: &str) {
        if value.trim().is_empty() {
            // An empty value is invalid on the wire (INVALID_PARAMS) —
            // skip the call entirely.
            return;
        }
        let (method, params) = match key {
            "agent" | "mode" => (
                "session/agent/update",
                json!({"sessionId": session_id, "agentName": value}),
            ),
            "thinking" => (
                "config/model/write",
                json!({
                    "sessionId": session_id,
                    "reasoningEffort": value,
                    "sessionOnly": true,
                }),
            ),
            _ => (
                "config/model/write",
                json!({
                    "sessionId": session_id,
                    "model": value,
                    "sessionOnly": true,
                }),
            ),
        };
        if let Err(e) = self.call(method, params, method) {
            // Non-fatal: a config write the server rejects must not kill
            // the running turn (it used to surface as a WARNING system
            // message and clear the pending prompt).
            self.emit(AgentEvent::DebugLog(format!(
                "config write {method} rejected: {e:#}"
            )));
            return;
        }
        // refresh the session view (new model/agent in the snapshot)
        self.resume_session(session_id);
    }

    /// F2: approval decision → callback/result. UI option ids map to
    /// the ApprovalDecisionType enum.
    fn approve(&mut self, callback_id: &str, option_id: &str) {
        let decision = match option_id {
            "approve_once" | "approve" | "yes" | "allow" => "approve",
            "approve_always" | "approve_for_session" => "approve_for_session",
            _ => "deny",
        };
        self.callback_result(
            callback_id,
            json!({
                "type": "approval",
                "decision": {"type": decision, "pathScope": null},
                "feedback": null,
            }),
        );
    }

    fn answer(&mut self, callback_id: &str, answer: &str) {
        self.callback_result(
            callback_id,
            json!({"type": "user_input", "result": {"text": answer}}),
        );
    }

    fn callback_result(&mut self, callback_id: &str, output: Value) {
        let session_id = self
            .proj
            .callback_sessions
            .get(callback_id)
            .cloned()
            .unwrap_or_else(|| self.proj.session_id.clone());
        let params = json!({
            "sessionId": session_id,
            "result": {"callbackId": callback_id, "output": output},
        });
        if let Err(e) = self.call("callback/result", params, "callback/result") {
            self.error("callback/result", format!("{e:#}"));
        }
    }

    // -------------------------------------------------------------------
    // L1: setup/* + identity/read + account/read (metody globalne —
    // działają po samym Start+initialize, bez sesji; TUI woła je tak samo).
    // Feature detection: method_not_found (-32601) = stary CLI → pole
    // `unsupported` zamiast SessionError (raport §5).
    // -------------------------------------------------------------------

    fn is_method_not_found(code: i64, message: &str) -> bool {
        code == -32601 || message.contains("method_not_found")
    }

    fn setup_status(&mut self) {
        let res = match self.call_res("setup/status", json!({}), "setup/status") {
            Ok(v) => v,
            Err((code, message)) => {
                if Self::is_method_not_found(code, &message) {
                    self.emit(AgentEvent::SetupStatusReady {
                        status: mistraldeck_core::snapshot::SetupStatusView {
                            unsupported: true,
                            ..Default::default()
                        },
                    });
                } else {
                    self.error("setup/status", message);
                }
                return;
            }
        };
        self.emit(AgentEvent::SetupStatusReady {
            status: map_setup_status(&res),
        });
    }

    fn store_credential(
        &mut self,
        provider: Option<&str>,
        api_key: &str,
        custom_domain: Option<&str>,
    ) {
        let mut params = json!({ "apiKey": api_key });
        if let Some(p) = provider {
            params["provider"] = json!(p);
        }
        if let Some(d) = custom_domain {
            if !d.is_empty() {
                params["customDomain"] = json!(d);
            }
        }
        let res = match self.call_res("setup/store-credential", params, "setup/store-credential") {
            Ok(v) => v,
            Err((code, message)) => {
                if Self::is_method_not_found(code, &message) {
                    self.emit(AgentEvent::SetupStoreOutcome {
                        outcome: "unsupported".into(),
                        detail: None,
                    });
                } else {
                    self.error("setup/store-credential", message);
                }
                return;
            }
        };
        self.emit(AgentEvent::SetupStoreOutcome {
            outcome: str_field(&res, &["outcome"]),
            detail: res
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string),
        });
    }

    fn submit_choices(&mut self) {
        let res = match self.call_res("setup/submit-choices", json!({}), "setup/submit-choices") {
            Ok(v) => v,
            Err((code, message)) => {
                if Self::is_method_not_found(code, &message) {
                    self.emit(AgentEvent::SetupChoicesOutcome {
                        outcome: "unsupported".into(),
                        failures: Vec::new(),
                    });
                } else {
                    self.error("setup/submit-choices", message);
                }
                return;
            }
        };
        self.emit(AgentEvent::SetupChoicesOutcome {
            outcome: str_field(&res, &["outcome"]),
            failures: res
                .get("failures")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|f| {
                            f.as_str().map(str::to_string).unwrap_or_else(|| f.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default(),
        });
    }

    fn read_identity(&mut self) {
        // `identity/read` requires `sessionId` (IdentityReadParams) —
        // it is only meaningful once a session is attached; skip
        // silently before that (the engine re-asks on SessionReady).
        if self.proj.session_id.is_empty() {
            return;
        }
        let res = match self.call_res(
            "identity/read",
            json!({ "sessionId": self.proj.session_id }),
            "identity/read",
        ) {
            Ok(v) => v,
            Err((code, message)) => {
                if Self::is_method_not_found(code, &message) {
                    self.emit(AgentEvent::IdentityReady {
                        identity: mistraldeck_core::snapshot::IdentityView::default(),
                    });
                } else {
                    self.error("identity/read", message);
                }
                return;
            }
        };
        self.emit(AgentEvent::IdentityReady {
            identity: mistraldeck_core::snapshot::IdentityView {
                raw_json: serde_json::to_string(&res).unwrap_or_default(),
            },
        });
    }

    fn read_account(&mut self) {
        // `sessionId` is REQUIRED (AccountReadParams) — skip before a
        // session exists (the engine re-asks on SessionReady).
        if self.proj.session_id.is_empty() {
            return;
        }
        let res = match self.call_res(
            "account/read",
            json!({ "sessionId": self.proj.session_id }),
            "account/read",
        ) {
            Ok(v) => v,
            Err((code, message)) => {
                if Self::is_method_not_found(code, &message) {
                    self.emit(AgentEvent::AccountReady {
                        account: mistraldeck_core::snapshot::AccountView {
                            unsupported: true,
                            ..Default::default()
                        },
                    });
                } else {
                    self.error("account/read", message);
                }
                return;
            }
        };
        let acct = res.get("account").cloned().unwrap_or(res.clone());
        self.emit(AgentEvent::AccountReady {
            account: map_account(&acct),
        });
    }

    /// F3: full resync — the authoritative snapshot replaces the
    /// projection state (missed notifications / stale cursor).
    fn resync(&mut self) {
        self.proj.resync_needed = false;
        let id = self.proj.session_id.clone();
        if id.is_empty() {
            return;
        }
        self.resume_session(&id);
    }
}

// ---------------------------------------------------------------------------
// L1: tolerant wire → view mapping (pure, unit-tested against
// fixture shapes from docs/tui-capabilities-report.md §2/§3).
// ---------------------------------------------------------------------------

/// `setup/status` → [`SetupStatusView`] (camelCase with snake_case
/// fallbacks; missing bools = false, missing strings = "").
pub(crate) fn map_setup_status(res: &Value) -> mistraldeck_core::snapshot::SetupStatusView {
    let provider = res.get("provider").filter(|p| p.is_object()).map(|p| {
        mistraldeck_core::snapshot::SetupProviderView {
            name: str_field(p, &["name"]),
            api_base: str_field(p, &["apiBase", "api_base"]),
            api_key_env_var: str_field(p, &["apiKeyEnvVar", "api_key_env_var"]),
            browser_auth_base_url: str_field(p, &["browserAuthBaseUrl", "browser_auth_base_url"]),
            browser_auth_api_base_url: str_field(
                p,
                &["browserAuthApiBaseUrl", "browser_auth_api_base_url"],
            ),
            browser_auth_allow_origin_rewrite: bool_field(
                p,
                &["browserAuthAllowOriginRewrite", "browser_auth_allow_origin_rewrite"],
            ),
        }
    });
    mistraldeck_core::snapshot::SetupStatusView {
        provider,
        console_base_url: str_field(res, &["consoleBaseUrl", "console_base_url"]),
        vibe_base_url: str_field(res, &["vibeBaseUrl", "vibe_base_url"]),
        supports_browser_sign_in: bool_field(
            res,
            &["supportsBrowserSignIn", "supports_browser_sign_in"],
        ),
        has_api_key: bool_field(res, &["hasApiKey", "has_api_key"]),
        pinned_console_url: res
            .get("pinnedConsoleUrl")
            .or_else(|| res.get("pinned_console_url"))
            .and_then(Value::as_str)
            .map(str::to_string),
        unsupported: false,
    }
}

/// `account/read` body (the `account` object or the bare response)
/// → [`AccountView`] (models.py:119-126; `accessScope` absent on
/// older backends → None, never a render blocker).
pub(crate) fn map_account(acct: &Value) -> mistraldeck_core::snapshot::AccountView {
    let plan = acct.get("plan").filter(|p| p.is_object()).map(|p| {
        mistraldeck_core::snapshot::AccountPlanView {
            kind: str_field(p, &["kind"]),
            name: str_field(p, &["name"]),
            title: str_field(p, &["title"]),
        }
    });
    let action = |keys: &[&str]| {
        keys.iter().find_map(|k| acct.get(*k)).filter(|a| a.is_object()).map(|a| {
            mistraldeck_core::snapshot::AccountActionView {
                kind: str_field(a, &["kind"]),
                url: str_field(a, &["url"]),
            }
        })
    };
    mistraldeck_core::snapshot::AccountView {
        status: str_field(acct, &["status"]),
        plan,
        plan_offer: action(&["planOffer", "plan_offer"]),
        rate_limit_action: action(&["rateLimitAction", "rate_limit_action"]),
        teleport_eligible: bool_field(acct, &["teleportEligible", "teleport_eligible"]),
        api_key: acct
            .get("apiKey")
            .or_else(|| acct.get("api_key"))
            .filter(|k| k.is_object())
            .map(|k| mistraldeck_core::snapshot::AccountApiKeyView {
                preview: str_field(k, &["preview"]),
                scope: str_field(k, &["scope"]),
                access_scope: k
                    .get("accessScope")
                    .or_else(|| k.get("access_scope"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
        unsupported: false,
    }
}

#[cfg(test)]
mod l1_tests {
    use super::*;
    use serde_json::json;

    /// Report §3.0 — the full setup/status fixture shape.
    #[test]
    fn parses_setup_status_fixture() {
        let v = json!({
            "provider": {"name": "mistral", "apiBase": "https://api.mistral.ai/v1",
                         "apiKeyEnvVar": "MISTRAL_API_KEY",
                         "browserAuthBaseUrl": "https://console.mistral.ai",
                         "browserAuthApiBaseUrl": "https://api.mistral.ai",
                         "browserAuthAllowOriginRewrite": false},
            "consoleBaseUrl": "https://console.mistral.ai",
            "vibeBaseUrl": "https://code.mistral.ai",
            "activeModel": "m", "theme": "dark",
            "supportsBrowserSignIn": true,
            "hasApiKey": false,
            "enableSystemTrustStore": false,
            "pinnedConsoleUrl": null
        });
        let s = map_setup_status(&v);
        assert_eq!(s.provider.as_ref().unwrap().name, "mistral");
        assert_eq!(
            s.provider.as_ref().unwrap().browser_auth_api_base_url,
            "https://api.mistral.ai"
        );
        assert!(s.supports_browser_sign_in);
        assert!(!s.has_api_key);
        assert!(s.pinned_console_url.is_none());
        assert!(!s.unsupported);
    }

    #[test]
    fn setup_status_missing_fields_defaults() {
        let s = map_setup_status(&json!({}));
        assert!(s.provider.is_none());
        assert!(!s.supports_browser_sign_in && !s.has_api_key);
        assert_eq!(s.console_base_url, "");
    }

    /// Report §2 — the full AccountView fixture, plus null actions
    /// and a missing accessScope ("None on older backends").
    #[test]
    fn parses_account_fixture_and_variants() {
        let full = json!({
            "status": "ready",
            "plan": {"kind": "chat", "name": "Pro", "title": "[Subscription] Pro"},
            "planOffer": {"kind": "switch_api_key", "url": "…/code/extensions?focus=key"},
            "rateLimitAction": {"kind": "upgrade_to_pro", "url": "https://admin.mistral.ai/…"},
            "teleportEligible": true,
            "apiKey": {"preview": "abc…****", "scope": "vibe", "accessScope": "personal_and_shared"}
        });
        let a = map_account(&full);
        assert_eq!(a.status, "ready");
        assert_eq!(a.plan.as_ref().unwrap().title, "[Subscription] Pro");
        assert_eq!(a.plan_offer.as_ref().unwrap().kind, "switch_api_key");
        assert_eq!(a.rate_limit_action.as_ref().unwrap().kind, "upgrade_to_pro");
        assert_eq!(a.api_key.as_ref().unwrap().access_scope.as_deref(), Some("personal_and_shared"));

        let minimal = json!({
            "status": "missing_key", "plan": null, "planOffer": null,
            "rateLimitAction": null, "apiKey": {"preview": "x…"}
        });
        let a = map_account(&minimal);
        assert_eq!(a.status, "missing_key");
        assert!(a.plan.is_none() && a.plan_offer.is_none() && a.rate_limit_action.is_none());
        let k = a.api_key.unwrap();
        assert_eq!(k.preview, "x…");
        assert!(k.access_scope.is_none());
    }

    #[test]
    fn method_not_found_detection() {
        assert!(Worker::is_method_not_found(-32601, "setup/status: method_not_found"));
        assert!(Worker::is_method_not_found(-1, "error: method_not_found"));
        assert!(!Worker::is_method_not_found(-32602, "invalid params"));
        assert!(!Worker::is_method_not_found(-32000, "not connected"));
    }
}
