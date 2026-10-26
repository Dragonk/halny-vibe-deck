//! Live end-to-end test against the real `vibe-app-server` binary.
//! Skipped by default (needs the CLI installed); run locally with:
//!   cargo test -p mistraldeck-appsrv --test e2e -- --ignored --nocapture
//! Covers F0–F3 of docs/migration-appserver.md: initialize →
//! session/start → turn/enqueue → notifications → turn/completed →
//! session/list → session/resume in a FRESH process (full snapshot).

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use mistraldeck_appsrv::conn::{Conn, Inbound};
use mistraldeck_appsrv::proto::StateResponse;
use serde_json::{json, Value};

fn find_binary() -> Option<String> {
    // the CLI installs the app-server next to the vibe entry point
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let p = std::path::Path::new(dir).join("vibe-app-server");
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

/// Minimal synchronous client: one pending reply at a time, server
/// requests (callback/call) acked inline, notifications collected.
struct TestClient {
    conn: Conn,
    inbox: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<Inbound>>>,
    pending: HashMap<String, mpsc::Sender<Inbound>>,
    next_id: u64,
    pub events: Vec<(String, Value)>,
}

impl TestClient {
    fn spawn(bin: &str) -> anyhow::Result<Self> {
        let conn = Conn::spawn(&[bin.to_string()])?;
        let inbox = conn.inbox.clone();
        Ok(Self {
            conn,
            inbox,
            pending: HashMap::new(),
            next_id: 1,
            events: Vec::new(),
        })
    }

    /// Pump the inbox for up to `timeout`; returns true when the
    /// pending reply for the CURRENT request arrived.
    fn pump(&mut self, id: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let msg = self.inbox.lock().unwrap().try_recv();
            match msg {
                Ok(Inbound::Response { id: rid, result }) => {
                    if rid == id {
                        if let Some(tx) = self.pending.remove(&rid) {
                            let _ = tx.send(Inbound::Response {
                                id: rid,
                                result,
                            });
                        }
                        return true;
                    }
                }
                Ok(Inbound::ServerRequest { id, method, params }) => {
                    if method == "callback/call" {
                        self.conn.ack_callback(id, &params, true);
                    } else {
                        let _ = self.conn.respond(id, json!({}));
                    }
                }
                Ok(Inbound::Notification { method, params }) => {
                    self.events.push((method, params));
                }
                Err(_) => {
                    if Instant::now() > deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = format!("client-{}", self.next_id);
        self.next_id += 1;
        let (tx, rx) = mpsc::channel();
        self.pending.insert(id.clone(), tx);
        self.conn.request(method, params).map_err(|e| e.to_string())?;
        if !self.pump(&id, Duration::from_secs(120)) {
            self.pending.remove(&id);
            return Err("timeout".into());
        }
        match rx.recv() {
            Ok(Inbound::Response { result, .. }) => match result {
                Ok(v) => Ok(v),
                Err((c, m)) => Err(format!("{m} ({c})")),
            },
            _ => Err("reply channel closed".into()),
        }
    }

    fn wait_notification(&mut self, method: &str, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            for (m, p) in &self.events {
                if m == method {
                    return Some(p.clone());
                }
            }
            if Instant::now() > deadline {
                return None;
            }
            self.pump("__none__", Duration::from_millis(200));
        }
    }
}

#[test]
#[ignore]
fn live_full_cycle() {
    let Some(bin) = find_binary() else {
        panic!("vibe-app-server not found in PATH");
    };
    let mut c = TestClient::spawn(&bin).expect("spawn vibe-app-server");

    // 1) initialize
    let res = c
        .call(
            "initialize",
            json!({
                "clientInfo": {"name": "appsrv-e2e", "version": "0.1",
                                "entrypoint": "unknown", "terminalEmulator": "unknown"},
                "capabilities": {"callbackKinds": ["approval", "user_input"],
                                 "clientTools": [], "sessionSpawn": false}
            }),
        )
        .expect("initialize");
    assert_eq!(res["serverInfo"]["name"].as_str(), Some("vibe-app-server"));
    // LSP-style completion: the server only enters INITIALIZED after this
    // notification (server.py _handle_notification — undocumented in the
    // protocol ref, found in the installed 3.0.2 sources)
    c.conn.notify("initialized", json!({})).unwrap();
    println!("server: {}", res["serverInfo"]);

    // 2) session/start in an isolated temp dir
    let cwd = std::env::temp_dir().join(format!("appsrv-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    let res = c
        .call(
            "session/start",
            json!({"agentConfig": {"cwd": cwd}, "kind": "normal"}),
        )
        .expect("session/start");
    let r: StateResponse = serde_json::from_value(res).unwrap();
    let state = r.state.expect("state in session/start");
    let session_id = state.session.expect("session").id;
    assert!(!session_id.is_empty());
    let last_event_id = r.last_event_id.unwrap_or(0);
    println!("session: {session_id} (lastEventId={last_event_id})");

    // 3) turn/enqueue → notifications until turn/completed
    c.call(
        "session/turn/enqueue",
        json!({
            "sessionId": session_id,
            "entries": [{"role": "user",
                         "content": [{"type": "text", "text": "Say exactly: ok"}]}]
        }),
    )
    .expect("turn/enqueue");

    let done = c.wait_notification("turn/completed", Duration::from_secs(180));
    assert!(done.is_some(), "turn did not complete in time");

    let mut assistant_text = String::new();
    let mut saw_user = false;
    for (m, p) in &c.events {
        match m.as_str() {
            "history/entryAdded" => {
                let entry = &p["entry"];
                // wire 3.0.x: type "message" + role (user/assistant/system)
                if entry["type"] == "message" && entry["role"] == "user" {
                    saw_user = true;
                }
                if entry["type"] == "message" && entry["role"] == "assistant" {
                    assistant_text.push_str(entry["content"][0]["text"].as_str().unwrap_or(""));
                }
            }
            "history/entryUpdated" => {
                for op in p["patch"].as_array().unwrap_or(&vec![]) {
                    if op["op"] == "append" {
                        assistant_text.push_str(op["value"].as_str().unwrap_or(""));
                    }
                }
            }
            _ => {}
        }
    }
    assert!(
        saw_user,
        "no user entry in history; events: {:?}",
        c.events.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>()
    );
    assert!(
        assistant_text.to_lowercase().contains("ok"),
        "assistant did not answer: {assistant_text:?}"
    );
    println!("assistant: {assistant_text:?}");

    // 4) session/list shows the session
    let res = c.call("session/list", json!({})).expect("session/list");
    assert!(
        res.to_string().contains(&session_id),
        "session missing from list"
    );

    // 5) reconnect: a FRESH process, session/resume returns the full
    //    snapshot (F1/F3 — no replay buffering)
    drop(c);
    std::thread::sleep(Duration::from_millis(300));
    let mut c2 = TestClient::spawn(&bin).expect("re-spawn");
    c2.call(
        "initialize",
        json!({
            "clientInfo": {"name": "appsrv-e2e", "version": "0.1"},
            "capabilities": {"callbackKinds": ["approval", "user_input"]}
        }),
    )
    .expect("initialize #2");
    c2.conn.notify("initialized", json!({})).unwrap();
    let res = c2
        .call("session/resume", json!({"sessionId": session_id}))
        .expect("session/resume");
    let r: StateResponse = serde_json::from_value(res).unwrap();
    let state = r.state.expect("state in resume");
    let history_len = state.history.as_ref().map(|h| h.len()).unwrap_or(0);
    assert!(history_len >= 2, "resumed history empty: {history_len}");
    assert!(
        state.event_id.unwrap_or(0) >= last_event_id,
        "event id went backwards"
    );
    println!("resumed: {history_len} history entries");
}
