//! Session sidebar: local sessions grouped by project (working
//! directory), CLI (backend) sessions, hide-with-confirm and nav.
//! Ported from Nexus Sidebar.tsx (CSS classes identical).

use dioxus::prelude::*;
use vibedeck_core::snapshot::SessionMeta;

use crate::i18n::{tf, tr};
use crate::state::{lang, token, ClientState, View};

#[derive(Clone)]
struct ConfirmTarget {
    /// false = session, true = project
    is_project: bool,
    label: String,
    key: String,
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
    let mut visible: Vec<(usize, SessionMeta)> = sessions
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            let key = m.acp_session_id.clone().unwrap_or_else(|| m.id.clone());
            !cfg.hidden_sessions.iter().any(|h| h == &key)
                && (m.working_directory.is_empty()
                    || !cfg.hidden_projects.iter().any(|h| h == &m.working_directory))
        })
        .map(|(i, m)| (i, m.clone()))
        .collect();
    visible.sort_by(|a, b| b.1.last_active_ms.cmp(&a.1.last_active_ms));
    if let Some(limit) = cfg.sidebar_sessions_limit {
        if limit > 0 {
            visible.truncate(limit as usize);
        }
    }

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
                        let (x, y) = e.data().client_coordinates();
                        ctx.set(Some(CtxMenu {
                            x,
                            y,
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
                    let (x, y) = e.data().client_coordinates();
                    ctx.set(Some(CtxMenu {
                        x,
                        y,
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
                            let (x, y) = e.data().client_coordinates();
                            ctx.set(Some(CtxMenu {
                                x,
                                y,
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
