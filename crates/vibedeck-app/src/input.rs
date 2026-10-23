//! Bottom input bar: message editor, send/stop, @file mention popup,
//! slash-command popup.

use eframe::egui::{self, Ui};
use vibedeck_core::proto::AcpCommand;

use crate::app::VibedeckApp;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let t = app.tr();
    ui.horizontal(|ui| {
        let send_clicked = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        let response = ui.add(
            egui::TextEdit::multiline(&mut app.input)
                .hint_text(t.message_placeholder)
                .desired_rows(1)
                .desired_width(ui.available_width() - 120.0)
                .lock_focus(true),
        );
        let response_id = response.id;

        if app.busy {
            if ui.button(format!("⏹ {}", t.stop)).clicked() {
                if let Some(session) = app.current_session() {
                    if let Some(acp_session_id) = session.acp_session_id.clone() {
                        app.acp.send(AcpCommand::Cancel {
                            session_id: acp_session_id,
                        });
                    }
                }
            }
        } else {
            let send = ui.add(egui::Button::new(format!("➤ {}", t.send)));
            if (send.clicked() || send_clicked) && response.has_focus() {
                let text = std::mem::take(&mut app.input);
                app.send_message(text);
            }
        }

        // popup: @file mentions & /slash commands
        let token = trailing_token(&app.input);
        let popup_id = egui::Id::new("input-popup");
        match token {
            Some(InputToken::File(query)) => {
                egui::popup_below_widget(
                    ui,
                    popup_id,
                    &response,
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
                    &response,
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
        let _ = response_id;
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
