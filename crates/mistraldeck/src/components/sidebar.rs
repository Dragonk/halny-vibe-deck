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
    /// Past sessions of this project from the local store (`session/list`
    /// or the on-disk index). Clicking one opens the lazy preview — no
    /// agent process (Z6).
    history: Vec<mistraldeck_core::proto::AgentSessionInfo>,
    newest: i64,
}

/// Epoch ms from an ISO-8601 timestamp (`2026-10-29T14:07:03Z`), enough to
/// sort project groups that only exist in the history.
fn ms_from_iso(ts: &str) -> i64 {
    let bytes = ts.as_bytes();
    if bytes.len() < 10 {
        return 0;
    }
    let num = |a: usize, b: usize| -> i64 {
        std::str::from_utf8(&bytes[a..b])
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let (y, m, d) = (num(0, 4), num(5, 7), num(8, 10));
    let (hh, mm, ss) = if bytes.len() >= 19 {
        (num(11, 13), num(14, 16), num(17, 19))
    } else {
        (0, 0, 0)
    };
    if y == 0 || m == 0 || d == 0 {
        return 0;
    }
    // days_from_civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400_000 + (hh * 3600 + mm * 60 + ss) * 1000
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
    // per-project "show all" expansion (default: max 10 rows per project)
    let more_expanded = use_signal(Vec::<String>::new);
    let ctx = use_signal(|| None::<CtxMenu>);
    // "New project…" picker (Z2): `np_remote` switches it to an SSH host.
    let np_open = use_signal(|| false);
    let np_remote = use_signal(|| None::<(String, String)>);

    let s = state.read();
    let lang = lang(&state);
    let sessions = s.snapshot.sessions.clone();
    let current = s.snapshot.current;
    let backend = s.snapshot.backend_sessions.clone();
    let cfg = s.snapshot.config.clone();
    let view = s.view;
    let cfg_desktop = s.desktop_mode;
    let start_path = s
        .snapshot
        .current
        .and_then(|i| s.snapshot.sessions.get(i))
        .map(|m| m.working_directory.clone())
        .filter(|wd| !wd.is_empty())
        .unwrap_or_default();
    drop(s);
    let new_project_lbl = tr(&lang, "newProjectLabel").to_string();

    let desktop_mode = cfg_desktop;
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
                history: Vec::new(),
                newest: meta.last_active_ms,
            }),
        }
    }

    // ---- Project history (Z6) ----
    // Past sessions from the local store belong to their project group, and
    // a project with only history still shows up. Nothing is spawned here:
    // a click opens the on-disk preview (lazy resume).
    let open_agent_ids: Vec<String> = visible
        .iter()
        .filter_map(|(_, m)| m.agent_session_id.clone())
        .collect();
    for info in backend.iter() {
        if info.session_id.is_empty()
            || open_agent_ids.iter().any(|id| id == &info.session_id)
            || cfg.hidden_sessions.iter().any(|h| h == &info.session_id)
        {
            continue;
        }
        let wd = info.cwd.clone();
        if !wd.is_empty() && cfg.hidden_projects.iter().any(|h| h == &wd) {
            continue;
        }
        let newest = ms_from_iso(&info.start_time);
        let label = if wd.is_empty() {
            tr(&lang, "noProject").to_string()
        } else {
            short_dir(&wd)
        };
        match groups.iter_mut().find(|g| g.wd == wd) {
            Some(g) => {
                g.history.push(info.clone());
                if newest > g.newest {
                    g.newest = newest;
                }
            }
            None => groups.push(Group {
                wd: wd.clone(),
                label,
                sessions: Vec::new(),
                history: vec![info.clone()],
                newest,
            }),
        }
    }
    groups.sort_by(|a, b| b.newest.cmp(&a.newest));

    // ---- Prebuilt element lists (avoids nested closure captures) ----
    let group_els: Vec<Element> = groups
        .iter()
        .map(|g| group_el(state, lang.clone(), g, current, confirm, collapsed, more_expanded, ctx))
        .collect();
    let remote_panes = use_signal(std::collections::HashMap::<String, RemotePane>::new);
    let remotes: Vec<RemoteConnection> = cfg
        .remote_connections
        .iter()
        .filter(|c| c.enabled)
        .cloned()
        .collect();
    // SSH groups start expanded, so the session list must be fetched on
    // mount — the old code only loaded the pane when the group was
    // re-expanded by a click, leaving remote sessions invisible.
    {
        let state = state.clone();
        let remote_panes = remote_panes.clone();
        let conns = remotes.clone();
        use_effect(move || {
            // Subscribe to the snapshot too, so a config change (new/edited
            // SSH connection) reloads the panes (round 5, R1).
            let _rev = state.read().snapshot.sessions.len();
            for conn in &conns {
                let loaded = remote_panes
                    .read()
                    .get(&conn.id)
                    .map(|p| p.loaded)
                    .unwrap_or(false);
                if loaded {
                    continue;
                }
                let state = state.clone();
                let mut remote_panes = remote_panes.clone();
                let cid = conn.id.clone();
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
        });
    }
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
                np_open,
                np_remote,
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
                    let state = state.clone();
                    move |_| { { let mut st = state.clone(); st.write().mobile_sidebar_open = false; } }
                },
                "✕"
            }
            // Stable v1.0.27: in the browser the title bar is gone, so the
            // sessions panel owns the brand row at its very top.
            if !desktop_mode {
                div { class: "sidebar-brand",
                    crate::components::brand::BrandMark { size: 18.0, class: "sidebar-brand-mark".to_string() }
                    span { class: "sidebar-brand-name", "{tr(&lang, \"appName\")}" }
                }
            }
            // D3: the project button lives next to the SESSIONS label,
            // right-aligned. A session always belongs to a project, so this
            // is the only global entry point (Z2/Z4).
            div { class: "sidebar-sessions-header",
                span { class: "sidebar-sessions-label", "{tr(&lang, \"sessions\")}" }
                button {
                    class: "sidebar-np-btn",
                    title: "{new_project_lbl}",
                    onclick: {
                        let mut np_open = np_open;
                        move |_| np_open.set(true)
                    },
                    span { class: "sidebar-np-text", "{new_project_lbl}" }
                    span { class: "sidebar-np-plus", "＋" }
                }
            }
            // Round 4 (B1): physically ONE scrollable list — project groups
            // and SSH connection groups share the same scroll container, so
            // expanded sessions can never push the SSH section off screen.
            nav { class: "session-list session-list-projects",
                if group_els.is_empty() && remote_els.is_empty() {
                    span { class: "session-empty", "{tr(&lang, \"noSessionsYet\")}" }
                }
                {group_els.iter().cloned()}
                if !remote_els.is_empty() {
                    div { class: "sidebar-sessions-label ssh-section-label", "SSH" }
                    {remote_els.iter().cloned()}
                }
            }
            nav { class: "sidebar-nav",
                button {
                    class: "{nav_cls(View::Stats)}",
                    onclick: {
                        let state = state.clone();
                        move |_| {
                            // On a phone the drawer must close when navigating
                            // to a full-screen view (settings/history/stats).
                            { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
                            crate::app::set_view(state, View::Stats);
                        }
                    },
                    "{tr(&lang, \"statistics\")}"
                }
                button {
                    class: "{nav_cls(View::History)}",
                    onclick: {
                        let state = state.clone();
                        move |_| {
                            { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
                            crate::app::set_view(state, View::History);
                        }
                    },
                    "{tr(&lang, \"historyLabel\")}"
                }
                button {
                    class: "{nav_cls(View::Settings)}",
                    onclick: {
                        let state = state.clone();
                        move |_| {
                            { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
                            crate::app::set_view(state, View::Settings);
                        }
                    },
                    "{tr(&lang, \"settings\")}"
                }
                // v1.0.32/33 parity: in the browser the brand row is gone
                // (the frameless title bar never existed on web), so the
                // theme toggle and logout live at the bottom of the
                // sessions list — under Settings, same as stable. The
                // desktop webview keeps the header theme toggle.
                if !desktop_mode {
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

        if *np_open.read() {
            NewProjectModal { open: np_open, remote: np_remote.read().clone(), start_path: start_path.clone() }
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
    // GUI extras for backend sessions: pin / archive / mark-read / delete.
    // Look the session up in the snapshot to label the toggles correctly.
    let sess = if c.target.is_project {
        None
    } else {
        let s = state.read();
        s.snapshot
            .backend_sessions
            .iter()
            .find(|i| i.session_id == c.target.key)
            .map(|i| {
                (
                    i.pinned_at.is_some(),
                    i.archived_at.is_some(),
                    i.is_unseen,
                )
            })
    };
    let key = c.target.key.clone();
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
                if let Some((pinned, archived, unseen)) = sess {
                    button {
                        class: "ctx-menu-item",
                        onclick: {
                            let state = state.clone();
                            let mut ctx = ctx.clone();
                            let key = key.clone();
                            move |_| {
                                ctx.set(None);
                                let state = state.clone();
                                let key = key.clone();
                                spawn(async move {
                                    let _ = crate::api::api_pin_session(token(&state), key, !pinned).await;
                                    crate::state::refresh(state).await;
                                });
                            }
                        },
                        if pinned { "📌 Unpin session" } else { "📌 Pin session" }
                    }
                    button {
                        class: "ctx-menu-item",
                        onclick: {
                            let state = state.clone();
                            let mut ctx = ctx.clone();
                            let key = key.clone();
                            move |_| {
                                ctx.set(None);
                                let state = state.clone();
                                let key = key.clone();
                                spawn(async move {
                                    let _ = crate::api::api_archive_session(token(&state), key, !archived).await;
                                    crate::state::refresh(state).await;
                                });
                            }
                        },
                        if archived { "📦 Unarchive session" } else { "📦 Archive session" }
                    }
                    if unseen {
                        button {
                            class: "ctx-menu-item",
                            onclick: {
                                let state = state.clone();
                                let mut ctx = ctx.clone();
                                let key = key.clone();
                                move |_| {
                                    ctx.set(None);
                                    let state = state.clone();
                                    let key = key.clone();
                                    spawn(async move {
                                        let _ = crate::api::api_mark_session_seen(token(&state), key).await;
                                        crate::state::refresh(state).await;
                                    });
                                }
                            },
                            "👁 Mark as read"
                        }
                    }
                    button {
                        class: "ctx-menu-item danger",
                        onclick: {
                            let state = state.clone();
                            let mut ctx = ctx.clone();
                            let key = key.clone();
                            move |_| {
                                ctx.set(None);
                                let state = state.clone();
                                let key = key.clone();
                                spawn(async move {
                                    let _ = crate::api::api_delete_backend_session(token(&state), key).await;
                                    crate::state::refresh(state).await;
                                });
                            }
                        },
                        "🗑 Delete session from disk"
                    }
                }
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
    more_expanded: Signal<Vec<String>>,
    ctx: Signal<Option<CtxMenu>>,
) -> Element {
    // Default: at most 10 rows per project group; a "show all" row expands.
    const GROUP_LIMIT: usize = 10;
    let all_expanded = more_expanded.read().contains(&g.wd);
    let mut rows: Vec<Element> = g
        .sessions
        .iter()
        .map(|(idx, m)| session_row(state.clone(), lang.clone(), *idx, m, current, confirm.clone(), ctx.clone()))
        .collect();
    // Z6: past sessions of this project, opened as a lazy preview.
    // Pinned first, archived last (GUI extra sorting).
    let mut hist = g.history.clone();
    hist.sort_by(|a, b| {
        let pa = a.pinned_at.is_some();
        let pb = b.pinned_at.is_some();
        pa.cmp(&pb).then_with(|| {
            let aa = a.archived_at.is_some();
            let ab = b.archived_at.is_some();
            aa.cmp(&ab)
        }).then_with(|| {
            ms_from_iso(&b.start_time).cmp(&ms_from_iso(&a.start_time))
        })
    });
    let mut hist_rows: Vec<Element> = hist
        .iter()
        .map(|info| backend_el(state.clone(), &lang, info, confirm.clone(), ctx.clone(), false))
        .collect();
    let total = rows.len() + hist_rows.len();
    let more_el: Element = if total > GROUP_LIMIT && !all_expanded {
        let remaining = total - GROUP_LIMIT;
        let wd = g.wd.clone();
        let mut more_expanded = more_expanded.clone();
        let lbl = loc(&lang, &format!("Pokaż wszystkie ({remaining} więcej)"), &format!("Show all ({remaining} more)"));
        rsx! {
            button {
                class: "session-item session-more-row",
                onclick: move |_| {
                    let mut e = more_expanded.write();
                    if e.contains(&wd) { e.retain(|k| *k != wd); } else { e.push(wd.clone()); }
                },
                span { class: "session-name", "{lbl}" }
            }
        }
    } else {
        VNode::empty()
    };
    if !all_expanded && total > GROUP_LIMIT {
        rows.truncate(GROUP_LIMIT.min(rows.len()));
        hist_rows.truncate(GROUP_LIMIT.saturating_sub(rows.len()));
    }
    let title = if g.wd.is_empty() {
        tr(&lang, "noProject").to_string()
    } else {
        g.wd.clone()
    };
    let count = (g.sessions.len() + g.history.len()).to_string();
    let new_here_lbl = tr(&lang, "newSessionHere").to_string();
    let new_here_wd = g.wd.clone();
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
                // Z3/D2: a new session always starts in a project.
                span {
                    class: "pgh-plus",
                    title: "{new_here_lbl}",
                    onclick: {
                        let state = state.clone();
                        let wd = new_here_wd.clone();
                        move |e| {
                            e.stop_propagation();
                            let state = state.clone();
                            let wd = wd.clone();
                            spawn(async move {
                                let _ = crate::api::api_create_session(token(&state), wd).await;
                                crate::state::refresh(state.clone()).await;
                                { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
                                crate::app::set_view(state, View::Chat);
                            });
                        }
                    },
                    "＋"
                }
            }
            if !is_collapsed {
                {rows.iter().cloned()}
                {hist_rows.iter().cloned()}
                {more_el}
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

    // Round 4 (B4): the empty span rendered nothing — use the stable
    // edition's SVG arc spinner so a running session is visibly marked.
    let busy_el: Element = if m.busy {
        rsx! {
            svg {
                class: "session-busy-spinner",
                xmlns: "http://www.w3.org/2000/svg",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                title { "streaming" }
                path { d: "M21 12a9 9 0 1 1-6.219-8.56" }
            }
        }
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
    let is_remote = m.remote_id.is_some();

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
                        // Round 4 (E1): a remote (SSH) mirror already IS the
                        // remote session — switching to it must never go
                        // through the LOCAL backend open (that produced
                        // "session/resume: Session not found: <remote id>").
                        if is_remote {
                            let _ = crate::api::api_switch_session(tok, idx).await;
                        } else if empty && has_agent {
                            if let Some(sid) = agent_id.clone() {
                                let _ = crate::api::api_open_backend_session(tok, sid).await;
                            }
                        } else {
                            let _ = crate::api::api_switch_session(tok, idx).await;
                        }
                        crate::state::refresh(state).await;
                        { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
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
                    // round 12: green dot marks the ACTIVE session (stable parity)
                    if active {
                        span { class: "session-active-dot", title: "{tr(&lang, \"activeSession\")}" }
                    }
                    span { class: "session-name", "{name}" }
                    {busy_el}
                }
                div { class: "session-item-meta",
                    span { class: "session-date", "{date}" }
                }
            }
            // round 12: ✕ is visible ONLY on a running session and stops it
            // completely (cancel + kill the agent slot with subprocesses);
            // the menu itself lives under right-click (PPM)
            if busy {
                span {
                    class: "session-cancel-btn",
                    title: "{tr(&lang, \"stopSessionLabel\")}",
                    onclick: {
                        let state = state.clone();
                        let key = key.clone();
                        move |e| {
                            e.stop_propagation();
                            let state = state.clone();
                            let key = key.clone();
                            spawn(async move {
                                let _ = crate::api::api_cancel(token(&state)).await;
                                let _ = crate::api::api_stop_session(token(&state), key).await;
                                crate::state::refresh(state).await;
                            });
                        }
                    },
                    "✕"
                }
            }
        }
    }
}

/// One history row (`session/list` or the on-disk index). `show_dir` adds
/// the project directory to the meta line — off inside project groups.
fn backend_el(
    state: Signal<ClientState>,
    lang: &str,
    info: &mistraldeck_core::proto::AgentSessionInfo,
    confirm: Signal<Option<ConfirmTarget>>,
    ctx: Signal<Option<CtxMenu>>,
    show_dir: bool,
) -> Element {
    let sid = info.session_id.clone();
    // Round 4 (B3): never show a bare "(CLI)" placeholder — the server
    // enriches the name (index title / first prompt) before it arrives.
    let name = if info.name.is_empty() {
        tr(lang, "untitledSession").to_string()
    } else {
        info.name.clone()
    };
    let dir = short_dir(&info.cwd);
    let date = if info.start_time.len() >= 10 {
        info.start_time[..10].to_string()
    } else {
        String::new()
    };
    // GUI extras beyond the TUI: pinned/archive/unseen/worktree badges
    let pinned = info.pinned_at.is_some();
    let archived = info.archived_at.is_some();
    let unseen = info.is_unseen;
    let worktree = info.worktree.clone().unwrap_or_default();
    let cls = if archived {
        "session-item archived"
    } else {
        "session-item"
    };

    rsx! {
        button {
            class: "{cls}",
            onclick: {
                let state = state.clone();
                let sid = sid.clone();
                move |_| {
                    let state = state.clone();
                    let sid = sid.clone();
                    async move {
                        let _ = crate::api::api_open_backend_session(token(&state), sid).await;
                        crate::state::refresh(state).await;
                        { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
                                crate::app::set_view(state, View::Chat);
                    }
                }
            },
            oncontextmenu: {
                let mut ctx = ctx.clone();
                let sid = sid.clone();
                let name = name.clone();
                move |e| {
                    e.prevent_default();
                    let p = e.data().client_coordinates();
                    ctx.set(Some(CtxMenu {
                        x: p.x,
                        y: p.y,
                        target: ConfirmTarget {
                            is_project: false,
                            label: name.clone(),
                            key: sid.clone(),
                        },
                    }));
                }
            },
            div { class: "session-item-content",
                div { class: "session-item-row",
                    if pinned {
                        span { class: "session-badge", title: "pinned", "📌" }
                    }
                    if unseen {
                        span { class: "session-badge unseen-dot", title: "unread activity", "●" }
                    }
                    if !worktree.is_empty() {
                        span { class: "session-badge worktree-badge", title: "{worktree}", "🌿 {worktree}" }
                    }
                    span { class: "session-name", "{name}" }
                }
                div { class: "session-item-meta",
                    if show_dir {
                        span { class: "session-date", "{date} · {dir}" }
                    } else {
                        span { class: "session-date", "{date}" }
                    }
                    if archived {
                        span { class: "session-badge", title: "archived", "📦" }
                    }
                }
            }
            // round 12: history rows have no ✕ — hide/delete live under PPM
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
    np_open: Signal<bool>,
    np_remote: Signal<Option<(String, String)>>,
) -> Element {
    let conn_id = conn.id.clone();
    let group_key = format!("remote:{conn_id}");
    let is_collapsed = collapsed.read().contains(&group_key);
    let chev_cls = if is_collapsed {
        "project-chevron"
    } else {
        "project-chevron open"
    };
    let np_lbl = tr(&lang, "newProjectLabel").to_string();
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
                            drop(c);
                            // Expanding always re-lists the remote sessions
                            // (round 5, R1): the list must be fresh, not only
                            // on first load.
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
                        } else {
                            c.push(key);
                        }
                    }
                },
                span { class: "{chev_cls}", "▸" }
                span { class: "remote-badge", "📍" }
                span { class: "project-name", "{conn.name}" }
                span { class: "{dot_cls}", title: "{dot_title}" }
                // D4: same "New project…" entry as the local sessions header.
                span {
                    class: "pgh-plus",
                    title: "{np_lbl}",
                    onclick: {
                        let mut np_open = np_open;
                        let mut np_remote = np_remote;
                        let conn_id = conn_id.clone();
                        let conn_name = conn.name.clone();
                        let default_loc = conn.locations.first().cloned().unwrap_or_default();
                        move |e| {
                            e.stop_propagation();
                            np_remote.set(Some((conn_id.clone(), conn_name.clone())));
                            let _ = default_loc.clone();
                            np_open.set(true);
                        }
                    },
                    "＋"
                }
            }
            if !is_collapsed {
                // Round 5 (R1): when the lister ran but the host is
                // unreachable, say so instead of showing an empty group.
                if pane.loaded
                    && pane.sessions.is_empty()
                    && mirrors.is_empty()
                    && pane.status.as_ref().map(|s| !s.ok).unwrap_or(false)
                {
                    div { class: "session-empty",
                        "{loc(&lang, \"Nie można połączyć się z hostem (sprawdź ssh)\", \"Cannot reach the host (check the ssh command)\")}"
                    }
                } else if pane.loaded && pane.sessions.is_empty() && mirrors.is_empty() && folder_els.is_empty() {
                    div { class: "session-empty",
                        "{loc(&lang, \"Brak sesji na zdalnym hoście\", \"No sessions on the remote host\")}"
                    }
                }
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
                                { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
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
                        { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
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
                        { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
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

/// "New project…" picker (Z2, D3/D4).
///
/// Local mode browses the filesystem (`api_browse_dirs` — not confined to the
/// engine working directory, choosing a project means the whole disk); SSH
/// mode takes a path on the remote host. Creating a session here is the only
/// way to start one: a session always belongs to a project (D1).
#[component]
fn NewProjectModal(
    /// Set to `false` to close the dialog.
    open: Signal<bool>,
    /// `(connection id, display name)` — `None` = local project.
    remote: Option<(String, String)>,
    /// Initial directory (local) or remote path.
    start_path: String,
) -> Element {
    let state = use_context::<Signal<ClientState>>();
    let lang = lang(&state);
    let mut path = use_signal(|| start_path.clone());
    let mut dirs = use_signal(Vec::<crate::fsgit::FileEntry>::new);
    let name = use_signal(String::new);
    let mut err = use_signal(String::new);
    let mut busy = use_signal(|| false);
    let mut loaded = use_signal(|| false);
    let is_remote = remote.is_some();

    // Local: start in the session's project, else the home directory.
    if !is_remote && !*loaded.read() {
        loaded.set(true);
        let initial = start_path.clone();
        spawn(async move {
            let base = if initial.is_empty() {
                crate::fsgit::api_home_dir(token(&state)).await.unwrap_or_default()
            } else {
                initial
            };
            path.set(base.clone());
            match crate::fsgit::api_browse_dirs(token(&state), base).await {
                Ok(v) => dirs.set(v),
                Err(e) => err.set(e.to_string()),
            }
        });
    }

    let parent = {
        let p = path.read().clone();
        let trimmed = p.trim_end_matches('/');
        match trimmed.rfind('/') {
            Some(0) => "/".to_string(),
            Some(i) => trimmed[..i].to_string(),
            None => String::new(),
        }
    };

    let dir_rows: Vec<Element> = dirs
        .read()
        .iter()
        .map(|e| {
            let target = e.path.clone();
            let label = e.name.clone();
            rsx! {
                button {
                    key: "{target}",
                    class: "np-dir-row",
                    onclick: {
                        let mut path = path;
                        let mut dirs = dirs;
                        let mut err = err;
                        let state = state.clone();
                        move |_| {
                            let target = target.clone();
                            path.set(target.clone());
                            let state = state.clone();
                            spawn(async move {
                                match crate::fsgit::api_browse_dirs(token(&state), target).await {
                                    Ok(v) => dirs.set(v),
                                    Err(e) => err.set(e.to_string()),
                                }
                            });
                        }
                    },
                    "📁 {label}"
                }
            }
        })
        .collect();

    let remote_clone = remote.clone();
    let create = {
        let state = state.clone();
        let remote = remote_clone.clone();
        move |_| {
            let state = state.clone();
            let remote = remote.clone();
            let base = path.read().trim().to_string();
            let extra = name.read().trim().to_string();
            busy.set(true);
            spawn(async move {
                let mut target = base;
                if !extra.is_empty() {
                    target = format!("{}/{}", target.trim_end_matches('/'), extra);
                }
                if target.is_empty() {
                    err.set("…".to_string());
                    busy.set(false);
                    return;
                }
                let res = match remote.clone() {
                    Some((conn_id, _)) => {
                        // Remote: the engine creates the session over SSH; a
                        // missing directory surfaces as an error here.
                        crate::api::api_start_remote_session(token(&state), conn_id, target).await
                    }
                    None => {
                        if extra.is_empty() {
                            crate::api::api_create_session(token(&state), target).await
                        } else {
                            match crate::fsgit::api_create_dir(token(&state), target.clone()).await {
                                Ok(()) => {
                                    crate::api::api_create_session(token(&state), target).await
                                }
                                Err(e) => Err(e),
                            }
                        }
                    }
                };
                match res {
                    Ok(()) => {
                        crate::state::refresh(state.clone()).await;
                        { let mut st = state.clone(); st.write().mobile_sidebar_open = false; }
                                crate::app::set_view(state, View::Chat);
                        open.set(false);
                    }
                    Err(e) => err.set(e.to_string()),
                }
                busy.set(false);
            });
        }
    };

    let title = match &remote {
        Some((_, n)) => format!("{} · {n}", tr(&lang, "newProjectTitle")),
        None => tr(&lang, "newProjectTitle").to_string(),
    };
    let close = {
        let mut open = open;
        move |_| open.set(false)
    };

    rsx! {
        div { class: "modal-overlay", onclick: close,
            div { class: "modal-box np-modal", onclick: move |e| e.stop_propagation(),
                div { class: "modal-title", "{title}" }
                div { class: "np-path-row",
                    span { class: "np-path", title: "{path}", "{path}" }
                    if !is_remote {
                        button {
                            class: "field-btn",
                            disabled: parent.is_empty(),
                            onclick: {
                                let mut path = path;
                                let mut dirs = dirs;
                                let state = state.clone();
                                let parent = parent.clone();
                                move |_| {
                                    let target = parent.clone();
                                    path.set(target.clone());
                                    let state = state.clone();
                                    spawn(async move {
                                        if let Ok(v) = crate::fsgit::api_browse_dirs(token(&state), target).await {
                                            dirs.set(v);
                                        }
                                    });
                                }
                            },
                            "↑ {tr(&lang, \"upLabel\")}"
                        }
                    }
                }
                if !is_remote {
                    div { class: "np-dir-list", {dir_rows.iter().cloned()} }
                }
                div { class: "field",
                    div { class: "field-label",
                        if is_remote { "{tr(&lang, \"remotePathLabel\")}" } else { "{tr(&lang, \"newProjectName\")}" }
                    }
                    input {
                        class: "field-input",
                        r#type: "text",
                        value: if is_remote { path.read().clone() } else { name.read().clone() },
                        oninput: {
                            let mut name = name;
                            let mut path = path;
                            let remote_mode = is_remote;
                            move |e| {
                                if remote_mode {
                                    path.set(e.value());
                                } else {
                                    name.set(e.value());
                                }
                            }
                        },
                    }
                }
                if !err.read().is_empty() {
                    div { class: "git-error", "{err.read()}" }
                }
                div { class: "modal-actions",
                    button {
                        class: "modal-btn",
                        onclick: { let mut open = open; move |_| open.set(false) },
                        "{tr(&lang, \"cancelLabel\")}"
                    }
                    button {
                        class: "modal-btn modal-btn-primary",
                        disabled: *busy.read(),
                        onclick: create,
                        if *busy.read() { "…" } else { "{tr(&lang, \"newProjectUse\")}" }
                    }
                }
            }
        }
    }
}
