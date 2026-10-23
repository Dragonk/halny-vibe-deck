//! The chat view, 1:1 with the Nexus ChatView:
//! - empty state: pixel-art flame + "New chat" / "Start a conversation"
//!   + agent status subtitle,
//! - **user messages**: right-aligned bubble, panel background, radius
//!   18/18/4/18, padding 10×18,
//! - **AI messages**: no bubble — a 28×28 accent avatar with the white
//!   pixel flame + markdown on the app background, with thinking blocks
//!   and rich tool calls inline (bash blocks, file chips, edit diffs,
//!   subagent transcripts, question chips).

use eframe::egui::{self, ScrollArea, Ui};
use vibedeck_core::types::{Message, MessageSender, ToolCall, ToolStatus};
use vibedeck_vibe::stats::VibeSubagentSession;

use crate::app::VibedeckApp;
use crate::i18n::Str;
use crate::state::AcpStatus;
use crate::theme::{self, Palette};

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let p = app.palette();

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
    let acp_id = app.sessions[idx].acp_session_id.clone();
    let busy = app.busy;
    let t = app.tr();
    ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .id_salt("chat-scroll")
        .show(ui, |ui| {
            ui.add_space(10.0);
            for msg in &messages {
                match msg.sender {
                    MessageSender::User => user_bubble(ui, msg, &p),
                    MessageSender::Ai => ai_row(app, ui, msg, busy, acp_id.as_deref(), t, &p),
                    MessageSender::System => system_line(ui, msg, &p),
                }
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
                .size(20.0)
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
        ui.label(egui::RichText::new(subtitle).size(14.0).color(p.muted));

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

/// User message: right-aligned bubble (panel bg, radius 18/18/4/18).
fn user_bubble(ui: &mut Ui, msg: &Message, p: &Palette) {
    let avail = ui.available_width();
    let max_w = (avail * 0.75).max(120.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        egui::Frame::default()
            .inner_margin(egui::Margin::symmetric(18.0, 10.0))
            .rounding(egui::Rounding {
                nw: 18.0,
                ne: 18.0,
                sw: 4.0,
                se: 18.0,
            })
            .fill(p.panel)
            .show(ui, |ui| {
                // hard width so long text always wraps inside the bubble
                ui.set_width(ui.available_width().min(max_w - 24.0));
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
                ui.label(egui::RichText::new(&msg.content).color(p.text));
            });
    });
}

/// AI message: 28×28 accent avatar with the white flame + plain text —
/// no bubble, exactly like `.msg-ai` in the Nexus stylesheet.
fn ai_row(
    app: &mut VibedeckApp,
    ui: &mut Ui,
    msg: &Message,
    busy: bool,
    acp_id: Option<&str>,
    t: &'static Str,
    p: &Palette,
) {
    egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(12.0, 0.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // avatar
                let (rect, _response) =
                    ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, egui::Rounding::same(6.0), p.accent);
                theme::paint_flame_mono_at(&painter, rect, egui::Color32::WHITE);
                if busy && !msg.is_complete {
                    let anchor = rect.right_bottom() + egui::vec2(-7.0, -7.0);
                    egui::Spinner::new()
                        .size(14.0)
                        .paint_at(ui, egui::Rect::from_min_size(anchor, egui::vec2(14.0, 14.0)));
                }
                ui.add_space(12.0);

                // content column — explicit width so everything wraps
                let content_w = ui.available_width() - 8.0;
                ui.vertical(|ui| {
                    ui.set_width(content_w);
                    // thinking blocks
                    for block in &msg.thinking {
                        ui.collapsing(
                            egui::RichText::new(format!(
                                "🧠 {}{}",
                                t.thinking,
                                if block.is_complete { "" } else { "…" }
                            ))
                            .small()
                            .color(p.muted),
                            |ui| {
                                ui.style_mut().override_text_style =
                                    Some(egui::TextStyle::Small);
                                ui.label(&block.content);
                            },
                        );
                    }
                    // tool calls
                    for tc in &msg.tool_calls {
                        tool_call(app, ui, tc, acp_id, t, p);
                    }
                    // message body — plain markdown text, no bubble
                    crate::md::markdown(ui, &msg.content);
                    if !msg.is_complete && msg.content.is_empty() && msg.tool_calls.is_empty() {
                        ui.label(egui::RichText::new("…").color(p.muted));
                    }
                });
            });
        });
}

fn system_line(ui: &mut Ui, msg: &Message, p: &Palette) {
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new(&msg.content)
                .small()
                .weak()
                .color(p.muted),
        );
    });
}

// -- rich tool calls -----------------------------------------------------------

const OUT_CAP: usize = 4000;

/// One tool call, rendered like the Nexus MessageBubble blocks:
/// status color, context chips, bash stdout/stderr/exit code, edit diffs,
/// grep patterns, subagent transcripts.
fn tool_call(
    app: &mut VibedeckApp,
    ui: &mut Ui,
    tc: &ToolCall,
    acp_id: Option<&str>,
    t: &'static Str,
    p: &Palette,
) {
    let (icon, color) = match tc.status {
        ToolStatus::Running => ("⏳", p.yellow),
        ToolStatus::Done => ("✔", p.green),
        ToolStatus::Error => ("✖", p.red),
    };
    let input: Option<serde_json::Value> = tc
        .input_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok());
    let kind = tool_kind(tc, input.as_ref());

    let title = format!("{icon} {}", tc.title);
    let header = ui
        .collapsing(
            egui::RichText::new(title).color(color).small().strong(),
            |ui| {
                egui::Frame::default()
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        p.border.gamma_multiply(0.55),
                    ))
                    .fill(p.surface)
                    .rounding(egui::Rounding::same(6.0))
                    .inner_margin(egui::Margin::same(8.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        tool_body(app, ui, tc, &kind, input.as_ref(), acp_id, t, p);
                    });
            },
        )
        .header_response;
    let _ = header;
}

#[derive(Clone, Copy, PartialEq)]
enum ToolKind {
    Bash,
    Read,
    Edit,
    Grep,
    Subagent,
    Question,
    Generic,
}

fn tool_kind(tc: &ToolCall, input: Option<&serde_json::Value>) -> ToolKind {
    let hay = format!("{} {}", tc.kind, tc.title).to_lowercase();
    let has = |k: &str| input.is_some_and(|v| v.get(k).is_some());
    if has("command") || hay.contains("bash") || hay.contains("shell") || hay.contains("terminal") {
        ToolKind::Bash
    } else if has("old_string") || has("new_string") || hay.contains("edit") {
        ToolKind::Edit
    } else if hay.contains("subagent") || hay.contains("task") || has("prompt") {
        ToolKind::Subagent
    } else if hay.contains("question") || hay.contains("ask_user") {
        ToolKind::Question
    } else if has("pattern") || hay.contains("grep") || hay.contains("search") {
        ToolKind::Grep
    } else if has("path") || has("file_path") || hay.contains("read") {
        ToolKind::Read
    } else {
        ToolKind::Generic
    }
}

fn str_field(input: &Option<serde_json::Value>, key: &str) -> Option<String> {
    input
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn tool_body(
    app: &mut VibedeckApp,
    ui: &mut Ui,
    tc: &ToolCall,
    kind: &ToolKind,
    input: Option<&serde_json::Value>,
    acp_id: Option<&str>,
    t: &'static Str,
    p: &Palette,
) {
    // file changes (when the ACP reports them)
    if !tc.file_changes.is_empty() {
        for fc in &tc.file_changes {
            let action = match fc.action {
                vibedeck_core::types::FileAction::Added => "+",
                vibedeck_core::types::FileAction::Modified => "~",
                vibedeck_core::types::FileAction::Deleted => "−",
            };
            ui.label(
                egui::RichText::new(format!("{action} {}", fc.path))
                    .monospace()
                    .small()
                    .color(p.accent),
            );
        }
        ui.add_space(4.0);
    }

    match kind {
        ToolKind::Bash => {
            let cmd = str_field(&input.cloned(), "command")
                .or_else(|| input.and_then(|v| v.get("command").and_then(|c| c.as_str()).map(str::to_string)))
                .unwrap_or_else(|| tc.title.clone());
            ui.label(egui::RichText::new(format!("$ {cmd}")).monospace().small().color(p.text));
            ui.add_space(4.0);
            render_output(ui, tc, t, p);
        }
        ToolKind::Read => {
            let path = str_field(&input.cloned(), "path")
                .or_else(|| str_field(&input.cloned(), "file_path"))
                .unwrap_or_else(|| tc.title.clone());
            chip(ui, p, &path);
            ui.add_space(4.0);
            render_output(ui, tc, t, p);
        }
        ToolKind::Edit => {
            let old = str_field(&input.cloned(), "old_string").unwrap_or_default();
            let new = str_field(&input.cloned(), "new_string").unwrap_or_default();
            if !old.is_empty() || !new.is_empty() {
                diff_block(ui, p, &old, &new);
                ui.add_space(4.0);
            } else {
                let path = str_field(&input.cloned(), "path")
                    .or_else(|| str_field(&input.cloned(), "file_path"))
                    .unwrap_or_else(|| tc.title.clone());
                chip(ui, p, &path);
                ui.add_space(4.0);
            }
            render_output(ui, tc, t, p);
        }
        ToolKind::Grep => {
            let pattern = str_field(&input.cloned(), "pattern").unwrap_or_default();
            if !pattern.is_empty() {
                chip(ui, p, &format!("pattern: {pattern}"));
            }
            let path = str_field(&input.cloned(), "path");
            if let Some(path) = path {
                chip(ui, p, &path);
            }
            ui.add_space(4.0);
            render_output(ui, tc, t, p);
        }
        ToolKind::Question => {
            let question = str_field(&input.cloned(), "question")
                .unwrap_or_else(|| tc.title.clone());
            ui.label(egui::RichText::new(question).color(p.text));
            if let Some(choices) = input.and_then(|v| v.get("choices")).and_then(|c| c.as_array()) {
                ui.add_space(4.0);
                for choice in choices {
                    if let Some(txt) = choice.as_str() {
                        chip(ui, p, txt);
                    }
                }
            }
        }
        ToolKind::Subagent => {
            let prompt = str_field(&input.cloned(), "prompt")
                .or_else(|| str_field(&input.cloned(), "description"))
                .unwrap_or_default();
            if !prompt.is_empty() {
                ui.label(
                    egui::RichText::new(format!("{} {prompt}", t.tool_subagent_prompt))
                        .small()
                        .color(p.muted),
                );
                out_block(ui, p, &prompt, p.text, false);
                ui.add_space(4.0);
            }
            if let Some(output) = &tc.output {
                if !output.trim().is_empty() {
                    ui.label(
                        egui::RichText::new(t.tool_subagent_response)
                            .small()
                            .color(p.muted),
                    );
                    out_block(ui, p, output, p.text, false);
                    ui.add_space(4.0);
                }
            }
            // full subagent conversation from the session logs
            if let Some(acp_id) = acp_id {
                let logs: Option<Vec<VibeSubagentSession>> =
                    if app.ensure_subagent_logs(acp_id) {
                        app.subagent_logs.get(acp_id).cloned()
                    } else {
                        None
                    };
                if let Some(logs) = logs {
                    for sub in &logs {
                        ui.collapsing(
                            egui::RichText::new(format!(
                                "🤖 {} — {} ({})",
                                sub.agent_name,
                                t.tool_transcript,
                                sub.messages.len()
                            ))
                            .small()
                            .color(egui::Color32::from_rgb(0x4d, 0xa3, 0xff)),
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(260.0)
                                    .auto_shrink([false, true])
                                    .id_salt(format!("sub-log-{}", sub.session_id))
                                    .show(ui, |ui| {
                                        for m in &sub.messages {
                                            let content =
                                                m.content.clone().unwrap_or_default();
                                            let preview: String = content
                                                .chars()
                                                .take(600)
                                                .collect();
                                            let (marker, color) = match m.role.as_str() {
                                                "user" => ("🧑", p.accent),
                                                "assistant" => ("🤖", p.green),
                                                "tool" => ("🔧", p.yellow),
                                                _ => ("·", p.muted),
                                            };
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label(
                                                    egui::RichText::new(marker)
                                                        .small()
                                                        .color(color),
                                                );
                                                ui.label(
                                                    egui::RichText::new(preview)
                                                        .small()
                                                        .color(p.text),
                                                );
                                            });
                                        }
                                    });
                            },
                        );
                    }
                }
            }
        }
        ToolKind::Generic => {
            if let Some(input) = &tc.input_json {
                if !input.trim().is_empty() {
                    out_block(ui, p, input, p.muted, true);
                    ui.add_space(4.0);
                }
            }
            render_output(ui, tc, t, p);
        }
    }
}

/// Tool output: JSON `{stdout, stderr, returncode}` blocks, `<tool_error>`
/// red panels, or a plain capped text block.
fn render_output(ui: &mut Ui, tc: &ToolCall, t: &'static Str, p: &Palette) {
    let Some(output) = &tc.output else { return };
    if output.trim().is_empty() {
        return;
    }
    if is_error_text(output) {
        ui.label(
            egui::RichText::new(format!("✖ {}", t.error))
                .small()
                .strong()
                .color(p.red),
        );
        out_block(ui, p, output, p.red, true);
        return;
    }
    // bash-style JSON output
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(output) {
        let stdout = v.get("stdout").and_then(|s| s.as_str()).unwrap_or("");
        let stderr = v.get("stderr").and_then(|s| s.as_str()).unwrap_or("");
        let rc = v.get("returncode").and_then(|r| r.as_i64());
        if !stdout.is_empty() {
            ui.label(
                egui::RichText::new(t.tool_stdout).small().weak().color(p.muted),
            );
            out_block(ui, p, stdout, p.text, true);
        }
        if !stderr.is_empty() {
            ui.label(
                egui::RichText::new(t.tool_stderr).small().weak().color(p.yellow),
            );
            out_block(ui, p, stderr, p.yellow, true);
        }
        if let Some(rc) = rc {
            let color = if rc == 0 { p.green } else { p.red };
            ui.label(
                egui::RichText::new(format!("{}: {rc}", t.tool_return_code))
                    .small()
                    .monospace()
                    .color(color),
            );
        }
        if stdout.is_empty() && stderr.is_empty() && rc.is_none() {
            out_block(ui, p, output, p.text, true);
        }
        return;
    }
    ui.label(
        egui::RichText::new(t.tool_output).small().weak().color(p.muted),
    );
    out_block(ui, p, output, p.text, true);
}

fn is_error_text(output: &str) -> bool {
    let low = output.to_lowercase();
    low.contains("<tool_error>") || low.starts_with("error:") || low.contains("error running tool")
}

/// Monospace output block on a slightly darker surface, capped at OUT_CAP
/// characters.
fn out_block(ui: &mut Ui, _p: &Palette, text: &str, color: egui::Color32, mono: bool) {
    let capped: String = if text.chars().count() > OUT_CAP {
        let head: String = text.chars().take(OUT_CAP).collect();
        format!("{head}\n… ({}/{} chars)", OUT_CAP, text.chars().count())
    } else {
        text.to_string()
    };
    egui::Frame::default()
        .fill(ui.visuals().extreme_bg_color)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            let rt = egui::RichText::new(capped.as_str()).small().color(color);
            let rt = if mono { rt.monospace() } else { rt };
            ui.label(rt);
        });
}

/// SEARCH/REPLACE diff block: red `-` old lines, green `+` new lines.
fn diff_block(ui: &mut Ui, p: &Palette, old: &str, new: &str) {
    egui::Frame::default()
        .fill(ui.visuals().extreme_bg_color)
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            for line in old.lines().take(24) {
                ui.label(
                    egui::RichText::new(format!("- {line}"))
                        .small()
                        .monospace()
                        .color(p.red),
                );
            }
            for line in new.lines().take(24) {
                ui.label(
                    egui::RichText::new(format!("+ {line}"))
                        .small()
                        .monospace()
                        .color(p.green),
                );
            }
        });
}

/// A small border chip with monospace text (paths, patterns, choices).
fn chip(ui: &mut Ui, p: &Palette, text: &str) {
    egui::Frame::default()
        .stroke(egui::Stroke::new(1.0_f32, p.border))
        .rounding(egui::Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(6.0, 2.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .monospace()
                    .small()
                    .color(p.accent),
            );
        });
}
