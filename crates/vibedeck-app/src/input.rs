//! Bottom input bar, mirroring the Nexus InputBar:
//! a toolbar row on top (model / mode / thinking chips + cwd),
//! then the rounded input box with a 42×42 send/stop button,
//! then the status row with the working directory.

use eframe::egui::{self, Ui};

use crate::app::VibedeckApp;
use crate::theme::Palette;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();
    let t = app.tr();

    // -- toolbar row (model / mode / thinking / cwd) ---------------------------
    ui.horizontal(|ui| {
        option_chip(app, ui, &p, "model", t.model);
        option_chip(app, ui, &p, "mode", t.mode);
        option_chip(app, ui, &p, "thinking", t.thinking);

        // working directory
        let cwd = app.current_workdir().to_string_lossy().to_string();
        let cwd_name = short_path(&cwd);
        if chip_button(ui, &p, &format!("📂 {cwd_name}"))
            .on_hover_text(&cwd)
            .clicked()
        {
            app.folder_picker.open = true;
            app.folder_picker.dir = app.current_workdir();
            app.folder_picker.entry = app.settings.working_directory.clone();
        }
    });

    ui.add_space(6.0);

    // -- input box --------------------------------------------------------------
    let send_clicked = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    let mut text_response: Option<egui::Response> = None;

    egui::Frame::default()
        .fill(p.input_bg)
        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle))
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(6.0, 6.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut app.input)
                        .hint_text(egui::RichText::new(t.message_placeholder).color(p.muted))
                        .desired_rows(1)
                        .desired_width(ui.available_width() - 52.0)
                        .text_color(p.text)
                        .lock_focus(true),
                );
                text_response = Some(response.clone());

                // send/stop: 42×42, radius 10 — accent while enabled.
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(42.0, 42.0), egui::Sense::click());
                let painter = ui.painter_at(rect);
                let fill = if app.busy {
                    p.panel_hover
                } else if app.input.trim().is_empty() {
                    p.panel
                } else {
                    p.accent
                };
                painter.rect_filled(rect, egui::Rounding::same(10.0), fill);
                let glyph = if app.busy { "⏹" } else { "➤" };
                let glyph_color = if app.busy || app.input.trim().is_empty() {
                    p.muted
                } else {
                    egui::Color32::WHITE
                };
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(16.0),
                    glyph_color,
                );
                let _ = painter;
                if response.clicked() {
                    if app.busy {
                        if let Some(session) = app.current_session() {
                            if let Some(acp_session_id) = session.acp_session_id.clone() {
                                app.acp.send(vibedeck_core::proto::AcpCommand::Cancel {
                                    session_id: acp_session_id,
                                });
                            }
                        }
                    } else {
                        let text = std::mem::take(&mut app.input);
                        app.send_message(text);
                    }
                }
            });
        });

    // Enter sends while the text edit has focus.
    if send_clicked {
        if let Some(response) = &text_response {
            if response.has_focus() && !app.busy && !app.input.trim().is_empty() {
                let text = std::mem::take(&mut app.input);
                app.send_message(text);
            }
        }
    }

    // popup: @file mentions & /slash commands
    if let Some(response) = text_response.as_ref() {
        let token = trailing_token(&app.input);
        let popup_id = egui::Id::new("input-popup");
        match token {
            Some(InputToken::File(query)) => {
                app.ensure_file_index(); // lazily built — keeps startup fast
                egui::popup_below_widget(
                    ui,
                    popup_id,
                    response,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    |ui: &mut egui::Ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        let matches = crate::picker::search_files(&app.file_index, &query, 12);
                        ui.set_min_width(320.0);
                        if matches.is_empty() {
                            ui.label(egui::RichText::new("…").weak());
                        }
                        for (path, is_dir) in matches {
                            let icon = if is_dir { "📁" } else { "📄" };
                            if ui
                                .add(egui::Button::new(format!("{icon} {path}")).small())
                                .clicked()
                            {
                                insert_token(&mut app.input, &path);
                            }
                        }
                    },
                );
            }
            Some(InputToken::Command(query)) => {
                egui::popup_below_widget(
                    ui,
                    popup_id,
                    response,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    |ui: &mut egui::Ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        ui.set_min_width(320.0);
                        let commands = app.commands.clone();
                        let hits: Vec<_> = commands
                            .iter()
                            .filter(|c| c.name.contains(query.as_str()))
                            .take(12)
                            .collect();
                        if hits.is_empty() {
                            ui.label(egui::RichText::new(t.commands).weak());
                        }
                        for cmd in hits {
                            if ui
                                .add(egui::Button::new(format!("/{}", cmd.name)).small())
                                .on_hover_text(&cmd.description)
                                .clicked()
                            {
                                insert_token(&mut app.input, &cmd.name);
                            }
                        }
                    },
                );
            }
            None => {
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        }
    }

    // -- status bar: cwd --------------------------------------------------------
    ui.add_space(2.0);
    let cwd = app.current_workdir().to_string_lossy().to_string();
    ui.label(
        egui::RichText::new(if cwd.is_empty() { "—" } else { &cwd })
            .size(10.0)
            .color(p.dim),
    );
}

/// A Nexus `.toolbar-btn` chip: 12px text, panel background, subtle border,
/// radius 5; shows a dropdown with the option choices.
fn option_chip(app: &mut VibedeckApp, ui: &mut Ui, p: &Palette, id: &str, fallback: &str) {
    let Some(opt) = app.option_value(id).cloned() else {
        return;
    };
    let label_text = if opt.current_label().is_empty() {
        format!("— {fallback}")
    } else {
        opt.current_label()
    };
    let menu = chip_button(ui, p, &truncate(&label_text, 26));
    if menu.clicked() {
        ui.memory_mut(|m| m.open_popup(egui::Id::new(format!("chip-{id}"))));
    }
    let popup_id = egui::Id::new(format!("chip-{id}"));
    egui::popup_below_widget(
        ui,
        popup_id,
        &menu,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui: &mut egui::Ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.set_min_width(220.0);
            ui.set_max_width(320.0);
            ui.label(
                egui::RichText::new(if opt.name.is_empty() { fallback } else { &opt.name })
                    .small()
                    .weak(),
            );
            ui.separator();
            for choice in &opt.choices {
                let selected = choice.value == opt.current_value;
                let label = if choice.name.is_empty() {
                    choice.value.clone()
                } else {
                    choice.name.clone()
                };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if selected {
                                format!("● {label}")
                            } else {
                                format!("  {label}")
                            })
                            .size(13.0)
                            .color(if selected {
                                p.accent
                            } else {
                                p.text
                            }),
                        )
                        .fill(egui::Color32::TRANSPARENT),
                    )
                    .on_hover_text(&choice.description)
                    .clicked()
                {
                    app.set_config_option(id, choice.value.clone());
                    ui.memory_mut(|m| m.close_popup());
                }
            }
            if opt.choices.is_empty() {
                ui.label(egui::RichText::new("—").weak());
            }
        },
    );
    menu.on_hover_text(&label_text);
}

fn chip_button(ui: &mut Ui, p: &Palette, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(12.0).color(p.muted))
            .fill(p.panel)
            .rounding(egui::Rounding::same(5.0))
            .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
    )
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars - 1).collect::<String>())
    }
}

enum InputToken {
    File(String),
    Command(String),
}

/// Detect a trailing `@query` or `/query` token in the input.
fn trailing_token(input: &str) -> Option<InputToken> {
    let line = input.lines().last().unwrap_or("");
    let (is_command, rest) = if let Some(rest) = line.strip_prefix('/') {
        (true, rest)
    } else if let Some(at) = line.rfind('@') {
        let rest = &line[at + 1..];
        if rest.contains(char::is_whitespace) {
            return None;
        }
        (false, rest)
    } else {
        return None;
    };
    let query = rest.to_string();
    Some(if is_command {
        InputToken::Command(query)
    } else {
        InputToken::File(query)
    })
}

/// Replace the trailing token with the chosen completion.
fn insert_token(input: &mut String, completion: &str) {
    let Some(line_idx) = input.rfind('\n').map(|i| i + 1) else {
        // single-line input
        if let Some(at) = input.rfind('@') {
            input.truncate(at);
            input.push('@');
            input.push_str(completion);
            input.push(' ');
        }
        return;
    };
    let at_in_line = input[line_idx..].rfind('@').map(|i| line_idx + i);
    if let Some(at) = at_in_line {
        input.truncate(at);
        input.push('@');
        input.push_str(completion);
        input.push(' ');
    }
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
