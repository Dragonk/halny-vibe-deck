//! The chat view, 1:1 with the Nexus ChatView:
//! - empty state: pixel-art flame + "New chat" / "Start a conversation"
//!   + agent status subtitle,
//! - message bubbles: user right (#2c1c17) / AI left (#1d1d21), 6px radius,
//! - thinking blocks & tool calls inside the AI bubble.

use eframe::egui::{self, ScrollArea, Ui};
use vibedeck_core::types::{FileAction, Message, MessageSender, ToolCall, ToolStatus};

use crate::app::VibedeckApp;
use crate::state::AcpStatus;
use crate::theme;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();
    let t = app.tr();

    let has_session = app.current.is_some();
    let empty = app
        .current_session()
        .map(|s| s.messages.is_empty())
        .unwrap_or(true);

    if !has_session || empty {
        empty_state(app, ui, has_session);
        return;
    }

    let Some(idx) = app.current else { return };
    // Move session data into the closure without borrowing `app`.
    let messages: Vec<Message> = app.sessions[idx].messages.clone();
    ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.add_space(10.0);
            for msg in &messages {
                message_bubble(ui, msg, t.tool_output, t.thinking, &p);
                ui.add_space(8.0);
            }
            ui.add_space(6.0);
        });
}

/// The Nexus empty state: flame + title + status subtitle.
fn empty_state(app: &mut VibedeckApp, ui: &mut Ui, has_session: bool) {
    let p = app.palette();
    let t = app.tr();

    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        theme::paint_flame(ui, &p, 0.45);
        ui.add_space(14.0);

        // title — mirrors Nexus: newChat / startConversation / appName
        let title = if !has_session {
            t.new_chat
        } else {
            t.start_conversation
        };
        ui.label(
            egui::RichText::new(title)
                .size(22.0)
                .strong()
                .color(p.text),
        );
        ui.add_space(6.0);

        // subtitle — agent status
        let subtitle = match app.status {
            AcpStatus::Connected => t.agent_ready,
            AcpStatus::Connecting => t.connecting_to_agent,
            AcpStatus::Error => t.agent_connection_error,
            AcpStatus::Disconnected => t.configure_acp,
        };
        ui.label(egui::RichText::new(subtitle).color(p.muted));

        if app.status == AcpStatus::Error && !app.acp_error.is_empty() {
            ui.add_space(8.0);
            egui::Frame::default()
                .fill(p.user_msg)
                .stroke(egui::Stroke::new(1.0_f32, p.red))
                .rounding(6.0)
                .inner_margin(egui::Margin::same(10.0))
                .show(ui, |ui| {
                    ui.set_max_width(480.0);
                    ui.label(
                        egui::RichText::new(format!("⚠ {}", app.acp_error))
                            .small()
                            .color(p.red),
                    );
                });
        }
        if app.status == AcpStatus::Disconnected {
            ui.add_space(8.0);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new(format!("🛠 {}", t.view_setup))
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(p.accent),
                )
                .clicked()
            {
                app.setup_pane.refresh_status_async();
                app.view = crate::state::View::Setup;
            }
        }
    });
}

fn message_bubble(
    ui: &mut Ui,
    msg: &Message,
    output_label: &str,
    thinking_label: &str,
    p: &theme::Palette,
) {
    let is_user = msg.sender == MessageSender::User;

    // Bubble: user right / AI left, max ~75% width (Nexus style).
    let avail = ui.available_width();
    let max_w = (avail * 0.75).max(120.0);
    let frame = egui::Frame::default()
        .inner_margin(egui::Margin::same(10.0))
        .rounding(6.0)
        .stroke(egui::Stroke::new(
            1.0_f32,
            if is_user { p.border_subtle } else { p.border_subtle },
        ))
        .fill(if is_user { p.user_msg } else { p.ai_msg });

    let layout = if is_user {
        egui::Layout::right_to_left(egui::Align::Min)
    } else {
        egui::Layout::left_to_right(egui::Align::Min)
    };
    ui.with_layout(layout, |ui| {
        frame.show(ui, |ui| {
            ui.set_max_width(max_w - 12.0);
            // attachments (@files)
            if !msg.attachments.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for att in &msg.attachments {
                        ui.label(
                            egui::RichText::new(format!("📎 {}", att.path))
                                .monospace()
                                .small()
                                .color(p.accent),
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
                    .color(p.muted),
                    |ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
                        ui.label(&block.content);
                    },
                );
            }
            // tool calls
            for tc in &msg.tool_calls {
                tool_call(ui, tc, output_label, p);
            }
            // message body
            if is_user {
                ui.label(egui::RichText::new(&msg.content).strong().color(p.text));
            } else {
                crate::md::markdown(ui, &msg.content);
                if !msg.is_complete && msg.content.is_empty() && msg.tool_calls.is_empty() {
                    ui.label(egui::RichText::new("…").color(p.muted));
                }
            }
        });
    });
}

fn tool_call(ui: &mut Ui, tc: &ToolCall, output_label: &str, p: &theme::Palette) {
    let (icon, color) = match tc.status {
        ToolStatus::Running => ("⏳", p.yellow),
        ToolStatus::Done => ("✔", p.green),
        ToolStatus::Error => ("✖", p.red),
    };
    let title = format!("{} {} [{}]", icon, tc.title, tc.kind);
    let _ = ui.collapsing(egui::RichText::new(title).color(color).small(), |ui| {
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
}
