//! Regression tests for the "new session in a project" round:
//! 1. a fresh project session gets the model/mode/thinking chip lists
//!    immediately (no agent process needed) — reported "can't pick
//!    model/mode/thinking in a new session";
//! 2. the first prompt in such a session runs cleanly: the wire must not
//!    carry a `config/model/write` call that the server rejects
//!    ("WARNING config/model/write: Invalid request parameters"), and no
//!    WARNING system message may land in the chat.

use std::io::Write;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(12);

fn tmp_root() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vd-engine-new-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

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

fn boot() -> (std::path::PathBuf, &'static mistraldeck_engine::Engine) {
    let root = tmp_root();
    let shim = stub_shim(&root);
    let trace = root.join("stub-trace.log");
    let _ = std::fs::remove_file(&trace);
    std::env::set_var("VD_STUB_TRACE", &trace);
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
    (root, engine)
}

#[test]
fn new_project_session_has_chips_and_clean_first_turn() {
    let (root, engine) = boot();
    let mut rx = engine.subscribe();

    engine.create_local_session_in(&root.display().to_string());

    // 1. chips exist right after creation (no agent process yet)
    let snap = engine.snapshot();
    let ids: Vec<&str> = snap.config_options.iter().map(|o| o.id.as_str()).collect();
    assert!(
        ids.contains(&"model") && ids.contains(&"mode") && ids.contains(&"thinking"),
        "fresh session must offer model/mode/thinking, got {ids:?}"
    );

    // 2. first prompt: turn completes, no config write on the wire, no WARNING
    engine.send_message_with("test".to_string(), Vec::new());

    let deadline = Instant::now() + TIMEOUT;
    loop {
        while let Ok(ev) = rx.try_recv() {
            let _ = ev;
        }
        let snap = engine.snapshot();
        let assistant: String = snap
            .current_messages
            .iter()
            .filter(|m| m.sender == mistraldeck_core::types::MessageSender::Ai)
            .map(|m| m.content.clone())
            .collect();
        let warnings: Vec<&str> = snap
            .current_messages
            .iter()
            .filter(|m| m.sender == mistraldeck_core::types::MessageSender::System)
            .map(|m| m.content.as_str())
            .filter(|c| c.contains("WARNING"))
            .collect();
        if !assistant.is_empty() && !snap.busy {
            let stub = std::fs::read_to_string(root.join("stub-trace.log")).unwrap_or_default();
            assert!(
                !stub.contains("config/model/write"),
                "first turn of a new session must not write config/model/write:\n{stub}"
            );
            assert!(
                warnings.is_empty(),
                "no WARNING system message expected in chat, got {warnings:?}"
            );
            return;
        }
        if Instant::now() > deadline {
            let stub = std::fs::read_to_string(root.join("stub-trace.log")).unwrap_or_default();
            panic!("no assistant reply within {TIMEOUT:?}\n--- stub trace ---\n{stub}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
