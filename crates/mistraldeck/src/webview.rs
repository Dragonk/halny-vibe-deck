/// Open the desktop webview window pointed at the running server.
/// Only compiled with the `gui` feature (wry + tao, same webkit2gtk
/// version dioxus-desktop 0.7 uses). Blocking: runs the event loop until
/// the window is closed; the HTTP server keeps running either way.
#[cfg(feature = "gui")]
pub fn open_webview(url: &str) -> Result<(), String> {
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
            .with_title("Mistral Deck")
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