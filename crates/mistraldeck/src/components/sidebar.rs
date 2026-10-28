//! Session sidebar: local sessions grouped by project (working
//! directory), CLI (backend) sessions, hide-with-confirm and nav.
//! Ported from Nexus Sidebar.tsx (CSS classes identical).

use dioxus::prelude::*;
use mistraldeck_core::snapshot::{RemoteConnection, RemoteSessionInfo, RemoteStatus, SessionMeta};

use crate::i18n::{tf, tr};
use crate::state::{lang, token, ClientState, View};

/// Bilingual inline label (same pattern as settings.rs).
fn loc(lang: &str, pl: &str, en: &str) -> String {
    if lang == "pl" {
        pl.to_string()
    } else {
        en.to_string()
    }
}

#[derive(Clone, Default)]
struct ConfirmTarget {
    /// false = session, true = project
    is_project: bool,
    label: String,
    key: String,
}

/// Per-connection data fetched on demand (sidebar expand).
#[derive(Clone, Default)]
struct RemotePane {
    status: Option<RemoteStatus>,
    sessions: Vec<RemoteSessionInfo>,
    loaded: bool,
}

/// One entry of a unified remote folder: a locally mirrored session, or a
/// CLI session discovered on the remote host (Nexus v0.13.1).
#[derive(Clone)]
enum RemoteItem {
    Mirror(usize, SessionMeta),
    Cli(RemoteSessionInfo),
}

/// Open context menu (right click): fixed position + the hide target.
#[derive(Clone)]
struct CtxMenu {
    x: f64,
    y: f64,
    target: ConfirmTarget,
}

#[derive(Clone)]
struct Group {
    wd: String,
    label: String,
    sessions: Vec<(usize, SessionMeta)>,
    newest: i64,
}

fn date_str(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}.{y}")
}

/// Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn short_dir(dir: &str) -> String {
    let name = dir.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("");
    if name.is_empty() { dir.to_string() } else { name.to_string() }
}

pub fn Sidebar() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let confirm = use_signal(|| None::<ConfirmTarget>);
    let collapsed = use_signal(Vec::<String>::new);
    let ctx = use_signal(|| None::<CtxMenu>);

    let s = state.read();
    let lang = lang(&state);
    let sessions = s.snapshot.sessions.clone();
    let current = s.snapshot.current;
    let backend = s.snapshot.backend_sessions.clone();
    let cfg = s.snapshot.config.clone();
    let view = s.view;
    drop(s);

    // v1.0.33 parity: web-only bottom controls (theme cycle + logout).
    // Same theme naming/cycle as the header (themeAuto/themeDark/themeLight).
    let language = cfg.language.clone();
    let theme = cfg.theme.clone();
    let (theme_icon, theme_title) = if matches!(theme.as_str(), "" | "system" | "auto") {
        ('\u{25D1}', tr(&lang, "themeAuto"))
    } else if theme == "dark" {
        ('\u{263c}', tr(&lang, "themeDark"))
    } else {
        ('\u{263e}', tr(&lang, "themeLight"))
    };

    // ---- Filter (hidden sessions/projects) + limit ----
    // Remote mirrors (SSH) are excluded here: they render inside their
    // connection group below.
    let mut visible: Vec<(usize, SessionMeta)> = sessions
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.remote_id.is_none()
                && {
                    let key = m.agent_session_id.clone().unwrap_or_else(|| m.id.clone());
                    !cfg.hidden_sessions.iter().any(|h| h == &key)
                        && (m.working_directory.is_empty()
                            || !cfg.hidden_projects.iter().any(|h| h == &m.working_directory))
                }
        })
        .map(|(i, m)| (i, m.clone()))
        .collect();
    visible.sort_by(|a, b| b.1.last_active_ms.cmp(&a.1.last_active_ms));
    if let Some(limit) = cfg.sidebar_sessions_limit {
        if limit > 0 {
            visible.truncate(limit as usize);
        }
    }

    // ---- Remote mirrors: sessions bound to an SSH connection ----
    let mut remote_mirrors: Vec<(usize, SessionMeta)> = sessions
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.remote_id.is_some()
                && {
                    let key = m.agent_session_id.clone().unwrap_or_else(|| m.id.clone());
                    !cfg.hidden_sessions.iter().any(|h| h == &key)
                }
        })
        .map(|(i, m)| (i, m.clone()))
        .collect();
    remote_mirrors.sort_by(|a, b| b.1.last_active_ms.cmp(&a.1.last_active_ms));

    // ---- Group by working directory ----
    let mut groups: Vec<Group> = Vec::new();
    for (idx, meta) in &visible {
        let wd = meta.working_directory.clone();
        let label = if wd.is_empty() {
            tr(&lang, "noProject").to_string()
        } else {
            short_dir(&wd)
        };
        match groups.iter_mut().find(|g| g.wd == wd) {
            Some(g) => {
                g.sessions.push((*idx, meta.clone()));
                if meta.last_active_ms > g.newest {
                    g.newest = meta.last_active_ms;
                }
            }
            None => groups.push(Group {
                wd: wd.clone(),
                label,
                sessions: vec![(*idx, meta.clone())],
                newest: meta.last_active_ms,
            }),
        }
    }
    groups.sort_by(|a, b| b.newest.cmp(&a.newest));

    // ---- Prebuilt element lists (avoids nested closure captures) ----
    let group_els: Vec<Element> = groups
        .iter()
        .map(|g| group_el(state, lang.clone(), g, current, confirm, collapsed, ctx))
        .collect();
    let backend_els: Vec<Element> = backend
        .iter()
        .map(|info| backend_el(state, lang.clone(), info, confirm))
        .collect();
    let remote_panes = use_signal(std::collections::HashMap::<String, RemotePane>::new);
    let remotes: Vec<RemoteConnection> = cfg
        .remote_connections
        .iter()
        .filter(|c| c.enabled)
        .cloned()
        .collect();
    let remote_els: Vec<Element> = remotes
        .iter()
        .map(|conn| {
            let mirrors: Vec<(usize, SessionMeta)> = remote_mirrors
                .iter()
                .filter(|(_, m)| m.remote_id.as_deref() == Some(conn.id.as_str()))
                .cloned()
                .collect();
            let mut pane = remote_panes.read().get(&conn.id).cloned().unwrap_or_default();
            // v0.13.x hide parity: remote CLI sessions respect hidden
            // sessions/projects (Nexus Sidebar isHidden filter).
            pane.sessions.retain(|s| {
                !cfg.hidden_sessions.iter().any(|h| h == &s.session_id)
                    && (s.cwd.is_empty() || !cfg.hidden_projects.iter().any(|h| h == &s.cwd))
            });
            remote_conn_el(
                state,
                lang.clone(),
                conn,
                mirrors,
                pane,
                remote_panes.clone(),
                collapsed,
                current,
                confirm,
                ctx,
            )
        })
        .collect();
    let modal_el = confirm
        .read()
        .clone()
        .map(|t| confirm_modal(state, lang.clone(), t, confirm));
    let ctx_el = ctx
        .read()
        .clone()
        .map(|c| ctx_menu_el(state, lang.clone(), c, ctx, confirm));

    let nav_cls = |v: View| {
        if view == v {
            "sidebar-nav-item active".to_string()
        } else {
            "sidebar-nav-item".to_string()
        }
    };

    let sidebar_cls = if state.read().mobile_sidebar_open {
        "sidebar mobile-open"
    } else {
        "sidebar"
    };

    rsx! {
        aside { class: "{sidebar_cls}",
            button {
                class: "mobile-panel-close",
                title: "{tr(&lang, \"closePanelLabel\")}",
                onclick: {
                    let mut state = state.clone();
                    move |_| { state.write().mobile_sidebar_open = false; }
                },
                "✕"
            }
            div { class: "sidebar-header",
                button {
                    class: "new-session-btn",
                    onclick: {
                        let state = state.clone();
                        move |_| {
                            let state = state.clone();
                            async move {
                                let empty = {
                                    let s = state.read();
                                    s.snapshot
                                        .sessions
                                        .iter()
                                        .enumerate()
                                        .find(|(_, m)| m.message_count == 0 && m.agent_session_id.is_some())
                                        .map(|(i, _)| i)
                                };
                                match empty {
                                    Some(i) => {
                                        let _ = crate::api::api_switch_session(token(&state), i).await;
                                    }
                                    None => {
                                        let _ = crate::api::api_create_session(token(&state)).await;
                                    }
                                }
                                crate::state::refresh(state).await;
                                crate::app::set_view(state, View::Chat);
                            }
                        }
                    },
                    span { class: "new-session-icon", "+" }
                    "{tr(&lang, \"newSession\")}"
                }
            }
            div { class: "sidebar-sessions-label", "{tr(&lang, \"sessions\")}" }
            nav { class: "session-list session-list-projects",
                if group_els.is_empty() {
                    span { class: "session-empty", "{tr(&lang, \"noSessionsYet\")}" }
                }
                {group_els.iter().cloned()}
            }
            if !backend.is_empty() {
                div { class: "sidebar-sessions-label", "CLI" }
                nav { class: "session-list",
                    {backend_els.iter().cloned()}
                }
            }
            if !remote_els.is_empty() {
                div { class: "sidebar-sessions-label", "SSH" }
                nav { class: "session-list",
                    {remote_els.iter().cloned()}
                }
            }
            nav { class: "sidebar-nav",
                button {
                    class: "{nav_cls(View::Stats)}",
                    onclick: { let state = state.clone(); move |_| { crate::app::set_view(state, View::Stats); } },
                    "{tr(&lang, \"statistics\")}"
                }
                button {
                    class: "{nav_cls(View::History)}",
                    onclick: { let state = state.clone(); move |_| { crate::app::set_view(state, View::History); } },
                    "{tr(&lang, \"historyLabel\")}"
                }
                button {
                    class: "{nav_cls(View::Settings)}",
                    onclick: { let state = state.clone(); move |_| { crate::app::set_view(state, View::Settings); } },
                    "{tr(&lang, \"settings\")}"
                }
                // v1.0.32/33 parity: in the browser the brand row is gone
                // (the frameless title bar never existed on web), so the
                // theme toggle and logout live at the bottom of the
                // sessions list — under Settings, same as stable. The
                // desktop webview keeps the header theme toggle.
                if !crate::state::is_desktop() {
                    button {
                        class: "sidebar-nav-item",
                        title: "{theme_title}",
                        onclick: {
                            let state = state.clone();
                            let language = language.clone();
                            let theme = theme.clone();
                            let dock = cfg.dock_open;
                            let limit = cfg.sidebar_sessions_limit.unwrap_or(0);
                            move |_| {
                                let state = state.clone();
                                let language = language.clone();
                                let theme = theme.clone();
                                let dock = dock;
                                let limit = limit;
                                async move {
                                    let next_theme = if matches!(theme.as_str(), "" | "system" | "auto") {
                                        "dark"
                                    } else if theme == "dark" {
                                        "light"
                                    } else {
                                        "auto"
                                    };
                                    crate::state::save_prefs(state, language, next_theme.to_string(), dock, limit).await;
                                }
                            }
                        },
                        "{theme_icon}"
                        span { style: "margin-left:8px;", "{theme_title}" }
                    }
                    button {
                        class: "sidebar-nav-item sidebar-logout-btn",
                        title: "{tr(&lang, \"logoutLabel\")}",
                        onclick: move |_| {
                            async move {
                                // Clear the server session cookie; the
                                // reload lands on the /login page.
                                let _ = dioxus::document::eval(
                                    "fetch('/api/auth/logout',{method:'POST'}).finally(function(){location.href='/login';});",
                                ).await;
                            }
                        },
                        span { class: "sidebar-logout-icon",
                            svg {
                                width: "14", height: "14", view_box: "0 0 24 24",
                                fill: "none", stroke: "currentColor",
                                "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
                                path { d: "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" }
                                polyline { points: "16 17 21 12 16 7" }
                                line { x1: "21", y1: "12", x2: "9", y2: "12" }
                            }
                        }
                        span { style: "margin-left:8px;", "{tr(&lang, \"logoutLabel\")}" }
                    }
                }
            }
            div { class: "sidebar-resize-handle" }
        }

        if let Some(el) = modal_el {
            {el}
        }
        if let Some(el) = ctx_el {
            {el}
        }
    }
}

/// Confirm modal for hiding a session/project (UI-only hide).
fn confirm_modal(
    state: Signal<ClientState>,
    lang: String,
    t: ConfirmTarget,
    mut confirm: Signal<Option<ConfirmTarget>>,
) -> Element {
    let msg = if t.is_project {
        tf(&lang, "deleteProjectConfirmMsg", &[("name", t.label.as_str())])
    } else {
        tf(&lang, "deleteSessionConfirmMsg", &[("name", t.label.as_str())])
    };
    let kind_label = if t.is_project {
        tr(&lang, "deleteProjectLabel")
    } else {
        tr(&lang, "deleteSessionLabel")
    };
    rsx! {
        div { class: "modal-overlay", onclick: move |_| confirm.set(None),
            div { class: "modal-box", onclick: move |e| e.stop_propagation(),
                div { class: "modal-title", "{tr(&lang, \"deleteConfirmTitle\")}" }
                div { class: "modal-message", "{msg}" }
                div { class: "modal-actions",
                    button { class: "modal-btn", onclick: move |_| confirm.set(None), "{tr(&lang, \"cancelLabel\")}" }
                    button {
                        class: "modal-btn modal-btn-danger",
                        onclick: move |_| {
                            confirm.set(None);
                            let state = state.clone();
                            let t = t.clone();
                            spawn(async move {
                                let tok = token(&state);
                                if t.is_project {
                                    let _ = crate::api::api_hide_project(tok, t.key.clone()).await;
                                } else {
                                    let _ = crate::api::api_hide_session(tok, t.key.clone()).await;
                                }
                                crate::state::refresh(state).await;
                            });
                        },
                        "🗑 {kind_label}"
                    }
                }
            }
        }
    }
}

/// The right-click context menu (hide session/project). A full-screen
/// overlay behind it closes the menu on any click.
fn ctx_menu_el(
    state: Signal<ClientState>,
    lang: String,
    c: CtxMenu,
    mut ctx: Signal<Option<CtxMenu>>,
    confirm: Signal<Option<ConfirmTarget>>,
) -> Element {
    let item_label = if c.target.is_project {
        tr(&lang, "hideProject")
    } else {
        tr(&lang, "hideSession")
    };
    rsx! {
        div {
            class: "ctx-overlay",
            onclick: move |_| ctx.set(None),
            oncontextmenu: {
                let mut ctx = ctx.clone();
                move |e| {
                    e.prevent_default();
                    ctx.set(None);
                }
            },
            div {
                class: "ctx-menu",
                style: "left:{c.x}px; top:{c.y}px;",
                onclick: move |e| e.stop_propagation(),
                button {
                    class: "ctx-menu-item danger",
                    onclick: {
                        let mut ctx = ctx.clone();
                        let mut confirm = confirm.clone();
                        let t = c.target.clone();
                        move |_| {
                            ctx.set(None);
                            confirm.set(Some(t.clone()));
                        }
                    },
                    "🗑 {item_label}"
                }
            }
            // keep the engine state alive for the api calls in confirm modal
            {let _ = state.clone(); VNode::empty()}
        }
    }
}

fn group_el(
    state: Signal<ClientState>,
    lang: String,
    g: &Group,
    current: Option<usize>,
    confirm: Signal<Option<ConfirmTarget>>,
    collapsed: Signal<Vec<String>>,
    ctx: Signal<Option<CtxMenu>>,
) -> Element {
    let rows: Vec<Element> = g
        .sessions
        .iter()
        .map(|(idx, m)| session_row(state.clone(), lang.clone(), *idx, m, current, confirm.clone(), ctx.clone()))
        .collect();
    let title = if g.wd.is_empty() {
        tr(&lang, "noProject").to_string()
    } else {
        g.wd.clone()
    };
    let count = g.sessions.len().to_string();
    let is_collapsed = collapsed.read().contains(&g.wd);
    let chev_cls = if is_collapsed {
        "project-chevron"
    } else {
        "project-chevron open"
    };

    rsx! {
        div { class: "project-group",
            button {
                class: "project-group-header",
                title: "{title}",
                onclick: {
                    let mut collapsed = collapsed.clone();
                    let key = g.wd.clone();
                    move |_| {
                        let mut c = collapsed.write();
                        if c.contains(&key) {
                            c.retain(|k| k != &key);
                        } else {
                            c.push(key.clone());
                        }
                    }
                },
                oncontextmenu: {
                    let mut ctx = ctx.clone();
                    let label = g.label.clone();
                    let wd = g.wd.clone();
                    move |e| {
                        e.prevent_default();
                        let p = e.data().client_coordinates();
                        ctx.set(Some(CtxMenu {
                            x: p.x,
                            y: p.y,
                            target: ConfirmTarget {
                                is_project: true,
                                label: label.clone(),
                                key: wd.clone(),
                            },
                        }));
                    }
                },
                span { class: "{chev_cls}", "▸" }
                span { class: "project-name", "{g.label}" }
                span { class: "project-count", "{count}" }
            }
            if !is_collapsed {
                {rows.iter().cloned()}
            }
        }
    }
}

fn session_row(
    state: Signal<ClientState>,
    lang: String,
    idx: usize,
    m: &SessionMeta,
    current: Option<usize>,
    confirm: Signal<Option<ConfirmTarget>>,
    ctx: Signal<Option<CtxMenu>>,
) -> Element {
    let active = current == Some(idx);
    let cls = if active { "session-item active" } else { "session-item" };

    let busy_el: Element = if m.busy {
        rsx! { span { class: "session-busy-spinner", title: "streaming" } }
    } else {
        VNode::empty()
    };
    let date = date_str(m.last_active_ms);
    let key = m.agent_session_id.clone().unwrap_or_else(|| m.id.clone());
    let name = m.name.clone();
    let has_agent = m.agent_session_id.is_some();
    let empty = m.message_count == 0;
    let agent_id = m.agent_session_id.clone();
    let busy = m.busy;

    rsx! {
        button {
            class: "{cls}",
            onclick: {
                let state = state.clone();
                let agent_id = agent_id.clone();
                move |_| {
                    let state = state.clone();
                    let agent_id = agent_id.clone();
                    async move {
                        let tok = token(&state);
                        if empty && has_agent {
                            if let Some(sid) = agent_id.clone() {
                                let _ = crate::api::api_open_backend_session(tok, sid).await;
                            }
                        } else {
                            let _ = crate::api::api_switch_session(tok, idx).await;
                        }
                        crate::state::refresh(state).await;
                        crate::app::set_view(state, View::Chat);
                    }
                }
            },
            oncontextmenu: {
                let mut ctx = ctx.clone();
                let name = name.clone();
                let key = key.clone();
                move |e| {
                    e.prevent_default();
                    let p = e.data().client_coordinates();
                    ctx.set(Some(CtxMenu {
                        x: p.x,
                        y: p.y,
                        target: ConfirmTarget {
                            is_project: false,
                            label: name.clone(),
                            key: key.clone(),
                        },
                    }));
                }
            },
            div { class: "session-item-content",
                div { class: "session-item-row",
                    span { class: "session-name", "{name}" }
                    {busy_el}
                }
                div { class: "session-item-meta",
                    span { class: "session-date", "{date}" }
                }
            }
            span {
                class: "session-cancel-btn",
                title: if busy { "{tr(&lang, \"cancelLabel\")}" } else { "{tr(&lang, \"deleteSessionLabel\")}" },
                onclick: {
                    let state = state.clone();
                    let mut confirm = confirm.clone();
                    let mut ctx = ctx.clone();
                    let key = key.clone();
                    let name = name.clone();
                    let busy = busy;
                    move |e| {
                        e.stop_propagation();
                        if busy {
                            // streaming: ✕ = cancel the running prompt
                            let state = state.clone();
                            spawn(async move {
                                let _ = crate::api::api_cancel(token(&state)).await;
                                crate::state::refresh(state).await;
                            });
                        } else {
                            // idle: open the context menu (hide -> confirm)
                            let p = e.data().client_coordinates();
                            ctx.set(Some(CtxMenu {
                                x: p.x,
                                y: p.y,
                                target: ConfirmTarget {
                                    is_project: false,
                                    label: name.clone(),
                                    key: key.clone(),
                                },
                            }));
                            confirm.set(None);
                        }
                    }
                },
                "✕"
            }
        }
    }
}

fn backend_el(
    state: Signal<ClientState>,
    _lang: String,
    info: &mistraldeck_core::proto::AgentSessionInfo,
    confirm: Signal<Option<ConfirmTarget>>,
) -> Element {
    let sid = info.session_id.clone();
    let name = if info.name.is_empty() {
        "(CLI)".to_string()
    } else {
        info.name.clone()
    };
    let dir = short_dir(&info.cwd);
    let date = if info.start_time.len() >= 10 {
        info.start_time[..10].to_string()
    } else {
        String::new()
    };

    rsx! {
        button {
            class: "session-item",
            onclick: {
                let state = state.clone();
                let sid = sid.clone();
                move |_| {
                    let state = state.clone();
                    let sid = sid.clone();
                    async move {
                        let _ = crate::api::api_open_backend_session(token(&state), sid).await;
                        crate::state::refresh(state).await;
                        crate::app::set_view(state, View::Chat);
                    }
                }
            },
            div { class: "session-item-content",
                div { class: "session-item-row",
                    span { class: "session-name", "{name}" }
                }
                div { class: "session-item-meta",
                    span { class: "session-date", "{date} · {dir}" }
                }
            }
            span {
                class: "session-cancel-btn",
                onclick: {
                    let mut confirm = confirm.clone();
                    let sid = sid.clone();
                    let name = name.clone();
                    move |e| {
                        e.stop_propagation();
                        confirm.set(Some(ConfirmTarget {
                            is_project: false,
                            label: name.clone(),
                            key: sid.clone(),
                        }));
                    }
                },
                "✕"
            }
        }
    }
}

/// One SSH remote connection group in the sidebar (Nexus
/// remote-project-group): header with the live dot and the unified
/// folder list — configured locations plus per-cwd session groups,
/// deduplicated (Nexus v0.13.1).
#[allow(clippy::too_many_arguments)]
fn remote_conn_el(
    state: Signal<ClientState>,
    lang: String,
    conn: &RemoteConnection,
    mirrors: Vec<(usize, SessionMeta)>,
    pane: RemotePane,
    remote_panes: Signal<std::collections::HashMap<String, RemotePane>>,
    collapsed: Signal<Vec<String>>,
    current: Option<usize>,
    confirm: Signal<Option<ConfirmTarget>>,
    ctx: Signal<Option<CtxMenu>>,
) -> Element {
    let conn_id = conn.id.clone();
    let group_key = format!("remote:{conn_id}");
    let is_collapsed = collapsed.read().contains(&group_key);
    let chev_cls = if is_collapsed {
        "project-chevron"
    } else {
        "project-chevron open"
    };
    let (dot_cls, dot_title) = match &pane.status {
        Some(st) if st.ok => (
            "remote-dot ok",
            loc(&lang, "Połączenie aktywne", "Connection up"),
        ),
        Some(_) => (
            "remote-dot down",
            loc(&lang, "Połączenie nieaktywne", "Connection down"),
        ),
        None => (
            "remote-dot",
            loc(&lang, "Sprawdzam...", "Checking..."),
        ),
    };

    // v0.13.1: unified per-remote folder list — configured locations
    // plus session cwd groups, deduplicated. A folder with sessions
    // renders as a collapsible group (chevron + count), a folder without
    // sessions renders as a location item that starts a new remote
    // session on click; the ＋ button always starts a session.
    let mirrored: Vec<String> = mirrors
        .iter()
        .filter_map(|(_, m)| m.agent_session_id.clone())
        .collect();
    let norm_wd = |p: &str| -> String {
        if p.len() > 1 && p.ends_with('/') {
            p[..p.len() - 1].to_string()
        } else {
            p.to_string()
        }
    };
    // group the connection's sessions (open mirrors + unmirrored remote
    // CLI sessions) per working directory
    let mut wd_groups: Vec<(String, Vec<RemoteItem>)> = Vec::new();
    for (idx, m) in &mirrors {
        let wd = m.working_directory.clone();
        match wd_groups.iter_mut().find(|(w, _)| w == &wd) {
            Some((_, list)) => list.push(RemoteItem::Mirror(*idx, m.clone())),
            None => wd_groups.push((wd, vec![RemoteItem::Mirror(*idx, m.clone())])),
        }
    }
    for s in pane.sessions.iter().filter(|s| !mirrored.contains(&s.session_id)) {
        let wd = s.cwd.clone();
        match wd_groups.iter_mut().find(|(w, _)| w == &wd) {
            Some((_, list)) => list.push(RemoteItem::Cli(s.clone())),
            None => wd_groups.push((wd, vec![RemoteItem::Cli(s.clone())])),
        }
    }
    let group_for = |nt: &str| -> Vec<RemoteItem> {
        wd_groups
            .iter()
            .find(|(w, _)| norm_wd(w) == nt)
            .map(|(_, l)| l.clone())
            .unwrap_or_default()
    };
    // configured locations first (config order), then unseen cwd groups
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut folders: Vec<(String, Vec<RemoteItem>)> = Vec::new();
    for l in &conn.locations {
        let nt = norm_wd(l);
        if seen.contains(&nt) {
            continue;
        }
        seen.insert(nt.clone());
        folders.push((l.clone(), group_for(&nt)));
    }
    for (gwd, glist) in &wd_groups {
        let nt = norm_wd(gwd);
        if seen.contains(&nt) {
            continue;
        }
        seen.insert(nt);
        folders.push((gwd.clone(), glist.clone()));
    }
    let folder_els: Vec<Element> = folders
        .iter()
        .map(|(wd, list)| {
            if list.is_empty() {
                remote_loc_item_el(state.clone(), &conn_id, wd)
            } else {
                remote_folder_el(
                    state.clone(),
                    lang.clone(),
                    &conn_id,
                    wd,
                    list,
                    collapsed.clone(),
                    current,
                    confirm.clone(),
                    ctx.clone(),
                )
            }
        })
        .collect();

    rsx! {
        div { class: "project-group remote-project-group",
            button {
                class: "project-group-header",
                title: "{conn.ssh_cmd}",
                onclick: {
                    let state = state.clone();
                    let mut collapsed = collapsed.clone();
                    let remote_panes = remote_panes.clone();
                    let conn_id = conn_id.clone();
                    move |_| {
                        let key = format!("remote:{conn_id}");
                        let mut c = collapsed.write();
                        if c.contains(&key) {
                            c.retain(|k| k != &key);
                            let loaded = remote_panes
                                .read()
                                .get(&conn_id)
                                .map(|p| p.loaded)
                                .unwrap_or(false);
                            drop(c);
                            if !loaded {
                                let state = state.clone();
                                let mut remote_panes = remote_panes.clone();
                                let cid = conn_id.clone();
                                spawn(async move {
                                    let tok = token(&state);
                                    let status = crate::api::api_check_remote_status(tok.clone(), cid.clone())
                                        .await
                                        .unwrap_or_default();
                                    let sessions = crate::api::api_remote_sessions(tok, cid.clone())
                                        .await
                                        .unwrap_or_default();
                                    remote_panes.write().insert(
                                        cid,
                                        RemotePane { status: Some(status), sessions, loaded: true },
                                    );
                                });
                            }
                        } else {
                            c.push(key);
                        }
                    }
                },
                span { class: "{chev_cls}", "▸" }
                span { class: "remote-badge", "📍" }
                span { class: "project-name", "{conn.name}" }
                span { class: "{dot_cls}", title: "{dot_title}" }
            }
            if !is_collapsed {
                {folder_els.iter().cloned()}
            }
        }
    }
}

/// One unified remote folder that has sessions (Nexus remote-cwd-group,
/// v0.13.1): collapsible header (chevron + count) with a ＋ that always
/// starts a new remote session in that directory; rows are the
/// connection's mirrored sessions and its unmirrored remote CLI
/// sessions.
#[allow(clippy::too_many_arguments)]
fn remote_folder_el(
    state: Signal<ClientState>,
    lang: String,
    conn_id: &str,
    wd: &str,
    items: &[RemoteItem],
    collapsed: Signal<Vec<String>>,
    current: Option<usize>,
    confirm: Signal<Option<ConfirmTarget>>,
    ctx: Signal<Option<CtxMenu>>,
) -> Element {
    let key = format!("remote:{conn_id}:{wd}");
    let is_collapsed = collapsed.read().contains(&key);
    let chev_cls = if is_collapsed {
        "project-chevron"
    } else {
        "project-chevron open"
    };
    let label = if wd.is_empty() {
        loc(&lang, "Bez projektu", "No project")
    } else {
        short_dir(wd)
    };
    let count = items.len().to_string();
    let rows: Vec<Element> = items
        .iter()
        .map(|it| match it {
            RemoteItem::Mirror(idx, m) => session_row(
                state.clone(),
                lang.clone(),
                *idx,
                m,
                current,
                confirm.clone(),
                ctx.clone(),
            ),
            RemoteItem::Cli(s) => remote_cli_el(state.clone(), lang.clone(), conn_id, s, ctx.clone()),
        })
        .collect();
    let wd2 = wd.to_string();
    let cid = conn_id.to_string();

    rsx! {
        div { class: "remote-cwd-group",
            button {
                class: "project-group-header remote-cwd-header",
                title: "{wd}",
                onclick: {
                    let mut collapsed = collapsed.clone();
                    let key = key.clone();
                    move |_| {
                        let mut c = collapsed.write();
                        if c.contains(&key) {
                            c.retain(|k| k != &key);
                        } else {
                            c.push(key.clone());
                        }
                    }
                },
                span { class: "{chev_cls}", "▸" }
                span { class: "project-name", "{label}" }
                span { class: "project-count", "{count}" }
                span {
                    class: "pgh-plus",
                    title: "{loc(&lang, \"Nowa sesja w tym katalogu\", \"New session in this directory\")}",
                    onclick: {
                        let state = state.clone();
                        let cid = cid.clone();
                        let wd = wd2.clone();
                        move |e| {
                            e.stop_propagation();
                            let state = state.clone();
                            let cid = cid.clone();
                            let wd = wd.clone();
                            spawn(async move {
                                let _ = crate::api::api_start_remote_session(token(&state), cid, wd).await;
                                crate::state::refresh(state.clone()).await;
                                crate::app::set_view(state, View::Chat);
                            });
                        }
                    },
                    "＋"
                }
            }
            if !is_collapsed {
                {rows.iter().cloned()}
            }
        }
    }
}

/// A configured folder without sessions (Nexus v0.13.1): a location
/// item — clicking it starts a new remote session there.
fn remote_loc_item_el(state: Signal<ClientState>, conn_id: &str, loc: &str) -> Element {
    let name = short_dir(loc);
    let title = loc.to_string();
    let conn_id = conn_id.to_string();
    let loc = loc.to_string();

    rsx! {
        button {
            class: "session-item remote-location-item",
            title: "{title}",
            onclick: {
                let state = state.clone();
                let conn_id = conn_id.clone();
                let loc = loc.clone();
                move |_| {
                    let state = state.clone();
                    let conn_id = conn_id.clone();
                    let loc = loc.clone();
                    spawn(async move {
                        let _ = crate::api::api_start_remote_session(token(&state), conn_id, loc).await;
                        crate::state::refresh(state.clone()).await;
                        crate::app::set_view(state, View::Chat);
                    });
                }
            },
            div { class: "session-item-content",
                div { class: "session-item-row",
                    span { class: "remote-loc-icon", "📁" }
                    span { class: "session-name", "{name}" }
                }
            }
        }
    }
}

/// A CLI session discovered on the remote host (opens a local mirror).
fn remote_cli_el(
    state: Signal<ClientState>,
    lang: String,
    conn_id: &str,
    s: &RemoteSessionInfo,
    ctx: Signal<Option<CtxMenu>>,
) -> Element {
    let name = if s.title.is_empty() {
        loc(&lang, "Sesja zdalna", "Remote session")
    } else {
        s.title.clone()
    };
    let dir = short_dir(&s.cwd);
    let date = if s.updated_at.len() >= 10 {
        s.updated_at[..10].to_string()
    } else {
        String::new()
    };
    let meta = if date.is_empty() {
        dir.clone()
    } else if dir.is_empty() {
        date
    } else {
        format!("{date} · {dir}")
    };
    let cid = conn_id.to_string();
    let sid = s.session_id.clone();
    let title = s.title.clone();
    let cwd = s.cwd.clone();
    let name = name.clone();

    rsx! {
        button {
            class: "session-item remote-location-item",
            title: "{cwd}",
            oncontextmenu: {
                let mut ctx = ctx.clone();
                let name = name.clone();
                let key = sid.clone();
                move |e| {
                    e.prevent_default();
                    let p = e.data().client_coordinates();
                    ctx.set(Some(CtxMenu {
                        x: p.x,
                        y: p.y,
                        target: ConfirmTarget {
                            is_project: false,
                            label: name.clone(),
                            key: key.clone(),
                        },
                    }));
                }
            },
            onclick: {
                let state = state.clone();
                let cid = cid.clone();
                let sid = sid.clone();
                let title = title.clone();
                let cwd = cwd.clone();
                move |_| {
                    let state = state.clone();
                    let cid = cid.clone();
                    let sid = sid.clone();
                    let title = title.clone();
                    let cwd = cwd.clone();
                    async move {
                        let _ = crate::api::api_open_remote_session(token(&state), cid, sid, title, cwd).await;
                        crate::state::refresh(state.clone()).await;
                        crate::app::set_view(state, View::Chat);
                    }
                }
            },
            div { class: "session-item-content",
                div { class: "session-item-row",
                    span { class: "remote-loc-icon", "⌨" }
                    span { class: "session-name", "{name}" }
                }
                div { class: "session-item-meta",
                    span { class: "session-date", "{meta}" }
                }
            }
        }
    }
}
