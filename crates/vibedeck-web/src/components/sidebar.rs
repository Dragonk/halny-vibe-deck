//! Session sidebar: local sessions grouped by project (working
//! directory), CLI (backend) sessions, hide-with-confirm and nav.
//! Ported from Nexus Sidebar.tsx (CSS classes identical).

use dioxus::prelude::*;
use vibedeck_core::snapshot::{RemoteConnection, RemoteSessionInfo, RemoteStatus, SessionMeta};

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

    // ---- Filter (hidden sessions/projects) + limit ----
    // Remote mirrors (SSH) are excluded here: they render inside their
    // connection group below.
    let mut visible: Vec<(usize, SessionMeta)> = sessions
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.remote_id.is_none()
                && {
                    let key = m.acp_session_id.clone().unwrap_or_else(|| m.id.clone());
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
                    let key = m.acp_session_id.clone().unwrap_or_else(|| m.id.clone());
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
            let pane = remote_panes.read().get(&conn.id).cloned().unwrap_or_default();
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

    rsx! {
        aside { class: "sidebar",
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
                                        .find(|(_, m)| m.message_count == 0 && m.acp_session_id.is_some())
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
    let key = m.acp_session_id.clone().unwrap_or_else(|| m.id.clone());
    let name = m.name.clone();
    let has_acp = m.acp_session_id.is_some();
    let empty = m.message_count == 0;
    let acp_id = m.acp_session_id.clone();
    let busy = m.busy;

    rsx! {
        button {
            class: "{cls}",
            onclick: {
                let state = state.clone();
                let acp_id = acp_id.clone();
                move |_| {
                    let state = state.clone();
                    let acp_id = acp_id.clone();
                    async move {
                        let tok = token(&state);
                        if empty && has_acp {
                            if let Some(sid) = acp_id.clone() {
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
    info: &vibedeck_core::proto::AcpSessionInfo,
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
/// remote-project-group): header with the live dot, quick-start
/// locations, remote CLI sessions and the connection's open mirrors
/// grouped per remote working directory.
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

    // quick-start locations
    let loc_els: Vec<Element> = conn
        .locations
        .iter()
        .map(|l| {
            let name = short_dir(l);
            let title = l.clone();
            rsx! {
                button {
                    class: "session-item remote-location-item",
                    title: "{title}",
                    onclick: {
                        let state = state.clone();
                        let conn_id = conn_id.clone();
                        let l = l.clone();
                        move |_| {
                            let state = state.clone();
                            let conn_id = conn_id.clone();
                            let l = l.clone();
                            spawn(async move {
                                let _ = crate::api::api_start_remote_session(token(&state), conn_id, l).await;
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
        })
        .collect();

    // mirrors grouped per remote working directory
    let mut wd_groups: Vec<(String, Vec<(usize, SessionMeta)>)> = Vec::new();
    for (idx, m) in &mirrors {
        let wd = m.working_directory.clone();
        match wd_groups.iter_mut().find(|(w, _)| w == &wd) {
            Some((_, list)) => list.push((*idx, m.clone())),
            None => wd_groups.push((wd, vec![(*idx, m.clone())])),
        }
    }
    let wd_els: Vec<Element> = wd_groups
        .iter()
        .map(|(wd, list)| {
            remote_wd_el(
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
        })
        .collect();

    // remote CLI sessions not yet mirrored locally
    let mirrored: Vec<String> = mirrors
        .iter()
        .filter_map(|(_, m)| m.acp_session_id.clone())
        .collect();
    let cli_els: Vec<Element> = pane
        .sessions
        .iter()
        .filter(|s| !mirrored.contains(&s.session_id))
        .map(|s| remote_cli_el(state.clone(), lang.clone(), &conn_id, s))
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
                {loc_els.iter().cloned()}
                {wd_els.iter().cloned()}
                {cli_els.iter().cloned()}
            }
        }
    }
}

/// One remote working-directory sub-group (Nexus remote-cwd-group):
/// header + the mirrored sessions + a "+" that starts a new remote
/// session in that directory.
#[allow(clippy::too_many_arguments)]
fn remote_wd_el(
    state: Signal<ClientState>,
    lang: String,
    conn_id: &str,
    wd: &str,
    list: &[(usize, SessionMeta)],
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
    let count = list.len().to_string();
    let rows: Vec<Element> = list
        .iter()
        .map(|(idx, m)| {
            session_row(state.clone(), lang.clone(), *idx, m, current, confirm.clone(), ctx.clone())
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

/// A CLI session discovered on the remote host (opens a local mirror).
fn remote_cli_el(
    state: Signal<ClientState>,
    lang: String,
    conn_id: &str,
    s: &RemoteSessionInfo,
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

    rsx! {
        button {
            class: "session-item remote-location-item",
            title: "{cwd}",
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
