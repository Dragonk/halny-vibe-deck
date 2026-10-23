//! Right dock (the "workspace" panel, in the spirit of Claude Cowork):
//! a lazy project file tree with a text preview and a Git tab with
//! `git status` + per-file diffs, refreshed after every completed turn.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Ui};

use crate::app::VibedeckApp;
use crate::i18n::Str;
use crate::state::View;

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", ".venv", "venv", "__pycache__", "dist", "build",
    "target", ".next", ".cache", ".mypy_cache", ".pytest_cache", "vendor", "Pods",
];

const PREVIEW_MAX_BYTES: u64 = 512 * 1024;
const PREVIEW_MAX_LINES: usize = 4000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DockTab {
    #[default]
    Files,
    Git,
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
    /// Absolute path of the previewed file + its content.
    pub preview: Option<(String, String)>,
    pub git: GitState,
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
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.git.status_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
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
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.git.diff_rx = Some(rx);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
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
                let tab = app.dock.tab;
                if ui
                    .selectable_label(tab == DockTab::Files, egui::RichText::new(t.dock_files).small())
                    .clicked()
                {
                    app.dock.tab = DockTab::Files;
                }
                if ui
                    .selectable_label(tab == DockTab::Git, egui::RichText::new(t.dock_git).small())
                    .clicked()
                {
                    app.dock.tab = DockTab::Git;
                    let cwd = app.current_workdir();
                    app.dock.refresh_status(&cwd);
                }
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

            match app.dock.tab {
                DockTab::Files => files_tree(app, ui, t),
                DockTab::Git => git_view(app, ui, t),
            }
        });
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

    // Preview under the tree.
    if let Some((path, content)) = app.dock.preview.clone() {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("📄 {}", short(&path)))
                .small()
                .strong()
                .color(p.text),
        )
        .default_open(true)
        .id_salt(format!("preview-{}", path))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .auto_shrink([false, true])
                .id_salt(format!("preview-scroll-{}", path))
                .show(ui, |ui| {
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                    ui.label(egui::RichText::new(content.as_str()).small().color(p.text));
                });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("{} — {}", t.preview_lines, content.lines().count()))
                    .small()
                    .color(p.dim),
            );
        });
    }
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
            .preview
            .as_ref()
            .map(|(sel, _)| sel == &path_str)
            .unwrap_or(false);
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
                app.dock.preview = read_preview(&path);
            }
        }
        if is_dir && expanded {
            ui.indent(format!("dir-{}", path_str), |ui| {
                dir_children(ui, app, p, &path, depth + 1);
            });
        }
    }
}

fn read_preview(path: &Path) -> Option<(String, String)> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > PREVIEW_MAX_BYTES {
        return Some((path.to_string_lossy().to_string(), "(file too large)".into()));
    }
    let raw = std::fs::read(path).ok()?;
    // Skip binary-looking files.
    if raw.iter().take(2048).any(|b| *b == 0) {
        return Some((path.to_string_lossy().to_string(), "(binary file)".into()));
    }
    let text = String::from_utf8_lossy(&raw).to_string();
    let mut lines = text.lines();
    let mut out = String::new();
    for (i, line) in lines.by_ref().enumerate() {
        if i >= PREVIEW_MAX_LINES {
            out.push_str("… (truncated)");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    Some((path.to_string_lossy().to_string(), out))
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
