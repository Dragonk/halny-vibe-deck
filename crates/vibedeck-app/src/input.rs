//! Bottom input bar, mirroring the Nexus InputBar 1:1:
//! a rounded input card (surface bg, 1px border, radius 12) with the
//! textarea on top and a toolbar row below (top border-subtle): on the left
//! the folder (working-directory) icon button; on the right the model /
//! mode / thinking chips (`.ic-model-select` / `.ic-mode-btn`: 11px text,
//! panel bg, subtle border, radius 6, popup opens **upward**) and the
//! 30×30 send/stop button (radius 7, accent while enabled).
//!
//! Popup management: egui 0.29 keeps a SINGLE popup slot in memory
//! (`opened_popup: Option<Id>`), and `close_popup()` takes no argument —
//! it closes whatever popup is open. The `@`/`/` suggestion popup must
//! therefore only close the token popup when the token popup itself is
//! open, otherwise it would instantly kill a chip dropdown clicked in the
//! same frame (the v0.5.1 bug: chip clicks "did nothing").

use eframe::egui::{self, AboveOrBelow, Align, Layout, Sense, Ui};

use crate::app::VibedeckApp;
use crate::theme::Palette;

/// The three option-chip popup ids (model / mode / thinking).
const CHIP_IDS: [&str; 3] = ["model", "mode", "thinking"];

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

    // -- popup arbitration (single memory slot — see module docs) ---------
    // Editing the text closes any open chip dropdown: the user moved on
    // from the pickers to typing.
    if text_response.as_ref().map(|r| r.changed()).unwrap_or(false) {
        for id in CHIP_IDS {
            let pid = egui::Id::new(format!("chip-{id}"));
            if ui.memory(|m| m.is_popup_open(pid)) {
                ui.memory_mut(|m| m.close_popup());
            }
        }
    }
    // A trailing `@`/`/` token re-opens the suggestion popup — but only
    // when no chip dropdown is currently open (a click on a chip in this
    // or a previous frame wins). When the token disappears, close the
    // suggestion popup — but only if it is the one open.
    let token = trailing_token(&app.input);
    let popup_id = egui::Id::new("input-popup");
    let chip_open = CHIP_IDS.iter().any(|id| {
        ui.memory(|m| m.is_popup_open(egui::Id::new(format!("chip-{id}"))))
    });
    match token {
        Some(_) => {
            if !chip_open {
                ui.memory_mut(|m| m.open_popup(popup_id));
            }
        }
        None => {
            if ui.memory(|m| m.is_popup_open(popup_id)) {
                ui.memory_mut(|m| m.close_popup());
            }
        }
    }

    // popup: @file mentions & /slash commands (open while a token trails)
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
                let rows = slash_rows(app, &query, 14);
                egui::popup_above_or_below_widget(
                    ui,
                    popup_id,
                    response,
                    AboveOrBelow::Above,
                    egui::PopupCloseBehavior::CloseOnClickOutside,
                    |ui: &mut egui::Ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        ui.set_min_width(320.0);
                        if rows.is_empty() {
                            ui.label(egui::RichText::new(t.commands).weak());
                        }
                        for row in &rows {
                            let resp = menu_row(ui, &p, &row.label, false)
                                .on_hover_text(&row.hover);
                            if resp.clicked() {
                                match &row.action {
                                    SlashAction::Command(name) => {
                                        insert_token(&mut app.input, name);
                                    }
                                    SlashAction::Insert(content) => {
                                        insert_slash_content(&mut app.input, content);
                                    }
                                }
                            }
                        }
                    },
                );
            }
            None => {}
        }
    }
}

/// One row of the `/` suggestion list (commands from the agent, skills and
/// prompts from `~/.vibe`, exactly like the Nexus slash picker).
struct SlashRow {
    label: String,
    hover: String,
    action: SlashAction,
}

enum SlashAction {
    /// Insert `/<name> ` into the input.
    Command(String),
    /// Replace the `/token` with raw content (skills / prompts).
    Insert(String),
}

fn slash_rows(app: &VibedeckApp, query: &str, limit: usize) -> Vec<SlashRow> {
    let query = query.to_lowercase();
    let mut rows: Vec<SlashRow> = Vec::new();
    for cmd in &app.commands {
        if !cmd.name.to_lowercase().contains(&query) {
            continue;
        }
        let mut hover = if cmd.description.is_empty() {
            format!("/{}", cmd.name)
        } else {
            cmd.description.clone()
        };
        if !cmd.hint.is_empty() {
            hover = format!("{hover} — {}", cmd.hint);
        }
        rows.push(SlashRow {
            label: format!("/{}", cmd.name),
            hover,
            action: SlashAction::Command(cmd.name.clone()),
        });
    }
    for skill in &app.slash_skills {
        if !skill.0.to_lowercase().contains(&query) {
            continue;
        }
        rows.push(SlashRow {
            label: format!("{} · {}", skill.0, app.tr().skill_label),
            hover: skill.1.chars().take(160).collect(),
            action: SlashAction::Insert(skill.1.clone()),
        });
    }
    for prompt in &app.slash_prompts {
        if !prompt.0.to_lowercase().contains(&query) {
            continue;
        }
        rows.push(SlashRow {
            label: format!("{} · {}", prompt.0, app.tr().prompt_label),
            hover: prompt.1.chars().take(160).collect(),
            action: SlashAction::Insert(prompt.1.clone()),
        });
    }
    rows.truncate(limit);
    rows
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
/// **above** the input card (`.ic-mode-dropdown`). Clicking again closes.
///
/// Always visible: without a session the chip shows the fallback label and
/// an empty menu — the config options arrive with the first ACP session
/// (which the app now primes right after connecting).
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
    let is_open = ui.memory(|m| m.is_popup_open(popup_id));
    if response.clicked() {
        // Toggle: open the dropdown, or close it when already open.
        if is_open {
            ui.memory_mut(|m| m.close_popup());
        } else {
            ui.memory_mut(|m| m.open_popup(popup_id));
        }
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

/// Replace the trailing `/token` with raw content (skills / prompts),
/// followed by a newline — the Nexus `insertSlashItem` behaviour.
fn insert_slash_content(input: &mut String, content: &str) {
    let line_start = input.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &input[line_start..];
    if !line.starts_with('/') {
        return;
    }
    input.truncate(line_start);
    input.push_str(content);
    if !input.ends_with('\n') {
        input.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_trailing_tokens() {
        assert!(matches!(trailing_token("hello @sr"), Some(InputToken::File(_))));
        assert!(matches!(trailing_token("/comp"), Some(InputToken::Command(_))));
        assert!(matches!(trailing_token("no /cmd mid line"), None));
        assert!(matches!(trailing_token("@done "), None));
    }

    #[test]
    fn inserts_slash_content() {
        let mut input = "/ski".to_string();
        insert_slash_content(&mut input, "be excellent");
        assert_eq!(input, "be excellent\n");
        let mut input = "prompt:\n/pro".to_string();
        insert_slash_content(&mut input, "answer");
        assert_eq!(input, "prompt:\nanswer\n");
    }
}
