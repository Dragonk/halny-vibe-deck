//! The app-server worker: owns the `vibe-app-server` subprocess,
//! executes [`AcpCommand`]s from the engine and streams [`AcpEvent`]s
//! back — the exact surface of `mistraldeck_acp::AcpHandle`, so the
//! engine can swap transports without touching the UI layer.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use mistraldeck_core::proto::{AcpCommand, AcpEvent, AcpSessionInfo};

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

pub const CLIENT_NAME: &str = "Halny Vibe Deck";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Handle held by the engine — identical shape to `AcpHandle`
/// (`send` / `cmd_sender` / `events`).
pub struct ApsHandle {
    cmd_tx: Sender<AcpCommand>,
    event_rx: Mutex<Receiver<AcpEvent>>,
}

impl ApsHandle {
    /// Spawn the worker thread. Cheap: no subprocess starts until
    /// [`AcpCommand::Start`] arrives.
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = channel::<AcpCommand>();
        let (event_tx, event_rx) = channel::<AcpEvent>();
        std::thread::Builder::new()
            .name("mistraldeck-appsrv".into())
            .spawn(move || Worker::new(event_tx, cmd_rx).run())
            .expect("failed to spawn app-server worker thread");
        Self {
            cmd_tx,
            event_rx: Mutex::new(event_rx),
        }
    }

    pub fn send(&self, cmd: AcpCommand) {
        // the worker always drains commands; ignore send errors only
        // if the worker is gone (app shutting down)
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn cmd_sender(&self) -> Sender<AcpCommand> {
        self.cmd_tx.clone()
    }

    pub fn events(&self) -> std::sync::MutexGuard<'_, Receiver<AcpEvent>> {
        self.event_rx.lock().expect("app-server receiver poisoned")
    }
}

struct Worker {
    events: Sender<AcpEvent>,
    cmds: Receiver<AcpCommand>,
    conn: Option<Conn>,
    /// Shared inbox handle (survives conn swaps; borrow-free draining).
    inbox: Arc<Mutex<Receiver<Inbound>>>,
    proj: Projection,
    /// Reply channels keyed by OUR request id.
    pending: Arc<Mutex<HashMap<String, Sender<Inbound>>>>,
}

impl Worker {
    fn new(events: Sender<AcpEvent>, cmds: Receiver<AcpCommand>) -> Self {
        // placeholder inbox until a connection exists
        let (_tx, rx) = channel::<Inbound>();
        Self {
            events,
            cmds,
            conn: None,
            inbox: Arc::new(Mutex::new(rx)),
            proj: Projection::new(),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn emit(&self, ev: AcpEvent) {
        let _ = self.events.send(ev);
    }

    fn error(&self, context: &str, message: impl Into<String>) {
        self.emit(AcpEvent::SessionError {
            context: context.to_string(),
            message: message.into(),
        });
    }

    fn run(mut self) {
        loop {
            match self.cmds.recv_timeout(Duration::from_millis(100)) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
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

    fn handle(&mut self, cmd: AcpCommand) {
        match cmd {
            AcpCommand::Start { argv } => self.start(argv),
            AcpCommand::Stop => {
                self.conn = None;
                self.proj = Projection::new();
            }
            AcpCommand::ListSessions => self.list_sessions(),
            AcpCommand::NewSession { cwd, model } => {
                self.new_session(&cwd, model.as_deref());
            }
            AcpCommand::LoadSession { session_id, .. }
            | AcpCommand::ResumeSession { session_id, .. } => {
                self.resume_session(&session_id);
            }
            AcpCommand::Prompt { session_id, text } => {
                self.enqueue(&session_id, &text);
            }
            AcpCommand::Cancel { .. } => self.interrupt(),
            AcpCommand::SetConfigOption {
                session_id,
                key,
                value,
            } => self.set_config_option(&session_id, &key, &value),
            AcpCommand::Approve {
                request_id,
                option_id,
            } => self.approve(&request_id, &option_id),
            AcpCommand::AnswerQuestion {
                request_id,
                answer,
            } => self.answer(&request_id, &answer),
        }
    }

    /// Spawn + initialize handshake (F1).
    fn start(&mut self, argv: Vec<String>) {
        if self.conn.is_some() {
            return; // already running
        }
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
                self.emit(AcpEvent::Initialized { protocol_version: 1 });
                self.emit(AcpEvent::DebugLog(format!(
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

    /// Request/response with the timeout, pumping the inbox while
    /// waiting so notifications keep flowing in order (response
    /// boundary — docs/appserver-protocol.md §10).
    fn call(&mut self, method: &str, params: Value, context: &str) -> Result<Value> {
        if self.conn.is_none() {
            bail!("{context}: not connected");
        }
        let id = self
            .conn
            .as_ref()
            .expect("checked")
            .request(method, params)?;
        let (tx, rx) = channel();
        self.pending
            .lock()
            .expect("pending")
            .insert(id.clone(), tx);
        let deadline = Instant::now() + REQUEST_TIMEOUT;
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
                        Err((code, message)) => {
                            Err(anyhow!("{context}: {message} ({code})"))
                        }
                    };
                }
                Ok(_) => {} // routed elsewhere
                Err(_) => {}
            }
            if self.conn.is_none() {
                bail!("{context}: connection gone");
            }
            if Instant::now() > deadline {
                self.pending.lock().expect("pending").remove(&id);
                bail!("{context}: request timed out");
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn list_sessions(&mut self) {
        let res = match self.call("session/list", json!({}), "session/list") {
            Ok(v) => v,
            Err(e) => {
                self.error("session/list", format!("{e:#}"));
                return;
            }
        };
        let items: Vec<SessionListItem> = res
            .get("sessions")
            .cloned()
            .or_else(|| Some(res.clone()))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let infos: Vec<AcpSessionInfo> = items
            .into_iter()
            .map(|s| AcpSessionInfo {
                session_id: s.id,
                name: s.title.unwrap_or_default(),
                cwd: s.cwd.unwrap_or_default(),
                start_time: value_to_string(s.updated_at),
            })
            .collect();
        self.emit(AcpEvent::SessionsList(infos));
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
        match self.call("session/resume", params, "session/resume") {
            Ok(res) => self.apply_state_response(res, "session/resume"),
            Err(e) => self.error("session/resume", format!("{e:#}")),
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

    fn enqueue(&mut self, session_id: &str, text: &str) {
        let params = json!({
            "sessionId": session_id,
            "entries": [
                {"role": "user",
                 "content": [{"type": "text", "text": text}]}
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
            self.emit(AcpEvent::DebugLog(
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
    fn set_config_option(&mut self, session_id: &str, key: &str, value: &str) {
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
            self.error(method, format!("{e:#}"));
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
