//! History view: title/content search (debounced, min 2 chars), time and
//! project filters, hidden sessions/projects with restore, and lazy rows
//! from the Vibe CLI stats store (Nexus `HistoryView` parity). Rows come
//! from `api_vibe_stats` (disk sessions) plus remote CLI sessions of every
//! enabled SSH connection (Nexus remoteRows - remote work shows here too);
//! reactivation refreshes the CLI session list first so any disk session
//! can be opened, then goes through `api_open_backend_session` (remote
//! rows through the lazy `api_open_remote_session`) like the sidebar.

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{lang, token, ClientState, View};
use crate::vibestats::{ContentMatchView, SessionStatsRow};

/// Rows render lazily: PAGE_SIZE at a time, grown by the "load more" row
/// (Nexus grows the window with an intersection-observer sentinel).
const PAGE_SIZE: usize = 100;

pub fn HistoryView() -> Element {
    let state = use_context::<Signal<ClientState>>();
    let mut loaded = use_signal(|| false);
    let mut fetched = use_signal(|| false);
    let mut rows = use_signal(|| Vec::<SessionStatsRow>::new());
    let mut remote_rows = use_signal(|| Vec::<RemoteHist>::new());
    let mut query = use_signal(String::new);
    // 0 = title only, 1 = title + message content.
    let mut search_mode = use_signal(|| 0u8);
    let mut time_filter = use_signal(|| "all".to_string());
    let mut show_hidden = use_signal(|| false);
    let mut project_filter = use_signal(String::new);
    let mut content_matches = use_signal(|| Vec::<ContentMatchView>::new());
    let mut searching = use_signal(|| false);
    let mut search_gen = use_signal(|| 0u32);
    let mut visible_count = use_signal(|| PAGE_SIZE);

    let s = state.read();
    let lang = lang(&state);
    let hidden_sessions = s.snapshot.config.hidden_sessions.clone();
    let hidden_projects = s.snapshot.config.hidden_projects.clone();
    drop(s);

    if !*loaded.read() {
        loaded.set(true);
        let st = state.clone();
        spawn(async move {
            if let Ok(r) = crate::vibestats::api_vibe_stats(token(&st)).await {
                rows.set(r);
            }
            // Remote rows (Nexus HistoryView remoteRows): CLI sessions of
            // every enabled connection - remote work shows in History too.
            let conns: Vec<(String, String)> = {
                let s = st.read();
                s.snapshot
                    .config
                    .remote_connections
                    .iter()
                    .filter(|c| c.enabled)
                    .map(|c| (c.id.clone(), c.name.clone()))
                    .collect()
            };
            let mut remote: Vec<RemoteHist> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (cid, cname) in conns {
                if let Ok(list) = crate::api::api_remote_sessions(token(&st), cid.clone()).await {
                    for s in list {
                        if s.session_id.is_empty() {
                            continue;
                        }
                        if !seen.insert(format!("{cid}:{}", s.session_id)) {
                            continue;
                        }
                        remote.push(RemoteHist {
                            conn_id: cid.clone(),
                            conn_name: cname.clone(),
                            session_id: s.session_id,
                            title: s.title,
                            cwd: s.cwd,
                            updated_at: s.updated_at.clone(),
                            updated_ms: s
                                .updated_at
                                .parse::<i64>()
                                .ok()
                                .or_else(|| iso_to_ms(&s.updated_at)),
                        });
                    }
                }
            }
            remote_rows.set(remote);
            fetched.set(true);
        });
    }

    let all = rows.read().clone();
    let remote_all = remote_rows.read().clone();
    let loading = !*fetched.read();
    let q = query.read().trim().to_lowercase();
    let q_len = q.chars().count();
    let mode = *search_mode.read();
    let pf = project_filter.read().clone();
    let days = match time_filter.read().as_str() {
        "30d" => Some(30i64),
        "90d" => Some(90),
        "180d" => Some(180),
        _ => None,
    };
    let now = mistraldeck_core::types::now_ms();
    let cutoff = days.map(|d| now - d * 86_400_000);

    // Project filter list (Nexus projectList): every working directory,
    // hidden projects only while "show hidden" is on, sorted.
    let mut projects: Vec<String> = Vec::new();
    for r in &all {
        if r.working_directory.is_empty() {
            continue;
        }
        if !*show_hidden.read() && hidden_projects.iter().any(|h| h == &r.working_directory) {
            continue;
        }
        if !projects.contains(&r.working_directory) {
            projects.push(r.working_directory.clone());
        }
    }
    for r in &remote_all {
        if r.cwd.is_empty() {
            continue;
        }
        if !*show_hidden.read() && hidden_projects.iter().any(|h| h == &r.cwd) {
            continue;
        }
        if !projects.contains(&r.cwd) {
            projects.push(r.cwd.clone());
        }
    }
    projects.sort();

    // Content-search hits as a lookup map (first match per session).
    let mut snips: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for m in content_matches.read().iter() {
        snips.insert(m.session_id.clone(), m.snippet.clone());
    }

    // Rows (Nexus filtered): hidden + project + cutoff + query filters,
    // newest first by start time.
    let mut rows_out: Vec<Row> = Vec::new();
    for r in &all {
        let hidden = hidden_sessions.iter().any(|h| h == &r.session_id)
            || (!r.working_directory.is_empty()
                && hidden_projects.iter().any(|h| h == &r.working_directory));
        let t0 = iso_to_ms(&r.start_time);
        let past = cutoff
            .map(|c| t0.map(|t| t < c).unwrap_or(false))
            .unwrap_or(false);
        if past || (!*show_hidden.read() && hidden) {
            continue;
        }
        if !pf.is_empty() && r.working_directory != pf {
            continue;
        }
        if !q.is_empty() {
            let in_title = r.title.to_lowercase().contains(&q);
            let in_content = mode == 1 && q_len >= 2 && snips.contains_key(&r.session_id);
            if !in_title && !in_content {
                continue;
            }
        }
        let title = if r.title.is_empty() {
            tr(&lang, "untitledSession").to_string()
        } else {
            r.title.clone()
        };
        let dir_short = if r.working_directory.is_empty() {
            "—".to_string()
        } else {
            base_name(&r.working_directory)
        };
        let duration_ms = iso_to_ms(&r.end_time)
            .unwrap_or(0)
            .saturating_sub(t0.unwrap_or(0));
        rows_out.push(Row {
            key: r.session_id.clone(),
            title,
            date: iso_date(&r.start_time),
            duration: format_duration(duration_ms as f64 / 1000.0),
            messages: r.messages.to_string(),
            cost: format!("{:.2}", r.cost),
            dir: r.working_directory.clone(),
            dir_short,
            model: if r.model.is_empty() {
                "unknown".to_string()
            } else {
                r.model.clone()
            },
            hidden,
            sort_ms: t0.unwrap_or(0),
            remote: None,
        });
    }
    // Remote rows (Nexus HistoryView remoteRows): hidden + project +
    // cutoff + title-query filters, newest first by updatedAt.
    let remote_word = if lang == "pl" { "Zdalne" } else { "Remote" };
    for r in &remote_all {
        let hidden = hidden_sessions.iter().any(|h| h == &r.session_id)
            || (!r.cwd.is_empty() && hidden_projects.iter().any(|h| h == &r.cwd));
        let t0 = r.updated_ms.unwrap_or(0);
        let past = cutoff.map(|c| t0 < c).unwrap_or(false);
        if past || (!*show_hidden.read() && hidden) {
            continue;
        }
        if !pf.is_empty() && r.cwd != pf {
            continue;
        }
        if !q.is_empty() && !r.title.to_lowercase().contains(&q) {
            continue;
        }
        let title = if r.title.is_empty() {
            tr(&lang, "untitledSession").to_string()
        } else {
            r.title.clone()
        };
        let dir_short = if r.cwd.is_empty() {
            "\u{2014}".to_string()
        } else {
            base_name(&r.cwd)
        };
        rows_out.push(Row {
            key: format!("{}:{}", r.conn_id, r.session_id),
            title,
            date: iso_date(&r.updated_at),
            duration: "\u{2014}".to_string(),
            messages: "\u{2014}".to_string(),
            cost: "0.00".to_string(),
            dir: r.cwd.clone(),
            dir_short,
            model: remote_word.to_string(),
            hidden,
            sort_ms: t0,
            remote: Some(r.clone()),
        });
    }
    rows_out.sort_by(|a, b| b.sort_ms.cmp(&a.sort_ms));

    let shown = (*visible_count.read()).min(rows_out.len());
    let total = rows_out.len();
    let counter = tf(
        &lang,
        "historyShowingCount",
        &[("shown", &shown.to_string()), ("total", &total.to_string())],
    );

    let row_els: Vec<Element> = rows_out
        .iter()
        .take(*visible_count.read())
        .map(|r| {
            let snippet = if mode == 1 && q_len >= 2 {
                snips.get(&r.key).cloned()
            } else {
                None
            };
            row_el(state.clone(), lang.clone(), r.clone(), snippet)
        })
        .collect();

    // Project select options (precomputed: rsx cannot build them inline).
    let cur_pf = pf.clone();
    let lang_sel = lang.clone();
    let opt_els: Vec<Element> = std::iter::once(rsx! {
        option { value: "", selected: cur_pf.is_empty(), "{tr(&lang_sel, \"allProjects\")}" }
    })
    .chain(projects.iter().map(|wd| {
        let selected = *wd == cur_pf;
        rsx! { option { key: "{wd}", value: "{wd}", selected: selected, title: "{wd}", "{base_name(wd)}" } }
    }))
    .collect();

    // "Load more" row: counter + click to grow the window (the Dioxus
    // substitute for Nexus's intersection-observer sentinel).
    let load_more_el: Element = if total > shown {
        rsx! {
            div {
                class: "history-load-more history-more",
                onclick: move |_| {
                    let n = *visible_count.read();
                    visible_count.set(n + PAGE_SIZE);
                },
                "{counter} · {tr(&lang, \"loadMoreLabel\")}"
            }
        }
    } else {
        VNode::empty()
    };

    // Hidden projects section: only while "show hidden" is on (Nexus).
    let hidden_section_el: Element = if *show_hidden.read() && !hidden_projects.is_empty() {
        let items: Vec<Element> = hidden_projects
            .iter()
            .map(|wd| {
                let state = state.clone();
                let lang2 = lang.clone();
                let wd = wd.clone();
                rsx! {
                    div { class: "history-hidden-row",
                        span { class: "history-hidden-path", title: "{wd}", "{wd}" }
                        button {
                            class: "history-restore-btn",
                            onclick: {
                                let state = state.clone();
                                let wd = wd.clone();
                                move |_| {
                                    let state = state.clone();
                                    let wd = wd.clone();
                                    async move {
                                        let _ = crate::api::api_restore_project(token(&state), wd).await;
                                        crate::state::refresh(state).await;
                                    }
                                }
                            },
                            "{tr(&lang2, \"restoreLabel\")}"
                        }
                    }
                }
            })
            .collect();
        let lang3 = lang.clone();
        rsx! {
            div { class: "history-hidden-projects",
                div { class: "section-label", "{tr(&lang3, \"hiddenProjectsLabel\")}" }
                {items.iter().cloned()}
            }
        }
    } else {
        VNode::empty()
    };

    let content_el: Element = if loading {
        rsx! { div { class: "empty-text", "{tr(&lang, \"loading\")}…" }
        }
    } else if total == 0 {
        rsx! { div { class: "empty-text", "{tr(&lang, \"noSessionsFound\")}" } }
    } else {
        rsx! {
            div { class: "session-rows",
                {row_els.iter().cloned()}
                {load_more_el}
            }
        }
    };

    let pill_cls = |name: &str| {
        if *time_filter.read() == name {
            "scope-pill active".to_string()
        } else {
            "scope-pill".to_string()
        }
    };
    let title_pill = if mode == 0 { "scope-pill active" } else { "scope-pill" };
    let content_pill = if mode == 1 { "scope-pill active" } else { "scope-pill" };
    let search_placeholder = if mode == 1 {
        tr(&lang, "searchContentPlaceholder").to_string()
    } else {
        tr(&lang, "searchTitlePlaceholder").to_string()
    };

    rsx! {
        div { class: "history-view",
            div { class: "history-header",
                h2 { class: "settings-title", "{tr(&lang, \"historyLabel\")}" }
                div { class: "history-controls",
                    div { class: "scope-switch",
                        button { class: "{pill_cls(\"30d\")}", onclick: move |_| { time_filter.set("30d".into()); visible_count.set(PAGE_SIZE); }, "{tr(&lang, \"time30d\")}" }
                        button { class: "{pill_cls(\"90d\")}", onclick: move |_| { time_filter.set("90d".into()); visible_count.set(PAGE_SIZE); }, "{tr(&lang, \"time90d\")}" }
                        button { class: "{pill_cls(\"180d\")}", onclick: move |_| { time_filter.set("180d".into()); visible_count.set(PAGE_SIZE); }, "{tr(&lang, \"time180d\")}" }
                        button { class: "{pill_cls(\"all\")}", onclick: move |_| { time_filter.set("all".into()); visible_count.set(PAGE_SIZE); }, "{tr(&lang, \"timeAll\")}" }
                    }
                    div { class: "scope-switch",
                        button {
                            class: "{title_pill}",
                            onclick: move |_| {
                                search_mode.set(0);
                                content_matches.set(Vec::new());
                                searching.set(false);
                                visible_count.set(PAGE_SIZE);
                            },
                            "{tr(&lang, \"searchTitleLabel\")}"
                        }
                        button {
                            class: "{content_pill}",
                            onclick: move |_| {
                                search_mode.set(1);
                                visible_count.set(PAGE_SIZE);
                            },
                            "{tr(&lang, \"searchContentLabel\")}"
                        }
                    }
                    input {
                        class: "field-input history-search",
                        placeholder: "{search_placeholder}",
                        value: "{query()}",
                        oninput: {
                            let state = state.clone();
                            move |e| {
                                let v = e.value();
                                query.set(v.clone());
                                visible_count.set(PAGE_SIZE);
                                let mode_v = *search_mode.read();
                                let q = v.trim().to_string();
                                if mode_v != 1 || q.chars().count() < 2 {
                                    content_matches.set(Vec::new());
                                    searching.set(false);
                                    return;
                                }
                                let gen = *search_gen.read() + 1;
                                search_gen.set(gen);
                                searching.set(true);
                                let st = state.clone();
                                spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                                    if *search_gen.read() != gen {
                                        return;
                                    }
                                    let res = crate::vibestats::api_search_session_content(token(&st), q).await;
                                    if *search_gen.read() != gen {
                                        return;
                                    }
                                    searching.set(false);
                                    match res {
                                        Ok(m) => content_matches.set(m),
                                        Err(_) => content_matches.set(Vec::new()),
                                    }
                                });
                            }
                        },
                    }
                    if *searching.read() {
                        span { class: "history-searching", "…" }
                    }
                    select {
                        class: "field-select history-project-select",
                        onchange: move |e: Event<FormData>| {
                            project_filter.set(e.value());
                            visible_count.set(PAGE_SIZE);
                        },
                        {opt_els.iter().cloned()}
                    }
                    label { class: "history-show-hidden checkbox-label",
                        input {
                            r#type: "checkbox",
                            checked: *show_hidden.read(),
                            onchange: move |e| {
                                show_hidden.set(e.checked());
                                visible_count.set(PAGE_SIZE);
                            },
                        },
                        "{tr(&lang, \"showHiddenLabel\")}"
                    }
                }
            }
            {hidden_section_el}
            div { class: "history-content",
                {content_el}
            }
        }
    }
}

#[derive(Clone)]
struct RemoteHist {
    conn_id: String,
    conn_name: String,
    session_id: String,
    title: String,
    cwd: String,
    updated_at: String,
    updated_ms: Option<i64>,
}

#[derive(Clone)]
struct Row {
    key: String,
    title: String,
    date: String,
    duration: String,
    messages: String,
    cost: String,
    dir: String,
    dir_short: String,
    model: String,
    hidden: bool,
    sort_ms: i64,
    remote: Option<RemoteHist>,
}

fn row_el(
    state: Signal<ClientState>,
    lang: String,
    row: Row,
    snippet: Option<String>,
) -> Element {
    let key = row.key.clone();
    // Hidden remote rows unhide by the raw CLI session id, not the
    // composite history key (Nexus unhideSessions([sessionId])).
    let restore_key = row
        .remote
        .as_ref()
        .map(|r| r.session_id.clone())
        .unwrap_or_else(|| row.key.clone());
    let dir = row.dir.clone();
    let hidden_suffix = if row.hidden {
        format!(" ({})", tr(&lang, "hiddenBadge"))
    } else {
        String::new()
    };
    let title_line = format!("{}{}", row.title, hidden_suffix);
    let remote_word = if lang == "pl" { "Zdalne" } else { "Remote" };
    let meta = if let Some(r) = &row.remote {
        format!("{} · {} · {}", row.date, remote_word, r.conn_name)
    } else {
        format!(
            "{} · {} · {} {} · {} ${}",
            row.date,
            row.duration,
            tr(&lang, "messagesLabel"),
            row.messages,
            tr(&lang, "costLabel"),
            row.cost
        )
    };
    let row_class = if row.hidden {
        "session-row session-row-clickable history-row history-row-hidden".to_string()
    } else {
        "session-row session-row-clickable history-row".to_string()
    };
    let hint = tr(&lang, "reactivateHint").to_string();

    let snippet_el: Element = match snippet {
        Some(s) => rsx! { div { class: "history-snippet", "…{s}…" } },
        None => VNode::empty(),
    };

    let restore_el: Element = if row.hidden {
        rsx! {
            button {
                class: "history-restore-btn",
                onclick: {
                    let state = state.clone();
                    let key = restore_key.clone();
                    let dir = dir.clone();
                    move |_| {
                        let state = state.clone();
                        let key = key.clone();
                        let dir = dir.clone();
                        async move {
                            let _ = crate::api::api_restore_session(token(&state), key).await;
                            if !dir.is_empty() {
                                let _ = crate::api::api_restore_project(token(&state), dir).await;
                            }
                            crate::state::refresh(state).await;
                        }
                    }
                },
                "{tr(&lang, \"restoreLabel\")}"
            }
        }
    } else {
        VNode::empty()
    };

    let reactivate_el: Element = {
        let remote = row.remote.clone();
        let lang_r = lang.clone();
        rsx! {
            button {
                class: "history-reactivate-btn",
                onclick: {
                    let state = state.clone();
                    let key = key.clone();
                    let remote = remote.clone();
                    move |_| {
                        let state = state.clone();
                        let key = key.clone();
                        let remote = remote.clone();
                        async move {
                            if let Some(r) = remote {
                                // Remote rows reactivate through the lazy
                                // open (SSH preview, no spawn - Nexus
                                // loadRemoteSession).
                                let _ = crate::api::api_open_remote_session(
                                    token(&state),
                                    r.conn_id,
                                    r.session_id,
                                    r.title,
                                    r.cwd,
                                )
                                .await;
                            } else {
                                let _ = crate::api::api_refresh_sessions(token(&state)).await;
                                let _ =
                                    crate::api::api_open_backend_session(token(&state), key).await;
                            }
                            crate::state::refresh(state).await;
                            crate::app::set_view(state, View::Chat);
                        }
                    }
                },
                "{tr(&lang_r, \"reactivateLabel\")}"
            }
        }
    };

    rsx! {
        div { class: "{row_class}",
            div {
                class: "session-row-main",
                title: "{hint}",
                onclick: {
                    let state = state.clone();
                    let key = key.clone();
                    let remote = row.remote.clone();
                    move |_| {
                        let state = state.clone();
                        let key = key.clone();
                        let remote = remote.clone();
                        async move {
                            if let Some(r) = remote {
                                let _ = crate::api::api_open_remote_session(
                                    token(&state),
                                    r.conn_id,
                                    r.session_id,
                                    r.title,
                                    r.cwd,
                                )
                                .await;
                            } else {
                                let _ = crate::api::api_refresh_sessions(token(&state)).await;
                                let _ =
                                    crate::api::api_open_backend_session(token(&state), key).await;
                            }
                            crate::state::refresh(state).await;
                            crate::app::set_view(state, View::Chat);
                        }
                    }
                },
                span { class: "session-row-title", "{title_line}" }
                div { class: "session-row-meta", "{meta}" }
                {snippet_el}
            }
            div { class: "session-row-tags",
                span { class: "tag primary", "{row.dir_short}" }
                span { class: "tag accent", "{row.model}" }
            }
            div { class: "history-row-actions",
                {restore_el}
                {reactivate_el}
            }
        }
    }
}

fn base_name(p: &str) -> String {
    p.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(p)
        .to_string()
}

/// Nexus `formatDuration`: `Ns` / `Nm` / `Nh` (one decimal above an hour).
fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{}s", secs.round() as i64)
    } else if secs < 3600.0 {
        format!("{}m", (secs / 60.0).round() as i64)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse an ISO timestamp (`YYYY-MM-DDTHH:MM:SS…`) into epoch milliseconds.
fn iso_to_ms(iso: &str) -> Option<i64> {
    let d = iso.get(0..10)?;
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let day: i64 = it.next()?.parse().ok()?;
    let mut ms = days_from_civil(y, m, day) * 86_400_000;
    if let Some(h) = iso.get(11..13) {
        ms += h.parse::<i64>().ok()? * 3_600_000;
    }
    if let Some(mi) = iso.get(14..16) {
        ms += mi.parse::<i64>().ok()? * 60_000;
    }
    if let Some(se) = iso.get(17..19) {
        ms += se.parse::<i64>().ok().unwrap_or(0) * 1000;
    }
    Some(ms)
}

/// `dd.mm.yyyy` display date from an ISO timestamp.
fn iso_date(iso: &str) -> String {
    let days = iso.get(0..10).and_then(|d| {
        let mut it = d.split('-');
        let y: i64 = it.next()?.parse().ok()?;
        let m: i64 = it.next()?.parse().ok()?;
        let dd: i64 = it.next()?.parse().ok()?;
        Some(days_from_civil(y, m, dd))
    });
    match days {
        Some(days) => {
            let (y, m, d) = civil_from_days(days);
            format!("{d:02}.{m:02}.{y}")
        }
        None => String::new(),
    }
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
