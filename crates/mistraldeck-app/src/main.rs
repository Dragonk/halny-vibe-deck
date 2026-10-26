//! Mistral Deck — desktop GUI for the Mistral Vibe CLI.
//!
//! **Unofficial community project — not affiliated with, endorsed by, or
//! supported by Mistral AI.** "Mistral" and "Vibe" are trademarks of
//! their respective owners; this app is a thin desktop shell around the
//! open-source `vibe-acp` CLI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console on Windows release builds

mod about;
mod charts;
mod app;
mod chat;
mod dock;
mod frameless;
mod i18n;
mod input;
mod md;
mod mermaid;
mod picker;
mod preview;
mod sessions;
mod settings;
mod setup;
mod state;
mod stats;
mod theme;

use eframe::{NativeOptions, Renderer};
use egui::ViewportBuilder;

/// The original Mistral Vibe Nexus window/taskbar icon.
fn icon_data() -> Option<egui::IconData> {
    let bytes: &[u8] = include_bytes!("../assets/icon.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    Some(egui::IconData {
        rgba: buf,
        width: info.width,
        height: info.height,
    })
}

fn main() -> eframe::Result<()> {
    let options = NativeOptions {
        viewport: ViewportBuilder::default()
            .with_inner_size([1240.0, 820.0])
            .with_min_inner_size([860.0, 600.0])
            .with_title("Mistral Deck — GUI for Mistral Vibe CLI (unofficial)")
            // Frameless window like the Nexus original: the app header is
            // the titlebar (drag / ─ □ ✕, see app/frameless.rs).
            .with_decorations(false)
            .with_icon(icon_data().unwrap_or_default()),
        renderer: Renderer::Wgpu, // GPU-first rendering, like the ArtCraft apps
        ..Default::default()
    };
    eframe::run_native(
        "Mistral Deck",
        options,
        Box::new(|cc| Ok(Box::new(app::MistraldeckApp::new(cc)) as Box<dyn eframe::App>)),
    )
}
