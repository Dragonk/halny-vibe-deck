//! The chat view: message bubbles, thinking blocks, tool calls.

use eframe::egui::{self, ScrollArea, Ui};
use vibedeck_core::types::{FileAction, Message, MessageSender, ToolCall, ToolStatus};

use crate::app::VibedeckApp;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let Some(idx) = app.current else {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(format!("💬 {}", app.tr().new_session))
                    .size(18.0)
                    .weak(),
            );
        });
        return;
    };
    // Move session data into the closure without borrowing `app`.
    let messages: Vec<Message> = app.sessions[idx].messages.clone();
    let t = app.tr();
    ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add_space(8.0);
            for msg in &messages {
                message_bubble(ui, msg, t.tool_output, t.thinking);
                ui.add_space(6.0);
            }
        });
}

fn message_bubble(ui: &mut Ui, msg: &Message, output_label: &str, thinking_label: &str) {
    let is_user = msg.sender == MessageSender::User;
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(10.0))
        .fill(if is_user {
            egui::Color32::from_rgb(0x1c, 0x2b, 0x45)
        } else {
            egui::Color32::from_rgb(0x17, 0x18, 0x1d)
        })
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            // attachments (@files)
            if !msg.attachments.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for att in &msg.attachments {
                        ui.label(
                            egui::RichText::new(format!("📎 {}", att.path))
                                .monospace()
                                .small()
                                .color(egui::Color32::from_rgb(0x4d, 0xa3, 0xff)),
                        );
                    }
                });
            }
            // thinking blocks
            for block in &msg.thinking {
                ui.collapsing(
                    egui::RichText::new(format!(
                        "🧠 {thinking_label}{}",
                        if block.is_complete { "" } else { "…" }
                    ))
                    .small()
                    .weak(),
                    |ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
                        ui.label(&block.content);
                    },
                );
            }
            // tool calls
            for tc in &msg.tool_calls {
                tool_call(ui, tc, output_label);
            }
            // message body
            if is_user {
                ui.label(egui::RichText::new(&msg.content).strong());
            } else {
                crate::md::markdown(ui, &msg.content);
                if !msg.is_complete && msg.content.is_empty() && msg.tool_calls.is_empty() {
                    ui.label(egui::RichText::new("…").weak());
                }
            }
        });
}

fn tool_call(ui: &mut Ui, tc: &ToolCall, output_label: &str) {
    let (icon, color) = match tc.status {
        ToolStatus::Running => ("⏳", egui::Color32::from_rgb(0xff, 0xc0, 0x45)),
        ToolStatus::Done => ("✔", egui::Color32::from_rgb(0x50, 0xc8, 0x78)),
        ToolStatus::Error => ("✖", egui::Color32::from_rgb(0xe5, 0x48, 0x4d)),
    };
    let title = format!("{} {} [{}]", icon, tc.title, tc.kind);
    let response = ui.collapsing(egui::RichText::new(title).color(color).small(), |ui| {
        ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
        if !tc.file_changes.is_empty() {
            for fc in &tc.file_changes {
                let action = match fc.action {
                    FileAction::Added => "+",
                    FileAction::Modified => "~",
                    FileAction::Deleted => "−",
                };
                ui.label(format!("{action} {}", fc.path));
            }
            ui.separator();
        }
        if let Some(input) = &tc.input_json {
            ui.label(input);
        }
        if let Some(output) = &tc.output {
            ui.separator();
            ui.label(egui::RichText::new(output_label).weak().italics());
            ui.label(output);
        }
    });
    let _ = response;
}
