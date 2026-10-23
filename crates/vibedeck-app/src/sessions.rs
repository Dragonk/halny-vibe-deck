//! Left sidebar (1:1 with the Nexus layout):
//! "+ New session", the *Sessions* list (name / date · cwd),
//! *Projects* (recent folders, Nexus `recentFolders`),
//! *CLI history* (sessions recorded by the `vibe` CLI via ACP `session/list`),
//! and the bottom navigation (Statistics / Settings / Setup) + disclaimer.

use eframe::egui::{self, Ui};
use vibedeck_core::proto::AcpCommand;

use crate::app::VibedeckApp;
use crate::state::View;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();
    let t = app.tr();

    // -- bottom navigation (fixed) ---------------------------------------------
    egui::TopBottomPanel::bottom("sidebar-footer")
        .frame(egui::Frame::default())
        .show_inside(ui, |ui| {
            ui.separator();
            nav_item(app, ui, "sidebar-nav-stats", "📊", t.view_stats, View::Stats, &p);
            nav_item(app, ui, "sidebar-nav-settings", "⚙", t.view_settings, View::Settings, &p);
            nav_item(app, ui, "sidebar-nav-setup", "🛠", t.view_setup, View::Setup, &p);
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(t.disclaimer_short)
                    .small()
                    .weak()
                    .color(p.dim),
            );
        });

    // -- scrollable content ------------------------------------------------------
    egui::CentralPanel::default()
        .frame(egui::Frame::default())
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .id_salt("sidebar-scroll")
                .show(ui, |ui| {
                    ui.add_space(2.0);

                    // + New session
                    if ui
                        .add_sized(
                            [ui.available_width(), 26.0],
                            egui::Button::new(
                                egui::RichText::new(format!("＋ {}", t.new_session))
                                    .strong()
                                    .color(p.text),
                            )
                            .fill(p.panel)
                            .stroke(egui::Stroke::new(1.0_f32, p.border)),
                        )
                        .clicked()
                    {
                        app.new_local_session();
                    }

                    ui.add_space(8.0);
                    section_label(ui, &p, t.sessions);

                    // local sessions (Nexus: name / date · cwd)
                    let current = app.current;
                    let limit = app.settings.sidebar_sessions_limit.unwrap_or(30);
                    let items: Vec<(String, String, String)> = app
                        .sessions
                        .iter()
                        .take(limit)
                        .map(|s| {
                            (
                                s.name.clone(),
                                fmt_datetime(s.last_active_ms),
                                short_path(&s.working_directory),
                            )
                        })
                        .collect();
                    for (i, (name, date, cwd)) in items.iter().enumerate() {
                        let selected = current == Some(i);
                        let busy_here = selected && app.busy;
                        let body = if busy_here {
                            format!("{name}\n{date} · {cwd} ⏳")
                        } else {
                            format!("{name}\n{date} · {cwd}")
                        };
                        let button = egui::Button::new(
                            egui::RichText::new(body)
                                .small()
                                .color(if selected {
                                    p.session_active_text
                                } else {
                                    p.text
                                }),
                        )
                        .fill(if selected {
                            p.session_active_bg
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::new(
                            1.0_f32,
                            if selected {
                                p.border
                            } else {
                                egui::Color32::TRANSPARENT
                            },
                        ));
                        let button = button.min_size(egui::vec2(ui.available_width(), 0.0));
                        if ui
                            .add(button)
                            .on_hover_text(format!("{name} — {cwd}"))
                            .clicked()
                        {
                            app.current = Some(i);
                        }
                    }
                    if app.sessions.is_empty() {
                        ui.label(egui::RichText::new(t.no_sessions).small().color(p.muted));
                    }

                    ui.add_space(8.0);
                    section_label(ui, &p, t.projects);

                    // projects: recent folders + add
                    let folders = app.settings.recent_folders.clone();
                    for folder in &folders {
                        let is_current = *folder == app.settings.working_directory;
                        let name = short_path(folder);
                        let button = egui::Button::new(
                            egui::RichText::new(format!("📂 {name}\n{folder}"))
                                .small()
                                .color(if is_current {
                                    p.session_active_text
                                } else {
                                    p.text
                                }),
                        )
                        .fill(if is_current {
                            p.session_active_bg
                        } else {
                            egui::Color32::TRANSPARENT
                        })
                        .stroke(egui::Stroke::NONE);
                        let button = button.min_size(egui::vec2(ui.available_width(), 0.0));
                        if ui
                            .add(button)
                            .on_hover_text(folder)
                            .clicked()
                        {
                            app.settings.working_directory = folder.clone();
                            app.settings.save();
                            app.new_local_session();
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!("＋ {}", t.add_project))
                                    .small()
                                    .color(p.accent),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
                        )
                        .clicked()
                    {
                        app.folder_picker.open = true;
                        app.folder_picker.dir = app.current_workdir();
                        app.folder_picker.entry = app.settings.working_directory.clone();
                    }

                    ui.add_space(8.0);

                    // CLI history: sessions recorded by the vibe CLI
                    if !app.backend_sessions.is_empty() {
                        section_label(
                            ui,
                            &p,
                            &format!("{} ({})", t.cli_history, app.backend_sessions.len()),
                        );
                        let backend_sessions = app.backend_sessions.clone();
                        for info in &backend_sessions {
                            let label = if info.name.is_empty() {
                                short_path(&info.cwd)
                            } else {
                                info.name.clone()
                            };
                            let sub = if info.start_time.is_empty() {
                                short_path(&info.cwd)
                            } else {
                                format!("{} · {}", short_time(&info.start_time), short_path(&info.cwd))
                            };
                            let button = egui::Button::new(
                                egui::RichText::new(format!("{label}\n{sub}"))
                                    .small()
                                    .color(p.text),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .stroke(egui::Stroke::NONE)
                            .min_size(egui::vec2(ui.available_width(), 0.0));
                            if ui
                                .add(button)
                                .on_hover_text(&info.cwd)
                                .clicked()
                            {
                                let cwd = if info.cwd.is_empty() {
                                    app.current_workdir().to_string_lossy().to_string()
                                } else {
                                    info.cwd.clone()
                                };
                                // create a local mirror of the backend session
                                let mut session = vibedeck_core::types::ChatSession::new(
                                    if info.name.is_empty() { "CLI session" } else { &info.name },
                                    &cwd,
                                );
                                session.working_directory = cwd.clone();
                                app.sessions.insert(0, session);
                                app.current = Some(0);
                                app.settings.working_directory = cwd.clone();
                                app.settings.remember_folder(&cwd);
                                app.settings.save();
                                app.mark_file_index_stale();
                                app.busy = true;
                                app.acp.send(AcpCommand::LoadSession {
                                    session_id: info.session_id.clone(),
                                    cwd,
                                    model: app.settings.preferred_model.clone(),
                                });
                            }
                        }
                    }
                });
        });
}

fn section_label(ui: &mut Ui, p: &crate::theme::Palette, text: &str) {
    ui.label(egui::RichText::new(text).small().color(p.muted));
    ui.add_space(2.0);
}

fn nav_item(
    app: &mut VibedeckApp,
    ui: &mut Ui,
    id: &str,
    icon: &str,
    label: &str,
    view: View,
    p: &crate::theme::Palette,
) {
    let selected = app.view == view;
    let button = egui::Button::new(
        egui::RichText::new(format!("{icon} {label}"))
            .small()
            .color(if selected { p.session_active_text } else { p.muted }),
    )
    .fill(if selected {
        p.panel_hover
    } else {
        egui::Color32::TRANSPARENT
    })
    .stroke(egui::Stroke::NONE);
    let button = button.min_size(egui::vec2(ui.available_width(), 0.0));
    let resp = ui
        .add(button)
        .on_hover_text(label);
    if resp.clicked() {
        app.view = view;
        match view {
            View::Settings => app.settings_pane.ensure_loaded(),
            View::Stats => app.stats_pane.ensure_loaded(),
            View::Setup => app.setup_pane.refresh_status_async(),
            View::Chat => {}
        }
    }
    let _ = id;
}

/// "23 Oct 16:45" from unix-ms.
fn fmt_datetime(ms: i64) -> String {
    let secs = (ms.max(0) as u64) / 1000;
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m) = (rem / 3600, (rem % 3600) / 60);
    let (y, mo, d) = civil_from_days(days as i64);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS
        .get(mo.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("Jan");
    format!("{d} {month} {y} {h:02}:{m:02}")
}

/// Howard Hinnant's civil-from-days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn short_path(path: &str) -> String {
    if path.is_empty() {
        return "—".into();
    }
    path.trim_end_matches('/')
        .trim_end_matches('\\')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_string()
}

fn short_time(s: &str) -> String {
    s.chars().take(16).collect::<String>().replacen('T', " ", 1)
}
