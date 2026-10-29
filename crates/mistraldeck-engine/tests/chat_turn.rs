//! Regression test for the "Agent is working… forever" bug (issue Z29).
//!
//! Drives the real engine against the scripted `vibe-app-server` stand-in
//! (`scripts/e2e/stub-agent.py`) in an isolated config dir, exactly like the
//! UI does: create a local session, send a prompt, expect the agent to be
//! handed `session/start` + `session/turn/enqueue` and stream a reply.
//!
//! The stub trace is written to `<tmp>/stub-trace.log` for diagnosis.

use std::io::Write;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(12);

fn tmp_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vd-engine-chat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// `agent_path` is spawned as argv[0], so the Python stub needs an
/// executable shim (the repo keeps it non-executable for git hygiene).
fn stub_shim(root: &std::path::Path) -> std::path::PathBuf {
    let stub = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/e2e/stub-agent.py");
    let shim = root.join("stub-agent.sh");
    let mut f = std::fs::File::create(&shim).expect("shim");
    writeln!(f, "#!/bin/sh").unwrap();
    writeln!(f, "exec python3 {:?} \"$@\"", stub.display().to_string()).unwrap();
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    shim
}

#[test]
fn local_session_turn_roundtrip() {
    let root = tmp_root();
    let shim = stub_shim(&root);
    let trace = root.join("stub-trace.log");
    let _ = std::fs::remove_file(&trace);
    std::env::set_var("VD_STUB_TRACE", &trace);
    // Isolated config: never touch the user's live ~/.config/halny-vibe-deck.
    std::env::set_var("XDG_CONFIG_HOME", &root);

    let cfg_dir = root.join("halny-vibe-deck");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
    let cfg = serde_json::json!({
        "agent_path": shim.display().to_string(),
        "working_directory": root.display().to_string(),
    });
    std::fs::write(
        cfg_dir.join("config.json"),
        serde_json::to_string_pretty(&cfg).unwrap(),
    )
    .expect("write config");

    let engine = mistraldeck_engine::Engine::init(mistraldeck_engine::EngineConfig::load());
    let mut rx = engine.subscribe();

    engine.create_local_session();
    engine.send_message_with("hello there".to_string(), Vec::new());

    let deadline = Instant::now() + TIMEOUT;
    let mut trace_lines: Vec<String> = Vec::new();
    loop {
        // Drain the bus so nothing backs up.
        while let Ok(ev) = rx.try_recv() {
            trace_lines.push(format!("{ev:?}"));
        }
        let snap = engine.snapshot();
        if snap.current.is_some() {
            let assistant: String = snap
                .current_messages
                .iter()
                .filter(|m| m.sender == mistraldeck_core::types::MessageSender::Ai)
                .map(|m| m.content.clone())
                .collect();
            if !assistant.is_empty() && !snap.busy {
                return; // the turn really ran
            }
        }
        if Instant::now() > deadline {
            let s = engine.snapshot();
            let cur = s
                .sessions
                .first()
                .map(|x| format!("{} busy={} agent={:?}", x.name, x.busy, x.agent_session_id))
                .unwrap_or_else(|| "<no session>".into());
            let stub = std::fs::read_to_string(&trace).unwrap_or_default();
            panic!(
                "no assistant reply within {TIMEOUT:?}\n\
                 session: {cur}\n\
                 --- stub trace ---\n{stub}\n\
                 --- bus events (tail) ---\n{}",
                trace_lines
                    .iter()
                    .rev()
                    .take(40)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
