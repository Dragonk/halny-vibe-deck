//! App shell: header + sidebar + main view + dock, the engine event
//! stream and the agent question / tool-approval overlays (App.tsx).

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{apply_event, lang, refresh, token, ClientState, View};

pub fn Shell() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let answer = use_signal(String::new);

    // one-shot: install the sidebar/dock panel resize logic (client
    // only; the server SSR build never evaluates JS)
    #[cfg(any(feature = "web", feature = "desktop"))]
    {
        let mut resize_init = use_signal(|| false);
        if !*resize_init.read() {
            resize_init.set(true);
            spawn(async move {
                dioxus::document::eval(RESIZE_JS);
                dioxus::document::eval(crate::components::mobile::MOBILE_JS);
                dioxus::document::eval(crate::components::mobile::BACK_JS);
                dioxus::document::eval(VISIBILITY_JS);
                // Round 4 (C2): mermaid renderer for the whole page — chat
                // AND the file-preview dock (md previews) both mount
                // `.mermaid-container` divs.
                dioxus::document::eval(MERMAID_JS);
                // Layout probe: only the desktop webview keeps the header
                // (the SSR pass and the browser both render the web layout,
                // so hydration stays stable).
                if crate::state::is_desktop() {
                    let mut st = state;
                    st.write().desktop_mode = true;
                }
            });
        }
    }

    // The engine event stream.
    let mut socket = crate::events::use_engine_socket();
    // Round 15: coalesce refetches — during a session switch/resume many
    // structural events arrive at once; each used to trigger a FULL
    // snapshot fetch + re-render, freezing the UI. One in flight, at most
    // one queued behind it.
    let mut refetch_running = use_signal(|| false);
    let mut refetch_pending = use_signal(|| false);
    use_future(move || async move {
        loop {
            match socket.recv().await {
                Ok(ev) => {
                    let refetch = apply_event(state, ev);
                    if refetch {
                        if *refetch_running.read() {
                            refetch_pending.set(true);
                        } else {
                            refetch_running.set(true);
                            refresh(state).await;
                            crate::state::drain_queue(state).await;
                            refetch_running.set(false);
                            if *refetch_pending.read() {
                                refetch_pending.set(false);
                                refresh(state).await;
                                crate::state::drain_queue(state).await;
                            }
                        }
                    }
                }
                Err(_) => {
                    // Stream dropped (server restart, token expiry, hidden
                    // tab). Round 4 (E9): ask the agent for the authoritative
                    // session state too, so messages emitted while the stream
                    // was down are backfilled, then refetch + drain the queue.
                    crate::state::sleep_ms(2_000).await;
                    let tok = state.read().token.clone().unwrap_or_default();
                    let _ = crate::api::api_resync_current(tok).await;
                    crate::state::sleep_ms(250).await;
                    refresh(state).await;
                    crate::state::drain_queue(state).await;
                }
            }
        }
    });

    // Element 6: look for app + CLI updates shortly after startup so the
    // banner appears without opening Settings (Nexus: 4s, silent - failures
    // keep the banner hidden).
    use_future(move || async move {
        crate::state::sleep_ms(4_000).await;
        crate::state::check_updates(state).await;
    });

    let s = state.read();
    let lang = lang(&state);
    let view = s.view;
    let question = s.snapshot.question.clone();
    let approval = s.snapshot.approval.clone();
    let desktop_mode = s.desktop_mode;
    drop(s);

    let view_el = match view {
        View::Chat => rsx! { crate::components::chat::ChatView {} },
        View::Stats => rsx! { crate::components::stats::StatsView {} },
        View::History => rsx! { crate::components::history::HistoryView {} },
        View::Settings => rsx! { crate::components::settings::SettingsView {} },
    };
    let question_el = question.map(|q| question_el(state, lang.clone(), q, answer));
    let approval_el = approval.map(|a| approval_el(state, lang.clone(), a));

    rsx! {
        link { rel: "stylesheet", href: asset!("/assets/main.css") }
        div { class: "app-root",
            // Stable v1.0.27 parity: the browser client has no title bar, so
            // the header is not rendered there at all — the sidebar owns the
            // brand (top) and the theme/logout controls (bottom), while two
            // floating buttons toggle the panels. The desktop webview keeps
            // the full header.
            if desktop_mode {
                crate::components::header::Header {}
            } else {
                crate::components::shell::WebFabs {}
            }
            crate::components::shell::UpdateBanner {}
            crate::components::mobile::MobileBridges {}
            div { class: "app-body",
                crate::components::sidebar::Sidebar {}
                main { class: "app-main", {view_el} }
                crate::components::dock::Dock {}
            }
            if let Some(el) = question_el {
                {el}
            }
            if let Some(el) = approval_el {
                {el}
            }
            // v1.0.17: JS half of the visibilitychange re-sync — the
            // script clicks this hidden bridge button when the tab
            // becomes visible again (same pattern as MobileBridges).
            button {
                id: "vd-visibility-resync",
                style: "display:none",
                onclick: {
                    let state = state.clone();
                    move |_| {
                        let state = state.clone();
                        async move {
                            // Ask the agent for the authoritative session
                            // state first: the tab may have missed events
                            // that never reached this server's projection
                            // (the stable-edition "15 minutes missing until
                            // reload" bug), then refetch the snapshot.
                            let token = state.read().token.clone().unwrap_or_default();
                            let _ = crate::api::api_resync_current(token).await;
                            // Let the resume response land on the bus
                            // before the snapshot is read.
                            crate::state::sleep_ms(250).await;
                            refresh(state).await;
                            crate::state::drain_queue(state).await;
                        }
                    }
                },
            }
        }
    }
}

/// Agent question overlay: choice buttons + free-form input.
fn question_el(
    state: Signal<ClientState>,
    _lang: String,
    q: halny_vibe_deck_core::snapshot::PendingQuestion,
    mut answer: Signal<String>,
) -> Element {
    let mut choice_els: Vec<Element> = Vec::new();
    for choice in &q.choices {
        let req = q.request_id.clone();
        let choice = choice.clone();
        let el = rsx! {
            button {
                class: "tool-approval-btn tool-approval-allow-once",
                onclick: {
                    let state = state.clone();
                    let req = req.clone();
                    let choice = choice.clone();
                    move |_| {
                        let state = state.clone();
                        let req = req.clone();
                        let choice = choice.clone();
                        async move {
                            let _ = crate::api::api_answer_question(token(&state), req, choice).await;
                            refresh(state).await;
                        }
                    }
                },
                "{choice}"
            }
        };
        choice_els.push(el);
    }
    let req_for_input = q.request_id.clone();

    rsx! {
        div { class: "tool-approval-bar",
            div { class: "tool-approval-request agent-question",
                div { class: "tool-approval-info", style: "flex-direction:column;align-items:flex-start;gap:8px;",
                    span { class: "agent-question-text", "{q.question}" }
                    div { class: "tool-approval-actions", style: "flex-wrap:wrap;",
                        {choice_els.iter().cloned()}
                        input {
                            style: "background:var(--panel);border:1px solid var(--border);border-radius:6px;padding:6px 10px;color:var(--text);outline:none;font-size:12px;",
                            placeholder: "…",
                            value: "{answer()}",
                            oninput: move |e| answer.set(e.value()),
                            onkeydown: {
                                let state = state.clone();
                                let req = req_for_input.clone();
                                move |e: Event<KeyboardData>| {
                                    if e.key() == Key::Enter {
                                        e.prevent_default();
                                        let text = answer().clone();
                                        if !text.trim().is_empty() {
                                            let req = req.clone();
                                            spawn(async move {
                                                let _ = crate::api::api_answer_question(
                                                    token(&state), req, text,
                                                )
                                                .await;
                                                refresh(state).await;
                                            });
                                        }
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

/// Tool approval overlay: allow once / allow always / deny.
fn approval_el(
    state: Signal<ClientState>,
    lang: String,
    a: halny_vibe_deck_core::snapshot::PendingApproval,
) -> Element {
    let tool_name = a.tool_name.clone().unwrap_or_else(|| a.method.clone());
    let display = if tool_name.len() > 40 {
        format!("{}…", &tool_name[..40])
    } else {
        tool_name
    };
    let question_text = tf(&lang, "allowToolQuestion", &[("tool", display.as_str())]);
    let req = a.request_id.clone();
    let params = a.params_json.clone();

    let mk_btn = |option: &'static str, cls: &'static str, label: String| -> Element {
        let req = req.clone();
        rsx! {
            button {
                class: "{cls}",
                onclick: {
                    let state = state.clone();
                    let req = req.clone();
                    move |_| {
                        let state = state.clone();
                        let req = req.clone();
                        async move {
                            let _ = crate::api::api_approve(
                                token(&state), req, option.to_string(),
                            )
                            .await;
                            refresh(state).await;
                        }
                    }
                },
                "{label}"
            }
        }
    };

    let btn_once = mk_btn("allow_once", "tool-approval-btn tool-approval-allow-once", tr(&lang, "allowOnce").to_string());
    let btn_always = mk_btn("allow_always", "tool-approval-btn tool-approval-allow-always", tr(&lang, "alwaysAllow").to_string());
    let btn_deny = mk_btn("reject", "tool-approval-btn tool-approval-deny", tr(&lang, "dontAllow").to_string());

    rsx! {
        div { class: "tool-approval-bar",
            div { class: "tool-approval-request",
                div { class: "tool-approval-info",
                    span { class: "tool-approval-icon", "?" }
                    span { class: "tool-approval-name", "{question_text}" }
                }
                div { style: "display:flex;flex-direction:column;gap:6px;align-items:flex-end;",
                    if !params.is_empty() {
                        div { style: "color:var(--text-muted);font-size:11px;max-width:520px;white-space:pre-wrap;max-height:120px;overflow:auto;", "{params}" }
                    }
                    div { class: "tool-approval-actions",
                        {btn_once}
                        {btn_always}
                        {btn_deny}
                    }
                }
            }
        }
    }
}

/// Sidebar (120–400 px) and dock (260–720 px) resize handles, like Nexus
/// Sidebar/Dock. The widths live in CSS vars on <html> (`.sidebar` and
/// `.dock` read them) and persist in localStorage.
/// Client-side mermaid bootstrap (round 4, C2: installed once at shell
/// level so the chat AND the md file preview both render ```mermaid
/// fences). Lazily injects mermaid from the CDN, then a MutationObserver
/// renders every `.mermaid-container[data-src]`. securityLevel 'strict'
/// like the stable edition; theme follows <html data-theme>; re-renders
/// on theme change. mermaid v10+ `render()` returns a promise when no
/// callback is passed, so one code path covers v10–v12.
#[cfg(any(feature = "web", feature = "desktop"))]
pub(crate) const MERMAID_JS: &str = r#"
(function(){
  if (window.__vdMermaid) return; window.__vdMermaid = true;
  function theme(){ return document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'default'; }
  function load(cb){
    if (window.mermaid) return cb();
    if (window.__vdMermaidLoading) return window.__vdMermaidLoading.then(cb);
    window.__vdMermaidLoading = new Promise(function(res){
      var s = document.createElement('script');
      s.src = 'https://cdn.jsdelivr.net/npm/mermaid@12/dist/mermaid.min.js';
      s.onload = res; s.onerror = res;
      document.head.appendChild(s);
    });
    window.__vdMermaidLoading.then(cb);
  }
  function fail(el, err){
    el.classList.remove('mermaid-ready');
    el.classList.add('mermaid-error');
    el.innerHTML = '<div class="mermaid-error-title">Mermaid error</div><pre></pre>';
    var pre = el.querySelector('pre');
    if (pre) pre.textContent = String(err && err.message ? err.message : err);
  }
  function renderEl(el){
    load(function(){
      if (!window.mermaid) {
        el.classList.add('mermaid-error');
        el.innerHTML = '<div class="mermaid-error-title">mermaid.js unavailable</div>';
        return;
      }
      try {
        window.mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: theme() });
        var src = el.getAttribute('data-src') || '';
        var id = 'vdGraph' + (el.__vdId || (el.__vdId = Math.random().toString(36).slice(2)));
        var r;
        try { r = window.mermaid.render(id, src); } catch (e) { return fail(el, e); }
        if (r && typeof r.then === 'function') {
          r.then(function(o){ el.innerHTML = (o && o.svg) ? o.svg : String(o); el.classList.add('mermaid-ready'); el.classList.remove('mermaid-error'); })
           .catch(function(e){ fail(el, e); });
        } else if (typeof r === 'string') {
          el.innerHTML = r; el.classList.add('mermaid-ready');
        }
      } catch (e) { fail(el, e); }
    });
  }
  function scan(){
    document.querySelectorAll('.mermaid-container:not([data-vd-done])').forEach(function(el){
      el.setAttribute('data-vd-done', '1');
      renderEl(el);
    });
  }
  function rescan(){
    // theme switch: re-render everything from its data-src
    document.querySelectorAll('.mermaid-container').forEach(function(el){
      el.removeAttribute('data-vd-done');
      el.classList.remove('mermaid-ready');
    });
    scan();
  }
  var mo = new MutationObserver(function(){ scan(); });
  mo.observe(document.body, { childList: true, subtree: true });
  // re-render diagrams when the app theme flips (data-theme on <html>)
  var moTheme = new MutationObserver(function(muts){
    for (var i = 0; i < muts.length; i++) {
      if (muts[i].attributeName === 'data-theme') { rescan(); break; }
    }
  });
  moTheme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  var mq = window.matchMedia('(prefers-color-scheme: dark)');
  if (mq && mq.addEventListener) mq.addEventListener('change', rescan);
  scan();
})();
"#;

/// v1.0.17: on return to a (frozen) tab the event stream may have
/// dropped events while backgrounded — clicking the hidden
/// `vd-visibility-resync` bridge button triggers a full snapshot
/// refresh + queue drain in Rust.
#[cfg(any(feature = "web", feature = "desktop"))]
const VISIBILITY_JS: &str = r#"
(function () {
  if (window.__vdVisibilityInit) return; window.__vdVisibilityInit = 1;
  var last = 0;
  function resync(why) {
    var now = Date.now();
    // One resync per 5 s is plenty: a focus + visibilitychange burst must
    // not hammer the agent with resume calls.
    if (now - last < 5000) return;
    last = now;
    window.__vdResyncWhy = why;
    var btn = document.getElementById('vd-visibility-resync');
    if (btn) btn.click();
  }
  // Stable v1.0.36: a frozen/half-open tab loses events in several ways —
  // cover every "the user is back" signal, not only visibilitychange.
  document.addEventListener('visibilitychange', function () {
    if (document.visibilityState === 'visible') resync('visibility');
  });
  window.addEventListener('pageshow', function () { resync('pageshow'); });
  window.addEventListener('online', function () { resync('online'); });
  window.addEventListener('focus', function () { resync('focus'); });
  // The events socket re-opening means the stream was broken: pull state.
  (function () {
    var opened = 0;
    try {
      var _ws = window.WebSocket;
      window.WebSocket = function (url, protocols) {
        var sock = protocols ? new _ws(url, protocols) : new _ws(url);
        sock.addEventListener('open', function () {
          if (opened++ > 0) resync('ws-open');
        });
        return sock;
      };
      window.WebSocket.prototype = _ws.prototype;
    } catch (e) {}
  })();
})();
"#;

#[cfg(any(feature = "web", feature = "desktop"))]
const RESIZE_JS: &str = r#"
(function () {
  if (window.__vdResizeInit) return; window.__vdResizeInit = 1;
  var root = document.documentElement;
  function save(k, v) { try { localStorage.setItem(k, v); } catch (e) {} }
  function load(k, d) { try { var v = localStorage.getItem(k); return v || d; } catch (e) { return d; } }
  root.style.setProperty('--vd-sidebar-w', load('vdSidebarW', '220px'));
  root.style.setProperty('--vd-dock-w', load('vdDockW', '400px'));
  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }
  function arm(handleSel, onDown) {
    // Pointer events (faa86ac): touch/PWA drags never fired mouse
    // events, and a mouseup outside the window left the panel stuck.
    // setPointerCapture keeps move/up on the handle even outside it.
    document.addEventListener('pointerdown', function (e) {
      var h = e.target.closest ? e.target.closest(handleSel) : null;
      if (!h || e.button !== 0) return;
      e.preventDefault();
      try { h.setPointerCapture(e.pointerId); } catch (err) {}
      onDown(e);
    });
  }
  function drag(startX, startW, apply, done) {
    function move(e) { apply(e.clientX - startX); }
    function up() {
      document.removeEventListener('pointermove', move);
      document.removeEventListener('pointerup', up);
      document.removeEventListener('pointercancel', up);
      done();
    }
    document.addEventListener('pointermove', move);
    document.addEventListener('pointerup', up);
    document.addEventListener('pointercancel', up);
  }
  // sidebar: the handle sits on the sidebar's RIGHT edge — dragging right
  // (positive dx) widens the panel.
  arm('.sidebar-resize-handle', function (e) {
    var x0 = e.clientX, w0 = parseFloat(load('vdSidebarW', '220'));
    drag(x0, w0, function (dx) {
      var w = clamp(w0 + dx, 120, 400);
      root.style.setProperty('--vd-sidebar-w', w + 'px');
    }, function () { save('vdSidebarW', root.style.getPropertyValue('--vd-sidebar-w')); });
  });
  // dock: the handle sits on the dock's LEFT edge — dragging left
  // (negative dx) widens the panel.
  arm('.dock-resize-handle', function (e) {
    var x0 = e.clientX, w0 = parseFloat(load('vdDockW', '400'));
    drag(x0, w0, function (dx) {
      var w = clamp(w0 - dx, 260, 720);
      root.style.setProperty('--vd-dock-w', w + 'px');
    }, function () { save('vdDockW', root.style.getPropertyValue('--vd-dock-w')); });
  });
})();
"#;

/// Browser-mode floating panel buttons (stable v1.0.27/v1.0.30 parity).
///
/// The web client renders no header, so the sessions toggle lives in the
/// top-left corner (with the agent-status dot) and the preview toggle floats
/// on the right edge, above the chat at any width. They are mutually
/// exclusive with the open panels: `.panel-open` / `.dock-open` hide them
/// instead of stacking another control on top of the panel.
#[component]
pub fn WebFabs() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let s = state.read();
    let status = s.snapshot.status.clone();
    let sidebar_open = s.mobile_sidebar_open;
    let dock_open = s.snapshot.config.dock_open && s.view == View::Chat;
    drop(s);

    let dot_cls = match status.as_str() {
        "connected" => "web-fab-dot ok",
        "error" => "web-fab-dot err",
        _ => "web-fab-dot off",
    };
    let left_cls = if sidebar_open {
        "web-fab web-fab-left panel-open"
    } else if dock_open {
        "web-fab web-fab-left dock-open"
    } else {
        "web-fab web-fab-left"
    };
    let lang_w = crate::state::lang(&state);
    let right_cls = if dock_open {
        "web-fab web-fab-right active dock-open"
    } else if sidebar_open {
        "web-fab web-fab-right sidebar-open"
    } else {
        "web-fab web-fab-right"
    };

    rsx! {
        button {
            class: "{left_cls}",
            title: "{tr(&lang_w, \"menuLabel\")}",
            onclick: {
                let mut state = state.clone();
                move |_| {
                    // Opening the session list hides the preview panel (the
                    // two FABs are mutually exclusive on every width).
                    state.write().mobile_sidebar_open = true;
                }
            },
            svg {
                width: "18", height: "18", view_box: "0 0 24 24",
                fill: "none", stroke: "currentColor",
                "stroke-width": "2", "stroke-linecap": "round",
                line { x1: "3", y1: "6", x2: "21", y2: "6" }
                line { x1: "3", y1: "12", x2: "21", y2: "12" }
                line { x1: "3", y1: "18", x2: "21", y2: "18" }
            }
            span { class: "{dot_cls}" }
        }
        button {
            class: "{right_cls}",
            title: "{tr(&lang_w, \"dockToggle\")}",
            onclick: {
                let state = state.clone();
                move |_| {
                    let mut state = state.clone();
                    spawn(async move {
                        let open = {
                            let s = state.read();
                            s.snapshot.config.dock_open && s.view == View::Chat
                        };
                        if open {
                            // Close the preview panel (persisted).
                            crate::components::mobile::set_dock_open(state, false).await;
                        } else {
                            // Open the preview panel: close the session list
                            // and make sure the chat view is active (the dock
                            // only renders in the chat view).
                            {
                                let mut s = state.write();
                                s.mobile_sidebar_open = false;
                            }
                            crate::app::set_view(state, View::Chat);
                            crate::components::mobile::set_dock_open(state, true).await;
                        }
                    });
                }
            },
            svg {
                width: "18", height: "18", view_box: "0 0 24 24",
                fill: "none", stroke: "currentColor",
                "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
                rect { x: "3", y: "3", width: "18", height: "18", rx: "2" }
                line { x1: "15", y1: "3", x2: "15", y2: "21" }
            }
        }
    }
}

/// Element 6: update banner (Nexus App.tsx update-banner) - app + CLI
/// updates appear shortly after startup, with live download progress and a
/// dismiss button; app/CLI update actions run through the shared
/// ClientState helpers (also used by the Settings Updates section).
pub fn UpdateBanner() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);

    let s = state.read();
    let show = !s.update_dismissed
        && (s.update_app.as_ref().map(|a| a.update_available).unwrap_or(false)
            || s.update_cli.as_ref().map(|c| c.update_available).unwrap_or(false));
    let app = s.update_app.clone();
    let cli = s.update_cli.clone();
    let busy = s.update_busy;
    let error = s.update_error.clone();
    let prog = s.update_progress.clone();
    drop(s);

    let app_avail = app.as_ref().map(|a| a.update_available).unwrap_or(false);
    let cli_avail = cli.as_ref().map(|c| c.update_available).unwrap_or(false);
    let app_latest = app.as_ref().and_then(|a| a.latest_version.clone()).unwrap_or_default();
    let cli_latest = cli.as_ref().and_then(|c| c.latest_version.clone()).unwrap_or_default();

    let loc = |pl: &str, en: &str| -> String {
        if lang == "pl" { pl.to_string() } else { en.to_string() }
    };

    let mut progress_text = String::new();
    let mut pct = 0i64;
    let mut downloading = false;
    if busy {
        if let Some(p) = prog.as_ref() {
            match p.phase.as_str() {
                "download" => {
                    downloading = true;
                    let received = p.received as f64 / 1048576.0;
                    let total = p
                        .total
                        .map(|t| format!("{:.1}", t as f64 / 1048576.0))
                        .unwrap_or_else(|| "?".to_string());
                    progress_text = format!(
                        "{} {}% ({received:.1} / {total} MB)",
                        loc("Pobieranie", "Downloading"),
                        p.percent.unwrap_or(0),
                    );
                }
                "install" => progress_text = loc("Instalowanie…", "Installing…"),
                _ => progress_text = loc("Restartowanie…", "Restarting…"),
            }
            pct = p.percent.unwrap_or(0);
        }
    }
    let desktop = crate::state::is_desktop();

    rsx! {
        if show {
            div { class: "update-banner",
            div { class: "update-banner-texts",
                if app_avail {
                    span { "{loc(\"Dostępna aktualizacja aplikacji\", \"App update available\")}: {app_latest}" }
                }
                if cli_avail {
                    span { "{loc(\"Nowa wersja CLI\", \"New CLI version available\")}: {cli_latest}" }
                }
                if let Some(e) = &error {
                    span { class: "update-banner-error", "❌ {e}" }
                }
                if busy && !progress_text.is_empty() {
                    span { class: "update-banner-progress", "{progress_text}" }
                }
                if busy && downloading && pct > 0 {
                    div { class: "update-progress-track",
                        div { class: "update-progress-fill", style: "width: {pct}%" }
                    }
                }
            }
            div { class: "update-banner-actions",
                if desktop && app_avail {
                    button {
                        class: "update-banner-btn",
                        disabled: busy,
                        onclick: {
                            let state = state;
                            move |_| {
                                let state = state;
                                spawn(async move { crate::state::start_app_update(state).await; });
                            }
                        },
                        "{loc(\"Aktualizuj aplikację\", \"Update app\")}"
                    }
                }
                if cli_avail {
                    button {
                        class: "update-banner-btn",
                        disabled: busy,
                        onclick: {
                            let state = state;
                            move |_| {
                                let state = state;
                                spawn(async move { crate::state::upgrade_local_cli(state).await; });
                            }
                        },
                        "{loc(\"Aktualizuj CLI\", \"Update CLI\")}"
                    }
                }
                button {
                    class: "update-banner-dismiss",
                    title: "{loc(\"Ukryj\", \"Dismiss\")}",
                    onclick: {
                        let state = state;
                        move |_| {
                            let mut state = state;
                            state.write().update_dismissed = true;
                        }
                    },
                    "✕"
                }
            }
            }
        }
    }
}
