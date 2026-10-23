//! Stats view: usage dashboard built from the Mistral Vibe CLI session
//! logs (`~/.vibe/logs/session/`) — totals, per-session cost/token table
//! and a drill-down with the message log and subagent sessions.

use eframe::egui::{self, Ui};
use vibedeck_vibe::stats::{
    aggregate, load_session_messages, load_subagent_sessions, VibeMessage, VibeSession,
    VibeSubagentSession,
};

use crate::app::VibedeckApp;

#[derive(Debug, Default)]
pub struct StatsPane {
    pub sessions: Vec<VibeSession>,
    pub loaded: bool,
    pub selected: Option<usize>,
    pub messages: Vec<VibeMessage>,
    pub subagents: Vec<VibeSubagentSession>,
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

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let t = app.tr();
    app.stats_pane.ensure_loaded();

    // -- live usage of the current session (usage_update events) ------------
    let usage = &app.usage;
    if usage.updates > 0 {
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

    ui.heading(egui::RichText::new(t.view_stats).strong());
    ui.add_space(6.0);

    // Totals
    let totals = aggregate(&app.stats_pane.sessions);
    let cached_total: f64 = app.stats_pane.sessions.iter().map(|s| s.stats.cached_tokens).sum();
    let prompt_total: f64 = totals.prompt_tokens;
    egui::Grid::new("stats-totals")
        .num_columns(2)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            stat_row(ui, t.stats_sessions, &totals.sessions.to_string());
            stat_row(ui, t.stats_prompt_tokens, &fmt_count(totals.prompt_tokens));
            stat_row(
                ui,
                t.stats_completion_tokens,
                &fmt_count(totals.completion_tokens),
            );
            stat_row(ui, t.stats_llm_tokens, &fmt_count(totals.total_llm_tokens));
            stat_row(ui, t.cache_tokens, &fmt_count(cached_total));
            stat_row(
                ui,
                t.cache_hit_rate,
                &if prompt_total > 0.0 {
                    format!("{:.1}%", (cached_total / prompt_total * 100.0).clamp(0.0, 100.0))
                } else {
                    "—".into()
                },
            );
            stat_row(ui, t.stats_cost, &fmt_cost(totals.cost));
            stat_row(
                ui,
                t.stats_tool_calls,
                &format!(
                    "{} / {} / {}",
                    fmt_count(totals.tool_calls_succeeded),
                    fmt_count(totals.tool_calls_failed),
                    fmt_count(totals.tool_calls_rejected)
                ),
            );
        });
    ui.add_space(8.0);

    if app.stats_pane.sessions.is_empty() {
        ui.label(egui::RichText::new(t.stats_no_data).weak());
        return;
    }
    if ui.button(format!("🔄 {}", t.reload)).clicked() {
        app.stats_pane.reload();
    }
    ui.add_space(8.0);

    // Per-session table
    let mut select = None;
    let max_cost = app
        .stats_pane
        .sessions
        .iter()
        .map(|s| s.stats.session_cost)
        .fold(0.0_f64, f64::max);
    egui::ScrollArea::vertical()
        .max_height(260.0)
        .auto_shrink(false)
        .show(ui, |ui| {
            egui::Grid::new("stats-sessions")
                .num_columns(7)
                .striped(true)
                .spacing([14.0, 4.0])
                .show(ui, |ui| {
                    header_row(
                        ui,
                        &[
                            "",
                            t.stats_title,
                            t.stats_model,
                            t.stats_branch,
                            t.stats_cost,
                            t.stats_llm_tokens,
                            t.stats_messages,
                        ],
                    );
                    for (i, s) in app.stats_pane.sessions.iter().enumerate() {
                        let selected = app.stats_pane.selected == Some(i);
                        if ui
                            .selectable_label(selected, if selected { "▶" } else { "·" })
                            .clicked()
                        {
                            select = Some(i);
                        }
                        let title = s.title.clone().unwrap_or_else(|| s.session_id.clone());
                        let title = if title.len() > 34 {
                            format!("{}…", title.chars().take(33).collect::<String>())
                        } else {
                            title
                        };
                        if ui.selectable_label(selected, title).clicked() {
                            select = Some(i);
                        }
                        ui.label(
                            egui::RichText::new(
                                s.config
                                    .as_ref()
                                    .and_then(|c| c.active_model.clone())
                                    .unwrap_or_else(|| "—".into()),
                            )
                            .small(),
                        );
                        ui.label(
                            egui::RichText::new(s.git_branch.clone().unwrap_or_default())
                                .small()
                                .weak(),
                        );
                        bar(
                            ui,
                            s.stats.session_cost,
                            max_cost,
                            fmt_cost(s.stats.session_cost),
                        );
                        ui.label(
                            egui::RichText::new(fmt_count(s.stats.session_total_llm_tokens))
                                .small()
                                .monospace(),
                        );
                        ui.label(
                            egui::RichText::new(
                                s.total_messages
                                    .map(|m| m.to_string())
                                    .unwrap_or_else(|| "—".into()),
                            )
                            .small(),
                        );
                        ui.end_row();
                    }
                });
        });
    if let Some(i) = select {
        app.stats_pane.select(i);
    }

    // Drill-down
    if let Some(idx) = app.stats_pane.selected {
        if let Some(s) = app.stats_pane.sessions.get(idx) {
            ui.add_space(10.0);
            ui.separator();
            ui.label(
                egui::RichText::new(s.title.clone().unwrap_or_else(|| s.session_id.clone()))
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} — {} · {} {}",
                    short_time(&s.start_time),
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
    }
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
        ui.label(egui::RichText::new(marker).color(color));
        ui.label(egui::RichText::new(preview).small());
    });
}

fn stat_row(ui: &mut Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).weak());
    ui.label(egui::RichText::new(value).strong().monospace());
    ui.end_row();
}

/// Compact label:value pair (no end_row) for the live-usage grid.
fn stat_cell(ui: &mut Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).size(11.0).weak());
    ui.label(egui::RichText::new(value).size(11.0).strong().monospace());
}

fn header_row(ui: &mut Ui, labels: &[&str]) {
    for label in labels {
        ui.strong(egui::RichText::new(*label).small());
    }
    ui.end_row();
}

/// Tiny inline bar made of `█` characters (no painter API, works on every
/// egui version).
fn bar(ui: &mut Ui, value: f64, max: f64, text: String) {
    ui.horizontal(|ui| {
        let width = if max > 0.0 {
            ((value / max).clamp(0.0, 1.0) * 12.0).round() as usize
        } else {
            0
        };
        ui.label(
            egui::RichText::new(format!("{}{}", "█".repeat(width), "░".repeat(12 - width)))
                .small()
                .color(egui::Color32::from_rgb(0x50, 0xc8, 0x78)),
        );
        ui.label(egui::RichText::new(text).small().monospace());
    });
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
