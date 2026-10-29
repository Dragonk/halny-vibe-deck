//! Server bootstrap (CLI flags, engine, axum router, listeners).
//! Compiled only with the `server` feature; the headless appserver
//! reuses this through `main_dispatch` with `force_headless = true`.

pub fn main_dispatch(force_headless: bool) {
        // ---- CLI configuration (headless / remote-control mode) ----
        let args: Vec<String> = std::env::args().collect();
        if let Some(pos) = args.iter().position(|a| a == "--set-login") {
            let user = args.get(pos + 1).cloned().unwrap_or_default();
            let pass = args.get(pos + 2).cloned().unwrap_or_default();
            if user.is_empty() || pass.is_empty() {
                eprintln!("Usage: mistraldeck --set-login USER PASSWORD");
                std::process::exit(2);
            }
            mistraldeck_engine::EngineConfig::set_login(&user, &pass);
            println!("Login credentials saved (hashed) to the Halny Vibe Deck config file.");
            std::process::exit(0);
        }

        // ---- Launch mode: --mode auto|gui|headless (default: auto) ----
        let mode = args
            .iter()
            .position(|a| a == "--mode")
            .and_then(|p| args.get(p + 1))
            .cloned()
            .unwrap_or_else(|| "auto".to_string());
        let mut gui = match mode.as_str() {
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
                    "mistraldeck: unknown --mode '{other}' (use auto|gui|headless); staying headless"
                );
                false
            }
        };

        // Headless-only build (mistraldeck-appserver): never open the webview.
        if gui && force_headless {
            println!("mistraldeck: headless build (mistraldeck-appserver) — the GUI window is disabled, serving headless.");
            gui = false;
        }

        let cfg = mistraldeck_engine::EngineConfig::load();
        let port = args
            .iter()
            .position(|a| a == "--port")
            .and_then(|p| args.get(p + 1))
            .and_then(|v| v.parse::<u16>().ok())
            .or_else(|| std::env::var("PORT").ok().and_then(|v| v.parse::<u16>().ok()))
            .unwrap_or(8080);
        // Serving model (Nexus parity):
        // - the local app server ALWAYS binds 127.0.0.1 and never asks for
        //   a login — starting Halny Vibe Deck locally goes straight into the UI;
        // - remote browser access is a SEPARATE HTTPS listener (serve.rs)
        //   with a login page, active only when remote access is enabled
        //   AND credentials are configured.
        let remote_on = cfg.remote_enabled && cfg.auth.is_some();
        if cfg.remote_enabled && !remote_on {
            eprintln!(
                "mistraldeck: remote access enabled but no credentials configured - set login & password locally (Settings → Zdalne, or --set-login). Remote listener disabled."
            );
        }
        let remote_port = cfg.remote_port;
        // When the configured remote port equals the app port, the app port
        // itself becomes the 0.0.0.0 + login surface (see below) — no
        // separate listener needed, nothing is silently disabled anymore.
        let start_remote = remote_on && remote_port != port;
        if remote_on {
            let ips = crate::serve::local_lan_ips().join(", ");
            let main_desc = if remote_port == port {
                format!("https://0.0.0.0:{port}")
            } else {
                format!("https://0.0.0.0:{remote_port}")
            };
            println!(
                "mistraldeck: remote access on {main_desc} (device: {ips}) — HTTPS + login required"
            );
        }

        // Manual serving (instead of dioxus::serve). Every app surface is
        // HTTPS-only (stable parity — plain HTTP is never served on the
        // app port):
        // - the main port: HTTPS on 127.0.0.1, no login (loopback is
        //   trusted; a browser accepts the self-signed certificate once);
        // - the remote listener (serve.rs): HTTPS on 0.0.0.0 with the
        //   login page, reachable via the device's IP;
        // - the desktop webview gets its OWN private loopback listener on
        //   an ephemeral port, because webview engines reject self-signed
        //   certificates. That listener never leaves 127.0.0.1 and is not
        //   the app's server surface.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("mistraldeck: failed to start the async runtime");
        rt.block_on(async move {
            crate::serve::set_local_port(port);
            let cfg_local = mistraldeck_engine::EngineConfig::load();
            let engine = mistraldeck_engine::Engine::init(cfg_local.clone());
            // Index the session history from disk. No agent process and no
            // handshake at boot (Z5): the agent is contacted on the first
            // prompt or when a session is resumed.
            engine.prime();
            let router = dioxus::server::router(crate::app::App);
            // PWA static routes (sw.js / manifest / icon) served from the
            // binary itself, and the v0.13.4 gzip layer over both listeners.
            let router = crate::pwa::install_pwa_routes(router);
            // Main app listener: HTTPS-only.
            // - remote access OFF: loopback only (127.0.0.1), no login — the
            //   local app surface;
            // - remote access ON: 0.0.0.0 with the login page, exactly like
            //   the stable edition's serve listener — reachable via the
            //   device's IP and every request (incl. localhost) requires
            //   authentication.
            {
                let local_router = router
                    .clone()
                    .layer(axum::middleware::from_fn(crate::pwa::no_store_html_mw));
                let (host, require_login) = if remote_on {
                    ("0.0.0.0", true)
                } else {
                    ("127.0.0.1", false)
                };
                tokio::spawn(async move {
                    if let Err(e) =
                        crate::serve::run_local_https(local_router, host, port, require_login).await
                    {
                        eprintln!("mistraldeck: local HTTPS server error: {e}");
                    }
                });
            }
            // Desktop webview: private loopback HTTP listener on an
            // ephemeral port (webviews cannot accept the self-signed cert).
            #[cfg(feature = "gui")]
            if gui {
                let wv = tokio::net::TcpListener::bind(("127.0.0.1", 0u16))
                    .await
                    .unwrap_or_else(|e| panic!("mistraldeck: cannot bind webview listener: {e}"));
                let wv_port = wv
                    .local_addr()
                    .map(|a| a.port())
                    .unwrap_or(port);
                println!("mistraldeck: desktop webview UI at http://127.0.0.1:{wv_port} (loopback only, webview transport)");
                let wv_router = router
                    .clone()
                    .layer(axum::middleware::from_fn(crate::pwa::no_store_html_mw))
                    .layer(axum::middleware::from_fn(crate::pwa::gzip_mw));
                tokio::spawn(async move {
                    if let Err(e) = axum::serve(wv, wv_router).await {
                        eprintln!("mistraldeck: webview server error: {e}");
                    }
                });
                // `desktop=1`: the webview keeps the frameless title bar, a browser never does.
                let url = format!("http://127.0.0.1:{wv_port}/?desktop=1");
                std::thread::Builder::new()
                    .name("mistraldeck-gui".into())
                    .spawn(move || {
                        // Wait until the webview server is actually listening.
                        for _ in 0..150 {
                            if std::net::TcpStream::connect(("127.0.0.1", wv_port)).is_ok() {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                        match crate::webview::open_webview(&url) {
                            Ok(()) => {
                                // Window closed -> quit the whole app
                                // (embedded server included). Otherwise the
                                // process would keep running invisibly.
                                std::process::exit(0);
                            }
                            Err(e) => {
                                eprintln!(
                                    "mistraldeck: GUI unavailable ({e}); the server keeps running at https://127.0.0.1:{port}"
                                );
                            }
                        }
                    })
                    .ok();
            }
            #[cfg(not(feature = "gui"))]
            if gui {
                eprintln!(
                    "mistraldeck: GUI requested, but this build has no gui support (build with --features server,gui); staying headless."
                );
            }
            // systemd: Type=notify gets READY now that the UI listener is
            // up; WatchdogSec keeps being fed in the background (v1.0.30/31
            // parity — no cgroup self-heal needed, this process stays in
            // the unit cgroup).
            mistraldeck_engine::systemd::notify_state("READY=1");
            mistraldeck_engine::systemd::start_watchdog();
            if start_remote {
                let remote_router = router.clone().layer(axum::middleware::from_fn(crate::pwa::no_store_html_mw));
                tokio::spawn(async move {
                    if let Err(e) = crate::serve::run_remote(remote_router, remote_port).await {
                        eprintln!("mistraldeck: remote HTTPS server error: {e}");
                    }
                });
            }
            std::future::pending::<()>().await
        });
}
