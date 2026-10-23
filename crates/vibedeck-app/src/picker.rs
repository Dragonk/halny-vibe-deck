//! File indexing for `@` mentions and a built-in folder picker
//! (no native dialog dependencies — pure egui, works everywhere).

use std::path::Path;

use eframe::egui::{self, Ui};

use crate::app::VibedeckApp;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    "target",
    ".next",
    ".cache",
    ".mypy_cache",
    ".pytest_cache",
    "vendor",
    "Pods",
];

/// Walk `root` (max depth 4) and collect relative paths, capped at 4000
/// entries. Directories that are heavy build/git internals are skipped.
pub fn index_files(root: &Path) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    walk(root, "", 0, &mut out);
    out
}

fn walk(dir: &Path, rel_base: &str, depth: usize, out: &mut Vec<(String, bool)>) {
    if depth > 4 || out.len() >= 4000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let rel = if rel_base.is_empty() {
            name.clone()
        } else {
            format!("{rel_base}/{name}")
        };
        out.push((rel.clone(), is_dir));
        if is_dir {
            walk(&entry.path(), &rel, depth + 1, out);
        }
        if out.len() >= 4000 {
            return;
        }
    }
}

/// Substring search over the index, capped, directories first.
pub fn search_files(index: &[(String, bool)], query: &str, limit: usize) -> Vec<(String, bool)> {
    let q = query.to_lowercase();
    let mut dirs: Vec<_> = index
        .iter()
        .filter(|(p, d)| *d && (q.is_empty() || p.to_lowercase().contains(&q)))
        .cloned()
        .collect();
    let mut files: Vec<_> = index
        .iter()
        .filter(|(p, d)| !*d && (q.is_empty() || p.to_lowercase().contains(&q)))
        .cloned()
        .collect();
    let mut hits: Vec<(String, bool)> = Vec::with_capacity(limit);
    hits.append(&mut dirs);
    hits.append(&mut files);
    hits.truncate(limit);
    hits
}

/// In-app modal folder picker: breadcrumb navigation + list.
/// The scroll area is height-bounded and the confirm/cancel row sits
/// *below* it, so the buttons are always reachable regardless of how
/// many entries the current directory has.
pub fn show_folder_picker(app: &mut VibedeckApp, ui: &mut Ui) {
    let t = app.tr();
    let p = app.palette();

    ui.horizontal(|ui| {
        if ui.button(format!("⬆ {}", t.parent_dir)).clicked() {
            if let Some(parent) = app.folder_picker.dir.parent().map(Path::to_path_buf) {
                app.folder_picker.dir = parent;
            }
        }
        if ui.button(t.home).clicked() {
            if let Some(home) = dirs::home_dir() {
                app.folder_picker.dir = home;
            }
        }
        if ui
            .button(format!("＋ {}", t.pick_folder))
            .on_hover_text(app.folder_picker.dir.to_string_lossy().to_string())
            .clicked()
        {
            let dir = app.folder_picker.dir.to_string_lossy().to_string();
            app.settings.working_directory = dir.clone();
            app.settings.remember_folder(&dir);
            app.settings.save();
            app.mark_file_index_stale();
            app.reset_dock();
            app.folder_picker.open = false;
            app.new_local_session();
        }
    });
    ui.label(
        egui::RichText::new(app.folder_picker.dir.to_string_lossy().to_string())
            .monospace()
            .small()
            .color(p.muted),
    );
    ui.separator();

    let mut entries: Vec<_> = std::fs::read_dir(&app.folder_picker.dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|e| e.to_lowercase());

    egui::ScrollArea::vertical()
        .max_height(280.0)
        .auto_shrink([false, false])
        .id_salt("folder-picker-list")
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            for name in &entries {
                if ui
                    .selectable_label(false, egui::RichText::new(format!("📁 {name}")).small())
                    .clicked()
                {
                    let mut next = app.folder_picker.dir.clone();
                    next.push(name);
                    app.folder_picker.dir = next;
                }
            }
            if entries.is_empty() {
                ui.label(egui::RichText::new("…").weak());
            }
        });

    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new(format!("✔ {}", t.pick_folder))
                        .strong()
                        .color(egui::Color32::WHITE),
                )
                .fill(p.accent),
            )
            .clicked()
        {
            let dir = app.folder_picker.dir.to_string_lossy().to_string();
            app.settings.working_directory = dir.clone();
            app.settings.remember_folder(&dir);
            app.settings.save();
            app.mark_file_index_stale();
            app.reset_dock();
            app.folder_picker.open = false;
            app.new_local_session();
        }
        if ui.button(t.cancel).clicked() {
            app.folder_picker.open = false;
        }
    });
}
