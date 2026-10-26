//! Setup view: dependency status (uv, mistral-vibe, vibe-acp) with
//! one-click install/upgrade, streaming progress.

use std::sync::mpsc::Receiver;

use eframe::egui::{self, Ui};
use mistraldeck_setup::{SetupProgress, SetupStatus};

use crate::app::MistraldeckApp;

#[derive(Default)]
pub struct SetupPane {
    pub status: SetupStatus,
    pub log: Vec<(String, bool)>, // (message, is_error)
    pub rx: Option<Receiver<SetupProgress>>,
    /// Result channel of an in-flight background dependency probe.
    status_rx: Option<Receiver<SetupStatus>>,
    /// Set while `refresh_status_async` is running.
    probing: bool,
    /// The Settings "auto-detect" button is waiting for a probe result.
    pub autodetect: bool,
}

impl SetupPane {
    /// Synchronous probe — kept for callers that explicitly want to block.
    #[allow(dead_code)]
    pub fn refresh_status(&mut self) {
        self.status = mistraldeck_setup::check_setup();
    }

    /// Probe dependencies on a background thread (used at startup and when
    /// opening the Setup view — never blocks the UI thread).
    pub fn refresh_status_async(&mut self) {
        if self.probing {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel::<SetupStatus>();
        self.status_rx = Some(rx);
        self.probing = true;
        std::thread::Builder::new()
            .name("mistraldeck-setup-probe".into())
            .spawn(move || {
                let _ = tx.send(mistraldeck_setup::check_setup());
            })
            .expect("failed to spawn setup probe thread");
    }

    pub fn is_working(&self) -> bool {
        self.rx.is_some()
    }

    /// Drain the background dependency probe.
    /// Returns `true` if a fresh probe result just arrived.
    pub fn poll_status(&mut self) -> bool {
        let mut fresh = false;
        if let Some(rx) = self.status_rx.take() {
            match rx.try_recv() {
                Ok(status) => {
                    self.status = status;
                    self.probing = false;
                    fresh = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => self.status_rx = Some(rx),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.probing = false,
            }
        }
        fresh
    }

    /// Drain install progress from the background thread.
    pub fn poll(&mut self) {
        let Some(rx) = self.rx.take() else { return };
        loop {
            match rx.try_recv() {
                Ok(SetupProgress::Info(msg)) => self.log.push((msg, false)),
                Ok(SetupProgress::Success(msg)) => self.log.push((msg, false)),
                Ok(SetupProgress::Error(msg)) => self.log.push((msg, true)),
                Ok(SetupProgress::Done(status)) => {
                    self.status = status;
                    break; // finished: keep rx taken (is_working() → false)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.rx = Some(rx);
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
    }
}

pub fn show(app: &mut MistraldeckApp, ui: &mut Ui) {
    app.setup_pane.poll();
    let t = app.tr();

    ui.heading(t.setup_title);
    ui.label(egui::RichText::new(t.setup_hint).weak());
    ui.add_space(10.0);

    egui::Grid::new("setup-status")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            row(ui, "uv", &app.setup_pane.status.uv_path, t);
            row(ui, "vibe-acp", &app.setup_pane.status.vibe_acp_path, t);
            let vibe_version = app
                .setup_pane
                .status
                .vibe_cli_version
                .clone()
                .unwrap_or_else(|| "—".into());
            ui.label("vibe");
            ui.label(egui::RichText::new(vibe_version).monospace());
            ui.end_row();
        });

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new(
                egui::RichText::new(format!("🔄 {}", t.setup_check)).strong(),
            ))
            .clicked()
        {
            app.setup_pane.refresh_status_async();
        }
        let working = app.setup_pane.is_working();
        let install_label = if working {
            t.setup_installing.to_string()
        } else {
            t.setup_install.to_string()
        };
        if ui
            .add_enabled(
                !working,
                egui::Button::new(egui::RichText::new(install_label).strong()),
            )
            .clicked()
        {
            let (tx, rx) = std::sync::mpsc::channel::<SetupProgress>();
            app.setup_pane.rx = Some(rx);
            std::thread::Builder::new()
                .name("mistraldeck-setup".into())
                .spawn(move || {
                    let tx = tx;
                    let _ = mistraldeck_setup::install_dependencies(|p| {
                        let _ = tx.send(p.clone());
                    });
                    // Always terminate with a fresh status.
                    let _ = tx.send(SetupProgress::Done(mistraldeck_setup::check_setup()));
                })
                .expect("failed to spawn setup thread");
        }
    });

    // After a successful install, offer to connect immediately.
    if let Some(path) = app.setup_pane.status.vibe_acp_path.clone() {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(t.setup_done).color(egui::Color32::from_rgb(0x50, 0xc8, 0x78)),
        );
        let path_str = path.to_string_lossy().to_string();
        if ui
            .add(egui::Button::new(
                egui::RichText::new(format!("🚀 {} ({})", t.view_chat, path_str)).strong(),
            ))
            .on_hover_text(&path_str)
            .clicked()
        {
            app.start_agent(&path_str);
            app.view = crate::state::View::Chat;
        }
    }

    ui.add_space(10.0);
    if !app.setup_pane.log.is_empty() {
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (msg, is_error) in &app.setup_pane.log {
                    let style = if *is_error {
                        egui::RichText::new(msg)
                            .small()
                            .color(egui::Color32::from_rgb(0xe5, 0x48, 0x4d))
                    } else {
                        egui::RichText::new(msg).small().weak()
                    };
                    ui.label(style);
                }
            });
    }
}

fn row(ui: &mut Ui, name: &str, path: &Option<std::path::PathBuf>, t: &crate::i18n::Str) {
    ui.label(name);
    match path {
        Some(p) => {
            ui.label(
                egui::RichText::new(format!("✅ {} — {}", t.setup_found, p.display()))
                    .color(egui::Color32::from_rgb(0x50, 0xc8, 0x78))
                    .monospace(),
            );
        }
        None => {
            ui.label(
                egui::RichText::new(format!("❌ {}", t.setup_missing))
                    .color(egui::Color32::from_rgb(0xe5, 0x48, 0x4d)),
            );
        }
    }
    ui.end_row();
}
