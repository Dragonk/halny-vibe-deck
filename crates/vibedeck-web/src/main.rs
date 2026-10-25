//! VibeDeck web UI entry. One codebase, three deployment modes:
//! - server: axum serves the app + engine (headless / remote access)
//! - web: the same client compiled to wasm (served by the server)
//! - desktop: the client inside a webview window (points at a local server)

#![allow(non_snake_case)]

// Windows: no console window — double-clicking the exe opens the GUI
// directly. Console output is only kept for debug builds.
#![cfg_attr(all(target_os = "windows", feature = "gui", not(debug_assertions)), windows_subsystem = "windows")]

pub mod api;
pub mod app;
pub mod components;
pub mod events;
pub mod fsgit;
#[cfg(feature = "server")]
pub mod preview;
pub mod highlight;
pub mod vibestats;
pub mod vibeconfig;
pub mod md;
pub mod i18n;
pub mod setup;
pub mod state;

fn main() {
    #[cfg(target_os = "windows")]
    {
        // With windows_subsystem = "windows" there is no console, so a
        // panic anywhere in the process is completely invisible (this is
        // exactly how the GUI thread used to die silently). Write every
        // panic to vibedeck.log next to the exe so failures can be
        // diagnosed after the fact.
        let log_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("vibedeck.log")))
            .unwrap_or_else(|| std::path::PathBuf::from("vibedeck.log"));
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

        // ---- Launch mode: --mode auto|gui|headless (default: auto) ----
        let mode = args
            .iter()
            .position(|a| a == "--mode")
            .and_then(|p| args.get(p + 1))
            .cloned()
            .unwrap_or_else(|| "auto".to_string());
        let gui = match mode.as_str() {
            "gui" | "desktop" => true,
            "headless" => false,
            "auto" => {
                // On Windows a desktop session is the norm — default to the
                // GUI window (double-click should just open the app). On
                // Unix, open the GUI only when a display server is available.
                cfg!(target_os = "windows")
                    || std::env::var_os("DISPLAY").is_some()
                    || std::env::var_os("WAYLAND_DISPLAY").is_some()
            }
            other => {
                eprintln!(
                    "vibedeck: unknown --mode '{other}' (use auto|gui|headless); staying headless"
                );
                false
            }
        };

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

        if gui {
            #[cfg(feature = "gui")]
            {
                let url = format!("http://127.0.0.1:{port}");
                std::thread::Builder::new()
                    .name("vibedeck-gui".into())
                    .spawn(move || {
                        // Wait until the HTTP server is actually listening.
                        for _ in 0..150 {
                            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        match open_webview(&url) {
                            Ok(()) => {
                                // Window closed -> quit the whole app
                                // (embedded server included). Otherwise the
                                // process would keep running invisibly.
                                std::process::exit(0);
                            }
                            Err(e) => {
                                eprintln!(
                                    "vibedeck: GUI unavailable ({e}); the server keeps running at {url}"
                                );
                            }
                        }
                    })
                    .ok();
            }
            #[cfg(not(feature = "gui"))]
            eprintln!(
                "vibedeck: GUI requested, but this build has no gui support (build with --features server,gui); staying headless."
            );
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

/// Open the desktop webview window pointed at the running server.
/// Only compiled with the `gui` feature (wry + tao, same webkit2gtk
/// version dioxus-desktop 0.7 uses). Blocking: runs the event loop until
/// the window is closed; the HTTP server keeps running either way.
#[cfg(feature = "gui")]
fn open_webview(url: &str) -> Result<(), String> {
    use tao::{
        event::{Event, WindowEvent},
        event_loop::ControlFlow,
        window::WindowBuilder,
    };
    use wry::WebViewBuilder;

    // tao panics when the event loop is created off the main thread; the
    // gui thread is a worker, so use the any-thread constructors. On
    // Windows the plain EventLoop::new() panicked on the worker thread and
    // silently killed the GUI (no console with windows_subsystem), which
    // is why double-clicking the exe showed no window at all.
    #[cfg(target_os = "linux")]
    let event_loop = {
        use tao::platform::unix::EventLoopBuilderExtUnix;
        tao::event_loop::EventLoopBuilder::new().with_any_thread(true).build()
    };
    #[cfg(target_os = "windows")]
    let event_loop = {
        use tao::platform::windows::EventLoopBuilderExtWindows;
        tao::event_loop::EventLoopBuilder::new().with_any_thread(true).build()
    };
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let event_loop = tao::event_loop::EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("VibeDeck")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 820.0))
        .build(&event_loop)
        .map_err(|e| e.to_string())?;

    let builder = WebViewBuilder::new().with_url(url.to_string());
    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    ))]
    let _webview = builder.build(&window).map_err(|e| e.to_string())?;
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "ios",
        target_os = "android"
    )))]
    let _webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or_else(|| "no gtk vbox in window".to_string())?;
        builder.build_gtk(vbox).map_err(|e| e.to_string())?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}