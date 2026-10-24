//! Stats view: live usage cards + full session analytics from the Vibe CLI
//! logs (tokens, cost per session, tool usage) with SVG charts - area, bar,
//! donut - and a time-range selector. Same card/chart CSS classes as the
//! Nexus stats view; data comes server-side from `vibestats.rs`.

use dioxus::prelude::*;

use crate::i18n::{tf, tr};
use crate::state::{lang, token, ClientState};
use crate::vibestats::SessionStatsRow;

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

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse the date part of an ISO timestamp (`YYYY-MM-DD...`) into days.
fn parse_days(s: &str) -> Option<i64> {
    let d = s.get(0..10)?;
    let mut it = d.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    Some(days_from_civil(y, m, d))
}

fn now_days() -> i64 {
    vibedeck_core::types::now_ms() / 86_400_000
}

fn iso_date(iso: &str) -> String {
    match parse_days(iso) {
        Some(days) => {
            let (y, m, d) = civil_from_days(days);
            format!("{d:02}.{m:02}.{y}")
        }
        None => iso.to_string(),
    }
}

fn fmt_tokens(n: f64) -> String {
    if n >= 1_000_000.0 {
        format!("{:.1}M", n / 1_000_000.0)
    } else if n >= 1_000.0 {
        format!("{:.1}k", n / 1_000.0)
    } else {
        format!("{}", n as u64)
    }
}

fn fmt_cost(v: f64) -> String {
    if v >= 100.0 {
        format!("${v:.0}")
    } else {
        format!("${v:.2}")
    }
}

// ---------------------------------------------------------------------------
// SVG charts (no dependencies, same palette ideas as the Nexus painter
// charts in the egui app: accent for prompts, muted for completions)
// ---------------------------------------------------------------------------

const ACCENT: &str = "#fa500f";
const MUTED: &str = "#8a8a8f";
const SUCCESS: &str = "#4ade80";
const ERROR: &str = "#f87171";

/// Stacked bar chart: prompt tokens (bottom, accent) + completion tokens.
fn bar_chart(prompt: &[f64], completion: &[f64], labels: &[String]) -> Element {
    let nb = prompt.len();
    if nb == 0 {
        return VNode::empty();
    }
    let step = 26.0;
    let w = nb as f64 * step;
    let h = 150.0;
    let base = h - 22.0;
    let peak = (0..nb)
        .map(|i| prompt[i] + completion[i])
        .fold(0.0f64, f64::max)
        .max(1.0);
    let chart_h = base - 8.0;

    let mut els: Vec<Element> = Vec::new();
    // horizontal grid + y labels
    for g in 0..=2usize {
        let gy = base - chart_h * (g as f64 / 2.0);
        els.push(rsx! {
            line { key: "g{g}", x1: "0", y1: "{gy}", x2: "{w}", y2: "{gy}",
                stroke: "var(--border)", stroke_width: "1", stroke_dasharray: "2 3" }
        });
        let val = fmt_tokens(peak * (g as f64 / 2.0));
        els.push(rsx! {
            text { key: "gt{g}", x: "2", y: "{gy - 3.0}", font_size: "9",
                fill: "var(--text-dim)", "{val}" }
        });
    }
    // bars
    let label_every = (nb / 7).max(1);
    for i in 0..nb {
        let x = i as f64 * step + 5.0;
        let hp = (prompt[i] / peak) * chart_h;
        let hc = (completion[i] / peak) * chart_h;
        if hp > 0.0 {
            els.push(rsx! {
                rect { key: "p{i}", x: "{x}", y: "{base - hp}", width: "16",
                    height: "{hp}", fill: ACCENT, rx: "2" }
            });
        }
        if hc > 0.0 {
            els.push(rsx! {
                rect { key: "c{i}", x: "{x}", y: "{base - hp - hc}", width: "16",
                    height: "{hc}", fill: MUTED, rx: "2" }
            });
        }
        if i % label_every == 0 {
            let lbl = labels.get(i).cloned().unwrap_or_default();
            els.push(rsx! {
                text { key: "x{i}", x: "{x + 8.0}", y: "{h - 6.0}", font_size: "9",
                    fill: "var(--text-dim)", text_anchor: "middle", "{lbl}" }
            });
        }
    }
    rsx! {
        svg { class: "chart-svg", view_box: "0 0 {w} {h}", preserve_aspect_ratio: "none",
            {els.iter().cloned()}
        }
    }
}

/// Area chart of a single series (cost per day), axis-free like the Nexus one.
fn area_chart(values: &[f64], labels: &[String]) -> Element {
    let nb = values.len();
    if nb == 0 {
        return VNode::empty();
    }
    let step = 26.0;
    let w = nb as f64 * step;
    let h = 150.0;
    let base = h - 22.0;
    let peak = values.iter().cloned().fold(0.0f64, f64::max).max(0.01);
    let chart_h = base - 8.0;
    let mut path = String::new();
    for (i, v) in values.iter().enumerate() {
        let x = i as f64 * step + 13.0;
        let y = base - (v / peak) * chart_h;
        if i == 0 {
            path.push_str(&format!("M {x:.1} {y:.1}"));
        } else {
            path.push_str(&format!(" L {x:.1} {y:.1}"));
        }
    }
    let fill_path = format!("{path} L {w} {base} L 0 {base} Z");
    let mut els: Vec<Element> = vec![
        rsx! { path { d: "{fill_path}", fill: "rgba(250,80,15,0.16)" } },
        rsx! { path { d: "{path}", fill: "none", stroke: ACCENT, stroke_width: "2" } },
    ];
    for g in 0..=2usize {
        let gy = base - chart_h * (g as f64 / 2.0);
        let val = format!("${:.1}", peak * (g as f64 / 2.0));
        els.push(rsx! {
            line { key: "gl{g}", x1: "0", y1: "{gy}", x2: "{w}", y2: "{gy}",
                stroke: "var(--border)", stroke_width: "1", stroke_dasharray: "2 3" }
        });
        els.push(rsx! {
            text { key: "gt{g}", x: "2", y: "{gy - 3.0}", font_size: "9",
                fill: "var(--text-dim)", "{val}" }
        });
    }
    let label_every = (nb / 7).max(1);
    for (i, v) in values.iter().enumerate() {
        if i % label_every == 0 {
            let x = i as f64 * step + 13.0;
            let lbl = labels.get(i).cloned().unwrap_or_default();
            let _ = v;
            els.push(rsx! {
                text { key: "x{i}", x: "{x}", y: "{h - 6.0}", font_size: "9",
                    fill: "var(--text-dim)", text_anchor: "middle", "{lbl}" }
            });
        }
    }
    rsx! {
        svg { class: "chart-svg", view_box: "0 0 {w} {h}", preserve_aspect_ratio: "none",
            {els.iter().cloned()}
        }
    }
}

/// Donut chart of tool call outcomes with a legend.
fn donut(lang: &str, ok: f64, fail: f64, rej: f64) -> Element {
    let total = ok + fail + rej;
    if total <= 0.0 {
        return rsx! { div { class: "chart-empty", "{tr(lang, \"noToolCalls\")}" } };
    }
    let r = 42.0;
    let c = 2.0 * std::f64::consts::PI * r;
    let mut els: Vec<Element> = vec![
        rsx! { circle { cx: "50", cy: "50", r: "{r}", fill: "none",
            stroke: "var(--panel)", stroke_width: "14" } },
    ];
    let mut offset = 0.0f64;
    let segs: [(f64, &str, &str); 3] = [
        (ok, SUCCESS, "succeeded"),
        (fail, ERROR, "failedLabel"),
        (rej, MUTED, "rejectedLabel"),
    ];
    for (idx, (v, color, key)) in segs.iter().enumerate() {
        let frac = v / total;
        if frac <= 0.0 {
            continue;
        }
        let len = frac * c;
        // SVG draws dashoffset going clockwise from 3 o'clock; rotate -90 to
        // start at 12 and accumulate offsets so segments do not overlap.
        let off = c - offset;
        els.push(rsx! {
            circle { key: "s{idx}", cx: "50", cy: "50", r: "{r}", fill: "none",
                stroke: "{color}", stroke_width: "14",
                stroke_dasharray: "{len:.2} {c:.2}", stroke_dashoffset: "{off:.2}",
                transform: "rotate(-90 50 50)" }
        });
        offset += len;
    }
    els.push(rsx! {
        text { x: "50", y: "50", text_anchor: "middle", dominant_baseline: "central",
            font_size: "16", fill: "var(--text)", "{fmt_tokens(total)}" }
    });
    let mut legend: Vec<Element> = Vec::new();
    for (v, color, key) in &segs {
        legend.push(rsx! {
            div { key: "{key}",
                span { class: "donut-dot", style: "background:{color};width:8px;height:8px;border-radius:50%;display:inline-block;margin-right:6px;" }
                span { style: "color:var(--text-muted);font-size:11px;",
                    "{tr(lang, key)} · {fmt_tokens(*v)}" }
            }
        });
    }
    rsx! {
        div { class: "donut-wrap",
            svg { class: "chart-donut", view_box: "0 0 100 100", {els.iter().cloned()} }
            div { class: "donut-legend", {legend.iter().cloned()} }
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn StatsView() -> Element {
    let state = use_context::<Signal<ClientState>>();

    let s = state.read();
    let lang = lang(&state);
    let usage = s.snapshot.usage.clone();
    let sessions = s.snapshot.sessions.clone();
    let busy = s.snapshot.busy;
    drop(s);

    let mut rows = use_signal(|| Vec::<SessionStatsRow>::new());
    let mut loaded = use_signal(|| false);
    let mut range = use_signal(|| 3u8); // 0=7d 1=30d 2=90d 3=all
    let mut load_err = use_signal(String::new);

    if !*loaded.read() {
        loaded.set(true);
        let st = state.clone();
        spawn(async move {
            match crate::vibestats::api_vibe_stats(token(&st)).await {
                Ok(r) => rows.set(r),
                Err(e) => load_err.set(e.to_string()),
            }
        });
    }

    let all = rows.read().clone();
    let err = load_err.read().clone();
    let cur_range = *range.read();
    let range_days: Option<i64> = match cur_range {
        0 => Some(7),
        1 => Some(30),
        2 => Some(90),
        _ => None,
    };
    let now = now_days();
    let mut sel: Vec<SessionStatsRow> = all
        .into_iter()
        .filter(|r| match (range_days, parse_days(&r.start_time)) {
            (Some(d), Some(sd)) => now - sd < d,
            (None, _) => true,
            _ => false,
        })
        .collect();
    sel.sort_by(|a, b| b.start_time.cmp(&a.start_time));

    // aggregates over the selection
    let sel_count = sel.len();
    let prompt: f64 = sel.iter().map(|r| r.prompt_tokens).sum();
    let completion: f64 = sel.iter().map(|r| r.completion_tokens).sum();
    let cached: f64 = sel.iter().map(|r| r.cached_tokens).sum();
    let cost: f64 = sel.iter().map(|r| r.cost).sum();
    let tps_n = sel.iter().filter(|r| r.tokens_per_second > 0.0).count();
    let tps_avg = if tps_n == 0 {
        0.0
    } else {
        sel.iter().map(|r| r.tokens_per_second).sum::<f64>() / tps_n as f64
    };
    let tool_ok: f64 = sel.iter().map(|r| r.tool_succeeded).sum();
    let tool_fail: f64 = sel.iter().map(|r| r.tool_failed).sum();
    let tool_rej: f64 = sel.iter().map(|r| r.tool_rejected).sum();

    // live cards data
    let session_count = sessions.len();
    let messages: usize = sessions.iter().map(|m| m.message_count).sum();
    let hit_pct = (usage.cache_hit_rate() * 100.0).round() as u64;

    // day buckets for the charts (last 30 days when "all")
    let nb = range_days.unwrap_or(30) as usize;
    let mut day_prompt = vec![0.0f64; nb];
    let mut day_completion = vec![0.0f64; nb];
    let mut day_cost = vec![0.0f64; nb];
    let labels: Vec<String> = (0..nb)
        .map(|i| {
            let (y, m, d) = civil_from_days(now - (nb - 1 - i) as i64);
            format!("{d:02}.{m:02}")
        })
        .collect();
    for r in &sel {
        if let Some(sd) = parse_days(&r.start_time) {
            let age = now - sd;
            if age >= 0 && (age as usize) < nb {
                let idx = nb - 1 - age as usize;
                day_prompt[idx] += r.prompt_tokens;
                day_completion[idx] += r.completion_tokens;
                day_cost[idx] += r.cost;
            }
        }
    }

    let bar_el = bar_chart(&day_prompt, &day_completion, &labels);
    let area_el = area_chart(&day_cost, &labels);
    let donut_el = donut(&lang, tool_ok, tool_fail, tool_rej);

    // range buttons
    let range_defs: Vec<(u8, String)> = vec![
        (0, "7D".to_string()),
        (1, "30D".to_string()),
        (2, "90D".to_string()),
        (3, tr(&lang, "allTime").to_string()),
    ];
    let range_btn_els: Vec<Element> = range_defs
        .into_iter()
        .map(|(k, lbl)| {
            let cls = if k == cur_range {
                "time-range-btn active".to_string()
            } else {
                "time-range-btn".to_string()
            };
            rsx! {
                button { key: "{k}", class: "{cls}",
                    onclick: move |_| { range.set(k); },
                    "{lbl}"
                }
            }
        })
        .collect();

    // analytics rows (all sessions from the logs, newest first)
    let mut sorted = sessions.clone();
    sorted.sort_by(|a, b| b.last_active_ms.cmp(&a.last_active_ms));
    sorted.truncate(20);
    let analytics_rows: Vec<Element> = sel
        .iter()
        .take(30)
        .enumerate()
        .map(|(i, r)| {
            let title = if r.title.is_empty() {
                tr(&lang, "untitledSession").to_string()
            } else {
                r.title.clone()
            };
            let model = if r.model.is_empty() {
                "—".to_string()
            } else {
                r.model.clone()
            };
            let date = iso_date(&r.start_time);
            let toks = fmt_tokens(r.total_tokens.max(r.prompt_tokens + r.completion_tokens));
            let cost_s = fmt_cost(r.cost);
            let msgs = r.messages;
            rsx! {
                div { key: "{i}", class: "session-row",
                    div { class: "session-row-main",
                        div { class: "session-row-title", "{title}" }
                        div { class: "session-row-meta", "{model} · {date} · {toks} · {msgs}" }
                    }
                    span { class: "session-row-cost", "{cost_s}" }
                }
            }
        })
        .collect();

    // live session rows (clickable, from the engine snapshot)
    let log_hit_pct: u64 = if prompt > 0.0 {
        (cached / prompt * 100.0).round() as u64
    } else {
        0
    };
    let live_rows: Vec<Element> = sorted
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let state = state.clone();
            let idx = sessions.iter().position(|x| x.id == m.id).unwrap_or(i);
            let name = m.name.clone();
            let date = {
                let days = m.last_active_ms.div_euclid(86_400_000);
                let (y, mm, d) = civil_from_days(days);
                format!("{d:02}.{mm:02}.{y}")
            };
            let count = m.message_count;
            rsx! {
                div { key: "L{i}", class: "session-row",
                    onclick: {
                        let state = state.clone();
                        move |_| {
                            let state = state.clone();
                            async move {
                                let _ = crate::api::api_switch_session(token(&state), idx).await;
                                crate::state::refresh(state).await;
                                crate::app::set_view(state, crate::state::View::Chat);
                            }
                        }
                    },
                    div { class: "session-row-main",
                        div { class: "session-row-title", "{name}" }
                        div { class: "session-row-meta", "{date} · {count}" }
                    }
                }
            }
        })
        .collect();

    rsx! {
        div { class: "stats-view",
            div { class: "stats-scroll",
                h2 { class: "stats-title", "{tr(&lang, \"statistics\")}" }
                div { class: "pill-row",
                    if busy {
                        span { class: "loading-indicator", "{tr(&lang, \"agentWorkingLabel\")}" }
                    }
                    button {
                        class: "refresh-btn",
                        onclick: {
                            let state = state.clone();
                            move |_| {
                                let state = state.clone();
                                async move {
                                    let _ = crate::api::api_refresh_sessions(token(&state)).await;
                                    let st = state.clone();
                                    match crate::vibestats::api_vibe_stats(token(&st)).await {
                                        Ok(r) => rows.set(r),
                                        Err(e) => load_err.set(e.to_string()),
                                    }
                                    crate::state::refresh(state).await;
                                }
                            }
                        },
                        "⟳ {tr(&lang, \"refresh\")}"
                    }
                }
                div { class: "time-range-btns",
                    {range_btn_els.iter().cloned()}
                }
                if !err.is_empty() {
                    div { class: "git-error", "{err}" }
                }
                div { class: "cards-row",
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"totalTokens\")}" }
                        }
                        div { class: "card-value primary", "{fmt_tokens(usage.total_tokens)}" }
                        div { class: "card-meta",
                            "{tr(&lang, \"inputLabel\")} {fmt_tokens(usage.prompt_tokens)} · {tr(&lang, \"outputLabel\")} {fmt_tokens(usage.completion_tokens)}"
                        }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"cacheHitLabel\")}" }
                        }
                        div { class: "card-value success", "{hit_pct}%" }
                        div { class: "card-meta", "{fmt_tokens(usage.cached_tokens)} / {fmt_tokens(usage.prompt_tokens)}" }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"tokPerSecAvg\")}" }
                        }
                        div { class: "card-value accent", "{usage.tokens_per_second.round() as u64}" }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"totalSpend\")}" }
                        }
                        div { class: "card-value primary", "{fmt_cost(cost)}" }
                        div { class: "card-meta", "{tf(&lang, \"sessionsCount\", &[(\"count\", &sel_count.to_string())])}" }
                    }
                }
                div { class: "cards-row",
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"activity\")}" }
                        }
                        div { class: "card-chart", {bar_el} }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"totalSpend\")} / {tr(&lang, \"activity\")}" }
                        }
                        div { class: "card-chart", {area_el} }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"toolCalls\")}" }
                        }
                        div { class: "card-chart", {donut_el} }
                    }
                }
                div { class: "cards-row",
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"totalTokens\")} ({tr(&lang, \"allTime\")})" }
                        }
                        div { class: "card-value", "{fmt_tokens(prompt + completion)}" }
                        div { class: "card-meta",
                            "{tr(&lang, \"inputLabel\")} {fmt_tokens(prompt)} · {tr(&lang, \"outputLabel\")} {fmt_tokens(completion)}"
                        }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"cacheHitLabel\")} ({tr(&lang, \"allTime\")})" }
                        }
                        div { class: "card-value success",
                            "{log_hit_pct}%"
                        }
                        div { class: "card-meta", "{fmt_tokens(cached)} / {fmt_tokens(prompt)}" }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"tokPerSecAvg\")} ({tr(&lang, \"allTime\")})" }
                        }
                        div { class: "card-value accent", "{tps_avg.round() as u64}" }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div { class: "card-subtitle", "{tr(&lang, \"messagesLabel\")}" }
                        }
                        div { class: "card-value", "{messages}" }
                        div { class: "card-meta", "{tr(&lang, \"recentSessions\")}: {session_count}" }
                    }
                }
                div { class: "session-list-card",
                    div { class: "section-label", "{tr(&lang, \"allSessions\")}" }
                    div { class: "session-rows",
                        if analytics_rows.is_empty() {
                            div { class: "empty-text", "{tr(&lang, \"noData\")}" }
                        }
                        {analytics_rows.iter().cloned()}
                    }
                }
                div { class: "session-list-card",
                    div { class: "section-label", "{tr(&lang, \"recentSessions\")}" }
                    div { class: "session-rows",
                        if live_rows.is_empty() {
                            div { class: "empty-text", "{tr(&lang, \"noSessionsYet\")}" }
                        }
                        {live_rows.iter().cloned()}
                    }
                }
            }
        }
    }
}
