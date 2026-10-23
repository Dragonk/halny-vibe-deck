//! Stats view: Nexus-style usage dashboard built from the Mistral Vibe CLI
//! session logs (`~/.vibe/logs/session/`) — time-range toolbar (7d / 30d /
//! 90d / all / custom), cost / token / cache / activity / tool-usage cards
//! with painter charts, a recent-sessions list and a drill-down with the
//! message log and subagent sessions.

use eframe::egui::{self, Ui};
use vibedeck_vibe::stats::{
    aggregate, load_session_messages, load_subagent_sessions, VibeMessage, VibeSession,
    VibeSubagentSession,
};

use crate::app::VibedeckApp;
use crate::charts;
use crate::theme::Palette;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeRange {
    #[default]
    All,
    D7,
    D30,
    D90,
    Custom,
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
    s.start_time
        .get(0..10)
        .and_then(parse_ymd)
        .unwrap_or(0)
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
    Some(
        (days_from_civil(y, mo, d) * 86_400 + h * 3_600 + mi * 60 + sec) as f64,
    )
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

    // ---- toolbar: title + range pills + custom dates + model filter + refresh
    let p = app.palette();
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

    // model filter
    let mut models: Vec<String> = pane
        .sessions
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
    let mut model_pick: Option<String> = None;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(t.stats_model).small().weak());
        egui::ComboBox::from_id_salt("stats-model-filter")
            .selected_text(if pane.model_filter.is_empty() {
                t.stats_all_models.to_string()
            } else {
                pane.model_filter.clone()
            })
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut model_pick, Some(String::new()), t.stats_all_models);
                for m in &models {
                    ui.selectable_value(&mut model_pick, Some(m.clone()), m);
                }
            });
        if let Some(m) = model_pick {
            pane.model_filter = m;
        }
        if ui.button(format!("↻ {}", t.reload)).clicked() {
            pane.reload();
        }
    });
    ui.add_space(6.0);

    if pane.sessions.is_empty() {
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

    // day buckets for the charts
    let mut day_costs: Vec<f64> = vec![0.0; (to_days - from_days + 1).max(1) as usize];
    let mut day_tokens: Vec<f64> = vec![0.0; day_costs.len()];
    let mut cumulative: Vec<f64> = Vec::with_capacity(day_costs.len());
    for s in &selected {
        let idx = (session_day(s) - from_days) as usize;
        if idx < day_costs.len() {
            day_costs[idx] += s.stats.session_cost;
            day_tokens[idx] += s.stats.session_total_llm_tokens;
        }
    }
    let mut acc = 0.0;
    for v in &day_costs {
        acc += *v;
        cumulative.push(acc);
    }
    // bar chart: at most the 28 most recent days
    let bar_days = day_tokens.len().min(28);
    let bar_tokens: Vec<f64> = day_tokens[day_tokens.len() - bar_days..].to_vec();

    // heatmap: sessions per day, 12 weeks, Monday-first, oldest week first
    let weeks = 12usize;
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
    egui::Grid::new("stats-cards")
        .num_columns(2)
        .spacing([12.0, 12.0])
        .show(ui, |ui| {
            ui.vertical(|ui| {
                card(ui, &p, t.stats_total_spend, |ui| {
                    big(ui, &fmt_cost(totals.cost), p.accent);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {} · {} {}",
                            fmt_cost(totals.cost / n),
                            t.stats_avg_cost,
                            selected.len(),
                            t.stats_sessions
                        ))
                        .small()
                        .weak(),
                    );
                    ui.add_space(4.0);
                    charts::area_chart(ui, "stats-cost-area", &cumulative, p.accent, &|v| {
                        fmt_cost(v)
                    });
                });
            });
            ui.vertical(|ui| {
                card(ui, &p, t.stats_total_tokens, |ui| {
                    big(ui, &fmt_count(totals.total_llm_tokens), p.text);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {} · {} {}",
                            fmt_count(totals.prompt_tokens),
                            t.stats_prompt_tokens,
                            fmt_count(totals.completion_tokens),
                            t.stats_completion_tokens
                        ))
                        .small()
                        .weak(),
                    );
                    ui.add_space(4.0);
                    charts::bar_chart(ui, "stats-tokens-bar", &bar_tokens, p.session_active_text, &|v| {
                        fmt_count(v)
                    });
                });
            });
            ui.end_row();

            ui.vertical(|ui| {
                card(ui, &p, t.cache_hit_rate, |ui| {
                    big(
                        ui,
                        &if prompt > 0.0 {
                            format!("{:.1}%", (cached / prompt * 100.0).clamp(0.0, 100.0))
                        } else {
                            "—".into()
                        },
                        p.session_active_text,
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {}",
                            fmt_count(cached),
                            t.cache_tokens
                        ))
                        .small()
                        .weak(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{:.1} {}",
                            avg_tps, t.tok_per_sec
                        ))
                        .small()
                        .monospace(),
                    );
                });
            });
            ui.vertical(|ui| {
                card(ui, &p, t.stats_activity, |ui| {
                    big(ui, &selected.len().to_string(), p.text);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {} · {} {}",
                            fmt_duration(avg_dur),
                            t.stats_avg_duration,
                            fmt_duration(total_dur),
                            t.stats_total_duration
                        ))
                        .small()
                        .weak(),
                    );
                    ui.add_space(4.0);
                    charts::heatmap(ui, "stats-heat", &heat, weeks, p.accent);
                });
            });
            ui.end_row();

            ui.vertical(|ui| {
                card(ui, &p, t.stats_tool_usage, |ui| {
                    donut_tools(ui, t, &p, totals);
                });
            });
            ui.vertical(|ui| {
                card(ui, &p, t.stats_recent_sessions, |ui| {
                    session_list(ui, t, &p, &pane.sessions, &filtered, pane.selected);
                });
            });
            ui.end_row();
        });

    // click on a recent session?
    let mut clicked: Option<usize> = None;
    if let Some(resp) = ui
        .ctx()
        .memory(|m| m.data.get_temp::<Option<usize>>(egui::Id::new("stats-session-click")))
    {
        clicked = resp;
        ui.ctx().memory_mut(|m| m.data.remove_temp::<Option<usize>>(egui::Id::new("stats-session-click")));
    }
    if let Some(i) = clicked {
        app.stats_pane.select(i);
    }

    drill_down(app, ui, t);
}

// -- helpers ----------------------------------------------------------------------

fn card(ui: &mut Ui, p: &Palette, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::Frame::default()
        .fill(p.panel)
        .rounding(6.0)
        .inner_margin(egui::Margin::same(12.0))
        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(title)
                    .small()
                    .strong()
                    .color(p.session_active_text),
            );
            ui.add_space(4.0);
            ui.set_min_width(260.0);
            body(ui);
        });
}

fn big(ui: &mut Ui, text: &str, color: egui::Color32) {
    ui.label(egui::RichText::new(text).size(22.0).strong().color(color));
}

fn donut_tools(ui: &mut Ui, t: &crate::i18n::Str, p: &Palette, totals: vibedeck_vibe::stats::AggregateStats) {
    if totals.tool_calls_succeeded + totals.tool_calls_failed + totals.tool_calls_rejected <= 0.0 {
        ui.label(egui::RichText::new("—").weak());
        return;
    }
    charts::donut(
        ui,
        "stats-tools-donut",
        &[
            (totals.tool_calls_succeeded, p.green, t.stats_succeeded.to_string()),
            (totals.tool_calls_failed, p.red, t.stats_failed.to_string()),
            (totals.tool_calls_rejected, p.yellow, t.stats_rejected.to_string()),
        ],
    );
}

fn session_list(
    ui: &mut Ui,
    t: &crate::i18n::Str,
    p: &Palette,
    sessions: &[VibeSession],
    filtered: &[usize],
    selected: Option<usize>,
) {
    let mut click: Option<usize> = None;
    let list: Vec<(usize, &VibeSession)> =
        filtered.iter().map(|&i| (i, &sessions[i])).collect();
    let rows: Vec<(usize, String, String, String, String, String)> = list
        .into_iter()
        .map(|(i, s)| {
            let title = s
                .title
                .clone()
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| t.stats_untitled.to_string());
            let date = short_time(&s.start_time);
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
            (i, title, format!("{date} · {dur}"), project, model, fmt_cost(s.stats.session_cost))
        })
        .take(6)
        .collect();
    egui::ScrollArea::vertical()
        .max_height(190.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, title, meta, project, model, cost) in rows {
                let is_sel = selected == Some(i);
                let resp = egui::Frame::default()
                    .fill(if is_sel { p.panel_hover } else { p.surface })
                    .rounding(5.0)
                    .inner_margin(egui::Margin::same(8.0))
                    .stroke(egui::Stroke::new(
                        if is_sel { 1.5_f32 } else { 1.0_f32 },
                        if is_sel { p.accent } else { p.border_subtle },
                    ))
                    .show(ui, |ui| {
                        ui.set_min_width(200.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(title.clone()).small().strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(cost.clone())
                                            .small()
                                            .monospace()
                                            .weak(),
                                    );
                                },
                            );
                        });
                        ui.label(egui::RichText::new(meta.clone()).small().weak());
                        ui.horizontal_wrapped(|ui| {
                            for tag in [&project, &model] {
                                if tag.is_empty() {
                                    continue;
                                }
                                egui::Frame::default()
                                    .fill(p.bg)
                                    .rounding(4.0)
                                    .inner_margin(egui::Margin::symmetric(6.0, 1.0))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(tag.clone())
                                                .size(10.0)
                                                .monospace()
                                                .color(p.session_active_text),
                                        );
                                    });
                            }
                        });
                    });
                if resp.response.interact(egui::Sense::click()).clicked() {
                    click = Some(i);
                }
            }
        });
    if let Some(i) = click {
        ui.ctx().memory_mut(|m| {
            m.data.insert_temp(egui::Id::new("stats-session-click"), Some(i));
        });
    }
    ui.label(
        egui::RichText::new(format!("{}: {}", t.stats_sessions, filtered.len()))
            .small()
            .weak(),
    );
}

fn drill_down(app: &mut VibedeckApp, ui: &mut Ui, t: &crate::i18n::Str) {
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
    let date = short_time(&s.start_time);
    let dur = fmt_duration(session_duration_s(s));
    ui.add_space(10.0);
    ui.separator();
    ui.label(egui::RichText::new(title).strong());
    ui.label(
        egui::RichText::new(format!(
            "{} · {} — {} · {} {}",
            date,
            dur,
            short_time(&s.end_time),
            t.stats_branch,
            s.git_branch.clone().unwrap_or_else(|| "—".into()),
        ))
        .small()
        .weak(),
    );
    if let Some(env) = &s.environment {
        if let Some(wd) = &env.working_directory {
            ui.label(egui::RichText::new(wd.clone()).small().monospace().weak());
        }
    }
    ui.add_space(8.0);

    ui.label(egui::RichText::new(t.stats_messages).strong());
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .auto_shrink(false)
        .show(ui, |ui| {
            for msg in &app.stats_pane.messages {
                message_row(ui, msg);
            }
        });

    for sub in &app.stats_pane.subagents {
        egui::CollapsingHeader::new(format!(
            "🤖 {} ({})",
            sub.agent_name,
            short_time(&sub.start_time)
        ))
        .default_open(false)
        .id_salt(format!("sub-{}", sub.session_id))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .auto_shrink(false)
                .id_salt(format!("sub-scroll-{}", sub.session_id))
                .show(ui, |ui| {
                    for msg in &sub.messages {
                        message_row(ui, msg);
                    }
                });
        });
    }
}

fn live_usage(app: &mut VibedeckApp, ui: &mut Ui, t: &crate::i18n::Str) {
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

fn message_row(ui: &mut Ui, msg: &VibeMessage) {
    let content = msg.content.clone().unwrap_or_default();
    let preview: String = content.chars().take(400).collect();
    let preview = if content.chars().count() > 400 {
        format!("{preview}…")
    } else {
        preview
    };
    let (marker, color) = match msg.role.as_str() {
        "user" => ("🧑", egui::Color32::from_rgb(0x4d, 0xa3, 0xff)),
        "assistant" => ("🤖", egui::Color32::from_rgb(0x50, 0xc8, 0x78)),
        "tool" => ("🔧", egui::Color32::from_rgb(0xff, 0xc0, 0x45)),
        other => (other, ui.visuals().weak_text_color()),
    };
    ui.horizontal_wrapped(|ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(marker).color(color));
        ui.label(egui::RichText::new(preview).small());
    });
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
