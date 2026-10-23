//! Bottom input bar, mirroring the Nexus InputBar 1:1:
//! a rounded input card (surface bg, 1px border, radius 12) with the
//! textarea on top and a toolbar row below (top border-subtle): on the left
//! the folder (working-directory) icon button; on the right the model /
//! mode / thinking chips (`.ic-model-select` / `.ic-mode-btn`: 11px text,
//! panel bg, subtle border, radius 6, popup opens **upward**) and the
//! 30×30 send/stop button (radius 7, accent while enabled).

use eframe::egui::{self, AboveOrBelow, Align, Layout, Sense, Ui};

use crate::app::VibedeckApp;
use crate::theme::Palette;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();
    let t = app.tr();

    let mut text_response: Option<egui::Response> = None;

    // -- input card (Nexus `.input-card`) ----------------------------------
    egui::Frame::default()
        .fill(p.surface)
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .rounding(12.0)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                // textarea (`.input-card-textarea`: padding 12 14 8)
                egui::Frame::default()
                    .inner_margin(egui::Margin {
                        left: 14.0,
                        right: 14.0,
                        top: 10.0,
                        bottom: 6.0,
                    })
                    .show(ui, |ui| {
                        let response = ui.add(
                            egui::TextEdit::multiline(&mut app.input)
                                .hint_text(egui::RichText::new(t.message_placeholder).color(p.dim))
                                .desired_rows(1)
                                .desired_width(ui.available_width())
                                .text_color(p.text)
                                .lock_focus(true),
                        );
                        text_response = Some(response.clone());
                    });

                // toolbar (`.input-card-toolbar`: top border-subtle, pad 4 8 6)
                ui.separator();
                egui::Frame::default()
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 2.0,
                        bottom: 6.0,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // left: working-directory folder button (`.ic-btn` 28×28)
                            folder_button(app, ui, &p);

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                // rightmost: send / stop (`.ic-send-btn` 30×30)
                                send_button(app, ui, &p);
                                ui.add_space(4.0);
                                // chips: thinking, mode, model (right→left order)
                                option_chip(app, ui, &p, "thinking", t.thinking);
                                option_chip(app, ui, &p, "mode", t.mode);
                                option_chip(app, ui, &p, "model", t.model);
                            });
                        });
                    });
            });
        });

    // Enter sends while the text edit has focus.
    let send_clicked = ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
    if send_clicked {
        if let Some(response) = &text_response {
            if response.has_focus() && !app.busy && !app.input.trim().is_empty() {
                let text = std::mem::take(&mut app.input);
                app.send_message(text);
            }
        }
    }

    // popup: @file mentions & /slash commands (open while a token trails)
    let token = trailing_token(&app.input);
    let popup_id = egui::Id::new("input-popup");
    if token.is_some() {
        ui.memory_mut(|m| m.open_popup(popup_id));
    } else {
        ui.memory_mut(|m| m.close_popup());
    }
    if let Some(response) = text_response.as_ref() {
        match token {
            Some(InputToken::File(query)) => {
                app.ensure_file_index(); // lazily built — keeps startup fast
                egui::popup_above_or_below_widget(
                    ui,
                    popup_id,
                    response,
                    AboveOrBelow::Above,
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
                egui::popup_above_or_below_widget(
                    ui,
                    popup_id,
                    response,
                    AboveOrBelow::Above,
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
            None => {}
        }
    }
}

/// The working-directory folder button (Nexus `.ic-btn`, 28×28, radius 6,
/// folder SVG). Click opens the folder picker; hover shows the full path.
fn folder_button(app: &mut VibedeckApp, ui: &mut Ui, p: &Palette) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::click());
    let color = if response.hovered() { p.text } else { p.muted };
    if response.hovered() {
        ui.painter().rect_filled(rect, egui::Rounding::same(6.0), p.panel_hover);
    }
    // minimal folder glyph: tab + body (mirrors the Nexus SVG)
    let body = egui::Rect::from_min_size(rect.left_top() + egui::vec2(5.0, 9.0), egui::vec2(18.0, 11.0));
    let tab = egui::Rect::from_min_size(rect.left_top() + egui::vec2(5.0, 6.0), egui::vec2(8.0, 3.5));
    let painter = ui.painter_at(rect);
    painter.rect_filled(tab, egui::Rounding::same(1.5), color);
    painter.rect_filled(body, egui::Rounding::same(2.0), color);
    let _ = painter;
    let cwd = app.current_workdir().to_string_lossy().to_string();
    if response.clicked() {
        app.folder_picker.open = true;
        app.folder_picker.dir = app.current_workdir();
        app.folder_picker.entry = app.settings.working_directory.clone();
    }
    response.on_hover_text(if cwd.is_empty() { "—" } else { &cwd });
}

/// `.ic-send-btn` / `.ic-stop-btn`: 30×30, radius 7 — accent + ➤ while the
/// input has text, panel + dim while empty, ⏹ while the agent works.
fn send_button(app: &mut VibedeckApp, ui: &mut Ui, p: &Palette) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::click());
    let enabled = !app.busy && !app.input.trim().is_empty();
    let painter = ui.painter_at(rect);
    let (fill, glyph, glyph_color, size) = if app.busy {
        (p.panel_hover, "⏹", p.text, 13.0)
    } else if enabled {
        (p.accent, "➤", egui::Color32::WHITE, 14.0)
    } else {
        (p.panel, "➤", p.dim, 14.0)
    };
    painter.rect_filled(rect, egui::Rounding::same(7.0), fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(size),
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
        } else if enabled {
            let text = std::mem::take(&mut app.input);
            app.send_message(text);
        }
    }
    response.on_hover_text(if app.busy { "Stop" } else { "Send (Enter)" });
}

/// A Nexus `.ic-model-select` / `.ic-mode-btn` chip: 11px text, panel
/// background, 1px subtle border, radius 6; a click opens the choices
/// **above** the input card (`.ic-mode-dropdown`).
///
/// Always visible: without a session the chip shows the fallback label and
/// an empty menu, exactly like the v0.4.0 pickers.
fn option_chip(app: &mut VibedeckApp, ui: &mut Ui, p: &Palette, id: &str, fallback: &str) {
    let opt = app.option_value(id).cloned();
    let label = match &opt {
        Some(o) if !o.current_label().is_empty() => o.current_label(),
        _ => fallback.to_string(),
    };
    let label = truncate(&label, 18);
    let popup_id = egui::Id::new(format!("chip-{id}"));

    let pad = egui::vec2(8.0, 3.0);
    let font = egui::FontId::proportional(11.0);
    let text_color = p.muted;
    let galley = ui
        .painter()
        .layout_no_wrap(format!("{label} ▾"), font, text_color);
    let size = galley.size() + pad * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);
    let (fill, border) = if response.hovered() {
        (p.panel_hover, p.border)
    } else {
        (p.panel, p.border_subtle)
    };
    painter.rect_filled(rect, egui::Rounding::same(6.0), fill);
    painter.rect_stroke(rect, egui::Rounding::same(6.0), egui::Stroke::new(1.0_f32, border));
    painter.galley(rect.left_top() + pad, galley, text_color);
    let _ = painter;
    if response.clicked() {
        ui.memory_mut(|m| m.open_popup(popup_id));
    }
    let hover_label = label.clone();
    response.clone().on_hover_text(hover_label);

    let selected = opt
        .as_ref()
        .map(|o| o.current_value.clone())
        .unwrap_or_default();
    egui::popup_above_or_below_widget(
        ui,
        popup_id,
        &response,
        AboveOrBelow::Above,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui: &mut egui::Ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.set_min_width(if id == "model" { 240.0 } else { 180.0 });
            ui.set_max_width(340.0);
            match &opt {
                Some(o) => {
                    ui.label(
                        egui::RichText::new(if o.name.is_empty() { fallback } else { &o.name })
                            .small()
                            .weak(),
                    );
                    ui.separator();
                    for choice in &o.choices {
                        let is_selected = choice.value == selected;
                        let row = menu_row(ui, p, &choice.name, is_selected)
                            .on_hover_text(&choice.description);
                        if row.clicked() {
                            app.set_config_option(id, choice.value.clone());
                            ui.memory_mut(|m| m.close_popup());
                        }
                    }
                    if o.choices.is_empty() {
                        ui.label(egui::RichText::new("—").weak());
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new(fallback)
                            .small()
                            .weak(),
                    );
                    ui.separator();
                    ui.label(egui::RichText::new("—").weak());
                }
            }
        },
    );
}

/// One `.ic-mode-item`: 12px, padding 7×12, radius 5, hover panel-hover,
/// accent text + ● when selected. The caller handles the click.
fn menu_row(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> egui::Response {
    let pad = egui::vec2(12.0, 7.0);
    let font = egui::FontId::proportional(12.0);
    let text = if selected { format!("● {label}") } else { label.to_string() };
    let color = if selected { p.accent } else { p.text };
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let size = galley.size() + pad * 2.0;
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, egui::Rounding::same(5.0), p.panel_hover);
    }
    ui.painter().galley(rect.left_top() + pad, galley, color);
    response
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

/// Replace the trailing `@`/`/` token with the chosen completion.
fn insert_token(input: &mut String, completion: &str) {
    let line_start = input.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &input[line_start..];
    let (tok_char, tok_idx) = if line.starts_with('/') {
        ('/', line_start)
    } else if let Some(i) = line.rfind('@') {
        ('@', line_start + i)
    } else {
        return;
    };
    input.truncate(tok_idx);
    input.push(tok_char);
    input.push_str(completion);
    input.push(' ');
}
