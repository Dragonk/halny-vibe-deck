//! VibeDeck web UI entry. One codebase, three deployment modes:
//! - server: axum serves the app + engine (headless / remote access)
//! - web: the same client compiled to wasm (served by the server)
//! - desktop: the client inside a webview window (points at a local server)

#![allow(non_snake_case)]

pub mod api;
pub mod app;
pub mod components;
pub mod events;
pub mod state;

fn main() {
    #[cfg(feature = "server")]
    {
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
