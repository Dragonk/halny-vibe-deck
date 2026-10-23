//! Bottom input bar, mirroring the Nexus InputBar:
//! a rounded input box, send/stop, and a row of model / mode / cwd pickers.

use eframe::egui::{self, Ui};
use vibedeck_core::proto::AcpCommand;

use crate::app::VibedeckApp;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();
    let t = app.tr();

    // -- input box ------------------------------------------------------------
    let send_clicked = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    let mut text_response: Option<egui::Response> = None;

    egui::Frame::default()
        .fill(p.input_bg)
        .stroke(egui::Stroke::new(1.0_f32, p.border_subtle))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(10.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut app.input)
                        .hint_text(egui::RichText::new(t.message_placeholder).color(p.muted))
                        .desired_rows(1)
                        .desired_width(ui.available_width() - 96.0)
                        .text_color(p.text)
                        .lock_focus(true),
                );
                text_response = Some(response.clone());

                if app.busy {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(format!("⏹ {}", t.stop)).strong())
                                .fill(p.panel_hover),
                        )
                        .clicked()
                    {
                        if let Some(session) = app.current_session() {
                            if let Some(acp_session_id) = session.acp_session_id.clone() {
                                app.acp.send(AcpCommand::Cancel {
                                    session_id: acp_session_id,
                                });
                            }
                        }
                    }
                } else {
                    let send = ui.add(
                        egui::Button::new(
                            egui::RichText::new(format!("➤ {}", t.send))
                                .strong()
                                .color(egui::Color32::WHITE),
                        )
                        .fill(p.accent),
                    );
                    if (send.clicked() || send_clicked) && response.has_focus() {
                        let text = std::mem::take(&mut app.input);
                        app.send_message(text);
                    }
                }
            });
        });

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

    // -- model / mode / cwd row -------------------------------------------------
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let small_btn = |ui: &mut egui::Ui, label: String| {
            ui.add(
                egui::Button::new(egui::RichText::new(label).small().color(p.muted))
                    .fill(egui::Color32::TRANSPARENT)
                    .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
            )
        };

        // model picker
        let current_model = app
            .settings
            .preferred_model
            .clone()
            .or_else(|| app.models.first().cloned())
            .unwrap_or_default();
        let model_label = if current_model.is_empty() {
            t.model.to_string()
        } else {
            current_model.clone()
        };
        ui.menu_button(
            egui::RichText::new(model_label).small().color(p.muted),
            |ui| {
                let models = app.models.clone();
                for model in models {
                    if ui.selectable_label(model == current_model, &model).clicked() {
                        app.settings.preferred_model = Some(model.clone());
                        app.settings.save();
                        if let Some(session) = app.current_session() {
                            if let Some(acp_session_id) = session.acp_session_id.clone() {
                                app.acp.send(AcpCommand::SetConfigOption {
                                    session_id: acp_session_id,
                                    key: "model".into(),
                                    value: model,
                                });
                            }
                        }
                        ui.close_menu();
                    }
                }
                if app.models.is_empty() {
                    ui.label(egui::RichText::new("(—)").weak());
                }
            },
        );

        // mode picker
        ui.menu_button(egui::RichText::new(t.mode).small().color(p.muted), |ui| {
            let modes = app.modes.clone();
            for mode in modes {
                if ui.selectable_label(false, &mode.name).clicked() {
                    if let Some(session) = app.current_session() {
                        if let Some(acp_session_id) = session.acp_session_id.clone() {
                            app.acp.send(AcpCommand::SetConfigOption {
                                session_id: acp_session_id,
                                key: "mode".into(),
                                value: mode.id.clone(),
                            });
                        }
                    }
                    ui.close_menu();
                }
            }
            if app.modes.is_empty() {
                ui.label(egui::RichText::new("(—)").weak());
            }
        });

        // working directory picker
        let cwd = app.current_workdir().to_string_lossy().to_string();
        if small_btn(ui, format!("📂 {}", cwd))
            .on_hover_text(&cwd)
            .clicked()
        {
            app.folder_picker.open = true;
            app.folder_picker.dir = app.current_workdir();
            app.folder_picker.entry = app.settings.working_directory.clone();
        }

        // disclaimer, right-aligned
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(t.disclaimer_short).small().color(p.dim));
        });
    });
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
