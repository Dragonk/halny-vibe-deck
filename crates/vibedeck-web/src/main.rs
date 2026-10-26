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
            .unwrap_or(8080);
        // Serving model (Nexus parity):
        // - the local app server ALWAYS binds 127.0.0.1 and never asks for
        //   a login — starting VibeDeck locally goes straight into the UI;
        // - remote browser access is a SEPARATE HTTPS listener (serve.rs)
        //   with a login page, active only when remote access is enabled
        //   AND credentials are configured.
        let remote_on = cfg.remote_enabled && cfg.auth.is_some();
        if cfg.remote_enabled && !remote_on {
            eprintln!(
                "vibedeck: remote access enabled but no credentials configured - set login & password locally (Settings → Zdalne, or --set-login). Remote listener disabled."
            );
        }
        let remote_port = cfg.remote_port;
        if remote_on && remote_port == port {
            eprintln!(
                "vibedeck: remote port equals the local port ({port}) - pick a different port in Settings → Zdalne."
            );
        }
        let start_remote = remote_on && remote_port != port;

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

        // Manual serving (instead of dioxus::serve): two listeners — the
        // local one on 127.0.0.1 without auth, the remote HTTPS one from
        // serve.rs. Both share the same axum router + engine.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("vibedeck: failed to start the async runtime");
        rt.block_on(async move {
            let cfg = vibedeck_engine::EngineConfig::load();
            let engine = vibedeck_engine::Engine::init(cfg);
            // Connect the agent + index CLI sessions before the first
            // snapshot request lands (Prime like Nexus does).
            engine.prime();
            let router = dioxus::server::router(app::App);
            // PWA static routes (sw.js / manifest / icon) served from the
            // binary itself, and the v0.13.4 gzip layer over both listeners.
            let router = pwa::install_pwa_routes(router);
            // Local listener: localhost-only, no login.
            let local = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .unwrap_or_else(|e| panic!("vibedeck: cannot bind 127.0.0.1:{port}: {e}"));
            println!("vibedeck: local UI at http://127.0.0.1:{port}");
            let local_router = router
                .clone()
                .layer(axum::middleware::from_fn(pwa::gzip_mw));
            tokio::spawn(async move {
                if let Err(e) = axum::serve(local, local_router).await {
                    eprintln!("vibedeck: local server error: {e}");
                }
            });
            if start_remote {
                let remote_router = router.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve::run_remote(remote_router, remote_port).await {
                        eprintln!("vibedeck: remote HTTPS server error: {e}");
                    }
                });
            }
            std::future::pending::<()>().await
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
    use std::rc::Rc;
    use tao::{
        event::{Event, WindowEvent},
        event_loop::ControlFlow,
        window::{ResizeDirection, WindowBuilder},
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

    // Nexus-style frameless window: no OS title bar, the app header is
    // the drag region (see BOOT_JS / EDGE_RESIZE_JS below) and the window
    // buttons live in the header (IPC vd-min / vd-max / vd-close).
    let window = Rc::new(
        WindowBuilder::new()
            .with_title("VibeDeck")
            .with_decorations(false)
            .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 820.0))
            .build(&event_loop)
            .map_err(|e| e.to_string())?,
    );
    // Undecorated windows on Windows lose the drop shadow without this.
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        window.set_undecorated_shadow(true);
    }

    fn resize_dir(raw: &str) -> Option<ResizeDirection> {
        Some(match raw {
            "n" | "north" => ResizeDirection::North,
            "s" | "south" => ResizeDirection::South,
            "e" | "east" => ResizeDirection::East,
            "w" | "west" => ResizeDirection::West,
            "ne" | "northeast" => ResizeDirection::NorthEast,
            "nw" | "northwest" => ResizeDirection::NorthWest,
            "se" | "southeast" => ResizeDirection::SouthEast,
            "sw" | "southwest" => ResizeDirection::SouthWest,
            _ => return None,
        })
    }

    let builder = WebViewBuilder::new()
        .with_url(url.to_string())
        .with_initialization_script(BOOT_JS)
        .with_ipc_handler({
            let window = window.clone();
            move |req| {
                match req.body().as_str() {
                    "vd-drag" => {
                        let _ = window.drag_window();
                    }
                    "vd-min" => window.set_minimized(true),
                    "vd-max" => {
                        let maximized = window.is_maximized();
                        window.set_maximized(!maximized);
                    }
                    "vd-close" => {
                        // Window closed -> quit the whole app (embedded
                        // server included), same as the OS close button.
                        std::process::exit(0);
                    }
                    other => {
                        if let Some(dir) = other
                            .strip_prefix("vd-resize:")
                            .and_then(resize_dir)
                        {
                            let _ = window.drag_resize_window(dir);
                        }
                    }
                }
            }
        });
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

/// Injected into the desktop webview before the app boots:
/// - marks the page as desktop (`window.__vdDesktop`) so the UI can show
///   the frameless window buttons,
/// - makes the header the drag region (click/drag) with double-click to
///   maximize,
/// - starts OS edge resizing for the frameless window (vd-resize:<dir>).
#[cfg(feature = "gui")]
const BOOT_JS: &str = r#"
(function () {
  window.__vdDesktop = 1;
  function post(m) { try { window.ipc.postMessage(m); } catch (e) {} }
  function ready(fn) {
    if (document.readyState !== 'loading') fn();
    else document.addEventListener('DOMContentLoaded', fn);
  }
  ready(function () {
    var NO_DRAG = '.header-controls, button, input, select, textarea, a, .activity-tooltip, .window-btn';
    // Header = title bar drag region; double-click = maximize toggle.
    document.addEventListener('mousedown', function (e) {
      if (e.button !== 0) return;
      if (e.target.closest(NO_DRAG)) return;
      if (e.target.closest('.header')) post('vd-drag');
    });
    document.addEventListener('dblclick', function (e) {
      if (e.target.closest(NO_DRAG)) return;
      if (e.target.closest('.header')) post('vd-max');
    });
    // Edge resize for the frameless window.
    var EDGE = 6;
    function dirAt(e) {
      var w = window.innerWidth, h = window.innerHeight;
      var x = e.clientX, y = e.clientY;
      var left = x <= EDGE, right = x >= w - EDGE;
      var top = y <= EDGE, bottom = y >= h - EDGE;
      if (top && left) return 'nw';
      if (top && right) return 'ne';
      if (bottom && left) return 'sw';
      if (bottom && right) return 'se';
      if (left) return 'w';
      if (right) return 'e';
      if (top) return 'n';
      if (bottom) return 's';
      return null;
    }
    var armDir = null;
    var cursors = { n: 'ns-resize', s: 'ns-resize', w: 'ew-resize', e: 'ew-resize',
                    nw: 'nwse-resize', se: 'nwse-resize', ne: 'nesw-resize', sw: 'nesw-resize' };
    document.addEventListener('mousemove', function (e) {
      var over = e.target.closest ? e.target.closest('.app-root') : true;
      armDir = (over || !e.target.closest) ? dirAt(e) : null;
      document.body.style.cursor = armDir ? cursors[armDir] : '';
    });
    document.addEventListener('mousedown', function (e) {
      if (e.button !== 0) return;
      if (armDir) post('vd-resize:' + armDir);
    });
  });
})();
"#;