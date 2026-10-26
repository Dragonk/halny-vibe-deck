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
            });
        }
    }

    // The engine event stream.
    let mut socket = crate::events::use_engine_socket();
    use_future(move || async move {
        loop {
            match socket.recv().await {
                Ok(ev) => {
                    let refetch = apply_event(state, ev);
                    if refetch {
                        refresh(state).await;
                        crate::state::drain_queue(state).await;
                    }
                }
                Err(_) => {
                    // Stream dropped (server restart, token expiry).
                    crate::state::sleep_ms(2_000).await;
                    refresh(state).await;
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
            crate::components::header::Header {}
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
        }
    }
}

/// Agent question overlay: choice buttons + free-form input.
fn question_el(
    state: Signal<ClientState>,
    _lang: String,
    q: mistraldeck_core::snapshot::PendingQuestion,
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
    a: mistraldeck_core::snapshot::PendingApproval,
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
    document.addEventListener('mousedown', function (e) {
      var h = e.target.closest ? e.target.closest(handleSel) : null;
      if (!h || e.button !== 0) return;
      e.preventDefault();
      onDown(e);
    });
  }
  function drag(startX, startW, apply, done) {
    function move(e) { apply(startX - e.clientX); }
    function up() {
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', up);
      done();
    }
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', up);
  }
  // sidebar: drag right edge -> wider
  arm('.sidebar-resize-handle', function (e) {
    var x0 = e.clientX, w0 = parseFloat(load('vdSidebarW', '220'));
    drag(x0, w0, function (dx) {
      var w = clamp(w0 + dx, 120, 400);
      root.style.setProperty('--vd-sidebar-w', w + 'px');
    }, function () { save('vdSidebarW', root.style.getPropertyValue('--vd-sidebar-w')); });
  });
  // dock: drag left edge -> wider
  arm('.dock-resize-handle', function (e) {
    var x0 = e.clientX, w0 = parseFloat(load('vdDockW', '400'));
    drag(x0, w0, function (dx) {
      var w = clamp(w0 + dx, 260, 720);
      root.style.setProperty('--vd-dock-w', w + 'px');
    }, function () { save('vdDockW', root.style.getPropertyValue('--vd-dock-w')); });
  });
})();
"#;

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
