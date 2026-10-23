//! Stats view: Nexus-style usage dashboard built from the Mistral Vibe CLI
//! session logs (`~/.vibe/logs/session/`) — toolbar with time-range pills,
//! cost / token / activity / tool-usage cards with axis-labelled painter
//! charts, a cache-hit-rate card, a recent-sessions list with tags and a
//! drill-down that renders the conversation like the chat view
//! (markdown, avatars, user bubbles).

use eframe::egui::{self, Ui};
use vibedeck_vibe::stats::{
    aggregate, load_session_messages, load_subagent_sessions, VibeMessage, VibeSession,
    VibeSubagentSession,
};

use crate::app::VibedeckApp;
use crate::charts;
use crate::theme::Palette;
use crate::i18n::Str;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeRange {
    #[default]
    All,
    D7,
    D30,
    D90,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TokenFilter {
    #[default]
    Both,
    Input,
    Output,
}

#[derive(Debug, Default)]
pub struct StatsPane {
    pub sessions: Vec<VibeSession>,
    pub loaded: bool,
    pub selected: Option<usize>,
    pub messages: Vec<VibeMessage>,
    pub subagents: Vec<VibeSubagentSession>,
    pub range: TimeRange,
    /// Custom range bounds, `YYYY-MM-DD`.
    pub custom_from: String,
    pub custom_to: String,
    /// Empty = all models.
    pub model_filter: String,
    /// Token card series filter.
    pub token_filter: TokenFilter,
}

impl StatsPane {
    pub fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        self.sessions = vibedeck_vibe::stats::load_sessions();
        self.loaded = true;
        self.selected = None;
        self.messages.clear();
        self.subagents.clear();
    }

    fn select(&mut self, idx: usize) {
        self.selected = Some(idx);
        if let Some(session) = self.sessions.get(idx) {
            let id = session.session_id.clone();
            self.messages = load_session_messages(&id);
            self.subagents = load_subagent_sessions(&id);
        }
    }

    fn clear_selection(&mut self) {
        self.selected = None;
        self.messages.clear();
        self.subagents.clear();
    }
}

// -- date helpers (no chrono: civil-from-days / days-from-civil) ----------------

/// Days since 1970-01-01 (Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of `days_from_civil` → (y, m, d).
#[cfg_attr(not(test), allow(dead_code))]
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn now_days() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 / 86_400)
        .unwrap_or(0)
}

#[cfg_attr(not(test), allow(dead_code))]
fn ymd(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn parse_ymd(s: &str) -> Option<i64> {
    let mut it = s.split('-');
    let y = it.next()?.trim().parse::<i64>().ok()?;
    let m = it.next()?.trim().parse::<i64>().ok()?;
    let d = it.next()?.trim().parse::<i64>().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// `start_time` → its date part (`YYYY-MM-DD`, lexicographically sortable).
fn session_day(s: &VibeSession) -> i64 {
    s.start_time.get(0..10).and_then(parse_ymd).unwrap_or(0)
}

/// Parse the fixed ISO prefix `YYYY-MM-DDTHH:MM:SS` into epoch seconds.
fn iso_to_epoch(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> { s.get(a..z)?.parse::<i64>().ok() };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    Some((days_from_civil(y, mo, d) * 86_400 + h * 3_600 + mi * 60 + sec) as f64)
}

fn session_duration_s(s: &VibeSession) -> f64 {
    iso_to_epoch(&s.end_time)
        .zip(iso_to_epoch(&s.start_time))
        .map(|(e, b)| (e - b).max(0.0))
        .unwrap_or(0.0)
}

fn fmt_duration(mut secs: f64) -> String {
    if secs <= 0.0 {
        return "—".into();
    }
    let h = (secs / 3600.0) as i64;
    secs -= h as f64 * 3600.0;
    let m = (secs / 60.0) as i64;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {:02.0}s", secs - m as f64 * 60.0)
    } else {
        format!("{secs:.0}s")
    }
}

fn fmt_count(v: f64) -> String {
    let n = v.round() as i64;
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(' ');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn fmt_cost(v: f64) -> String {
    if v >= 10.0 {
        format!("${v:.2}")
    } else {
        format!("${v:.4}")
    }
}

/// Compact axis label: `23.10`.
fn day_label(days: i64) -> String {
    let (_, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}")
}

/// Locale-ish full date: `2026-10-23T22:02:03` → `23.10.2026`.
fn iso_date(iso: &str) -> String {
    let days = iso.get(0..10).and_then(parse_ymd).unwrap_or(0);
    let (y, m, d) = civil_from_days(days);
    format!("{d:02}.{m:02}.{y}")
}

fn short_time(s: &str) -> String {
    s.chars().take(16).collect::<String>().replacen('T', " ", 1)
}

fn short_path(p: &str) -> String {
    let p = p.trim_end_matches('/');
    match p.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => p.to_string(),
    }
}

// -- main view --------------------------------------------------------------------

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let t = app.tr();
    app.stats_pane.ensure_loaded();

    live_usage(app, ui, t);

    let p = app.palette();

    // ---- toolbar: title + range pills + custom dates + refresh --------------
    ui.heading(egui::RichText::new(t.view_stats).strong());
    ui.add_space(4.0);
    let pane = &mut app.stats_pane;
    let mut range = pane.range;
    ui.horizontal_wrapped(|ui| {
        let pill = |ui: &mut Ui, r: TimeRange, label: &str, active: bool| {
            ui.add(
                egui::Button::new(
                    egui::RichText::new(label)
                        .small()
                        .color(if active { p.text } else { p.muted }),
                )
                .fill(if active { p.accent } else { egui::Color32::TRANSPARENT })
                .rounding(egui::Rounding::same(999.0)),
            )
            .clicked()
            .then_some(r)
        };
        for (r, label) in [
            (TimeRange::D7, t.stats_7d),
            (TimeRange::D30, t.stats_30d),
            (TimeRange::D90, t.stats_90d),
            (TimeRange::All, t.stats_all_time),
            (TimeRange::Custom, t.stats_custom),
        ] {
            if let Some(r) = pill(ui, r, label, range == r) {
                range = r;
            }
        }
        if range == TimeRange::Custom {
            ui.label(egui::RichText::new(t.stats_from).small().weak());
            ui.add(
                egui::TextEdit::singleline(&mut pane.custom_from)
                    .desired_width(90.0)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("YYYY-MM-DD"),
            );
            ui.label(egui::RichText::new(t.stats_to).small().weak());
            ui.add(
                egui::TextEdit::singleline(&mut pane.custom_to)
                    .desired_width(90.0)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("YYYY-MM-DD"),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("↻").on_hover_text(t.reload).clicked() {
                pane.reload();
            }
        });
    });
    pane.range = range;

    let today = now_days();
    let (from_days, to_days) = match pane.range {
        TimeRange::D7 => (today - 6, today),
        TimeRange::D30 => (today - 29, today),
        TimeRange::D90 => (today - 89, today),
        TimeRange::All => (
            pane.sessions
                .iter()
                .map(session_day)
                .filter(|d| *d > 0)
                .min()
                .unwrap_or(today),
            today,
        ),
        TimeRange::Custom => {
            let f = parse_ymd(&pane.custom_from).unwrap_or(today - 29);
            let to = parse_ymd(&pane.custom_to).unwrap_or(today);
            (f.min(to), to.max(f))
        }
    };

    if pane.sessions.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(t.stats_no_data).weak());
        return;
    }

    // ---- filtering (indices into pane.sessions; newest first on disk) --------
    let mf = pane.model_filter.clone();
    let filtered: Vec<usize> = pane
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            session_day(s) >= from_days
                && session_day(s) <= to_days
                && (mf.is_empty()
                    || s.config
                        .as_ref()
                        .and_then(|c| c.active_model.as_deref())
                        .map(|m| m == mf)
                        .unwrap_or(false))
        })
        .map(|(i, _)| i)
        .collect();
    let selected: Vec<VibeSession> = filtered
        .iter()
        .map(|&i| pane.sessions[i].clone())
        .collect();

    let totals = aggregate(&selected);
    let n = selected.len().max(1) as f64;
    let tps_vals: Vec<f64> = selected
        .iter()
        .map(|s| s.stats.tokens_per_second)
        .filter(|v| *v > 0.0)
        .collect();
    let avg_tps = if tps_vals.is_empty() {
        0.0
    } else {
        tps_vals.iter().sum::<f64>() / tps_vals.len() as f64
    };

    // distinct models (for the cost-card selector)
    let mut models: Vec<String> = selected
        .iter()
        .filter_map(|s| {
            s.config
                .as_ref()
                .and_then(|c| c.active_model.clone())
                .filter(|m| !m.is_empty())
        })
        .collect();
    models.sort();
    models.dedup();

    // day buckets for the charts
    let nbuckets = (to_days - from_days + 1).max(1) as usize;
    let mut day_costs: Vec<f64> = vec![0.0; nbuckets];
    let mut day_prompt: Vec<f64> = vec![0.0; nbuckets];
    let mut day_completion: Vec<f64> = vec![0.0; nbuckets];
    for s in &selected {
        let idx = (session_day(s) - from_days) as usize;
        if idx < nbuckets {
            day_costs[idx] += s.stats.session_cost;
            day_prompt[idx] += s.stats.session_prompt_tokens;
            day_completion[idx] += s.stats.session_completion_tokens;
        }
    }
    let mut cumulative: Vec<f64> = Vec::with_capacity(nbuckets);
    let mut acc = 0.0;
    for v in &day_costs {
        acc += *v;
        cumulative.push(acc);
    }
    let x_labels: Vec<String> = (0..nbuckets).map(|i| day_label(from_days + i as i64)).collect();

    // heatmap: sessions per day, 13 weeks, Monday-first, oldest week first
    let weeks = 13usize;
    let mut heat = vec![0u32; weeks * 7];
    let today_dow = ((today % 7) + 7 + 3) % 7; // Monday = 0
    for s in &selected {
        let d = session_day(s);
        let delta = today - d;
        if (0..weeks as i64 * 7).contains(&delta) {
            let col_from_right = (delta + today_dow) / 7;
            if col_from_right < weeks as i64 {
                let w = weeks as i64 - 1 - col_from_right;
                let dow = ((d % 7) + 7 + 3) % 7;
                heat[(w * 7 + dow) as usize] += 1;
            }
        }
    }

    let cached: f64 = selected.iter().map(|s| s.stats.cached_tokens).sum();
    let prompt: f64 = totals.prompt_tokens;
    let total_dur: f64 = selected.iter().map(session_duration_s).sum();
    let avg_dur = total_dur / n;

    // ---- cards ----------------------------------------------------------------
    let card_w = ((ui.available_width() - 16.0) / 2.0).max(240.0) - 32.0;
    let mut session_click: Option<usize> = None;
    let pane = &mut app.stats_pane;
    let heat_color = egui::Color32::from_rgb(0x47, 0xd4, 0x7f);
    let blue = egui::Color32::from_rgb(0x4d, 0xa3, 0xff);

    egui::Grid::new("stats-cards")
        .num_columns(2)
        .spacing([16.0, 16.0])
        .show(ui, |ui| {
            // ---- Cost card (model selector inside, like Nexus) --------------
            ui.vertical(|ui| {
                card(ui, &p, card_w, |ui| {
                    card_header(
                        ui,
                        &p,
                        t.stats_total_spend,
                        &fmt_cost(totals.cost),
                        p.accent,
                        &[
                            format!("{} {}", selected.len(), t.stats_sessions),
                            format!("{} {} / {}", fmt_cost(totals.cost / n), t.stats_avg_cost, t.stats_sessions),
                        ],
                    );
                    if models.len() > 1 {
                        let mut model_pick: Option<String> = None;
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("stats-model-select")
                                .selected_text(if pane.model_filter.is_empty() {
                                    t.stats_all_models.to_string()
                                } else {
                                    pane.model_filter.clone()
                                })
                                .width(ui.available_width().min(200.0))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut model_pick, Some(String::new()), t.stats_all_models);
                                    for m in &models {
                                        ui.selectable_value(&mut model_pick, Some(m.clone()), m);
                                    }
                                });
                        });
                        if let Some(m) = model_pick {
                            pane.model_filter = m;
                        }
                        ui.add_space(4.0);
                    }
                    charts::area_chart(ui, "stats-cost-area", &cumulative, &x_labels, p.accent, &|v| {
                        fmt_cost(v)
                    });
                });
            });

            // ---- Token card (pills: both / input / output + legend) ----------
            ui.vertical(|ui| {
                card(ui, &p, card_w, |ui| {
                    let (disp_total, _disp_in, _disp_out) = match pane.token_filter {
                        TokenFilter::Both => (
                            totals.prompt_tokens + totals.completion_tokens,
                            totals.prompt_tokens,
                            totals.completion_tokens,
                        ),
                        TokenFilter::Input => (totals.prompt_tokens, totals.prompt_tokens, 0.0),
                        TokenFilter::Output => (totals.completion_tokens, 0.0, totals.completion_tokens),
                    };
                    card_header(
                        ui,
                        &p,
                        t.stats_total_tokens,
                        &fmt_count(disp_total),
                        blue,
                        &[
                            format!("{:.0} {}", avg_tps, t.tok_per_sec),
                            format!("{} / {}", fmt_count(totals.prompt_tokens), fmt_count(totals.completion_tokens)),
                        ],
                    );
                    ui.horizontal(|ui| {
                        for (f, label) in [
                            (TokenFilter::Both, t.stats_both),
                            (TokenFilter::Input, t.stats_input),
                            (TokenFilter::Output, t.stats_output),
                        ] {
                            let active = pane.token_filter == f;
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(label)
                                            .small()
                                            .color(if active { p.text } else { p.muted }),
                                    )
                                    .fill(if active { p.accent } else { egui::Color32::TRANSPARENT })
                                    .rounding(egui::Rounding::same(999.0)),
                                )
                                .clicked()
                            {
                                pane.token_filter = f;
                            }
                        }
                    });
                    ui.add_space(2.0);
                    let (series_a, series_b, color_a, color_b, legend) = match pane.token_filter {
                        TokenFilter::Both => (
                            day_prompt.clone(),
                            day_completion.clone(),
                            p.accent,
                            blue,
                            vec![
                                (totals.prompt_tokens, p.accent, t.stats_input.to_string()),
                                (totals.completion_tokens, blue, t.stats_output.to_string()),
                            ],
                        ),
                        TokenFilter::Input => (
                            day_prompt.clone(),
                            Vec::new(),
                            p.accent,
                            blue,
                            vec![(totals.prompt_tokens, p.accent, t.stats_input.to_string())],
                        ),
                        TokenFilter::Output => (
                            day_completion.clone(),
                            Vec::new(),
                            blue,
                            p.accent,
                            vec![(totals.completion_tokens, blue, t.stats_output.to_string())],
                        ),
                    };
                    charts::legend_row(ui, &legend);
                    charts::bar_chart(
                        ui,
                        "stats-tokens-bar",
                        &series_a,
                        &series_b,
                        color_a,
                        color_b,
                        &x_labels,
                        &|v| fmt_count(v),
                    );
                });
            });
            ui.end_row();

            // ---- Activity card (heatmap) -------------------------------------
            ui.vertical(|ui| {
                card(ui, &p, card_w, |ui| {
                    card_header(
                        ui,
                        &p,
                        t.stats_activity,
                        &selected.len().to_string(),
                        p.green,
                        &[
                            format!("{} {}", t.stats_avg_duration, fmt_duration(avg_dur)),
                            format!("{} {}", t.stats_total_duration, fmt_duration(total_dur)),
                        ],
                    );
                    charts::heatmap(ui, "stats-heat", &heat, weeks, heat_color);
                });
            });

            // ---- Tool usage card (donut with total) ---------------------------
            ui.vertical(|ui| {
                card(ui, &p, card_w, |ui| {
                    let tool_total = totals.tool_calls_succeeded
                        + totals.tool_calls_failed
                        + totals.tool_calls_rejected;
                    let mut meta = Vec::new();
                    if totals.tool_calls_rejected > 0.0 {
                        meta.push(format!("⚠ {} {}", totals.tool_calls_rejected as i64, t.stats_rejected));
                    }
                    if totals.tool_calls_failed > 0.0 {
                        meta.push(format!("✖ {} {}", totals.tool_calls_failed as i64, t.stats_failed));
                    }
                    card_header(
                        ui,
                        &p,
                        t.stats_tool_usage,
                        &(tool_total as i64).to_string(),
                        p.green,
                        &meta,
                    );
                    if tool_total <= 0.0 {
                        ui.label(egui::RichText::new(t.stats_no_data).weak());
                    } else {
                        let entries = vec![
                            (totals.tool_calls_succeeded, p.green, t.stats_succeeded.to_string()),
                            (totals.tool_calls_failed, p.red, t.stats_failed.to_string()),
                            (totals.tool_calls_rejected, p.yellow, t.stats_rejected.to_string()),
                        ];
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            charts::donut(ui, "stats-tools-donut", &entries, t.stats_totals, &(tool_total as i64).to_string());
                        });
                        ui.add_space(4.0);
                        charts::legend_row(ui, &entries);
                    }
                });
            });
            ui.end_row();

            // ---- Cache hit rate card -----------------------------------------
            ui.vertical(|ui| {
                card(ui, &p, card_w, |ui| {
                    let hit = if prompt > 0.0 {
                        format!("{:.1}%", (cached / prompt * 100.0).clamp(0.0, 100.0))
                    } else {
                        "—".into()
                    };
                    card_header(
                        ui,
                        &p,
                        t.cache_hit_rate,
                        &hit,
                        blue,
                        &[
                            format!("{} {}", fmt_count(cached), t.cache_tokens),
                            format!("{} {}", fmt_count(prompt), t.stats_prompt_tokens),
                        ],
                    );
                    ui.label(
                        egui::RichText::new(format!("{:.1} {}", avg_tps, t.tok_per_sec))
                            .small()
                            .monospace()
                            .weak(),
                    );
                });
            });

            // ---- Recent sessions card ----------------------------------------
            ui.vertical(|ui| {
                card(ui, &p, card_w, |ui| {
                    session_list(ui, t, &p, &pane.sessions, &filtered, pane.selected, &mut session_click);
                });
            });
            ui.end_row();
        });

    if let Some(i) = session_click {
        app.stats_pane.select(i);
    }

    drill_down(app, ui, t);
}

// -- helpers ----------------------------------------------------------------------

/// Nexus `.stat-card`: surface background, 12px rounding, 16px padding.
fn card(ui: &mut Ui, p: &Palette, min_w: f32, body: impl FnOnce(&mut Ui)) {
    egui::Frame::default()
        .fill(p.surface)
        .rounding(12.0)
        .inner_margin(egui::Margin::same(16.0))
        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle))
        .show(ui, |ui| {
            ui.set_min_width(min_w.max(180.0));
            ui.set_width(ui.available_width());
            body(ui);
        });
}

/// Nexus `.card-header`: subtitle + big value on the left, meta column on
/// the right.
fn card_header(
    ui: &mut Ui,
    p: &Palette,
    subtitle: &str,
    value: &str,
    value_color: egui::Color32,
    meta: &[String],
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(subtitle).size(12.0).color(p.muted));
            ui.label(
                egui::RichText::new(value)
                    .size(26.0)
                    .strong()
                    .color(value_color),
            );
        });
        if !meta.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width().min(150.0));
                    for m in meta {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            ui.label(egui::RichText::new(m).size(11.0).color(p.muted));
                        });
                    }
                });
            });
        }
    });
    ui.add_space(12.0);
}

/// Nexus `.session-row`: title + date/duration on the left, project/model
/// tags, cost on the right.
#[allow(clippy::too_many_arguments)]
fn session_list(
    ui: &mut Ui,
    t: &Str,
    p: &Palette,
    sessions: &[VibeSession],
    filtered: &[usize],
    selected: Option<usize>,
    click: &mut Option<usize>,
) {
    let list: Vec<(usize, &VibeSession)> =
        filtered.iter().map(|&i| (i, &sessions[i])).collect();
    if list.is_empty() {
        ui.label(egui::RichText::new(t.stats_no_data).weak());
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(302.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, s) in list.into_iter().take(20) {
                let title = s
                    .title
                    .clone()
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| t.stats_untitled.to_string());
                let short: String = if title.chars().count() > 64 {
                    let mut x: String = title.chars().take(64).collect();
                    x.push('…');
                    x
                } else {
                    title
                };
                let date = iso_date(&s.start_time);
                let dur = fmt_duration(session_duration_s(s));
                let project = s
                    .environment
                    .as_ref()
                    .and_then(|e| e.working_directory.clone())
                    .map(|w| short_path(&w))
                    .unwrap_or_default();
                let model = s
                    .config
                    .as_ref()
                    .and_then(|c| c.active_model.clone())
                    .unwrap_or_default();
                let cost = fmt_cost(s.stats.session_cost);
                let is_sel = selected == Some(i);
                let resp = egui::Frame::default()
                    .fill(if is_sel { p.panel_hover } else { p.bg })
                    .rounding(8.0)
                    .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                    .stroke(egui::Stroke::new(
                        if is_sel { 1.5_f32 } else { 0.0 },
                        p.accent,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new(short.clone()).size(13.0).strong());
                                ui.label(
                                    egui::RichText::new(format!("{date} · {dur}"))
                                        .size(11.0)
                                        .color(p.muted),
                                );
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(cost.clone())
                                            .size(13.0)
                                            .strong()
                                            .color(p.accent),
                                    );
                                    ui.add_space(8.0);
                                    if !model.is_empty() {
                                        tag(ui, p, &model, p.accent);
                                        ui.add_space(6.0);
                                    }
                                    if !project.is_empty() {
                                        tag(ui, p, &project, egui::Color32::from_rgb(0x4d, 0xa3, 0xff));
                                    }
                                },
                            );
                        });
                    });
                if resp.response.interact(egui::Sense::click()).clicked() {
                    *click = Some(i);
                }
            }
        });
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("{}: {}", t.stats_sessions, filtered.len()))
            .size(11.0)
            .color(p.muted),
    );
}

/// Small rounded tag pill (Nexus `.tag`).
fn tag(ui: &mut Ui, p: &Palette, text: &str, color: egui::Color32) {
    egui::Frame::default()
        .fill(color.gamma_multiply(0.12))
        .rounding(20.0)
        .inner_margin(egui::Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(11.0).color(color));
        });
    let _ = p;
}

/// Session drill-down: renders the conversation like the chat view —
/// markdown for assistant messages, right-aligned bubbles for user
/// messages, muted tool blocks otherwise.
fn drill_down(app: &mut VibedeckApp, ui: &mut Ui, t: &Str) {
    let p = app.palette();
    let Some(idx) = app.stats_pane.selected else {
        return;
    };
    let Some(s) = app.stats_pane.sessions.get(idx) else {
        return;
    };
    let title = s
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| s.session_id.clone());
    let date = iso_date(&s.start_time);
    let dur = fmt_duration(session_duration_s(s));
    let model = s
        .config
        .as_ref()
        .and_then(|c| c.active_model.clone())
        .unwrap_or_default();
    let project = s
        .environment
        .as_ref()
        .and_then(|e| e.working_directory.clone())
        .map(|w| short_path(&w))
        .unwrap_or_default();
    let branch = s.git_branch.clone().unwrap_or_else(|| "—".into());
    let cost = fmt_cost(s.stats.session_cost);
    let start_short = short_time(&s.start_time);
    let end_short = s.end_time.get(11..16).map(|x| x.to_string()).unwrap_or_default();
    let wd = s
        .environment
        .as_ref()
        .and_then(|e| e.working_directory.clone());
    let mut close = false;

    ui.add_space(12.0);
    egui::Frame::default()
        .fill(p.surface)
        .rounding(12.0)
        .inner_margin(egui::Margin::same(16.0))
        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if ui
                .button(egui::RichText::new(format!("← {}", t.stats_back)).small())
                .clicked()
            {
                close = true;
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(title).size(16.0).strong());
            ui.label(
                egui::RichText::new(format!(
                    "{} · {} — {} ({}) · {}: {}",
                    date, start_short, end_short, dur, t.stats_branch, branch
                ))
                .size(11.0)
                .color(p.muted),
            );
            ui.horizontal_wrapped(|ui| {
                if !project.is_empty() {
                    tag(ui, &p, &project, egui::Color32::from_rgb(0x4d, 0xa3, 0xff));
                    ui.add_space(6.0);
                }
                if !model.is_empty() {
                    tag(ui, &p, &model, p.accent);
                    ui.add_space(6.0);
                }
                tag(ui, &p, &cost, p.session_active_text);
            });
            if let Some(wd) = wd {
                ui.label(egui::RichText::new(wd).size(11.0).monospace().color(p.muted));
            }
            ui.add_space(8.0);

            ui.label(egui::RichText::new(t.stats_messages).strong());
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .max_height(420.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for msg in &app.stats_pane.messages {
                        transcript_row(ui, msg, &p);
                        ui.add_space(6.0);
                    }
                });

            for sub in &app.stats_pane.subagents {
                ui.add_space(6.0);
                egui::CollapsingHeader::new(format!(
                    "🤖 {} ({})",
                    sub.agent_name,
                    short_time(&sub.start_time)
                ))
                .default_open(false)
                .id_salt(format!("sub-{}", sub.session_id))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .auto_shrink([false, true])
                        .id_salt(format!("sub-scroll-{}", sub.session_id))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for msg in &sub.messages {
                                transcript_row(ui, msg, &p);
                                ui.add_space(6.0);
                            }
                        });
                });
            }
        });
    if close {
        app.stats_pane.clear_selection();
    }
}

/// One message of a drill-down transcript, rendered like the chat view.
fn transcript_row(ui: &mut Ui, msg: &VibeMessage, p: &Palette) {
    let content = msg.content.clone().unwrap_or_default();
    if content.trim().is_empty() {
        return;
    }
    match msg.role.as_str() {
        "user" => {
            let avail = ui.available_width();
            let max_w = (avail * 0.75).max(120.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                egui::Frame::default()
                    .inner_margin(egui::Margin::symmetric(18.0, 10.0))
                    .rounding(egui::Rounding {
                        nw: 18.0,
                        ne: 18.0,
                        sw: 4.0,
                        se: 18.0,
                    })
                    .fill(p.panel)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width().min(max_w - 24.0));
                        ui.label(egui::RichText::new(&content).color(p.text));
                    });
            });
        }
        "assistant" => {
            egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(12.0, 0.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                        let painter = ui.painter_at(rect);
                        painter.rect_filled(rect, egui::Rounding::same(6.0), p.accent);
                        crate::theme::paint_flame_mono_at(&painter, rect, egui::Color32::WHITE);
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - 8.0);
                            for tc in &msg.tool_calls {
                                let name =
                                    tc.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                                let raw = tc
                                    .pointer("/arguments/raw")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                ui.label(
                                    egui::RichText::new(format!("🔧 {name} {raw}"))
                                        .small()
                                        .monospace()
                                        .color(p.muted),
                                );
                            }
                            crate::md::markdown(ui, &content);
                        });
                    });
                });
        }
        _ => {
            ui.horizontal_wrapped(|ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new("🔧").small().color(p.yellow));
                let capped: String = content.chars().take(2000).collect();
                ui.label(
                    egui::RichText::new(capped)
                        .small()
                        .monospace()
                        .color(p.muted),
                );
            });
        }
    }
}

fn live_usage(app: &mut VibedeckApp, ui: &mut Ui, t: &Str) {
    let usage = &app.usage;
    if usage.updates == 0 {
        return;
    }
    let p = app.palette();
    egui::Frame::default()
        .fill(p.panel)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(10.0))
        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("⚡ {}", t.live_usage))
                    .small()
                    .strong()
                    .color(p.session_active_text),
            );
            ui.add_space(4.0);
            let model = app
                .option_value("model")
                .map(|o| o.current_value.clone())
                .unwrap_or_default();
            let prices = vibedeck_vibe::model_prices();
            let cost = vibedeck_vibe::estimate_cost(
                &prices,
                &model,
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.cached_tokens,
            );
            egui::Grid::new("stats-live")
                .num_columns(4)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    stat_cell(ui, t.stats_prompt_tokens, &fmt_count(usage.prompt_tokens));
                    stat_cell(
                        ui,
                        t.stats_completion_tokens,
                        &fmt_count(usage.completion_tokens),
                    );
                    stat_cell(ui, t.cache_tokens, &fmt_count(usage.cached_tokens));
                    stat_cell(
                        ui,
                        t.cache_hit_rate,
                        &format!("{:.1}%", usage.cache_hit_rate() * 100.0),
                    );
                    stat_cell(
                        ui,
                        t.tok_per_sec,
                        &format!("{:.1}", usage.tokens_per_second),
                    );
                    stat_cell(
                        ui,
                        t.context_used,
                        &if usage.context_size > 0.0 {
                            format!(
                                "{:.0}k / {:.0}k",
                                usage.context_used / 1000.0,
                                usage.context_size / 1000.0
                            )
                        } else {
                            "—".into()
                        },
                    );
                    stat_cell(ui, t.cost_estimate, &fmt_cost(cost));
                    stat_cell(
                        ui,
                        t.stats_model,
                        &if model.is_empty() { "—".to_string() } else { model },
                    );
                });
        });
    ui.add_space(10.0);
}

/// Compact label:value pair (no end_row) for the live-usage grid.
fn stat_cell(ui: &mut Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).size(11.0).weak());
    ui.label(egui::RichText::new(value).size(11.0).strong().monospace());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        for days in [0i64, 1, 100, 19_000, 20_000, 25_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn ymd_parse() {
        assert_eq!(parse_ymd("2026-10-23"), Some(days_from_civil(2026, 10, 23)));
        assert_eq!(parse_ymd("2026-13-01"), None);
        assert_eq!(parse_ymd("junk"), None);
    }

    #[test]
    fn iso_parse() {
        assert_eq!(iso_to_epoch("1970-01-01T00:00:00"), Some(0.0));
        assert_eq!(iso_to_epoch("2026-10-23 12:00:00.5").is_some(), true);
        assert_eq!(iso_to_epoch("bad"), None);
    }

    #[test]
    fn duration_fmt() {
        assert_eq!(fmt_duration(0.0), "—");
        assert_eq!(fmt_duration(45.0), "45s");
        assert_eq!(fmt_duration(330.0), "5m 30s");
        assert_eq!(fmt_duration(8040.0), "2h 14m");
    }
}
