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
        if remote_on && remote_port == port {
            eprintln!(
                "mistraldeck: remote port equals the local port ({port}) - pick a different port in Settings → Zdalne."
            );
        }
        let start_remote = remote_on && remote_port != port;

        if gui {
            #[cfg(feature = "gui")]
            {
                let url = format!("http://127.0.0.1:{port}");
                std::thread::Builder::new()
                    .name("mistraldeck-gui".into())
                    .spawn(move || {
                        // Wait until the HTTP server is actually listening.
                        for _ in 0..150 {
                            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
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
                                    "mistraldeck: GUI unavailable ({e}); the server keeps running at {url}"
                                );
                            }
                        }
                    })
                    .ok();
            }
            #[cfg(not(feature = "gui"))]
            eprintln!(
                "mistraldeck: GUI requested, but this build has no gui support (build with --features server,gui); staying headless."
            );
        }

        // Manual serving (instead of dioxus::serve): two listeners — the
        // local one on 127.0.0.1 without auth, the remote HTTPS one from
        // serve.rs. Both share the same axum router + engine.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("mistraldeck: failed to start the async runtime");
        rt.block_on(async move {
            let cfg = mistraldeck_engine::EngineConfig::load();
            let engine = mistraldeck_engine::Engine::init(cfg);
            // Connect the agent + index CLI sessions before the first
            // snapshot request lands (Prime like Nexus does).
            engine.prime();
            let router = dioxus::server::router(crate::app::App);
            // PWA static routes (sw.js / manifest / icon) served from the
            // binary itself, and the v0.13.4 gzip layer over both listeners.
            let router = crate::pwa::install_pwa_routes(router);
            // Local listener: localhost-only, no login.
            let local = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .unwrap_or_else(|e| panic!("mistraldeck: cannot bind 127.0.0.1:{port}: {e}"));
            println!("mistraldeck: local UI at http://127.0.0.1:{port}");
            // systemd: Type=notify gets READY now that the UI listener is
            // up; WatchdogSec keeps being fed in the background (v1.0.30/31
            // parity — no cgroup self-heal needed, this process stays in
            // the unit cgroup).
            mistraldeck_engine::systemd::notify_state("READY=1");
            mistraldeck_engine::systemd::start_watchdog();
            let local_router = router
                .clone()
                .layer(axum::middleware::from_fn(crate::pwa::no_store_html_mw))
                .layer(axum::middleware::from_fn(crate::pwa::gzip_mw));
            tokio::spawn(async move {
                if let Err(e) = axum::serve(local, local_router).await {
                    eprintln!("mistraldeck: local server error: {e}");
                }
            });
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
