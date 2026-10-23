//! VibeDeck — desktop GUI for the Mistral Vibe CLI.
//!
//! **Unofficial community project — not affiliated with, endorsed by, or
//! supported by Mistral AI.** "Mistral" and "Vibe" are trademarks of
//! their respective owners; this app is a thin desktop shell around the
//! open-source `vibe-acp` CLI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console on Windows release builds

mod about;
mod app;
mod chat;
mod i18n;
mod input;
mod md;
mod picker;
mod sessions;
mod settings;
mod setup;
mod state;
mod stats;

use eframe::{NativeOptions, Renderer};
use egui::ViewportBuilder;

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([860.0, 600.0])
            .with_title("VibeDeck — GUI for Mistral Vibe CLI (unofficial)"),
        renderer: Renderer::Wgpu, // GPU-first rendering, like the ArtCraft apps
        ..Default::default()
    };
    eframe::run_native(
        "VibeDeck",
        options,
        Box::new(|cc| Ok(Box::new(app::VibedeckApp::new(cc)) as Box<dyn eframe::App>)),
    )
}
