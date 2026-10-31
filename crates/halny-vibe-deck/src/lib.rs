//! Halny Vibe Deck — one codebase, three deployment modes:
//! - server: axum serves the app + engine (headless / remote access)
//! - web: the same client compiled to wasm (served by the server)
//! - desktop: the client inside a webview window (points at a local server)
//!
//! The crate is also reused by `halny-vibe-deck-appserver`, a headless-only
//! console build: it calls [`set_force_headless`] before [`run`] so the
//! desktop webview is never spawned.

#![allow(non_snake_case)]

pub mod api;
pub mod app;
pub mod components;
pub mod events;
pub mod fsgit;
#[cfg(feature = "server")]
pub mod launch;
#[cfg(feature = "server")]
pub mod preview;
#[cfg(feature = "server")]
pub mod serve;
#[cfg(feature = "server")]
pub mod pwa;
pub mod highlight;
pub mod vibestats;
pub mod vibeconfig;
pub mod md;
pub mod i18n;
pub mod setup;
pub mod state;
#[cfg(feature = "gui")]
pub mod webview;

use std::sync::atomic::{AtomicBool, Ordering};

/// Set by the headless appserver build (`halny-vibe-deck-appserver`) so
/// `--mode gui`/auto never tries to open the desktop webview.
static FORCE_HEADLESS: AtomicBool = AtomicBool::new(false);

/// Mark this process as headless-only (no desktop webview, ever).
pub fn set_force_headless() {
    FORCE_HEADLESS.store(true, Ordering::SeqCst);
}

/// Entry point shared by the desktop/server binary (`halny-vibe-deck`) and
/// the headless appserver (`halny-vibe-deck-appserver`).
pub fn run() {
    #[cfg(target_arch = "wasm32")]
    {
        // A panic in wasm is a trap: the module instance dies and every
        // later JS→WASM call (all event handlers) throws `unreachable` —
        // the UI renders once and then goes completely dead. Route panics
        // to the console with a symbolized Rust backtrace instead.
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    }
    #[cfg(target_os = "windows")]
    {
        // With windows_subsystem = "windows" there is no console, so a
        // panic anywhere in the process is completely invisible (this is
        // exactly how the GUI thread used to die silently). Write every
        // panic to halny-vibe-deck.log next to the exe so failures can be
        // diagnosed after the fact.
        let log_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("halny-vibe-deck.log")))
            .unwrap_or_else(|| std::path::PathBuf::from("halny-vibe-deck.log"));
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                use std::io::Write;
                let _ = writeln!(f, "panic: {info}");
            }
            prev_hook(info);
        }));
    }

    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        // In the browser (and the desktop webview) a panic is otherwise
        // only a bare "RuntimeError: unreachable" — log the location so
        // bugs are diagnosable from the devtools console.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            dioxus::logger::tracing::error!("panic: {info}");
            prev_hook(info);
        }));
    }

    #[cfg(feature = "server")]
    launch::main_dispatch(FORCE_HEADLESS.load(Ordering::SeqCst));

    #[cfg(all(not(feature = "server"), feature = "desktop"))]
    {
        // Desktop webview: talk to the local engine server.
        let port = std::env::var("HALNY_VIBE_DECK_PORT")
            .or_else(|_| std::env::var("MISTRALDECK_PORT"))
            .unwrap_or_else(|_| "8080".to_string());
        dioxus::fullstack::set_server_url(format!("http://127.0.0.1:{port}"));
        dioxus::launch(app::App);
    }


    #[cfg(all(not(feature = "server"), feature = "web"))]
    dioxus::launch(app::App);
}
