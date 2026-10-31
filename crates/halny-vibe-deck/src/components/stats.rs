//! Stats view: live usage cards + full session analytics from the Vibe CLI
//! logs (tokens, cost per session, tool usage) with SVG charts — area, bar,
//! donut, heatmap — a time-range selector with custom dates, a model filter
//! and a clickable session list with a per-session drill-down and transcript
//! (Nexus `StatsView` parity). Same card/chart CSS classes as the Nexus stats
//! view; data comes server-side from `vibestats.rs`.

use dioxus::prelude::*;

use crate::components::chat::MarkdownLite;
use crate::i18n::{tf, tr};
use crate::state::{lang, token, ClientState};
use crate::vibestats::{PreviewMessageView, SessionStatsRow, SubagentSessionView};

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

/// Parse an ISO timestamp into (day number, milliseconds within that day).
fn parse_iso(iso: &str) -> Option<(i64, i64)> {
    let days = parse_days(iso)?;
    let mut ms = 0i64;
    if let Some(rest) = iso.get(10..) {
        if let Some(t) = rest.strip_prefix('T') {
            let h: i64 = t.get(0..2)?.parse().ok()?;
            let m: i64 = t.get(3..5)?.parse().ok()?;
            let s: i64 = t.get(6..8).and_then(|x| x.parse().ok()).unwrap_or(0);
            ms = h * 3_600_000 + m * 60_000 + s * 1000;
        }
    }
    Some((days, ms))
}

fn now_days() -> i64 {
    halny_vibe_deck_core::types::now_ms() / 86_400_000
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

/// `YYYY-MM-DD` for a day number (custom date inputs).
fn iso_ymd(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Weekday index (0 = Sunday) for a day number (days since 1970-01-01, a
/// Thursday, hence the +4 shift).
fn weekday_sun0(days: i64) -> usize {
    (((days + 4) % 7 + 7) % 7) as usize
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

/// Compact y-axis token labels (no decimals under 1k).
fn fmt_y_tokens(v: f64) -> String {
    if v >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if v >= 1_000.0 {
        format!("{:.0}k", v / 1_000.0)
    } else {
        format!("{}", v.round() as i64)
    }
}

/// Axis cost label (Nexus `fmtYCost`).
fn fmt_y_cost(v: f64) -> String {
    if v >= 10.0 {
        format!("${v:.0}")
    } else {
        format!("${v:.2}")
    }
}

fn fmt_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{}s", secs.round() as i64)
    } else if secs < 3600.0 {
        format!("{}m", (secs / 60.0).round() as i64)
    } else {
        format!("{:.1}h", secs / 3600.0)
    }
}

/// Session duration in seconds from the ISO start/end times.
fn duration_secs(r: &SessionStatsRow) -> f64 {
    match (parse_iso(&r.start_time), parse_iso(&r.end_time)) {
        (Some((d1, m1)), Some((d2, m2))) => {
            (((d2 - d1) * 86_400_000 + (m2 - m1)) as f64 / 1000.0).max(0.0)
        }
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// SVG charts (no dependencies; Nexus geometry: viewBox 300x130, pads
// l44/r8/t8/b22, five y-ticks, x-ticks picked by a 40px minimum gap)
// ---------------------------------------------------------------------------

const C_ACCENT: &str = "var(--color-accent)";
const C_PRIMARY: &str = "var(--color-primary)";
const C_SUCCESS: &str = "var(--color-success)";
const C_WARNING: &str = "var(--color-warning)";
const C_DANGER: &str = "var(--color-danger)";

/// Nexus `AreaChart`: cumulative cost over session start days.
fn area_chart_cost(lang: &str, points: &[(i64, f64)]) -> Element {
    if points.len() < 2 {
        return rsx! { div { class: "chart-empty", "{tr(lang, \"noData\")}" } };
    }
    let w = 300.0f64;
    let h = 130.0f64;
    let (pl, pr, pt, pb) = (44.0, 8.0, 8.0, 22.0);
    let cw = w - pl - pr;
    let ch = h - pt - pb;
    let min_x = points[0].0;
    let max_x = points[points.len() - 1].0;
    let max_y = points.iter().map(|p| p.1).fold(0.0f64, f64::max).max(0.01);

    let sx = |x: i64| pl + ((x - min_x) as f64) / ((max_x - min_x) as f64).max(1.0) * cw;
    let sy = |y: f64| pt + ch - (y / max_y) * ch;

    let mut els: Vec<Element> = Vec::new();
    // area + line
    let mut path = String::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 {
            path.push_str(&format!("M {:.1} {:.1}", sx(*x), sy(*y)));
        } else {
            path.push_str(&format!(" L {:.1} {:.1}", sx(*x), sy(*y)));
        }
    }
    let last_x = sx(points[points.len() - 1].0);
    let fill_path = format!("{path} L {last_x:.1} {:.1} L {pl:.1} {:.1} Z", sy(0.0), sy(0.0));
    els.push(rsx! { path { d: "{fill_path}", fill: "rgba(115, 148, 230, 0.16)" } });
    els.push(rsx! { path { d: "{path}", fill: "none", stroke: C_PRIMARY, stroke_width: "1.5" } });
    // y grid + labels
    for g in 0..=4usize {
        let gy = pt + ch - ch * (g as f64 / 4.0);
        let val = fmt_y_cost(max_y * (g as f64 / 4.0));
        els.push(rsx! {
            line { key: "gy{g}", x1: "{pl}", y1: "{gy}", x2: "{w - pr}", y2: "{gy}",
                stroke: "var(--border-subtle)", stroke_width: "0.5" }
        });
        els.push(rsx! {
            text { key: "gt{g}", x: "{pl - 4.0}", y: "{gy + 3.0}", font_size: "8",
                text_anchor: "end", fill: "var(--text-muted)", "{val}" }
        });
    }
    // x ticks: min 40px gap, last point always labelled
    let mut ticks: Vec<usize> = Vec::new();
    let mut last_px = -999.0f64;
    for (i, (x, _)) in points.iter().enumerate() {
        let px = sx(*x);
        if px - last_px >= 40.0 {
            ticks.push(i);
            last_px = px;
        }
    }
    if let Some(last) = ticks.last_mut() {
        if *last != points.len() - 1 {
            let prev_px = sx(points[*last].0);
            let last_px2 = sx(points[points.len() - 1].0);
            if last_px2 - prev_px < 20.0 {
                *last = points.len() - 1;
            } else {
                ticks.push(points.len() - 1);
            }
        }
    }
    for &i in &ticks {
        let (x, _) = points[i];
        let (_y, m, d) = civil_from_days(x);
        let lbl = format!("{d:02}.{m:02}");
        els.push(rsx! {
            text { key: "x{i}", x: "{sx(x)}", y: "{h - 4.0}", font_size: "8",
                text_anchor: "middle", fill: "var(--text-muted)", "{lbl}" }
        });
    }
    rsx! {
        svg { class: "chart-svg", view_box: "0 0 300 130", preserve_aspect_ratio: "xMidYMid meet",
            {els.iter().cloned()}
        }
    }
}

/// Nexus `BarChart`: stacked values with a legend row and axes.
fn bar_chart_stacked(
    lang: &str,
    data: &[(String, Vec<f64>)],
    colors: &[&str],
    legend: &[String],
) -> Element {
    if data.is_empty() {
        return rsx! { div { class: "chart-empty", "{tr(lang, \"noData\")}" } };
    }
    let w = 300.0f64;
    let h = 130.0f64;
    let (pl, pr, pt, pb) = (44.0, 8.0, 8.0, 22.0);
    let cw = w - pl - pr;
    let ch = h - pt - pb;
    let n = data.len() as f64;
    let totals: Vec<f64> = data.iter().map(|(_, v)| v.iter().sum()).collect();
    let max_y = totals.iter().cloned().fold(0.0f64, f64::max).max(1.0);
    let gap = cw / n;
    let bar_w = (gap * 0.7).max(4.0);

    let mut els: Vec<Element> = Vec::new();
    for g in 0..=4usize {
        let gy = pt + ch - ch * (g as f64 / 4.0);
        let val = fmt_y_tokens(max_y * (g as f64 / 4.0));
        els.push(rsx! {
            line { key: "gy{g}", x1: "{pl}", y1: "{gy}", x2: "{w - pr}", y2: "{gy}",
                stroke: "var(--border-subtle)", stroke_width: "0.5" }
        });
        els.push(rsx! {
            text { key: "gt{g}", x: "{pl - 4.0}", y: "{gy + 3.0}", font_size: "8",
                text_anchor: "end", fill: "var(--text-muted)", "{val}" }
        });
    }
    for (i, (_, values)) in data.iter().enumerate() {
        let x = pl + i as f64 * gap + gap * 0.15;
        let mut y_off = 0.0f64;
        for (vi, v) in values.iter().enumerate() {
            let hh = (v / max_y) * ch;
            let y = pt + ch - y_off - hh;
            y_off += hh;
            if *v > 0.0 {
                let color = colors[vi % colors.len()];
                els.push(rsx! {
                    rect { key: "b{i}-{vi}", x: "{x}", y: "{y}", width: "{bar_w}",
                        height: "{hh}", rx: "1", fill: "{color}" }
                });
            }
        }
    }
    // x ticks by 40px min gap, last bar always labelled
    let mut ticks: Vec<usize> = Vec::new();
    let mut last_px = -999.0f64;
    for i in 0..data.len() {
        let px = pl + i as f64 * gap + gap * 0.5;
        if px - last_px >= 40.0 {
            ticks.push(i);
            last_px = px;
        }
    }
    if let Some(last) = ticks.last_mut() {
        if *last != data.len() - 1 {
            let prev_px = pl + *last as f64 * gap + gap * 0.5;
            let last_px2 = pl + (data.len() - 1) as f64 * gap + gap * 0.5;
            if last_px2 - prev_px < 20.0 {
                *last = data.len() - 1;
            } else {
                ticks.push(data.len() - 1);
            }
        }
    }
    for &i in &ticks {
        let lbl = data[i].0.clone();
        let x = pl + i as f64 * gap + gap * 0.5;
        els.push(rsx! {
            text { key: "x{i}", x: "{x}", y: "{h - 4.0}", font_size: "8",
                text_anchor: "middle", fill: "var(--text-muted)", "{lbl}" }
        });
    }

    let legend_els: Vec<Element> = legend
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let color = colors[i % colors.len()];
            rsx! {
                div { key: "lg{i}", style: "display:flex;align-items:center;gap:4px;",
                    span { style: "width:8px;height:8px;border-radius:2px;background:{color};flex-shrink:0;display:inline-block;" }
                    span { style: "font-size:10px;color:var(--text-muted);", "{l}" }
                }
            }
        })
        .collect();
    let legend_row: Element = if legend.len() > 1 {
        rsx! { div { style: "display:flex;gap:10px;padding-left:4px;", {legend_els.iter().cloned()} } }
    } else {
        VNode::empty()
    };

    rsx! {
        div { style: "display:flex;flex-direction:column;height:100%;gap:6px;",
            {legend_row}
            svg { class: "chart-svg", view_box: "0 0 300 130",
                preserve_aspect_ratio: "xMidYMid meet", style: "flex:1;",
                {els.iter().cloned()}
            }
        }
    }
}

/// Donut chart of tool call outcomes with the total in the center.
fn donut(lang: &str, ok: f64, fail: f64, rej: f64) -> Element {
    let total = ok + fail + rej;
    if total <= 0.0 {
        return rsx! { div { class: "chart-empty", "{tr(lang, \"noToolCalls\")}" } };
    }
    let r = 42.0;
    let c = 2.0 * std::f64::consts::PI * r;
    let mut els: Vec<Element> = vec![rsx! { circle { cx: "50", cy: "50", r: "{r}", fill: "none",
        stroke: "var(--panel)", stroke_width: "14" } }];
    let mut offset = 0.0f64;
    let segs: [(f64, &str, &str); 3] = [
        (ok, C_SUCCESS, "succeeded"),
        (fail, C_DANGER, "failedLabel"),
        (rej, C_WARNING, "rejectedLabel"),
    ];
    for (idx, (v, color, _key)) in segs.iter().enumerate() {
        let frac = v / total;
        if frac <= 0.0 {
            continue;
        }
        let len = frac * c;
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
        text { x: "50", y: "46", text_anchor: "middle", font_size: "9",
            fill: "var(--text-muted)", "{tr(lang, \"total\")}" }
    });
    els.push(rsx! {
        text { x: "50", y: "62", text_anchor: "middle", font_size: "13",
            font_weight: "600", fill: "var(--text)", "{fmt_tokens(total)}" }
    });
    let mut legend: Vec<Element> = Vec::new();
    for (v, color, key) in &segs {
        legend.push(rsx! {
            div { key: "{key}",
                span { class: "donut-dot", style: "background:{color};width:8px;height:8px;border-radius:50%;display:inline-block;margin-right:6px;" }
                span { style: "color:var(--text-muted);font-size:11px;",
                    "{tr(lang, key)} ({fmt_tokens(*v)})" }
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

/// Nexus `Heatmap`: 90 days of session counts, week columns starting Sunday,
/// five intensity levels driven by the `--heatmap-*` palette.
fn heatmap(lang: &str, sel: &[SessionStatsRow]) -> Element {
    let mut counts: std::collections::HashMap<i64, u64> = std::collections::HashMap::new();
    for r in sel {
        if let Some((d, _)) = parse_iso(&r.start_time) {
            *counts.entry(d).or_default() += 1;
        }
    }
    let now = now_days();
    let first = now - 89;
    let lead = weekday_sun0(first);
    let mut cols: Vec<Vec<Option<(i64, u64)>>> = Vec::new();
    let mut cur: Vec<Option<(i64, u64)>> = (0..lead).map(|_| None).collect();
    for i in 0..90i64 {
        let d = first + i;
        let n = counts.get(&d).copied().unwrap_or(0);
        cur.push(Some((d, n)));
        if cur.len() == 7 {
            cols.push(cur);
            cur = Vec::new();
        }
    }
    if !cur.is_empty() {
        while cur.len() < 7 {
            cur.push(None);
        }
        cols.push(cur);
    }
    let col_els: Vec<Element> = cols
        .iter()
        .enumerate()
        .map(|(ci, col)| {
            let cell_els: Vec<Element> = col
                .iter()
                .enumerate()
                .map(|(di, cell)| match cell {
                    None => rsx! { div { key: "e{ci}-{di}", class: "heatmap-cell heatmap-empty" } },
                    Some((d, n)) => {
                        let lvl = if *n == 0 {
                            0
                        } else if *n == 1 {
                            1
                        } else if *n <= 3 {
                            2
                        } else if *n <= 6 {
                            3
                        } else {
                            4
                        };
                        let (y, m, dd) = civil_from_days(*d);
                        let date_s = format!("{y:04}-{m:02}-{dd:02}");
                        let title = if *n == 0 {
                            format!("{date_s} · {}", tr(lang, "noSessionsYet"))
                        } else {
                            format!("{date_s} · {}", tf(lang, "sessionsCount", &[("count", &n.to_string())]))
                        };
                        rsx! {
                            div { key: "c{ci}-{di}", class: "heatmap-cell",
                                style: "background:var(--heatmap-{lvl});", title: "{title}" }
                        }
                    }
                })
                .collect();
            rsx! { div { key: "w{ci}", class: "heatmap-col", {cell_els.iter().cloned()} } }
        })
        .collect();
    rsx! { div { class: "heatmap", {col_els.iter().cloned()} } }
}

/// One message of the drill-down transcript, rendered like the chat view.
fn preview_msg_row(m: &PreviewMessageView) -> Element {
    if m.role == "user" {
        return rsx! {
            div { class: "subagent-work-msg",
                div { class: "subagent-work-user", "{m.content}" }
            }
        };
    }
    let think_els: Vec<Element> = m
        .thinking
        .iter()
        .map(|th| {
            let text = th.content.clone();
            rsx! { div { class: "subagent-work-thinking", "{text}" } }
        })
        .collect();
    let tool_els: Vec<Element> = m
        .tool_calls
        .iter()
        .map(|t| {
            let line = if t.status == "error" {
                format!("· {} ✕", t.title)
            } else {
                format!("· {}", t.title)
            };
            rsx! { div { class: "subagent-work-tool", "{line}" } }
        })
        .collect();
    let content = m.content.clone();
    let body: Element = if content.is_empty() {
        VNode::empty()
    } else {
        rsx! { div { class: "subagent-work-answer", MarkdownLite { text: content } } }
    };
    rsx! {
        div { class: "subagent-work-msg",
            {think_els.iter().cloned()}
            {tool_els.iter().cloned()}
            {body}
        }
    }
}

/// One subagent transcript block in the drill-down.
fn subagent_block(s: &SubagentSessionView, lang: &str) -> Element {
    let msg_els: Vec<Element> = s
        .messages
        .iter()
        .map(|m| {
            if m.role == "user" {
                rsx! {
                    div { class: "subagent-work-msg",
                        div { class: "subagent-work-user", "{m.content}" }
                    }
                }
            } else {
                let content = m.content.clone();
                let body: Element = if content.is_empty() {
                    VNode::empty()
                } else {
                    rsx! { div { class: "subagent-work-answer", MarkdownLite { text: content } } }
                };
                rsx! { div { class: "subagent-work-msg", {body} } }
            }
        })
        .collect();
    rsx! {
        div { class: "subagent-card",
            div { class: "subagent-header",
                span { class: "subagent-badge", "{tr(lang, \"subagent\")}" }
                span { class: "subagent-agent", "{s.agent_name}" }
                span { class: "subagent-work-tool", "{iso_date(&s.start_time)}" }
            },
            {msg_els.iter().cloned()}
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
    let busy = s.snapshot.busy;
    drop(s);

    let mut rows = use_signal(|| Vec::<SessionStatsRow>::new());
    let mut loaded = use_signal(|| false);
    let mut range = use_signal(|| 1u8); // 0=7d 1=30d 2=90d 3=all 4=custom
    let mut load_err = use_signal(String::new);
    let mut custom_from = use_signal(|| iso_ymd(now_days() - 30));
    let mut custom_to = use_signal(|| iso_ymd(now_days()));
    let mut cost_model = use_signal(|| "all".to_string());
    let mut token_filter = use_signal(|| 0u8); // 0=both 1=input 2=cached 3=output
    let expanded = use_signal(String::new);
    let mut visible = use_signal(|| 20usize);
    let previews = use_signal(|| Vec::<(String, Vec<PreviewMessageView>)>::new());
    let subs = use_signal(|| Vec::<(String, Vec<SubagentSessionView>)>::new());
    let detail_loading = use_signal(String::new);

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

    // ---- time-range filter ----
    let all = rows.read().clone();
    let err = load_err.read().clone();
    let cur_range = *range.read();
    let now = now_days();
    let cf = custom_from.read().clone();
    let ct = custom_to.read().clone();
    let from_d = parse_days(&cf);
    let to_d = parse_days(&ct);
    let mut sel: Vec<SessionStatsRow> = all
        .into_iter()
        .filter(|r| {
            let sd = match parse_days(&r.start_time) {
                Some(d) => d,
                None => return cur_range == 3,
            };
            match cur_range {
                0 => now - sd < 7,
                1 => now - sd < 30,
                2 => now - sd < 90,
                3 => true,
                _ => from_d.map_or(false, |f| sd >= f) && to_d.map_or(false, |t| sd <= t),
            }
        })
        .collect();
    sel.sort_by(|a, b| b.start_time.cmp(&a.start_time));

    // ---- aggregates over the selection ----
    let sel_count = sel.len();
    let prompt: f64 = sel.iter().map(|r| r.prompt_tokens).sum();
    let completion: f64 = sel.iter().map(|r| r.completion_tokens).sum();
    let cached: f64 = sel.iter().map(|r| r.cached_tokens).sum();
    let hit_pct = if prompt > 0.0 {
        (cached / prompt * 100.0).round() as i64
    } else {
        0
    };
    let tps_n = sel.iter().filter(|r| r.tokens_per_second > 0.0).count();
    let tps_avg = if tps_n == 0 {
        0.0
    } else {
        sel.iter().map(|r| r.tokens_per_second).sum::<f64>() / tps_n as f64
    };
    let total_dur: f64 = sel.iter().map(duration_secs).sum();
    let avg_dur = if sel_count == 0 { 0.0 } else { total_dur / sel_count as f64 };
    let tool_ok: f64 = sel.iter().map(|r| r.tool_succeeded).sum();
    let tool_fail: f64 = sel.iter().map(|r| r.tool_failed).sum();
    let tool_rej: f64 = sel.iter().map(|r| r.tool_rejected).sum();

    // ---- Cost card: per-model filter + cumulative area chart ----
    let mut models: Vec<String> = sel
        .iter()
        .flat_map(|r| {
            if r.models.is_empty() {
                vec![r.model.clone()]
            } else {
                r.models.clone()
            }
        })
        .filter(|m| !m.is_empty())
        .collect();
    models.sort();
    models.dedup();
    let cur_model = cost_model.read().clone();
    let cost_sel: Vec<&SessionStatsRow> = if cur_model != "all" && models.contains(&cur_model) {
        sel.iter()
            .filter(|r| {
                r.model == cur_model || r.models.iter().any(|m| *m == cur_model)
            })
            .collect()
    } else {
        sel.iter().collect()
    };
    let cost_total: f64 = cost_sel.iter().map(|r| r.cost).sum();
    let cost_avg = if cost_sel.is_empty() {
        0.0
    } else {
        cost_total / cost_sel.len() as f64
    };
    let mut by_start: Vec<(i64, f64)> = cost_sel
        .iter()
        .filter_map(|r| parse_days(&r.start_time).map(|d| (d, r.cost)))
        .collect();
    by_start.sort_by_key(|(d, _)| *d);
    let mut cum: Vec<(i64, f64)> = Vec::new();
    let mut run = 0.0f64;
    for (d, c) in by_start {
        run += c;
        cum.push((d, run));
    }
    let cost_chart = area_chart_cost(&lang, &cum);
    let cost_value = format!("${cost_total:.2}");
    let cost_meta = format!(
        "{} {} ${:.2}{}",
        tf(&lang, "sessionsCount", &[("count", &cost_sel.len().to_string())]),
        tr(&lang, "avg"),
        cost_avg,
        tr(&lang, "perSession")
    );
    let model_select: Element = if models.len() > 1 {
        let opt_els: Vec<Element> = std::iter::once(rsx! {
            option { value: "all", selected: cur_model == "all",
                "{tr(&lang, \"allModels\")}" }
        })
        .chain(models.iter().map(|m| {
            let selected = *m == cur_model;
            rsx! { option { key: "{m}", value: "{m}", selected: selected, "{m}" } }
        }))
        .collect();
            rsx! {
                select {
                    class: "stats-model-select",
                    onchange: move |e: Event<FormData>| cost_model.set(e.value()),
                    {opt_els.iter().cloned()}
                }
            }
    } else {
        VNode::empty()
    };

    // ---- Token card: pills + stacked bars with legend ----
    let cur_tf = *token_filter.read();
    let display_total = match cur_tf {
        0 => prompt + completion,
        1 => prompt,
        2 => cached,
        _ => completion,
    };
    let mut asc: Vec<&SessionStatsRow> = sel.iter().collect();
    asc.sort_by_key(|r| parse_days(&r.start_time).unwrap_or(0));
    let bar_data: Vec<(String, Vec<f64>)> = asc
        .iter()
        .filter_map(|r| {
            parse_days(&r.start_time).map(|d| {
                let (_y, m, dd) = civil_from_days(d);
                (
                    format!("{dd:02}.{m:02}"),
                    match cur_tf {
                        0 => vec![
                            (r.prompt_tokens - r.cached_tokens).max(0.0),
                            r.cached_tokens,
                            r.completion_tokens,
                        ],
                        1 => vec![
                            (r.prompt_tokens - r.cached_tokens).max(0.0),
                            r.cached_tokens,
                        ],
                        2 => vec![r.cached_tokens],
                        _ => vec![r.completion_tokens],
                    },
                )
            })
        })
        .collect();
    let (bar_colors, bar_legend): (Vec<&str>, Vec<String>) = match cur_tf {
        0 => (
            vec![C_ACCENT, C_WARNING, C_PRIMARY],
            vec![
                tr(&lang, "inputLabel").to_string(),
                tr(&lang, "inputCachedLabel").to_string(),
                tr(&lang, "outputLabel").to_string(),
            ],
        ),
        1 => (
            vec![C_ACCENT, C_WARNING],
            vec![
                tr(&lang, "inputLabel").to_string(),
                tr(&lang, "inputCachedLabel").to_string(),
            ],
        ),
        2 => (
            vec![C_WARNING],
            vec![tr(&lang, "inputCachedLabel").to_string()],
        ),
        _ => (
            vec![C_PRIMARY],
            vec![tr(&lang, "outputLabel").to_string()],
        ),
    };
    let bar_el = bar_chart_stacked(&lang, &bar_data, &bar_colors, &bar_legend);
    let token_breakdown = format!(
        "{} {} · {} {} · {} {}",
        tr(&lang, "inputLabel"),
        fmt_tokens(prompt),
        tr(&lang, "inputCachedLabel"),
        fmt_tokens(cached),
        tr(&lang, "outputLabel"),
        fmt_tokens(completion)
    );
    let pill_defs: Vec<(u8, String)> = vec![
        (0, tr(&lang, "both").to_string()),
        (1, tr(&lang, "inputLabel").to_string()),
        (2, tr(&lang, "inputCachedLabel").to_string()),
        (3, tr(&lang, "outputLabel").to_string()),
    ];
    let pill_els: Vec<Element> = pill_defs
        .iter()
        .map(|(k, lbl)| {
            let k = *k;
            let cls = if k == cur_tf {
                "pill active".to_string()
            } else {
                "pill".to_string()
            };
            rsx! {
                button { key: "p{k}", class: "{cls}",
                    onclick: move |_| token_filter.set(k),
                    "{lbl}"
                }
            }
        })
        .collect();

    // ---- Activity + tool usage cards ----
    let heatmap_el = heatmap(&lang, &sel);
    let donut_el = donut(&lang, tool_ok, tool_fail, tool_rej);

    // ---- range buttons + custom date inputs ----
    let range_defs: Vec<(u8, String)> = vec![
        (0, "7D".to_string()),
        (1, "30D".to_string()),
        (2, "90D".to_string()),
        (3, tr(&lang, "allTime").to_string()),
        (4, tr(&lang, "custom").to_string()),
    ];
    let range_btn_els: Vec<Element> = range_defs
        .iter()
        .map(|(k, lbl)| {
            let k = *k;
            let cls = if k == cur_range {
                "time-range-btn active".to_string()
            } else {
                "time-range-btn".to_string()
            };
            rsx! {
                button { key: "{k}", class: "{cls}",
                    onclick: move |_| range.set(k),
                    "{lbl}"
                }
            }
        })
        .collect();
    let custom_input_els: Vec<Element> = if cur_range == 4 {
        vec![
            rsx! {
                input { r#type: "date", class: "stats-date-input", value: "{cf}",
                    oninput: move |e| custom_from.set(e.value())
                }
            },
            rsx! { span { class: "session-row-meta", " – " } },
            rsx! {
                input { r#type: "date", class: "stats-date-input", value: "{ct}",
                    oninput: move |e| custom_to.set(e.value())
                }
            },
        ]
    } else {
        Vec::new()
    };

    // ---- session rows with drill-down (Nexus SessionList) ----
    let cur_visible = *visible.read();
    let cur_expanded = expanded.read().clone();
    let preview_cache = previews.read().clone();
    let subs_cache = subs.read().clone();
    let loading_now = detail_loading.read().clone();

    let analytics_rows: Vec<Element> = sel
        .iter()
        .take(cur_visible)
        .map(|r| {
            let sid = r.session_id.clone();
            let is_expanded = cur_expanded == sid;
            let title = if r.title.is_empty() {
                tr(&lang, "untitledSession").to_string()
            } else {
                r.title.clone()
            };
            let model = if r.model.is_empty() {
                "unknown".to_string()
            } else {
                r.model.clone()
            };
            let project_seg = r
                .working_directory
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("");
            let project = if project_seg.is_empty() {
                "—".to_string()
            } else {
                project_seg.to_string()
            };
            let date = iso_date(&r.start_time);
            let dur = fmt_duration(duration_secs(r));
            let cached_part = if r.cached_tokens > 0.0 {
                format!(
                    " ({} {})",
                    tr(&lang, "inputCachedLabel"),
                    fmt_tokens(r.cached_tokens)
                )
            } else {
                String::new()
            };
            let meta_line = format!(
                "{date} · {dur} · {} {}{} · {} {}",
                tr(&lang, "inputLabel"),
                fmt_tokens(r.prompt_tokens),
                cached_part,
                tr(&lang, "outputLabel"),
                fmt_tokens(r.completion_tokens),
            );
            let cache_cell: Element = if r.cached_tokens > 0.0 && r.prompt_tokens > 0.0 {
                let pct = (r.cached_tokens / r.prompt_tokens * 100.0).round() as i64;
                let title = format!(
                    "cached {} / prompt {}",
                    fmt_tokens(r.cached_tokens),
                    fmt_tokens(r.prompt_tokens)
                );
                rsx! { span { class: "session-row-cache", title: "{title}", "⚡ {pct}%" } }
            } else {
                VNode::empty()
            };
            let cost_s = format!("${:.2}", r.cost);

            let detail_el: Element = if is_expanded {
                let models_used = if r.models.is_empty() {
                    model.clone()
                } else {
                    r.models.join(", ")
                };
                let hit_s = if r.cached_tokens > 0.0 && r.prompt_tokens > 0.0 {
                    format!("{}%", (r.cached_tokens / r.prompt_tokens * 100.0).round() as i64)
                } else {
                    "—".to_string()
                };
                let started = match parse_iso(&r.start_time) {
                    Some((_d, ms)) => {
                        let hh = ms / 3_600_000;
                        let mm = (ms / 60_000) % 60;
                        format!("{} {hh:02}:{mm:02}", iso_date(&r.start_time))
                    }
                    None => iso_date(&r.start_time),
                };
                let detail_items: Vec<(&str, String)> = vec![
                    ("modelsUsedLabel", models_used),
                    ("inputLabel", fmt_tokens(r.prompt_tokens)),
                    ("inputCachedLabel", fmt_tokens(r.cached_tokens)),
                    ("cacheHitLabel", hit_s),
                    ("outputLabel", fmt_tokens(r.completion_tokens)),
                    ("totalTokensLabel", fmt_tokens(r.total_tokens)),
                    ("toolCallsLabel", fmt_tokens(r.tool_succeeded + r.tool_failed + r.tool_rejected)),
                    ("messagesLabel", r.messages.to_string()),
                    ("costLabel", format!("${:.4}", r.cost)),
                    ("durationLabel", fmt_duration(duration_secs(r))),
                    ("startedLabel", started),
                ];
                let grid_els: Vec<Element> = detail_items
                    .iter()
                    .map(|(key, val)| {
                        rsx! {
                            div { key: "{key}", class: "session-detail-item",
                                div { class: "session-detail-label", "{tr(&lang, key)}" }
                                div { class: "session-detail-value", "{val}" }
                            }
                        }
                    })
                    .collect();
                let project_item: Element = rsx! {
                    div { class: "session-detail-item wide",
                        div { class: "session-detail-label", "{tr(&lang, \"projectLabel\")}" }
                        div { class: "session-detail-value", title: "{r.working_directory}",
                            "{r.working_directory}"
                        }
                    }
                };
                // transcript (loaded lazily on expand)
                let preview = preview_cache
                    .iter()
                    .find(|(id, _)| *id == sid)
                    .map(|(_, p)| p.clone());
                let transcript_els: Vec<Element> = match &preview {
                    Some(msgs) if !msgs.is_empty() => {
                        msgs.iter().map(preview_msg_row).collect()
                    }
                    Some(_) => vec![rsx! {
                        div { class: "empty-text", "{tr(&lang, \"noData\")}" }
                    }],
                    None => vec![rsx! {
                        span { class: "loading-indicator", "{tr(&lang, \"loading\")}" }
                    }],
                };
                // subagent transcripts
                let sub_els: Vec<Element> = subs_cache
                    .iter()
                    .find(|(id, _)| *id == sid)
                    .map(|(_, list)| {
                        list.iter()
                            .map(|s| subagent_block(s, &lang))
                            .collect()
                    })
                    .unwrap_or_default();
                let sub_section: Element = if sub_els.is_empty() {
                    VNode::empty()
                } else {
                    rsx! {
                        div { class: "subagent-blocks",
                            {sub_els.iter().cloned()}
                        }
                    }
                };
                let load_note: Element = if loading_now == sid {
                    rsx! { span { class: "loading-indicator", "{tr(&lang, \"loading\")}" } }
                } else {
                    VNode::empty()
                };
                rsx! {
                    div { class: "session-detail",
                        div { class: "session-detail-grid",
                            {grid_els.iter().cloned()}
                            {project_item}
                        },
                        {load_note}
                        div { class: "session-detail-label", "{tr(&lang, \"messagesLabel\")}" }
                        {transcript_els.iter().cloned()}
                        {sub_section}
                    }
                }
            } else {
                VNode::empty()
            };

            let row_cls = format!(
                "session-row session-row-clickable{}",
                if is_expanded { " expanded" } else { "" }
            );
            rsx! {
                div { key: "{sid}", class: "{row_cls}",
                    onclick: {
                        let state = state.clone();
                        let sid_c = sid.clone();
                        let mut expanded = expanded.clone();
                        let mut detail_loading = detail_loading.clone();
                        move |_| {
                            let newv = if expanded.read().clone() == sid_c {
                                String::new()
                            } else {
                                sid_c.clone()
                            };
                            expanded.set(newv.clone());
                            if newv.is_empty() {
                                return;
                            }
                            let has_preview = previews.read().iter().any(|(id, _)| *id == newv);
                            let has_subs = subs.read().iter().any(|(id, _)| *id == newv);
                            if has_preview && has_subs {
                                return;
                            }
                            detail_loading.set(newv.clone());
                            let st = state.clone();
                            let sidf = newv.clone();
                            let mut previews = previews.clone();
                            let mut subs = subs.clone();
                            let mut detail_loading = detail_loading.clone();
                            spawn(async move {
                                if !previews.read().iter().any(|(id, _)| *id == sidf) {
                                    let msgs = crate::vibestats::api_session_preview(
                                        token(&st), sidf.clone(),
                                    )
                                    .await
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default();
                                    let mut v = previews.write();
                                    v.push((sidf.clone(), msgs));
                                }
                                if !subs.read().iter().any(|(id, _)| *id == sidf) {
                                    let list = crate::vibestats::api_subagent_logs(
                                        token(&st), sidf.clone(),
                                    )
                                    .await
                                    .unwrap_or_default();
                                    let mut v = subs.write();
                                    v.push((sidf.clone(), list));
                                }
                                detail_loading.set(String::new());
                            });
                        }
                    },
                    div { class: "session-row-main",
                        div { class: "session-row-title", "{title}" }
                        div { class: "session-row-meta", "{meta_line}" }
                    },
                    div { class: "session-row-tags",
                        span { class: "tag primary", "{project}" }
                        span { class: "tag accent", "{model}" }
                    },
                    {cache_cell},
                    span { class: "session-row-cost", "{cost_s}" },
                    {detail_el}
                }
            }
        })
        .collect();

    let more_left = sel.len().saturating_sub(cur_visible);
    let load_more_el: Element = if more_left > 0 {
        rsx! {
            button { class: "time-range-btn",
                onclick: move |_| {
                    let n = *visible.read() + 20;
                    visible.set(n);
                },
                "{tr(&lang, \"loadMoreLabel\")} (+{more_left.min(20)})"
            }
        }
    } else {
        VNode::empty()
    };

    rsx! {
        div { class: "stats-view",
            // Z8: one toolbar - title, range filters and refresh in a single
            // row, exactly like the stable edition (StatsView stats-toolbar).
            div { class: "stats-toolbar",
                span { class: "stats-title", "{tr(&lang, \"statistics\")}" }
                div { class: "time-range-btns",
                    {range_btn_els.iter().cloned()}
                    {custom_input_els.iter().cloned()}
                }
                if busy {
                    span { class: "loading-indicator", "{tr(&lang, \"agentWorkingLabel\")}" }
                }
                button {
                    class: "refresh-btn",
                    title: "{tr(&lang, \"refresh\")}",
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
                    "⟳"
                }
            }
            div { class: "stats-scroll",
                if !err.is_empty() {
                    div { class: "git-error", "{err}" }
                }
                // analytics cards (Nexus StatsView 1:1)
                div { class: "cards-row",
                    div { class: "stat-card",
                        div { class: "card-header",
                            div {
                                div { class: "card-subtitle", "{tr(&lang, \"totalSpend\")}" }
                                div { class: "card-value primary", "{cost_value}" }
                            }
                            div { class: "card-meta", "{cost_meta}" }
                        },
                        {model_select},
                        div { class: "card-chart", {cost_chart} }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div {
                                div { class: "card-subtitle", "{tr(&lang, \"totalTokens\")}" }
                                div { class: "card-value accent", "{fmt_tokens(display_total)}" }
                            }
                            div { class: "card-meta",
                                div { "{tps_avg.round() as i64} {tr(&lang, \"tokPerSecAvg\")}" }
                                div { class: "cache-hit-meta", "⚡ {tr(&lang, \"cacheHitLabel\")} {hit_pct}%" }
                                div { "{token_breakdown}" }
                            }
                        },
                        div { class: "pill-row", {pill_els.iter().cloned()} },
                        div { class: "card-chart", {bar_el} }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div {
                                div { class: "card-subtitle", "{tr(&lang, \"activity\")}" }
                                div { class: "card-value success",
                                    "{tf(&lang, \"sessionsCount\", &[(\"count\", &sel_count.to_string())])}"
                                }
                            }
                            div { class: "card-meta",
                                div { "{tr(&lang, \"avg\")} {fmt_duration(avg_dur)}" }
                                div { "{tr(&lang, \"total\")} {fmt_duration(total_dur)}" }
                            }
                        },
                        div { class: "card-chart heatmap-wrap", {heatmap_el} }
                    }
                    div { class: "stat-card",
                        div { class: "card-header",
                            div {
                                div { class: "card-subtitle", "{tr(&lang, \"toolCalls\")}" }
                                div { class: "card-value success",
                                    "{fmt_tokens(tool_ok + tool_fail + tool_rej)}" }
                            }
                            div { class: "card-meta",
                                if tool_rej > 0.0 {
                                    div { "{fmt_tokens(tool_rej)} {tr(&lang, \"rejectedLabel\")}" }
                                }
                                if tool_fail > 0.0 {
                                    div { "{fmt_tokens(tool_fail)} {tr(&lang, \"failedLabel\")}" }
                                }
                            }
                        },
                        div { class: "card-chart", {donut_el} }
                    }
                }
                // session list with drill-down
                div { class: "session-list-card",
                    div { class: "section-label", "{tr(&lang, \"allSessions\")} ({sel_count})" }
                    div { class: "session-rows",
                        if analytics_rows.is_empty() {
                            div { class: "empty-text", "{tr(&lang, \"noSessionsFound\")}" }
                        }
                        {analytics_rows.iter().cloned()}
                        {load_more_el}
                    }
                }
            }
        }
    }
}
