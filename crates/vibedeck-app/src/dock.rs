//! Right dock (the "workspace" panel, in the spirit of Claude Cowork):
//! a lazy project file tree and a Git tab with `git status` + per-file
//! diffs, refreshed after every completed turn — plus one closable
//! preview tab per opened file (markdown, tables, pdf, docx/pptx, images).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use eframe::egui::{self, Ui};

use crate::app::VibedeckApp;
use crate::i18n::Str;
use crate::state::View;

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", ".venv", "venv", "__pycache__", "dist", "build",
    "target", ".next", ".cache", ".mypy_cache", ".pytest_cache", "vendor", "Pods",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockTab {
    Files,
    Git,
    /// A preview tab for the file at this (absolute) path.
    Preview(String),
}

impl Default for DockTab {
    fn default() -> Self {
        DockTab::Files
    }
}

/// Why a preview is just a short note instead of content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoKind {
    Binary,
    TooLarge,
    Unsupported,
}

/// Parsed preview content, produced on a background thread.
#[derive(Clone, Debug)]
pub enum PreviewContent {
    /// Plain / source text (also: extracted docx / pptx / pdf text).
    Text(String),
    /// Markdown → rendered friendly preview.
    Markdown(String),
    /// csv / tsv / spreadsheet first sheet.
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        truncated: bool,
        /// sheet note (spreadsheet name / "N sheets: …").
        note: String,
    },
    /// Raw image bytes, decoded at render time.
    Image { name: String, bytes: Vec<u8> },
    /// A short "no preview" note.
    Info(InfoKind),
}

#[derive(Clone, Debug)]
pub enum PreviewState {
    Loading,
    Ready(PreviewContent),
    Failed(String),
}

/// One open preview tab.
#[derive(Clone, Debug)]
pub struct PreviewTab {
    pub path: String,
    pub state: PreviewState,
}

/// One `git status --porcelain` line.
#[derive(Clone, Debug)]
pub struct GitEntry {
    /// XY status codes (e.g. `M `, ` M`, `??`).
    pub xy: String,
    pub path: String,
}

impl GitEntry {
    fn label(&self) -> &'static str {
        match self.xy.as_str() {
            "??" => "new",
            _ if self.xy.contains('M') => "mod",
            _ if self.xy.contains('A') => "add",
            _ if self.xy.contains('D') => "del",
            _ if self.xy.contains('R') => "ren",
            _ => "?",
        }
    }
}

#[derive(Default)]
pub struct GitState {
    pub branch: String,
    pub entries: Vec<GitEntry>,
    pub diff_file: Option<String>,
    pub diff_text: String,
    status_rx: Option<Receiver<Result<(String, Vec<GitEntry>), String>>>,
    diff_rx: Option<Receiver<Result<String, String>>>,
}

impl GitState {
    pub fn working(&self) -> bool {
        self.status_rx.is_some() || self.diff_rx.is_some()
    }
}

#[derive(Default)]
pub struct Dock {
    pub open: bool,
    pub tab: DockTab,
    /// Expanded directory paths (absolute).
    pub expanded: HashSet<String>,
    /// Open preview tabs, oldest first.
    pub previews: Vec<PreviewTab>,
    pub git: GitState,
    preview_tx: Option<Sender<(String, Result<PreviewContent, String>)>>,
    preview_rx: Option<Receiver<(String, Result<PreviewContent, String>)>>,
}

/// Run `git` in `cwd` without flashing a console window on Windows.
fn git(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            if out.status.success() {
                Ok(text)
            } else {
                let err = String::from_utf8_lossy(&out.stderr).to_string();
                Err(err.trim().to_string())
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

fn parse_porcelain(text: &str) -> (String, Vec<GitEntry>) {
    let mut branch = String::new();
    let mut entries = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            // `## main...origin/main [ahead 1]`
            branch = rest
                .split("...")
                .next()
                .unwrap_or(rest)
                .split(' ')
                .next()
                .unwrap_or(rest)
                .to_string();
            continue;
        }
        if line.len() < 4 {
            continue;
        }
        entries.push(GitEntry {
            xy: line[..2].to_string(),
            path: line[3..].trim().to_string(),
        });
    }
    (branch, entries)
}

impl Dock {
    /// Request a background `git status` refresh.
    pub fn refresh_status(&mut self, cwd: &Path) {
        if self.git.status_rx.is_some() {
            return;
        }
        let cwd = cwd.to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        self.git.status_rx = Some(rx);
        std::thread::Builder::new()
            .name("vibedeck-git-status".into())
            .spawn(move || {
                let out = git(&cwd, &["status", "--porcelain=v1", "-b"]);
                let _ = tx.send(out.map(|text| parse_porcelain(&text)));
            })
            .expect("failed to spawn git status thread");
    }

    /// Request a background `git diff` for one file.
    pub fn refresh_diff(&mut self, cwd: &Path, file: &str) {
        self.git.diff_file = Some(file.to_string());
        self.git.diff_text.clear();
        let cwd = cwd.to_path_buf();
        let file = file.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        self.git.diff_rx = Some(rx);
        std::thread::Builder::new()
            .name("vibedeck-git-diff".into())
            .spawn(move || {
                let out = git(&cwd, &["diff", "--", &file])
                    .and_then(|_| git(&cwd, &["diff", "--cached", "--", &file]));
                let _ = tx.send(out);
            })
            .expect("failed to spawn git diff thread");
    }

    /// Open (or reveal) a preview tab for `path` and load it in the
    /// background.
    pub fn open_preview(&mut self, path: &str) {
        if self.preview_tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.preview_tx = Some(tx);
            self.preview_rx = Some(rx);
        }
        if !self.previews.iter().any(|t| t.path == path) {
            self.previews.push(PreviewTab {
                path: path.to_string(),
                state: PreviewState::Loading,
            });
        }
        if let Some(tx) = self.preview_tx.clone() {
            let path_owned = path.to_string();
            std::thread::Builder::new()
                .name("vibedeck-preview".into())
                .spawn(move || {
                    let result = crate::preview::load(Path::new(&path_owned));
                    let _ = tx.send((path_owned, result));
                })
                .expect("failed to spawn preview thread");
        }
        self.tab = DockTab::Preview(path.to_string());
    }

    /// Close the preview tab for `path` (and fall back to the Files tab
    /// when it was active).
    pub fn close_preview(&mut self, path: &str) {
        self.previews.retain(|t| t.path != path);
        if self.tab == DockTab::Preview(path.to_string()) {
            self.tab = DockTab::Files;
        }
    }

    /// Poll background preview loads; returns true when new data arrived.
    pub fn poll_preview(&mut self) -> bool {
        let Some(rx) = &self.preview_rx else {
            return false;
        };
        let mut fresh = false;
        loop {
            match rx.try_recv() {
                Ok((path, result)) => {
                    if let Some(tab) = self.previews.iter_mut().find(|t| t.path == path) {
                        tab.state = match result {
                            Ok(content) => PreviewState::Ready(content),
                            Err(e) => PreviewState::Failed(e),
                        };
                        fresh = true;
                    }
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        fresh
    }

    /// Poll background git results; returns true when new data arrived.
    pub fn poll_git(&mut self) -> bool {
        let mut fresh = false;
        if let Some(rx) = self.git.status_rx.take() {
            match rx.try_recv() {
                Ok(Ok((branch, entries))) => {
                    self.git.branch = branch;
                    self.git.entries = entries;
                    fresh = true;
                }
                Ok(Err(_)) => {
                    self.git.branch.clear();
                    self.git.entries.clear();
                }
                Err(TryRecvError::Empty) => {
                    self.git.status_rx = Some(rx);
                }
                Err(TryRecvError::Disconnected) => {}
            }
        }
        if let Some(rx) = self.git.diff_rx.take() {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    self.git.diff_text = text;
                    fresh = true;
                }
                Ok(Err(e)) => {
                    self.git.diff_text = format!("git: {e}");
                }
                Err(TryRecvError::Empty) => {
                    self.git.diff_rx = Some(rx);
                }
                Err(TryRecvError::Disconnected) => {}
            }
        }
        fresh
    }
}

/// The right dock panel — only in the Chat view.
pub fn panel(app: &mut VibedeckApp, ctx: &eframe::egui::Context) {
    if !app.dock.open || app.view != View::Chat {
        return;
    }
    let p = app.palette();
    let t = app.tr();
    let mut close = false;
    let mut close_preview: Option<String> = None;
    egui::SidePanel::right("dock")
        .resizable(true)
        .default_width(320.0)
        .width_range(240.0..=640.0)
        .frame(
            egui::Frame::default()
                .fill(p.surface)
                .inner_margin(egui::Margin::same(8.0))
                .stroke(egui::Stroke::new(1.0_f32, p.border_subtle)),
        )
        .show(ctx, |ui| {
            // -- header: tabs + close ------------------------------------
            ui.horizontal(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                egui::ScrollArea::horizontal()
                    .id_salt("dock-tabs")
                    .max_height(20.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let tab = app.dock.tab.clone();
                            if ui
                                .selectable_label(
                                    tab == DockTab::Files,
                                    egui::RichText::new(t.dock_files).small(),
                                )
                                .clicked()
                            {
                                app.dock.tab = DockTab::Files;
                            }
                            if ui
                                .selectable_label(
                                    tab == DockTab::Git,
                                    egui::RichText::new(t.dock_git).small(),
                                )
                                .clicked()
                            {
                                app.dock.tab = DockTab::Git;
                                let cwd = app.current_workdir();
                                app.dock.refresh_status(&cwd);
                            }
                            for preview in app.dock.previews.clone() {
                                let is_active =
                                    tab == DockTab::Preview(preview.path.clone());
                                ui.selectable_label(
                                    is_active,
                                    egui::RichText::new(short(&preview.path)).small(),
                                )
                                .on_hover_text(&preview.path);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("✕").small(),
                                        )
                                        .fill(egui::Color32::TRANSPARENT),
                                    )
                                    .on_hover_text(t.close)
                                    .clicked()
                                {
                                    close_preview = Some(preview.path.clone());
                                }
                            }
                        });
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(format!("⟳ {}", t.reload)).small(),
                            )
                            .fill(p.panel)
                            .rounding(egui::Rounding::same(4.0)),
                        )
                        .on_hover_text(t.reload)
                        .clicked()
                    {
                        let cwd = app.current_workdir();
                        app.dock.refresh_status(&cwd);
                    }
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new("✕").small())
                                .fill(egui::Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            let tab = app.dock.tab.clone();
            match tab {
                DockTab::Files => files_tree(app, ui, t),
                DockTab::Git => git_view(app, ui, t),
                DockTab::Preview(path) => preview_view(app, ui, t, &path),
            }
        });
    if let Some(path) = close_preview {
        app.dock.close_preview(&path);
    }
    if close {
        app.dock.open = false;
    }
}

// -- Files tab -----------------------------------------------------------------

fn files_tree(app: &mut VibedeckApp, ui: &mut Ui, t: &Str) {
    let p = app.palette();
    let root = app.current_workdir();

    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .id_salt("dock-files")
        .show(ui, |ui| {
            // Root always expanded.
            dir_children(ui, app, &p, &root, 0);
        });
    let _ = t;
}

/// Render the children of `dir` at `depth`, recursing into expanded dirs.
fn dir_children(ui: &mut Ui, app: &mut VibedeckApp, p: &crate::theme::Palette, dir: &Path, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<(String, bool)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            !(is_dir && SKIP_DIRS.contains(&name.as_str())) && !name.starts_with(".DS_Store")
        })
        .map(|e| {
            (
                e.file_name().to_string_lossy().to_string(),
                e.file_type().map(|t| t.is_dir()).unwrap_or(false),
            )
        })
        .collect();
    items.sort_by(|a, b| (b.1, a.0.clone()).cmp(&(a.1, b.0.clone())));

    for (name, is_dir) in items {
        let path = dir.join(&name);
        let path_str = path.to_string_lossy().to_string();
        let expanded = app.dock.expanded.contains(&path_str);
        let glyph = if is_dir {
            if expanded { "📂" } else { "📁" }
        } else {
            "📄"
        };
        let selected = app
            .dock
            .previews
            .iter()
            .any(|t| t.path == path_str);
        let label = egui::RichText::new(format!("{glyph} {name}"))
            .small()
            .color(if selected {
                p.session_active_text
            } else {
                p.text
            });
        if ui
            .add(
                egui::Button::new(label)
                    .fill(egui::Color32::TRANSPARENT)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .rounding(egui::Rounding::same(4.0)),
            )
            .on_hover_text(&path_str)
            .clicked()
        {
            if is_dir {
                if expanded {
                    app.dock.expanded.remove(&path_str);
                } else {
                    app.dock.expanded.insert(path_str.clone());
                }
            } else {
                // Open the file as a preview tab next to Files / Git.
                app.dock.open_preview(&path_str);
            }
        }
        if is_dir && expanded {
            ui.indent(format!("dir-{}", path_str), |ui| {
                dir_children(ui, app, p, &path, depth + 1);
            });
        }
    }
}

// -- Preview tabs ----------------------------------------------------------------

fn preview_view(app: &VibedeckApp, ui: &mut Ui, t: &Str, path: &str) {
    let p = app.palette();
    let Some(tab) = app.dock.previews.iter().find(|t| t.path == path) else {
        return;
    };
    ui.label(
        egui::RichText::new(&tab.path)
            .small()
            .color(p.muted),
    );
    ui.add_space(4.0);
    match &tab.state {
        PreviewState::Loading => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(egui::RichText::new(t.preview_loading).small().color(p.muted));
            });
        }
        PreviewState::Failed(err) => {
            ui.label(
                egui::RichText::new(format!("⚠ {}: {err}", t.preview_failed))
                    .small()
                    .color(p.red),
            );
        }
        PreviewState::Ready(content) => match content {
            PreviewContent::Text(text) => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .id_salt(format!("preview-scroll-{path}"))
                    .show(ui, |ui| {
                        ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                        ui.label(
                            egui::RichText::new(text.as_str())
                                .small()
                                .color(p.text),
                        );
                    });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("{} — {}", t.preview_lines, text.lines().count()))
                        .small()
                        .color(p.dim),
                );
            }
            PreviewContent::Markdown(md) => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .id_salt(format!("preview-md-{path}"))
                    .show(ui, |ui| {
                        crate::md::markdown(ui, md);
                    });
            }
            PreviewContent::Table {
                columns,
                rows,
                truncated,
                note,
            } => {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("{} × {}", rows.len(), columns.len()))
                            .small()
                            .color(p.dim),
                    );
                    if !note.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("· {note}"))
                                .small()
                                .color(p.dim),
                        );
                    }
                    if *truncated {
                        ui.label(
                            egui::RichText::new(format!("· {}", t.preview_truncated))
                                .small()
                                .color(p.dim),
                        );
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::both()
                    .auto_shrink([false, true])
                    .id_salt(format!("preview-table-{path}"))
                    .show(ui, |ui| {
                        egui::Grid::new(format!("preview-grid-{path}"))
                            .striped(true)
                            .min_col_width(60.0)
                            .show(ui, |ui| {
                                for col in columns {
                                    ui.label(
                                        egui::RichText::new(col.as_str())
                                            .small()
                                            .strong()
                                            .color(p.text),
                                    );
                                }
                                ui.end_row();
                                for row in rows.iter().take(300) {
                                    for cell in row {
                                        ui.label(
                                            egui::RichText::new(cell.as_str())
                                                .small()
                                                .color(p.text),
                                        );
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }
            PreviewContent::Image { name, bytes } => {
                let max_w = ui.available_width();
                let image =
                    egui::Image::from_bytes(format!("bytes://{name}"), bytes.clone())
                        .max_width(max_w);
                ui.add(image);
            }
            PreviewContent::Info(kind) => {
                let msg = match kind {
                    InfoKind::Binary => t.preview_binary,
                    InfoKind::TooLarge => t.preview_too_large,
                    InfoKind::Unsupported => t.preview_unsupported,
                };
                ui.label(egui::RichText::new(msg).small().color(p.muted));
            }
        },
    }
}

// -- Git tab -------------------------------------------------------------------

fn git_view(app: &mut VibedeckApp, ui: &mut Ui, t: &Str) {
    let p = app.palette();
    let git = &app.dock.git;

    if git.entries.is_empty() && git.branch.is_empty() {
        ui.label(
            egui::RichText::new(t.git_no_changes)
                .small()
                .color(p.muted),
        );
        return;
    }

    // Branch + counters.
    let dirty = git.entries.len();
    ui.label(
        egui::RichText::new(format!("⎇ {} · {} {}", git.branch, dirty, t.git_files))
            .small()
            .strong()
            .color(p.text),
    );
    ui.add_space(6.0);

    let entries = git.entries.clone();
    let selected = git.diff_file.clone();
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .id_salt("dock-git-files")
        .show(ui, |ui| {
            for entry in &entries {
                let is_sel = selected.as_deref() == Some(entry.path.as_str());
                let color = match entry.label() {
                    "new" => p.green,
                    "del" => p.red,
                    _ => p.yellow,
                };
                let label = egui::RichText::new(format!(
                    "{} {} [{}]",
                    match entry.label() {
                        "new" => "+",
                        "del" => "−",
                        _ => "~",
                    },
                    entry.path,
                    entry.label()
                ))
                .small()
                .color(if is_sel {
                    p.session_active_text
                } else {
                    color
                });
                if ui
                    .add(
                        egui::Button::new(label)
                            .fill(egui::Color32::TRANSPARENT)
                            .wrap_mode(egui::TextWrapMode::Truncate),
                    )
                    .on_hover_text(&entry.path)
                    .clicked()
                {
                    let cwd = app.current_workdir();
                    app.dock.refresh_diff(&cwd, &entry.path);
                }
            }
        });

    // Diff.
    if let Some(file) = &selected {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(format!("± {file}"))
                .small()
                .strong()
                .color(p.text),
        );
        ui.add_space(2.0);
        let diff = app.dock.git.diff_text.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(320.0)
            .id_salt("dock-git-diff")
            .show(ui, |ui| {
                if diff.is_empty() {
                    if app.dock.git.working() {
                        ui.label(egui::RichText::new("…").small().color(p.muted));
                    } else {
                        ui.label(egui::RichText::new(t.git_no_diff).small().color(p.muted));
                    }
                    return;
                }
                ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                for line in diff.lines().take(3000) {
                    let color = if line.starts_with('+') {
                        p.green
                    } else if line.starts_with('-') {
                        p.red
                    } else if line.starts_with("@@") {
                        p.muted
                    } else {
                        p.text
                    };
                    ui.label(
                        egui::RichText::new(line)
                            .small()
                            .color(color)
                            .weak(),
                    );
                }
            });
    }
}

fn short(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}
