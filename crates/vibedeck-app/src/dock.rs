//! Right dock (the "workspace" panel, in the spirit of Claude Cowork):
//! a lazy project file tree and a Git tab with `git status` + per-file
//! diffs, refreshed after every completed turn — plus one closable
//! preview tab per opened file (markdown, tables, pdf, docx/pptx, images).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use eframe::egui::{
    self, Color32, Pos2, Rect, Sense, Stroke, TextureHandle, Ui, Vec2,
};

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

// -- rich preview models (built on background threads) ----------------------------

/// One styled text span inside a docx block.
#[derive(Clone, Debug, Default)]
pub struct DocSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Explicit point size (w:sz).
    pub size: Option<f32>,
    /// Explicit color (w:color, hyperlinks).
    pub color: Option<Color32>,
}

/// The real document flow of a Word file.
#[derive(Clone, Debug)]
pub enum DocBlock {
    Heading { level: u8, spans: Vec<DocSpan> },
    Para { center: bool, right: bool, spans: Vec<DocSpan> },
    ListItem { level: u8, numbered: bool, spans: Vec<DocSpan> },
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
}

/// One PowerPoint slide: shapes with real positions, fills and text.
#[derive(Clone, Debug)]
pub struct Slide {
    /// Slide background (None → white).
    pub background: Option<Color32>,
    pub shapes: Vec<SlideShape>,
}

/// A shape on a slide (positions in EMU: 914400 per inch).
#[derive(Clone, Debug)]
pub struct SlideShape {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub fill: Option<Color32>,
    pub anchor_center: bool,
    pub paras: Vec<SlidePara>,
    /// Embedded picture (decoded).
    pub image: Option<egui::ColorImage>,
}

#[derive(Clone, Debug)]
pub struct SlidePara {
    /// 0 = left, 1 = center, 2 = right.
    pub align: u8,
    pub runs: Vec<SlideRun>,
}

#[derive(Clone, Debug)]
pub struct SlideRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub size_pt: f32,
    pub color: Option<Color32>,
}

/// One spreadsheet sheet (raw cells).
#[derive(Clone, Debug)]
pub struct SheetData {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
}

/// One rendered pdfium page.
#[derive(Clone, Debug)]
pub struct PdfPageImg {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Markdown preview split into text and mermaid-diagram segments.
#[derive(Clone, Debug)]
pub enum MdSegment {
    Text(String),
    Diagram {
        code: String,
        /// The rendered diagram (None → show the source as a fallback).
        png: Option<egui::ColorImage>,
    },
}

/// Parsed preview content, produced on a background thread.
#[derive(Clone, Debug)]
pub enum PreviewContent {
    /// Plain / source text (also: PDF text fallback).
    Text(String),
    /// Markdown with mermaid diagrams.
    MarkdownDoc { segments: Vec<MdSegment> },
    /// docx document flow.
    Document { blocks: Vec<DocBlock> },
    /// pptx slides (real rendering).
    Slides {
        width_emu: i64,
        height_emu: i64,
        slides: Vec<Slide>,
    },
    /// xlsx / ods: every sheet.
    Sheets { sheets: Vec<SheetData> },
    /// csv / tsv / spreadsheet first sheet.
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        truncated: bool,
        /// sheet note (spreadsheet name / "N sheets: …").
        note: String,
    },
    /// pdfium-rendered pages.
    Pdf {
        pages: Vec<PdfPageImg>,
        truncated: bool,
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
    /// GPU textures for binary previews (pdf pages, pptx pictures,
    /// mermaid diagrams), keyed "<path>#…".
    pub textures: HashMap<String, TextureHandle>,
    /// Active slide per preview path (pptx).
    pub slide_idx: HashMap<String, usize>,
    /// Active sheet per preview path (xlsx).
    pub sheet_idx: HashMap<String, usize>,
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
        let prefix = format!("{path}#");
        self.textures.retain(|k, _| !k.starts_with(&prefix));
        self.slide_idx.remove(path);
        self.sheet_idx.remove(path);
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

fn preview_view(app: &mut VibedeckApp, ui: &mut Ui, t: &Str, path: &str) {
    let p = app.palette();
    ui.label(
        egui::RichText::new(path)
            .small()
            .color(p.muted),
    );
    ui.add_space(4.0);

    // Split the dock state so content (shared) and the texture / index
    // caches (mutable) can be used at the same time.
    let Dock {
        previews,
        textures,
        slide_idx,
        sheet_idx,
        ..
    } = &mut app.dock;

    let Some(tab) = previews.iter().find(|t| t.path == path) else {
        return;
    };
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
            PreviewContent::MarkdownDoc { segments } => {
                render_markdown_doc(ui, &p, t, path, segments, textures);
            }
            PreviewContent::Document { blocks } => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .id_salt(format!("preview-doc-{path}"))
                    .show(ui, |ui| {
                        render_document(ui, &p, blocks);
                    });
            }
            PreviewContent::Slides {
                width_emu,
                height_emu,
                slides,
            } => {
                render_slides(
                    ui,
                    &p,
                    path,
                    *width_emu,
                    *height_emu,
                    slides,
                    slide_idx,
                    textures,
                );
            }
            PreviewContent::Sheets { sheets } => {
                render_sheets(ui, &p, t, path, sheets, sheet_idx);
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
                        render_grid(ui, &p, path, columns, rows, 300);
                    });
            }
            PreviewContent::Pdf { pages, truncated } => {
                render_pdf(ui, &p, t, path, pages, *truncated, textures);
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

// -- preview renderers ------------------------------------------------------------

/// Load (and cache) a GPU texture for a decoded image.
fn texture_for(
    ui: &mut Ui,
    textures: &mut HashMap<String, TextureHandle>,
    key: &str,
    image: egui::ColorImage,
) -> egui::load::SizedTexture {
    let ctx = ui.ctx().clone();
    egui::load::SizedTexture::from_handle(
        textures
            .entry(key.to_string())
            .or_insert_with(|| {
                ctx.load_texture(key, image, egui::TextureOptions::default())
            }),
    )
}

/// Markdown with mermaid diagrams.
fn render_markdown_doc(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    t: &Str,
    path: &str,
    segments: &[MdSegment],
    textures: &mut HashMap<String, TextureHandle>,
) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .id_salt(format!("preview-mdd-{path}"))
        .show(ui, |ui| {
            for (i, seg) in segments.iter().enumerate() {
                match seg {
                    MdSegment::Text(text) => crate::md::markdown(ui, text),
                    MdSegment::Diagram { code, png } => match png {
                        Some(img) => {
                            let key = format!("{path}#dia#{i}");
                            let tex = texture_for(ui, textures, &key, img.clone());
                            ui.add(
                                egui::Image::from_texture(tex)
                                    .max_width(ui.available_width() - 8.0)
                                    .rounding(4.0),
                            )
                            .on_hover_text(t.preview_diagram_service);
                            ui.add_space(6.0);
                        }
                        None => {
                            egui::Frame::group(ui.style())
                                .fill(ui.visuals().extreme_bg_color)
                                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                                .show(ui, |ui| {
                                    ui.style_mut().override_text_style =
                                        Some(egui::TextStyle::Monospace);
                                    ui.label(
                                        egui::RichText::new(code.as_str())
                                            .small()
                                            .color(p.text),
                                    );
                                });
                            ui.label(
                                egui::RichText::new(format!("⚠ {}", t.preview_diagram_failed))
                                    .small()
                                    .color(p.yellow),
                            );
                            ui.add_space(6.0);
                        }
                    },
                }
            }
        });
}

/// DocSpan list → a styled LayoutJob.
fn doc_spans_job(spans: &[DocSpan], base_color: Color32, base_size: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    for s in spans {
        let size = s.size.unwrap_or(if s.bold { base_size + 0.5 } else { base_size });
        let mut fmt = egui::text::TextFormat::simple(
            egui::FontId::proportional(size),
            s.color.unwrap_or(base_color),
        );
        fmt.italics = s.italic;
        if s.underline {
            fmt.underline = Stroke::new(1.0_f32, fmt.color);
        }
        if s.strike {
            fmt.strikethrough = Stroke::new(1.0_f32, fmt.color);
        }
        job.append(&s.text, 0.0, fmt);
    }
    job
}

/// The Word document flow.
fn render_document(ui: &mut Ui, p: &crate::theme::Palette, blocks: &[DocBlock]) {
    let text = p.text;
    let mut counters: Vec<u32> = vec![0; 8];
    for block in blocks {
        ui.add_space(2.0);
        match block {
            DocBlock::Heading { level, spans } => {
                let size = match level {
                    1 => 20.0,
                    2 => 17.0,
                    3 => 15.5,
                    4 => 14.5,
                    _ => 14.0,
                };
                let mut job = doc_spans_job(spans, text, size);
                job.wrap.max_width = ui.available_width() - 4.0;
                ui.add_space(4.0);
                ui.label(job);
                ui.add_space(1.0);
            }
            DocBlock::Para { center, right, spans } => {
                let mut job = doc_spans_job(spans, text, 13.5);
                job.wrap.max_width = ui.available_width() - 4.0;
                if *center {
                    job.halign = egui::Align::Center;
                } else if *right {
                    job.halign = egui::Align::RIGHT;
                }
                ui.label(job);
            }
            DocBlock::ListItem { level, numbered, spans } => {
                let lvl = (*level as usize).min(7);
                let marker = if *numbered {
                    counters[lvl] += 1;
                    for c in counters.iter_mut().skip(lvl + 1) {
                        *c = 0;
                    }
                    format!("{}.", counters[lvl])
                } else {
                    "•".to_string()
                };
                ui.horizontal(|ui| {
                    ui.add_space(6.0 + lvl as f32 * 14.0);
                    ui.label(
                        egui::RichText::new(marker)
                            .small()
                            .strong()
                            .color(p.muted),
                    );
                    let mut job = doc_spans_job(spans, text, 13.5);
                    job.wrap.max_width =
                        ui.available_width().min(ui.available_width() - 4.0);
                    ui.label(job);
                });
            }
            DocBlock::Table { header, rows } => {
                ui.add_space(2.0);
                egui::ScrollArea::horizontal()
                    .id_salt(format!("doc-tbl-{:?}", ui.id()))
                    .show(ui, |ui| {
                        render_grid(ui, p, "doc", header, rows, 100);
                    });
                ui.add_space(2.0);
            }
        }
    }
}

/// A striped egui::Grid table (csv/tsv and docx tables).
fn render_grid(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    id: &str,
    columns: &[String],
    rows: &[Vec<String>],
    max_rows: usize,
) {
    egui::Grid::new(format!("preview-grid-{id}"))
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
            for row in rows.iter().take(max_rows) {
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
}

/// Excel-style sheet tabs + painted grid (letters, row numbers, gridlines).
fn render_sheets(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    t: &Str,
    path: &str,
    sheets: &[SheetData],
    sheet_idx: &mut HashMap<String, usize>,
) {
    let active = sheet_idx.entry(path.to_string()).or_insert(0);
    if *active >= sheets.len() {
        *active = 0;
    }
    egui::ScrollArea::horizontal()
        .id_salt(format!("sheets-{path}"))
        .max_height(22.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, s) in sheets.iter().enumerate() {
                    if ui
                        .selectable_label(
                            i == *active,
                            egui::RichText::new(s.name.as_str()).small(),
                        )
                        .clicked()
                    {
                        *active = i;
                    }
                    ui.separator();
                }
            });
        });
    ui.add_space(4.0);
    let sheet = &sheets[*active];
    let ncols = sheet
        .rows
        .iter()
        .map(|r| r.len())
        .max()
        .unwrap_or(0)
        .min(64);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{}: {} × {}",
                t.preview_sheets,
                sheet.rows.len().saturating_sub(1),
                ncols
            ))
            .small()
            .color(p.dim),
        );
        if sheet.truncated {
            ui.label(
                egui::RichText::new(format!("· {}", t.preview_truncated))
                    .small()
                    .color(p.dim),
            );
        }
    });
    ui.add_space(4.0);
    if sheet.rows.is_empty() || ncols == 0 {
        ui.label(egui::RichText::new("—").small().color(p.muted));
        return;
    }
    render_sheet_grid(ui, p, path, sheet);
}

/// Excel column letters: 0 → A, 25 → Z, 26 → AA…
fn column_letter(mut idx: usize) -> String {
    let mut out = String::new();
    loop {
        out.insert(0, (b'A' + (idx % 26) as u8) as char);
        idx = idx / 26;
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    out
}

/// Paint an Excel-like grid: header row with letters, number column,
/// 1px gridlines, numbers right-aligned. Only visible cells are painted.
fn render_sheet_grid(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    path: &str,
    sheet: &SheetData,
) {
    const ROW_H: f32 = 20.0;
    const HEAD_H: f32 = 20.0;
    const NUM_W: f32 = 38.0;

    let rows = &sheet.rows;
    let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0).min(64);
    if ncols == 0 {
        return;
    }
    // Column widths from content, clamped.
    let mut widths = Vec::with_capacity(ncols);
    for c in 0..ncols {
        let maxw = rows
            .iter()
            .take(200)
            .map(|r| r.get(c).map(|s| s.chars().count()).unwrap_or(0))
            .max()
            .unwrap_or(0);
        widths.push((maxw as f32 * 6.6 + 14.0).clamp(48.0, 190.0));
    }
    let mut offsets = Vec::with_capacity(ncols + 1);
    let mut acc = NUM_W;
    offsets.push(acc);
    for w in &widths {
        acc += w;
        offsets.push(acc);
    }
    let total_w = acc;
    let total_h = HEAD_H + rows.len() as f32 * ROW_H;

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .id_salt(format!("sheet-grid-{path}"))
        .show(ui, |ui| {
            let (rect, _resp) =
                ui.allocate_exact_size(Vec2::new(total_w, total_h), Sense::hover());
            let painter = ui.painter_at(rect);
            let line = Stroke::new(1.0_f32, p.border_subtle);

            // Header row + number column.
            painter.rect_filled(
                Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.min.y + HEAD_H)),
                0.0,
                p.panel,
            );
            painter.rect_filled(
                Rect::from_min_max(rect.min, Pos2::new(rect.min.x + NUM_W, rect.bottom())),
                0.0,
                p.panel,
            );
            let font = egui::FontId::proportional(11.0);
            for c in 0..ncols {
                let cx = (offsets[c] + offsets[c + 1]) / 2.0 + rect.min.x;
                painter.text(
                    Pos2::new(cx, rect.min.y + HEAD_H / 2.0),
                    egui::Align2::CENTER_CENTER,
                    column_letter(c),
                    font.clone(),
                    p.muted,
                );
            }
            // Vertical gridlines.
            for c in 0..=ncols {
                let x = rect.min.x + offsets[c.min(ncols)];
                painter.line_segment(
                    [Pos2::new(x, rect.min.y), Pos2::new(x, rect.bottom())],
                    line,
                );
            }
            // Visible rows only.
            let clip = ui.clip_rect();
            let first = (((clip.min.y - rect.min.y - HEAD_H) / ROW_H).max(0.0) as usize)
                .min(rows.len());
            let last = ((((clip.max.y - rect.min.y - HEAD_H) / ROW_H).ceil() as usize) + 1)
                .min(rows.len())
                .max(first);
            for r in first..last {
                let y = rect.min.y + HEAD_H + r as f32 * ROW_H;
                painter.line_segment(
                    [Pos2::new(rect.min.x, y), Pos2::new(rect.right(), y)],
                    line,
                );
                painter.text(
                    Pos2::new(rect.min.x + NUM_W / 2.0, y + ROW_H / 2.0),
                    egui::Align2::CENTER_CENTER,
                    (r + 1).to_string(),
                    font.clone(),
                    p.muted,
                );
                let row = &rows[r];
                for c in 0..ncols {
                    let cell = row.get(c).map(|s| s.as_str()).unwrap_or("");
                    if cell.is_empty() {
                        continue;
                    }
                    let x0 = rect.min.x + offsets[c];
                    let x1 = rect.min.x + offsets[c + 1];
                    // Fit the text into the cell (single line, ellipsized).
                    let max_chars = (((x1 - x0 - 10.0) / 6.6) as usize).max(2);
                    let shown: String = if cell.chars().count() > max_chars {
                        let mut s: String = cell.chars().take(max_chars.saturating_sub(1)).collect();
                        s.push('…');
                        s
                    } else {
                        cell.to_string()
                    };
                    let color = p.text;
                    let is_num = cell.parse::<f64>().is_ok();
                    let pos = if is_num {
                        Pos2::new(x1 - 5.0, y + ROW_H / 2.0)
                    } else {
                        Pos2::new(x0 + 5.0, y + ROW_H / 2.0)
                    };
                    painter.text(
                        pos,
                        if is_num {
                            egui::Align2::RIGHT_CENTER
                        } else {
                            egui::Align2::LEFT_CENTER
                        },
                        shown,
                        font.clone(),
                        color,
                    );
                }
            }
        });
}

/// pptx: slide navigation + real slide rendering (shapes, fills, text,
/// embedded pictures).
#[allow(clippy::too_many_arguments)]
fn render_slides(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    path: &str,
    width_emu: i64,
    height_emu: i64,
    slides: &[Slide],
    slide_idx: &mut HashMap<String, usize>,
    textures: &mut HashMap<String, TextureHandle>,
) {
    let idx = slide_idx.entry(path.to_string()).or_insert(0);
    if *idx >= slides.len() {
        *idx = 0;
    }
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new("◀").small().fill(p.panel),
            )
            .clicked()
            && *idx > 0
        {
            *idx -= 1;
        }
        ui.label(
            egui::RichText::new(format!("{} / {}", *idx + 1, slides.len()))
                .small()
                .color(p.dim),
        );
        if ui
            .add(
                egui::Button::new("▶").small().fill(p.panel),
            )
            .clicked()
            && *idx + 1 < slides.len()
        {
            *idx += 1;
        }
    });
    ui.add_space(4.0);

    let slide = &slides[*idx];
    let aspect = height_emu as f32 / width_emu.max(1) as f32;
    let avail_w = ui.available_width();
    let h = (avail_w * aspect).clamp(140.0, 380.0);
    let w = h / aspect;

    let (rect, _resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let painter = ui.painter_at(rect);
    let bg = slide.background.unwrap_or(Color32::WHITE);
    painter.rect_filled(rect, 2.0, bg);

    // px per EMU on the canvas.
    let scale = w / width_emu.max(1) as f32;
    // pixels per point of text at this scale (1pt = 1/72 inch, 914400 EMU/inch).
    let px_per_pt = scale * 914400.0 / 72.0;

    let mut image_shapes: Vec<(Rect, &egui::ColorImage, usize)> = Vec::new();
    for (j, shape) in slide.shapes.iter().enumerate() {
        let sx = rect.min.x + shape.x as f32 * scale;
        let sy = rect.min.y + shape.y as f32 * scale;
        let sw = shape.w as f32 * scale;
        let sh = shape.h as f32 * scale;
        let sr = Rect::from_min_max(Pos2::new(sx, sy), Pos2::new(sx + sw, sy + sh));
        if let Some(fill) = shape.fill {
            painter.rect_filled(sr, 1.0, fill);
        }
        if let Some(img) = &shape.image {
            image_shapes.push((sr, img, j));
        }
        if shape.paras.is_empty() {
            continue;
        }
        // Layout all paragraphs (wrapped to the shape box).
        let mut galleys = Vec::with_capacity(shape.paras.len());
        let mut total_h = 0.0;
        for para in &shape.paras {
            let mut job = egui::text::LayoutJob::default();
            job.wrap.max_width = (sw - 4.0).max(8.0);
            let has_text = para.runs.iter().any(|r| !r.text.is_empty());
            if !has_text {
                // empty paragraph = spacer row
                let empty_h = 18.0 * px_per_pt;
                galleys.push((None, empty_h));
                total_h += empty_h;
                continue;
            }
            for run in &para.runs {
                let size_px = (run.size_pt * px_per_pt).clamp(6.0, 64.0);
                let mut fmt = egui::text::TextFormat::simple(
                    egui::FontId::proportional(size_px),
                    run.color.unwrap_or(Color32::BLACK),
                );
                fmt.italics = run.italic;
                job.append(&run.text, 0.0, fmt);
            }
            let galley = painter.layout_job(job);
            let gh = galley.rect.height() + 2.0 * px_per_pt;
            galleys.push((Some(galley), gh));
            total_h += gh;
        }
        let mut ty = if shape.anchor_center && total_h < sh {
            sr.center().y - total_h / 2.0
        } else {
            sr.min.y + 2.0
        };
        for (i, para) in shape.paras.iter().enumerate() {
            let (galley, gh) = &galleys[i];
            let gh = *gh;
            let Some(galley) = galley else {
                ty += gh;
                continue;
            };
            let galley_w = galley.rect.width();
            let tx = match para.align {
                1 => sr.min.x + (sw - galley_w).max(0.0) / 2.0,
                2 => sr.max.x - 2.0 - galley_w,
                _ => sr.min.x + 2.0,
            };
            painter.galley(Pos2::new(tx, ty), galley.clone(), Color32::WHITE);
            ty += gh;
        }
    }
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, p.border_subtle));

    // Embedded pictures on top (as positioned Image widgets).
    for (i, (sr, img, j)) in image_shapes.into_iter().enumerate() {
        let key = format!("{path}#s{}#i{}", idx, j);
        let _ = i;
        let tex = texture_for(ui, textures, &key, img.clone());
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(sr), |ui| {
            ui.add(egui::Image::from_texture(tex).fit_to_exact_size(sr.size()));
        });
    }
}

/// pdfium-rendered pages, scrollable, one page per texture.
fn render_pdf(
    ui: &mut Ui,
    p: &crate::theme::Palette,
    _t: &Str,
    path: &str,
    pages: &[PdfPageImg],
    truncated: bool,
    textures: &mut HashMap<String, TextureHandle>,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} pages", pages.len()))
                .small()
                .color(p.dim),
        );
        if truncated {
            ui.label(
                egui::RichText::new("· first pages only").small().color(p.dim),
            );
        }
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .id_salt(format!("preview-pdf-{path}"))
        .show(ui, |ui| {
            for (i, page) in pages.iter().enumerate() {
                let key = format!("{path}#pdf#{i}");
                let img = egui::ColorImage::from_rgba_unmultiplied(
                    [page.width as usize, page.height as usize],
                    &page.rgba,
                );
                let tex = texture_for(ui, textures, &key, img);
                let max_w = ui.available_width() - 4.0;
                egui::Frame::default()
                    .stroke(Stroke::new(1.0_f32, p.border_subtle))
                    .rounding(2.0)
                    .inner_margin(egui::Margin::same(0.0))
                    .show(ui, |ui| {
                        ui.set_min_width(max_w);
                        ui.add(egui::Image::from_texture(tex).max_width(max_w));
                    });
                ui.label(
                    egui::RichText::new(format!("— {} —", i + 1))
                        .small()
                        .color(p.dim),
                );
                ui.add_space(6.0);
            }
        });
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
