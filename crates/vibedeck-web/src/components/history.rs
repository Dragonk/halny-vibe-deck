//! History view: search, time filters, hidden sessions/projects with
//! restore, and CLI (backend) sessions with reactivation.
//! Ported from Nexus HistoryView (same CSS classes).

use dioxus::prelude::*;

use crate::i18n::tr;
use crate::state::{lang, token, ClientState, View};

pub fn HistoryView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut query = use_signal(String::new);
    let mut time_filter = use_signal(|| "all".to_string());
    let mut show_hidden = use_signal(|| false);

    let s = state.read();
    let lang = lang(&state);
    let sessions = s.snapshot.sessions.clone();
    let backend = s.snapshot.backend_sessions.clone();
    let hidden_sessions = s.snapshot.config.hidden_sessions.clone();
    let hidden_projects = s.snapshot.config.hidden_projects.clone();
    drop(s);

    let q = query.read().to_lowercase();
    let days = match time_filter.read().as_str() {
        "30d" => Some(30i64),
        "90d" => Some(90),
        "180d" => Some(180),
        _ => None,
    };
    let now = vibedeck_core::types::now_ms();
    let cutoff = days.map(|d| now - d * 86_400_000);

    let mut rows: Vec<Row> = Vec::new();
    for m in &sessions {
        let key = m.acp_session_id.clone().unwrap_or_else(|| m.id.clone());
        let hidden = hidden_sessions.iter().any(|h| h == &key)
            || (!m.working_directory.is_empty()
                && hidden_projects.iter().any(|h| h == &m.working_directory));
        rows.push(Row {
            key,
            title: if m.name.is_empty() { tr(&lang, "untitledSession").to_string() } else { m.name.clone() },
            meta: date_str(m.last_active_ms),
            detail: format!("{} · {}", m.message_count, tr(&lang, "messagesLabel")),
            dir: m.working_directory.clone(),
            hidden,
            past_cutoff: cutoff.map(|c| m.last_active_ms < c).unwrap_or(false),
            backend: None,
        });
    }
    for info in &backend {
        let hidden = hidden_sessions.iter().any(|h| h == &info.session_id);
        if rows.iter().any(|r| r.key == info.session_id) {
            continue;
        }
        rows.push(Row {
            key: info.session_id.clone(),
            title: if info.name.is_empty() { tr(&lang, "untitledSession").to_string() } else { info.name.clone() },
            meta: if info.start_time.len() >= 10 { info.start_time[..10].to_string() } else { String::new() },
            detail: "CLI".to_string(),
            dir: info.cwd.clone(),
            hidden,
            past_cutoff: false,
            backend: Some(info.session_id.clone()),
        });
    }

    let visible: Vec<&Row> = rows
        .iter()
        .filter(|r| {
            (!r.hidden || *show_hidden.read())
                && !r.past_cutoff
                && (q.is_empty() || r.title.to_lowercase().contains(&q))
        })
        .collect();

    let pill_cls = |name: &str| {
        if *time_filter.read() == name {
            "scope-pill active".to_string()
        } else {
            "scope-pill".to_string()
        }
    };

    rsx! {
        div { class: "history-view",
            div { class: "history-header",
                h2 { class: "settings-title", "{tr(&lang, \"historyLabel\")}" }
                div { class: "history-controls",
                    input {
                        class: "field-input history-search",
                        placeholder: "{tr(&lang, \"searchTitlePlaceholder\")}",
                        value: "{query()}",
                        oninput: move |e| query.set(e.value()),
                    }
                    div { class: "scope-switch",
                        button { class: "{pill_cls(\"30d\")}", onclick: move |_| time_filter.set("30d".into()), "{tr(&lang, \"time30d\")}" }
                        button { class: "{pill_cls(\"90d\")}", onclick: move |_| time_filter.set("90d".into()), "{tr(&lang, \"time90d\")}" }
                        button { class: "{pill_cls(\"180d\")}", onclick: move |_| time_filter.set("180d".into()), "{tr(&lang, \"time180d\")}" }
                        button { class: "{pill_cls(\"all\")}", onclick: move |_| time_filter.set("all".into()), "{tr(&lang, \"timeAll\")}" }
                    }
                    label { class: "history-show-hidden checkbox-label",
                        input {
                            r#type: "checkbox",
                            checked: *show_hidden.read(),
                            onchange: move |e| show_hidden.set(e.checked()),
                        }
                        "{tr(&lang, \"showHiddenLabel\")}"
                    }
                }
            }
            div { class: "history-content",
                if !hidden_projects.is_empty() {
                    div { class: "history-hidden-projects",
                        div { class: "section-label", "{tr(&lang, \"hiddenProjectsLabel\")}" }
                        for wd in hidden_projects.iter() {
                            {
                                let state = state.clone();
                                let wd = wd.clone();
                                let wd2 = wd.clone();
                                rsx! {
                                    div { class: "history-hidden-row",
                                        span { class: "history-hidden-path", "{wd}" }
                                        button {
                                            class: "history-restore-btn",
                                            onclick: {
                                                let state = state.clone();
                                                let wd2 = wd2.clone();
                                                move |_| {
                                                    let state = state.clone();
                                                    let wd2 = wd2.clone();
                                                    async move {
                                                        let _ = crate::api::api_restore_project(token(&state), wd2).await;
                                                        crate::state::refresh(state).await;
                                                    }
                                                }
                                            },
                                            "{tr(&lang, \"restoreLabel\")}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "session-rows",
                    if visible.is_empty() {
                        div { class: "empty-text", "{tr(&lang, \"noSessionsFound\")}" }
                    }
                    for row in visible.iter() {
                        { row_el(state.clone(), lang.clone(), (*row).clone()) }
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct Row {
    key: String,
    title: String,
    meta: String,
    detail: String,
    dir: String,
    hidden: bool,
    past_cutoff: bool,
    backend: Option<String>,
}

fn row_el(state: Signal<ClientState>, lang: String, row: Row) -> Element {
    let title = row.title.clone();
    let meta = row.meta.clone();
    let detail = row.detail.clone();
    let dir = row.dir.clone();
    let dir_short = dir.rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or("").to_string();
    let backend = row.backend.clone();
    let key = row.key.clone();

    rsx! {
        div { class: "session-row",
            div { class: "session-row-main",
                onclick: {
                    let state = state.clone();
                    let backend = backend.clone();
                    move |_| {
                        let state = state.clone();
                        let backend = backend.clone();
                        async move {
                            if let Some(sid) = backend.clone() {
                                let _ = crate::api::api_open_backend_session(token(&state), sid).await;
                            }
                            crate::state::refresh(state).await;
                            crate::app::set_view(state, View::Chat);
                        }
                    }
                },
                div { class: "session-row-title", "{title}" }
                div { class: "session-row-meta", "{meta} · {detail}" }
            }
            div { class: "session-row-tags",
                if !dir_short.is_empty() {
                    span { class: "tag primary", "{dir_short}" }
                }
                if row.hidden {
                    span { class: "tag accent", "hidden" }
                }
            }
            div { class: "history-row-actions",
                if row.hidden {
                    {
                        let state = state.clone();
                        let key = key.clone();
                        rsx! {
                            button {
                                class: "history-restore-btn",
                                onclick: {
                                    let state = state.clone();
                                    let key = key.clone();
                                    move |_| {
                                        let state = state.clone();
                                        let key = key.clone();
                                        async move {
                                            let _ = crate::api::api_restore_session(token(&state), key).await;
                                            crate::state::refresh(state).await;
                                        }
                                    }
                                },
                                "{tr(&lang, \"restoreLabel\")}"
                            }
                        }
                    }
                } else {
                    {
                        let state = state.clone();
                        let key = key.clone();
                        rsx! {
                            button {
                                class: "history-reactivate-btn",
                                onclick: {
                                    let state = state.clone();
                                    let key = key.clone();
                                    move |_| {
                                        let state = state.clone();
                                        let key = key.clone();
                                        async move {
                                            let _ = crate::api::api_hide_session(token(&state), key).await;
                                            crate::state::refresh(state).await;
                                        }
                                    }
                                },
                                "🗑"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn date_str(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}.{y}")
}

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
