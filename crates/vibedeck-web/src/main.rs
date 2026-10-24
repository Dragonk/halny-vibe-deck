//! VibeDeck web UI entry. One codebase, three deployment modes:
//! - server: axum serves the app + engine (headless / remote access)
//! - web: the same client compiled to wasm (served by the server)
//! - desktop: the client inside a webview window (points at a local server)

#![allow(non_snake_case)]

pub mod api;
pub mod app;
pub mod components;
pub mod events;
pub mod fsgit;
pub mod vibestats;
pub mod md;
pub mod i18n;
pub mod state;

fn main() {
    #[cfg(feature = "server")]
    {
        // ---- CLI configuration (headless / remote-control mode) ----
        let args: Vec<String> = std::env::args().collect();
        if let Some(pos) = args.iter().position(|a| a == "--set-login") {
            let user = args.get(pos + 1).cloned().unwrap_or_default();
            let pass = args.get(pos + 2).cloned().unwrap_or_default();
            if user.is_empty() || pass.is_empty() {
                eprintln!("Usage: vibedeck --set-login USER PASSWORD");
                std::process::exit(2);
            }
            vibedeck_engine::EngineConfig::set_login(&user, &pass);
            println!("Login credentials saved (hashed) to the VibeDeck config file.");
            std::process::exit(0);
        }

        let cfg = vibedeck_engine::EngineConfig::load();
        let port = args
            .iter()
            .position(|a| a == "--port")
            .and_then(|p| args.get(p + 1))
            .and_then(|v| v.parse::<u16>().ok())
            .or_else(|| std::env::var("PORT").ok().and_then(|v| v.parse::<u16>().ok()))
            .unwrap_or(if cfg.remote_port > 0 { cfg.remote_port } else { 8080 });
        // Remote control (or configured credentials) => bind on all
        // interfaces so the app is reachable from a browser; otherwise
        // stay on localhost only.
        let remote = cfg.remote_enabled || cfg.auth.is_some();
        if std::env::var_os("PORT").is_none() {
            std::env::set_var("PORT", port.to_string());
        }
        if std::env::var_os("IP").is_none() {
            std::env::set_var("IP", if remote { "0.0.0.0" } else { "127.0.0.1" });
        }

        dioxus::serve(|| async {
            let cfg = vibedeck_engine::EngineConfig::load();
            let engine = vibedeck_engine::Engine::init(cfg);
            if !engine.config().acp_path.is_empty() {
                engine.start_agent();
            }
            engine.refresh_sessions();
            Ok(dioxus::server::router(app::App))
        });
    }

    #[cfg(all(not(feature = "server"), feature = "desktop"))]
    {
        // Desktop webview: talk to the local engine server.
        let port = std::env::var("VIBEDECK_PORT").unwrap_or_else(|_| "8080".to_string());
        dioxus::fullstack::set_server_url(format!("http://127.0.0.1:{port}"));
        dioxus::launch(app::App);
    }

    #[cfg(all(not(feature = "server"), feature = "web"))]
    dioxus::launch(app::App);
}
