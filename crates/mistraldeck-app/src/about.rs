//! The About window: version, the unofficial-app disclaimer and credits.

use eframe::egui::{self, Ui};

use crate::app::MistraldeckApp;

pub fn show(app: &mut MistraldeckApp, ui: &mut Ui) {
    let t = app.tr();

    ui.heading(egui::RichText::new(t.about).strong());
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!(
            "{} v{} — Rust · egui · wgpu",
            t.app_name,
            env!("CARGO_PKG_VERSION")
        ))
        .monospace()
        .weak(),
    );
    ui.add_space(8.0);
    ui.label(t.disclaimer_long);
    ui.add_space(8.0);
    if let Some(v) = app.protocol_version {
        ui.label(
            egui::RichText::new(format!("ACP protocol: {v}"))
                .small()
                .weak(),
        );
    }
    ui.add_space(10.0);
    ui.label(egui::RichText::new(t.vibe_dir_open).small().weak());
    ui.hyperlink("https://github.com/Dragonk/mistral-vibe-nexus");
    ui.label(
        egui::RichText::new(
            "Dragonk/mistral-vibe-nexus (Apache-2.0) — Rust rewrite, unofficial community project",
        )
        .small()
        .weak(),
    );
    ui.add_space(10.0);
    if ui.button(t.close).clicked() {
        app.show_about = false;
    }
}
