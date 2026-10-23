//! Left sidebar (1:1 with the Nexus layout):
//! collapse toggle, "+ New session", the *Sessions* list (name / date · cwd
//! with the accent left bar on the active item), *Projects* (recent
//! folders), *CLI history* (sessions recorded by the `vibe` CLI), and the
//! bottom navigation (Statistics / Settings / Setup) + disclaimer.
//!
//! Collapsed, the sidebar becomes a 46px icon rail (Nexus-like).

use eframe::egui::{self, Ui};
use vibedeck_core::proto::AcpCommand;

use crate::app::VibedeckApp;
use crate::state::View;
use crate::theme::Palette;

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
                    .size(10.0)
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

                    // top row: title + collapse button
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(t.app_name)
                                .size(12.0)
                                .strong()
                                .color(p.muted),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(egui::RichText::new("«").small())
                                            .fill(egui::Color32::TRANSPARENT),
                                    )
                                    .on_hover_text(t.collapse)
                                    .clicked()
                                {
                                    app.settings.sidebar_collapsed = true;
                                    app.settings.save();
                                }
                            },
                        );
                    });
                    ui.add_space(2.0);

                    // + New session (accent-styled, like Nexus)
                    if ui
                        .add_sized(
                            [ui.available_width(), 30.0],
                            egui::Button::new(
                                egui::RichText::new(format!("+ {}", t.new_session))
                                    .size(12.0)
                                    .strong()
                                    .color(p.text),
                            )
                            .fill(p.panel)
                            .rounding(egui::Rounding::same(6.0))
                            .stroke(egui::Stroke::new(1.0_f32, p.border)),
                        )
                        .clicked()
                    {
                        app.new_local_session();
                    }

                    ui.add_space(8.0);
                    section_label(ui, &p, t.sessions);

                    // local sessions (Nexus session-item: name + date · cwd,
                    // 3px accent left bar on the active one, min-height 44)
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
                        let clicked = session_item(
                            ui,
                            &p,
                            name,
                            &format!("{date} · {cwd}"),
                            selected,
                            busy_here,
                        );
                        if clicked {
                            app.current = Some(i);
                            app.view = View::Chat;
                        }
                    }
                    if app.sessions.is_empty() {
                        ui.label(egui::RichText::new(t.no_sessions).size(11.0).color(p.muted));
                    }

                    ui.add_space(8.0);
                    section_label(ui, &p, t.projects);

                    // projects: recent folders + add
                    let folders = app.settings.recent_folders.clone();
                    for folder in &folders {
                        let is_current = *folder == app.settings.working_directory;
                        let name = short_path(folder);
                        let clicked = session_item(
                            ui,
                            &p,
                            &format!("📂 {name}"),
                            folder,
                            is_current,
                            false,
                        );
                        if clicked {
                            app.settings.working_directory = folder.clone();
                            app.settings.save();
                            app.mark_file_index_stale();
                            app.reset_dock();
                            app.new_local_session();
                            app.view = View::Chat;
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!("+ {}", t.add_project))
                                    .size(12.0)
                                    .color(p.accent),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .rounding(egui::Rounding::same(5.0))
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
                            let loading_this =
                                app.loading_session.as_deref() == Some(info.session_id.as_str());
                            if session_item(ui, &p, &label, &sub, false, loading_this) {
                                // Already mirroring this backend session locally
                                // → just switch to it, no reload needed.
                                if let Some(i) = app
                                    .sessions
                                    .iter()
                                    .position(|s| s.acp_session_id.as_deref()
                                        == Some(info.session_id.as_str()))
                                {
                                    app.current = Some(i);
                                    app.view = View::Chat;
                                } else if app
                                    .loading_session
                                    .as_deref()
                                    == Some(info.session_id.as_str())
                                {
                                    // `session/load` already in flight — ignore.
                                } else {
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
                                    app.reset_dock();
                                    app.view = View::Chat;
                                    app.busy = true;
                                    // The replayed history (`session/update`
                                    // replays) arrives before `SessionReady` —
                                    // mark the load so `SessionHistory` can be
                                    // attached to this mirror.
                                    app.loading_session = Some(info.session_id.clone());
                                    app.acp.send(AcpCommand::LoadSession {
                                        session_id: info.session_id.clone(),
                                        cwd,
                                        model: app.settings.preferred_model.clone(),
                                    });
                                }
                            }
                        }
                    }
                });
        });
}

/// One Nexus `.session-item`: two lines (name 12px / meta 10px dim),
/// min-height 44, hover = panel-hover, active = #542e21 bg + #ffcb95 text
/// + 3px accent left bar. Returns true when clicked.
fn session_item(
    ui: &mut Ui,
    p: &Palette,
    name: &str,
    meta: &str,
    selected: bool,
    busy: bool,
) -> bool {
    let width = ui.available_width();
    let min_h = 44.0;
    let (rect, response) = ui
        .allocate_exact_size(egui::vec2(width, min_h), egui::Sense::click());

    let bg = if selected {
        p.session_active_bg
    } else if response.hovered() {
        p.panel_hover
    } else {
        egui::Color32::TRANSPARENT
    };
    if bg != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, egui::Rounding::same(4.0), bg);
    }
    if selected {
        // 3px accent bar on the left (Nexus `.session-item.active`)
        ui.painter().rect_filled(
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(3.0, rect.height())),
            egui::Rounding::same(1.0),
            p.accent,
        );
    }

    let name_color = if selected {
        p.session_active_text
    } else {
        p.text
    };
    let text_x = rect.left() + 12.0;
    ui.painter().text(
        egui::pos2(text_x, rect.top() + 9.0),
        egui::Align2::LEFT_TOP,
        name,
        egui::FontId::proportional(12.0),
        name_color,
    );
    let meta_color = if selected {
        egui::Color32::from_rgba_premultiplied(0xff, 0xcb, 0x95, 140)
    } else {
        p.dim
    };
    ui.painter().text(
        egui::pos2(text_x, rect.top() + 25.0),
        egui::Align2::LEFT_TOP,
        if busy { format!("{meta} ⏳") } else { meta.to_string() },
        egui::FontId::proportional(10.0),
        meta_color,
    );
    response.clicked()
}

/// The collapsed 46px icon rail.
pub fn show_rail(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();
    let t = app.tr();

    // expand
    if ui
        .add_sized(
            [ui.available_width(), 28.0],
            egui::Button::new(egui::RichText::new("»").small())
                .fill(egui::Color32::TRANSPARENT),
        )
        .on_hover_text(t.expand)
        .clicked()
    {
        app.settings.sidebar_collapsed = false;
        app.settings.save();
    }

    // + new session
    if ui
        .add_sized(
            [ui.available_width(), 28.0],
            egui::Button::new(egui::RichText::new("+").small().color(p.text))
                .fill(p.panel)
                .rounding(egui::Rounding::same(6.0)),
        )
        .on_hover_text(t.new_session)
        .clicked()
    {
        app.new_local_session();
        app.view = View::Chat;
    }

    // session dots
    ui.add_space(6.0);
    let current = app.current;
    let count = app.sessions.len().min(8);
    for i in 0..count {
        let selected = current == Some(i);
        if ui
            .add_sized(
                [ui.available_width(), 10.0],
                egui::Button::new(
                    egui::RichText::new("●")
                        .size(8.0)
                        .color(if selected { p.accent } else { p.dim }),
                )
                .fill(egui::Color32::TRANSPARENT),
            )
            .on_hover_text(&app.sessions[i].name)
            .clicked()
        {
            app.current = Some(i);
            app.view = View::Chat;
        }
    }

    // bottom nav
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new("i").size(9.0).color(p.dim))
            .on_hover_text(t.disclaimer_short);
        rail_nav(app, ui, "rail-stats", "📊", t.view_stats, View::Stats, &p);
        rail_nav(app, ui, "rail-settings", "⚙", t.view_settings, View::Settings, &p);
        rail_nav(app, ui, "rail-setup", "🛠", t.view_setup, View::Setup, &p);
    });
}

fn rail_nav(
    app: &mut VibedeckApp,
    ui: &mut Ui,
    id: &str,
    icon: &str,
    label: &str,
    view: View,
    p: &Palette,
) {
    let selected = app.view == view;
    if ui
        .add_sized(
            [ui.available_width(), 30.0],
            egui::Button::new(
                egui::RichText::new(icon)
                    .size(15.0)
                    .color(if selected { p.accent } else { p.muted }),
            )
            .fill(if selected {
                p.panel_hover
            } else {
                egui::Color32::TRANSPARENT
            })
            .rounding(egui::Rounding::same(5.0)),
        )
        .on_hover_text(label)
        .clicked()
    {
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

fn section_label(ui: &mut Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(11.0).color(p.muted));
    ui.add_space(2.0);
}

fn nav_item(
    app: &mut VibedeckApp,
    ui: &mut Ui,
    id: &str,
    icon: &str,
    label: &str,
    view: View,
    p: &Palette,
) {
    let selected = app.view == view;
    let button = egui::Button::new(
        egui::RichText::new(format!("{icon} {label}"))
            .size(12.0)
            .color(if selected { p.session_active_text } else { p.muted }),
    )
    .fill(if selected {
        p.panel_hover
    } else {
        egui::Color32::TRANSPARENT
    })
    .rounding(egui::Rounding::same(4.0))
    .stroke(egui::Stroke::NONE);
    let button = button.min_size(egui::vec2(ui.available_width(), 0.0));
    let resp = ui.add(button).on_hover_text(label);
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
    let (_, mo, d) = civil_from_days(days as i64);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS
        .get(mo.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("Jan");
    format!("{d} {month} {h:02}:{m:02}")
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
