//! Left sidebar: local sessions, CLI (backend) sessions, new session,
//! working-directory switch.

use eframe::egui::{self, Ui};
use vibedeck_core::proto::AcpCommand;

use crate::app::VibedeckApp;
use crate::md::plain_preview;

pub fn show(app: &mut VibedeckApp, ui: &mut Ui) {
    let t = app.tr();

    ui.add_space(4.0);
    if ui
        .add(egui::Button::new(
            egui::RichText::new(format!("＋ {}", t.new_session)).strong(),
        ))
        .clicked()
    {
        app.new_local_session();
        app.refresh_file_index();
    }
    if ui.button(t.browse).clicked() {
        app.folder_picker.open = true;
        app.folder_picker.dir = app.current_workdir();
        app.folder_picker.entry = app.settings.working_directory.clone();
    }

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("📂 {}", t.working_dir))
            .small()
            .weak(),
    );
    let cwd = app.current_workdir().to_string_lossy().to_string();
    if ui
        .add(egui::Button::new(
            egui::RichText::new(cwd.clone()).small().monospace(),
        ))
        .on_hover_text(&cwd)
        .clicked()
    {
        app.folder_picker.open = true;
        app.folder_picker.dir = app.current_workdir();
    }

    ui.separator();

    // local sessions
    let current = app.current;
    let limit = app.settings.sidebar_sessions_limit.unwrap_or(30);
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .id_salt("local-sessions")
        .show(ui, |ui| {
            ui.label(egui::RichText::new(t.sessions).small().weak());
            let names: Vec<String> = app
                .sessions
                .iter()
                .take(limit)
                .map(|s| {
                    let preview = s
                        .messages
                        .last()
                        .map(|m| plain_preview(&m.content, 48))
                        .unwrap_or_default();
                    if preview.is_empty() {
                        s.name.clone()
                    } else {
                        preview
                    }
                })
                .collect();
            for (i, name) in names.iter().enumerate() {
                let selected = current == Some(i);
                let text = egui::RichText::new(format!("💬 {name}")).small();
                if ui
                    .selectable_label(selected, text)
                    .on_hover_text(name)
                    .clicked()
                {
                    app.current = Some(i);
                }
            }
            if app.sessions.is_empty() {
                ui.label(egui::RichText::new(t.no_sessions).small().weak());
            }
        });

    ui.separator();

    // sessions recorded by the CLI (vibe --prompt / interactive)
    let backend_sessions = app.backend_sessions.clone();
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .max_height(280.0)
        .id_salt("cli-sessions")
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} ({})",
                    t.import_from_cli,
                    backend_sessions.len()
                ))
                .small()
                .weak(),
            );
            for info in &backend_sessions {
                let label = if info.name.is_empty() {
                    format!("{} — {}", info.session_id, info.start_time)
                } else {
                    format!("{} — {}", info.name, info.start_time)
                };
                if ui
                    .selectable_label(false, egui::RichText::new(label).small())
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
                        if info.name.is_empty() {
                            "CLI session"
                        } else {
                            &info.name
                        },
                        &cwd,
                    );
                    session.working_directory = cwd.clone();
                    app.sessions.insert(0, session);
                    app.current = Some(0);
                    app.settings.working_directory = cwd.clone();
                    app.settings.save();
                    app.refresh_file_index();
                    app.busy = true;
                    app.acp.send(AcpCommand::LoadSession {
                        session_id: info.session_id.clone(),
                        cwd,
                        model: app.settings.preferred_model.clone(),
                    });
                }
            }
        });
}
