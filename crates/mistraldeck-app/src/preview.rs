//! Real, high-fidelity file preview loaders for the workspace dock.
//!
//! Everything runs on a background thread and produces a [`PreviewContent`]
//! the dock renders:
//!
//! - Markdown → rendered preview; ```mermaid``` blocks are drawn by the
//!   built-in offline diagram renderer (mermaid.rs; unsupported types fall
//!   back to the source code)
//! - CSV / TSV → table
//! - Excel (`.xlsx` / `.xls` / `.xlsm` / `.ods`) → every sheet, Excel-style
//!   grid with sheet tabs (first 500×64 cells per sheet)
//! - Word (`.docx`) → the real document flow: headings, bold/italic/
//!   underline/strike runs, bullet & numbered lists, tables, hyperlinks
//! - PowerPoint (`.pptx`) → real slide rendering: shape geometry, fills,
//!   positioned + styled text, embedded pictures
//! - PDF → real page rendering via pdfium (bundled next to the
//!   executable); falls back to extracted text without pdfium
//! - Images (png / jpg / gif / bmp / webp / tiff / ico) → inline image
//! - Text-ish formats (source code, config, logs…) → plain text
//! - Anything else → a "no preview" note (binary / unsupported)

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use eframe::egui::Color32;

use crate::dock::{
    DocBlock, DocSpan, MdSegment, PdfPageImg, PreviewContent, SheetData, Slide, SlidePara,
    SlideRun, SlideShape,
};

/// Text previews: read at most this much.
const TEXT_MAX_BYTES: u64 = 512 * 1024;
const TEXT_MAX_LINES: usize = 4000;
/// Spreadsheets / docs / pdf: read at most this much.
const DOC_MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Images: read at most this much.
const IMAGE_MAX_BYTES: u64 = 25 * 1024 * 1024;
/// Table previews cap.
const TABLE_MAX_ROWS: usize = 500;
const TABLE_MAX_COLS: usize = 64;
/// docx document flow cap.
const DOC_MAX_BLOCKS: usize = 1200;
/// pptx caps.
const SLIDE_MAX_SHAPES: usize = 80;
const SLIDE_MAX_PARAS: usize = 30;
const SLIDE_MAX_RUNS: usize = 24;
const SLIDE_MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
/// xlsx caps.
const SHEET_MAX: usize = 16;
/// pdf caps.
const PDF_MAX_PAGES: usize = 12;
const PDF_PAGE_WIDTH: i32 = 900;
/// mermaid cap: at most this many diagrams are split out of one document.
const MERMAID_MAX_DIAGRAMS: usize = 16;

const TEXT_EXTS: &[&str] = &[
    "txt", "log", "json", "yaml", "yml", "toml", "ini", "cfg", "conf", "properties", "env",
    "xml", "html", "htm", "css", "scss", "js", "mjs", "cjs", "ts", "tsx", "jsx", "vue", "svelte",
    "rs", "py", "rb", "go", "java", "kt", "swift", "c", "h", "cpp", "hpp", "cc", "cs", "php",
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "sql", "lua", "pl", "r", "m", "gradle",
    "svg", "lock", "gitignore", "editorconfig", "service", "desktop", "license", "readme",
    "makefile", "dockerfile", "procfile", "nix", "vim", "emacs", "tf", "proto", "graphql",
    "ipynb", "diff", "patch", "srt", "vtt", "ass", "lrc",
];

const BINARY_EXTS: &[&str] = &[
    "exe", "dll", "so", "dylib", "bin", "o", "a", "lib", "class", "jar", "pyc", "pyo",
    "zip", "tar", "gz", "bz2", "xz", "7z", "rar", "zst", "iso", "dmg", "msi", "deb", "rpm",
    "appimage", "woff", "woff2", "ttf", "otf", "mp3", "wav", "flac", "ogg", "mp4", "mkv",
    "avi", "mov", "webm", "sqlite", "db", "pyd", "node",
];

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico"];

/// Load a preview of `path` (called on a background thread).
pub fn load(path: &Path) -> Result<PreviewContent, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    match ext.as_str() {
        "md" | "markdown" => text_or_info(path, markdown_doc),
        "csv" => text_or_info(path, |t| delimited_table(&t, ',').into_content()),
        "tsv" => text_or_info(path, |t| delimited_table(&t, '\t').into_content()),
        "xlsx" | "xlsm" | "xls" | "ods" => spreadsheet(path, size),
        "docx" => docx(path, size),
        "pptx" => pptx(path, size),
        "pdf" => pdf(path, size),
        _ if IMAGE_EXTS.contains(&ext.as_str()) => {
            if size > IMAGE_MAX_BYTES {
                Ok(PreviewContent::Info(crate::dock::InfoKind::TooLarge))
            } else {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                Ok(PreviewContent::Image { name, bytes })
            }
        }
        _ if BINARY_EXTS.contains(&ext.as_str()) => {
            Ok(PreviewContent::Info(crate::dock::InfoKind::Binary))
        }
        _ if TEXT_EXTS.contains(&ext.as_str()) => text_or_info(path, |t| {
            PreviewContent::Text(cap_lines(&t, TEXT_MAX_LINES))
        }),
        _ => {
            // Unknown or no extension (Makefile, LICENSE…):
            // sniff — try UTF-8, fall back to "binary".
            match read_text(path) {
                Some(t) => Ok(PreviewContent::Text(cap_lines(&t, TEXT_MAX_LINES))),
                None => Ok(PreviewContent::Info(crate::dock::InfoKind::Binary)),
            }
        }
    }
}

fn read_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > TEXT_MAX_BYTES {
        return None;
    }
    let raw = std::fs::read(path).ok()?;
    if raw.iter().take(4096).any(|b| *b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&raw).to_string())
}

/// Read a text preview of `path`, or an `Info` note when unreadable
/// (too large / binary).
fn text_or_info(
    path: &Path,
    f: impl FnOnce(String) -> PreviewContent,
) -> Result<PreviewContent, String> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > TEXT_MAX_BYTES {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::TooLarge));
    }
    match read_text(path) {
        Some(t) => Ok(f(t)),
        None => Ok(PreviewContent::Info(crate::dock::InfoKind::Binary)),
    }
}

fn cap_lines(text: &str, max_lines: usize) -> String {
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        if i >= max_lines {
            out.push_str("…");
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// -- markdown + mermaid -----------------------------------------------------------

/// Split markdown into text and mermaid-diagram segments. Diagrams keep
/// their source — the actual rendering runs offline in the UI layer.
fn markdown_doc(text: String) -> PreviewContent {
    let mut segments: Vec<MdSegment> = Vec::new();
    let mut cur = String::new();
    let mut code = String::new();
    let mut in_mermaid = false;
    let mut diagrams = 0usize;
    for line in text.lines() {
        let t = line.trim_start();
        if !in_mermaid {
            if t.starts_with("```") && t[3..].trim_start().eq_ignore_ascii_case("mermaid") {
                if !cur.trim_end().is_empty() {
                    segments.push(MdSegment::Text(std::mem::take(&mut cur)));
                }
                in_mermaid = true;
                code.clear();
                continue;
            }
            cur.push_str(line);
            cur.push('\n');
        } else if t.starts_with("```") {
            in_mermaid = false;
            diagrams += 1;
            if diagrams <= MERMAID_MAX_DIAGRAMS {
                segments.push(MdSegment::Diagram {
                    code: code.trim().to_string(),
                });
            } else {
                // cap reached — keep the source as plain text
                cur.push_str("```mermaid\n");
                cur.push_str(&code);
                cur.push_str("\n```\n");
            }
        } else {
            code.push_str(line);
            code.push('\n');
        }
    }
    if in_mermaid {
        // Unterminated fence — show what we have as text.
        cur.push_str(&code);
    }
    if !cur.trim_end().is_empty() {
        segments.push(MdSegment::Text(cur));
    }
    PreviewContent::MarkdownDoc { segments }
}

// -- csv / tsv tables -------------------------------------------------------------

/// A parsed delimited table (CSV/TSV).
struct Table {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
    truncated: bool,
    note: String,
}

impl Table {
    fn into_content(self) -> PreviewContent {
        PreviewContent::Table {
            columns: self.columns,
            rows: self.rows,
            truncated: self.truncated,
            note: self.note,
        }
    }
}

/// Minimal CSV parser: quotes, doubled quotes, newlines inside quotes.
fn delimited_table(text: &str, delim: char) -> Table {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            d if d == delim && !in_quotes => {
                row.push(std::mem::take(&mut cell));
            }
            '\n' if !in_quotes => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            '\r' => {}
            other => cell.push(other),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }

    let truncated = rows.len() > TABLE_MAX_ROWS;
    let mut it = rows.into_iter();
    let columns = it
        .next()
        .unwrap_or_default()
        .into_iter()
        .take(TABLE_MAX_COLS)
        .collect::<Vec<_>>();
    let body: Vec<Vec<String>> = it
        .take(TABLE_MAX_ROWS)
        .map(|r| {
            let mut r: Vec<String> = r.into_iter().take(TABLE_MAX_COLS).collect();
            while r.len() < columns.len() {
                r.push(String::new());
            }
            r
        })
        .collect();
    Table {
        columns,
        rows: body,
        truncated,
        note: String::new(),
    }
}

// -- spreadsheets (calamine) --------------------------------------------------------

/// Every sheet of a workbook, as raw cell grids (the dock renders them
/// Excel-style with sheet tabs).
fn spreadsheet(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::TooLarge));
    }
    use calamine::Reader;
    let mut workbook =
        calamine::open_workbook_auto(path).map_err(|e| format!("workbook: {e}"))?;
    let names = workbook.sheet_names();
    let mut sheets: Vec<SheetData> = Vec::new();
    for name in names.iter().take(SHEET_MAX) {
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut truncated = false;
        if let Ok(range) = workbook.worksheet_range(name) {
            for (i, row) in range.rows().enumerate() {
                if i >= TABLE_MAX_ROWS {
                    truncated = true;
                    break;
                }
                rows.push(row.iter().take(TABLE_MAX_COLS).map(cell_string).collect());
            }
        }
        sheets.push(SheetData {
            name: name.clone(),
            rows,
            truncated,
        });
    }
    if sheets.is_empty() {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::Unsupported));
    }
    Ok(PreviewContent::Sheets { sheets })
}

fn cell_string(data: &calamine::Data) -> String {
    match data {
        calamine::Data::Empty => String::new(),
        calamine::Data::String(s) => s.clone(),
        calamine::Data::Float(f) => format_float(*f),
        calamine::Data::Int(i) => i.to_string(),
        calamine::Data::DateTime(d) => d.to_string(),
        calamine::Data::DateTimeIso(s) => s.clone(),
        calamine::Data::DurationIso(s) => s.clone(),
        calamine::Data::Bool(b) => b.to_string(),
        calamine::Data::Error(e) => format!("{e:?}"),
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        format!("{f}")
    }
}

// -- zip helpers --------------------------------------------------------------------

fn zip_read_text(path: &Path, entry: &str) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&zip_read_bytes(path, entry)?).to_string())
}

fn zip_read_bytes(path: &Path, entry: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let mut z = archive
        .by_name(entry)
        .map_err(|e| format!("{entry}: {e}"))?;
    z.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Collect zip entries matching a `prefix{n}suffix` pattern, sorted by `n`.
fn zip_numbered_entries(
    archive: &mut zip::ZipArchive<std::fs::File>,
    prefix: &str,
    suffix: &str,
) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String)> = Vec::new();
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index(i) else {
            continue;
        };
        let Ok(name) = entry.name().map(|n| n.to_string()) else {
            continue;
        };
        let Some(rest) = name
            .strip_prefix(prefix)
            .and_then(|r| r.strip_suffix(suffix))
        else {
            continue;
        };
        let Ok(num) = rest.parse::<u64>() else {
            continue;
        };
        out.push((num, name));
    }
    out.sort();
    out
}

// -- tiny quick-xml helpers ------------------------------------------------------------

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader as XmlReader;

/// The tag name without its namespace prefix ("w:t" → "t").
fn local(e: &BytesStart) -> String {
    e.local_name().as_ref().to_string()
}

/// The closing tag name without its namespace prefix ("w:lvl" → "lvl").
fn local_end(e: &BytesEnd) -> String {
    let qname = e.name();
    let name: &str = qname.as_ref();
    name.rsplit(':').next().unwrap_or(name).to_string()
}

/// An attribute value, XML-entity-decoded ("w:val" → "Heading1").
///
/// Attribute keys may carry a namespace prefix (`w:val`), so both the full
/// key and its local part are matched.
fn attr(e: &BytesStart, name: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        let key: &str = a.key.as_ref();
        if key == name || key.rsplit(':').next() == Some(name) {
            return Some(decode_entities(&a.value));
        }
    }
    None
}

/// Decode the handful of XML entities documents actually use.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(end) = tail.find(';') {
            let ent = &tail[1..end];
            match ent {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                    if let Ok(cp) = u32::from_str_radix(&ent[2..], 16) {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                        }
                    }
                }
                _ if ent.starts_with('#') => {
                    if let Ok(cp) = ent[1..].parse::<u32>() {
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                        }
                    }
                }
                _ => out.push_str(ent),
            }
            rest = &tail[end + 1..];
        } else {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// "#RRGGBB" → Color32.
fn hex_color(s: &str) -> Option<Color32> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color32::from_rgb(r, g, b))
}

/// Office theme scheme colors (approximate — stock Office palette).
fn scheme_color(name: &str) -> Option<Color32> {
    Some(match name {
        "bg1" | "lt1" => Color32::WHITE,
        "tx1" | "dk1" => Color32::BLACK,
        "bg2" | "lt2" => Color32::from_rgb(0xE7, 0xE6, 0xE6),
        "tx2" | "dk2" => Color32::from_rgb(0x44, 0x45, 0x51),
        "accent1" => Color32::from_rgb(0x44, 0x72, 0xC4),
        "accent2" => Color32::from_rgb(0xED, 0x7D, 0x31),
        "accent3" => Color32::from_rgb(0xA5, 0xA5, 0xA5),
        "accent4" => Color32::from_rgb(0xFF, 0xC0, 0x00),
        "accent5" => Color32::from_rgb(0x5B, 0x9B, 0xD5),
        "accent6" => Color32::from_rgb(0x70, 0xAD, 0x47),
        "hlink" => Color32::from_rgb(0x05, 0x63, 0xC1),
        "folHlink" => Color32::from_rgb(0x95, 0x4F, 0x72),
        _ => return None,
    })
}

/// Fill color from an `a:solidFill`-style element: first srgbClr/schemeClr child.
fn fill_color(e: &BytesStart) -> Option<Color32> {
    let name = local(e);
    let color = attr(e, "val")?;
    match name.as_str() {
        "srgbClr" => hex_color(&color),
        "schemeClr" => scheme_color(&color),
        _ => None,
    }
}

// -- docx: real document flow ----------------------------------------------------------

fn docx(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::TooLarge));
    }
    let xml = zip_read_text(path, "word/document.xml")?;
    // numId → is it a numbered (decimal etc.) list? (w:bullet → bullets)
    let numbering = zip_read_text(path, "word/numbering.xml")
        .map(|n| numbering_formats(&n))
        .unwrap_or_default();
    let blocks = parse_docx(&xml, &numbering);
    if blocks.is_empty() {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::Unsupported));
    }
    Ok(PreviewContent::Document { blocks })
}

/// numId → true when the list is numbered (not bullets).
///
/// `numbering.xml` maps `w:num w:numId=N` → `w:abstractNumId=M`, and each
/// abstractNum's level-0 `w:numFmt` says whether it is a bullet or a
/// numbered format.
fn numbering_formats(xml: &str) -> std::collections::HashMap<String, bool> {
    let mut abstract_fmt: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut num_abstract: Vec<(String, String)> = Vec::new();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut cur_abstract = String::new();
    let mut cur_lvl0 = false;
    let mut cur_num = String::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                match local(e).as_str() {
                    "abstractNum" => {
                        cur_abstract = attr(e, "abstractNumId").unwrap_or_default();
                    }
                    "lvl" => {
                        cur_lvl0 = attr(e, "ilvl").as_deref() == Some("0");
                    }
                    "numFmt" if cur_lvl0 => {
                        abstract_fmt
                            .insert(cur_abstract.clone(), attr(e, "val").unwrap_or_default());
                    }
                    "num" => {
                        cur_num = attr(e, "numId").unwrap_or_default();
                    }
                    "abstractNumId" if !cur_num.is_empty() => {
                        num_abstract.push((
                            cur_num.clone(),
                            attr(e, "val").unwrap_or_default(),
                        ));
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => match local_end(e).as_str() {
                "lvl" => cur_lvl0 = false,
                "num" => cur_num.clear(),
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    let mut out = std::collections::HashMap::new();
    for (num_id, fmt_id) in num_abstract {
        let numbered = abstract_fmt
            .get(&fmt_id)
            .map(|f| !matches!(f.as_str(), "bullet" | "none"))
            .unwrap_or(false);
        out.insert(num_id, numbered);
    }
    out
}

#[derive(Default)]
struct DocxPara {
    style: String,
    center: bool,
    right: bool,
    ilvl: u8,
    num_id: String,
    is_list: bool,
    spans: Vec<DocSpan>,
}

impl DocxPara {
    fn plain(&self) -> String {
        self.spans
            .iter()
            .map(|s| s.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn parse_docx(xml: &str, numbering: &std::collections::HashMap<String, bool>) -> Vec<DocBlock> {
    let mut blocks: Vec<DocBlock> = Vec::new();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut para = DocxPara::default();
    let mut in_para = false;
    let mut in_cell = false;
    let mut cell_text = String::new();

    let mut run = DocSpan::default();
    let mut in_run = false;
    let mut link_depth = 0usize;

    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut row_cells: Vec<String> = Vec::new();

    // text capture target: 0 = none, 1 = w:t (run text)
    let mut capture_text = false;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "p" => {
                        para = DocxPara::default();
                        in_para = true;
                    }
                    "r" => {
                        run = DocSpan::default();
                        in_run = true;
                    }
                    "b" if in_run => {
                        if !attr(&e, "val").is_some_and(|v| is_zero_val(&v)) {
                            run.bold = true;
                        }
                    }
                    "i" if in_run => {
                        if !attr(&e, "val").is_some_and(|v| is_zero_val(&v)) {
                            run.italic = true;
                        }
                    }
                    "u" if in_run => {
                        if !matches!(attr(&e, "val").as_deref(), Some("none")) {
                            run.underline = true;
                        }
                    }
                    "strike" if in_run => {
                        if !attr(&e, "val").is_some_and(|v| is_zero_val(&v)) {
                            run.strike = true;
                        }
                    }
                    "color" if in_run => {
                        run.color = attr(&e, "val").and_then(|v| hex_color(&v));
                    }
                    "sz" if in_run => {
                        run.size = attr(&e, "val")
                            .and_then(|v| v.parse::<f32>().ok())
                            .map(|half| half / 2.0);
                    }
                    "t" => capture_text = true,
                    "tab" if in_run => run.text.push('\t'),
                    "br" if in_run => run.text.push('\n'),
                    "pStyle" if in_para => {
                        para.style = attr(&e, "val").unwrap_or_default();
                    }
                    "jc" if in_para => match attr(&e, "val").as_deref() {
                        Some("center") => para.center = true,
                        Some("right") | Some("end") => para.right = true,
                        _ => {}
                    },
                    "numPr" if in_para => para.is_list = true,
                    "ilvl" if in_para => {
                        para.ilvl = attr(&e, "val")
                            .and_then(|v| v.parse::<u8>().ok())
                            .unwrap_or(0);
                    }
                    "numId" if in_para => {
                        para.num_id = attr(&e, "val").unwrap_or_default();
                        if para.num_id == "0" {
                            para.is_list = false;
                        }
                    }
                    "hyperlink" => link_depth += 1,
                    "tbl" if !in_cell => {
                        in_table = true;
                        table_rows.clear();
                    }
                    "tr" if in_table => row_cells.clear(),
                    "tc" if in_table => {
                        in_cell = true;
                        cell_text.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if capture_text && in_run {
                    run.text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(e)) => {
                let name = local_end(&e);
                match name.as_str() {
                    "t" => capture_text = false,
                    "r" => {
                        if in_run && !run.text.trim().is_empty() {
                            if link_depth > 0 {
                                run.underline = true;
                                run.color =
                                    Some(Color32::from_rgb(0x05, 0x63, 0xC1));
                            }
                            para.spans.push(run.clone());
                        }
                        in_run = false;
                    }
                    "hyperlink" => link_depth = link_depth.saturating_sub(1),
                    "p" => {
                        if in_para {
                            if in_cell {
                                let line = para.plain();
                                if !line.is_empty() {
                                    if !cell_text.is_empty() {
                                        cell_text.push('\n');
                                    }
                                    cell_text.push_str(&line);
                                }
                            } else if blocks.len() < DOC_MAX_BLOCKS {
                                if let Some(b) = para_block(&para, numbering) {
                                    blocks.push(b);
                                }
                            }
                        }
                        in_para = false;
                    }
                    "tc" if in_table => {
                        row_cells.push(std::mem::take(&mut cell_text));
                        in_cell = false;
                    }
                    "tr" if in_table => {
                        table_rows.push(std::mem::take(&mut row_cells));
                    }
                    "tbl" if in_table => {
                        if blocks.len() < DOC_MAX_BLOCKS && !table_rows.is_empty() {
                            let mut it = table_rows
                                .iter()
                                .take(TABLE_MAX_ROWS)
                                .map(|r| r.iter().take(TABLE_MAX_COLS).cloned().collect());
                            let header = it.next().unwrap_or_default();
                            blocks.push(DocBlock::Table {
                                header,
                                rows: it.collect(),
                            });
                        }
                        in_table = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    blocks
}

fn is_zero_val(v: &str) -> bool {
    matches!(v, "0" | "false")
}

fn para_block(
    para: &DocxPara,
    numbering: &std::collections::HashMap<String, bool>,
) -> Option<DocBlock> {
    let has_text = para.spans.iter().any(|s| !s.text.trim().is_empty());
    if !has_text {
        return None;
    }
    // Heading1..Heading9 / Title / Subtitle styles → heading levels.
    let style = para.style.as_str();
    let heading = if let Some(rest) = style.strip_prefix("Heading") {
        rest.trim_start_matches('0')
            .parse::<u8>()
            .ok()
            .filter(|l| (1..=6).contains(l))
    } else if style == "Title" {
        Some(1)
    } else if style == "Subtitle" {
        Some(2)
    } else {
        None
    };
    if let Some(level) = heading {
        return Some(DocBlock::Heading {
            level,
            spans: para.spans.clone(),
        });
    }
    if para.is_list {
        let numbered = numbering.get(&para.num_id).copied().unwrap_or(false);
        return Some(DocBlock::ListItem {
            level: para.ilvl.min(6),
            numbered,
            spans: para.spans.clone(),
        });
    }
    Some(DocBlock::Para {
        center: para.center,
        right: para.right,
        spans: para.spans.clone(),
    })
}

// -- pptx: real slide rendering ----------------------------------------------------------

fn pptx(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::TooLarge));
    }
    // Slide size (EMU), default 16:9.
    let mut width_emu = 12192000i64;
    let mut height_emu = 6858000i64;
    if let Ok(pres) = zip_read_text(path, "ppt/presentation.xml") {
        let mut reader = XmlReader::from_str(&pres);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    if local(&e) == "sldSz" {
                        width_emu = attr(&e, "cx")
                            .and_then(|v| v.parse::<i64>().ok())
                            .unwrap_or(width_emu);
                        height_emu = attr(&e, "cy")
                            .and_then(|v| v.parse::<i64>().ok())
                            .unwrap_or(height_emu);
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
    }
    if width_emu <= 0 || height_emu <= 0 {
        width_emu = 12192000;
        height_emu = 6858000;
    }

    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let entries = zip_numbered_entries(&mut archive, "ppt/slides/slide", ".xml");
    if entries.is_empty() {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::Unsupported));
    }
    let mut slides = Vec::new();
    for (num, name) in entries.into_iter().take(100) {
        let xml = zip_read_bytes(path, &name)?;
        let xml = String::from_utf8_lossy(&xml).to_string();
        let rels = zip_read_text(
            path,
            &format!("ppt/slides/_rels/slide{num}.xml.rels"),
        )
        .unwrap_or_default();
        let media = slide_media(path, &rels, size);
        let slide = parse_slide(&xml, &media);
        slides.push(slide);
    }
    Ok(PreviewContent::Slides {
        width_emu,
        height_emu,
        slides,
    })
}

/// Id → decoded picture (from the slide's rels → media entries).
type SlideMedia = std::collections::HashMap<String, eframe::egui::ColorImage>;

fn slide_media(path: &Path, rels: &str, _file_size: u64) -> SlideMedia {
    let mut map = std::collections::HashMap::new();
    let mut reader = XmlReader::from_str(rels);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local(&e) != "Relationship" {
                    continue;
                }
                let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) else {
                    continue;
                };
                // "../media/image1.png" → "ppt/media/image1.png"
                let entry = if let Some(rest) = target.strip_prefix("../") {
                    format!("ppt/{rest}")
                } else if let Some(rest) = target.strip_prefix('/') {
                    rest.to_string()
                } else {
                    format!("ppt/slides/{target}")
                };
                if let Ok(bytes) = zip_read_bytes(path, &entry) {
                    if bytes.len() as u64 > SLIDE_MAX_IMAGE_BYTES {
                        continue;
                    }
                    if let Ok(img) = image::load_from_memory(&bytes) {
                        let rgba = img.to_rgba8();
                        map.insert(
                            id,
                            eframe::egui::ColorImage::from_rgba_unmultiplied(
                                [rgba.width() as usize, rgba.height() as usize],
                                rgba.as_raw(),
                            ),
                        );
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    map
}

#[derive(Default)]
struct PptxShape {
    has_geom: bool,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    fill: Option<Color32>,
    no_fill: bool,
    anchor_center: bool,
    ph_type: String,
    image_rid: Option<String>,
    paras: Vec<SlidePara>,
}

fn parse_slide(xml: &str, media: &SlideMedia) -> Slide {
    let mut slide = Slide {
        background: None,
        shapes: Vec::new(),
    };
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut shape: Option<PptxShape> = None;
    let mut in_bg = false;
    let mut in_sppr = false;
    let mut in_txbody = false;
    let mut in_run = false;
    let mut in_rpr = false;
    let mut in_para = false;
    let mut para = SlidePara { align: 0, runs: Vec::new() };
    let mut run = SlideRun {
        text: String::new(),
        bold: false,
        italic: false,
        size_pt: 18.0,
        color: None,
    };
    let mut capture_text = false;
    let mut solid_target = 0u8; // 0 none, 1 shape fill, 2 text color, 3 slide bg

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local(e);
                match name.as_str() {
                    "bg" => in_bg = true,
                    "sp" | "pic" => {
                        shape = Some(PptxShape::default());
                    }
                    "ph" => {
                        if let Some(s) = shape.as_mut() {
                            s.ph_type = attr(e, "type").unwrap_or_default();
                        }
                    }
                    "spPr" => in_sppr = true,
                    "txBody" => in_txbody = true,
                    "bodyPr" => {
                        if let Some(s) = shape.as_mut() {
                            if attr(e, "anchor").as_deref() == Some("ctr") {
                                s.anchor_center = true;
                            }
                        }
                    }
                    "off" => {
                        if let Some(s) = shape.as_mut() {
                            s.x = attr(e, "x").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                            s.y = attr(e, "y").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                            s.has_geom = true;
                        }
                    }
                    "ext" => {
                        if let Some(s) = shape.as_mut() {
                            s.w = attr(e, "cx").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                            s.h = attr(e, "cy").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                            s.has_geom = true;
                        }
                    }
                    "solidFill" => {
                        solid_target = if in_bg {
                            3
                        } else if in_sppr {
                            1
                        } else if in_rpr {
                            2
                        } else {
                            0
                        };
                    }
                    "noFill" if in_sppr => {
                        if let Some(s) = shape.as_mut() {
                            s.no_fill = true;
                        }
                    }
                    "srgbClr" | "schemeClr" => {
                        let color = fill_color(e);
                        match solid_target {
                            3 => {
                                if color.is_some() {
                                    slide.background = color;
                                }
                            }
                            1 => {
                                if let (Some(s), Some(c)) = (shape.as_mut(), color) {
                                    s.fill = Some(c);
                                }
                            }
                            2 => {
                                if let Some(c) = color {
                                    run.color = Some(c);
                                }
                            }
                            _ => {}
                        }
                    }
                    "blip" => {
                        if let (Some(s), Some(rid)) =
                            (shape.as_mut(), attr(e, "r:embed"))
                        {
                            s.image_rid = Some(rid);
                        }
                    }
                    "p" if in_txbody => {
                        para = SlidePara { align: 0, runs: Vec::new() };
                        in_para = true;
                    }
                    "pPr" if in_para => {
                        match attr(e, "algn").as_deref() {
                            Some("ctr") => para.align = 1,
                            Some("r") | Some("just") => para.align = 2,
                            _ => {}
                        }
                    }
                    "r" if in_txbody => {
                        run = SlideRun {
                            text: String::new(),
                            bold: false,
                            italic: false,
                            size_pt: 18.0,
                            color: None,
                        };
                        in_run = true;
                    }
                    "rPr" if in_run => {
                        if let Some(v) = attr(e, "b") {
                            run.bold = !is_zero_val(&v);
                        }
                        if let Some(v) = attr(e, "i") {
                            run.italic = !is_zero_val(&v);
                        }
                        if let Some(v) = attr(e, "sz") {
                            if let Ok(sz) = v.parse::<f32>() {
                                run.size_pt = sz / 100.0;
                            }
                        }
                        in_rpr = true;
                    }
                    "t" => capture_text = true,
                    "br" if in_run => {
                        run.text.push('\n');
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref t)) => {
                if capture_text && in_run {
                    run.text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_end(e);
                match name.as_str() {
                    "t" => capture_text = false,
                    "rPr" => in_rpr = false,
                    "r" => {
                        if in_run {
                            if !run.text.is_empty()
                                && shape
                                    .as_ref()
                                    .map(|s| s.paras.len() < SLIDE_MAX_PARAS)
                                    .unwrap_or(false)
                                && para.runs.len() < SLIDE_MAX_RUNS
                            {
                                para.runs.push(run.clone());
                            }
                        }
                        in_run = false;
                    }
                    "p" if in_txbody => {
                        in_para = false;
                        if let Some(s) = shape.as_mut() {
                            if s.paras.len() < SLIDE_MAX_PARAS {
                                s.paras.push(para.clone());
                            }
                        }
                    }
                    "spPr" => in_sppr = false,
                    "txBody" => in_txbody = false,
                    "sp" | "pic" => {
                        if let Some(mut s) = shape.take() {
                            if !s.has_geom {
                                // Inherit-free fallback geometry by placeholder type.
                                let (x, y, w, h) = match s.ph_type.as_str() {
                                    "title" | "ctrTitle" => (610000, 440000, 11000000, 1300000),
                                    "subTitle" => (610000, 1900000, 11000000, 800000),
                                    "" => (610000, 610000, 11000000, 5800000),
                                    _ => (610000, 1800000, 11000000, 4400000),
                                };
                                (s.x, s.y, s.w, s.h) = (x, y, w, h);
                            }
                            if s.w > 0 && s.h > 0 && slide.shapes.len() < SLIDE_MAX_SHAPES {
                                if s.no_fill {
                                    s.fill = None;
                                }
                                slide.shapes.push(SlideShape {
                                    x: s.x,
                                    y: s.y,
                                    w: s.w,
                                    h: s.h,
                                    fill: s.fill,
                                    anchor_center: s.anchor_center,
                                    paras: s.paras,
                                    image: s
                                        .image_rid
                                        .as_deref()
                                        .and_then(|rid| media.get(rid))
                                        .cloned(),
                                });
                            }
                        }
                    }
                    "bg" => in_bg = false,
                    "solidFill" => solid_target = 0,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    slide
}

// -- pdf: real page rendering (pdfium) ----------------------------------------------------

fn pdf(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(crate::dock::InfoKind::TooLarge));
    }
    match render_pdf_pages(path) {
        Ok((pages, truncated)) => Ok(PreviewContent::Pdf { pages, truncated }),
        Err(_) => {
            // No pdfium (or render failure) → fall back to extracted text.
            let text = pdf_extract::extract_text(path).map_err(|e| format!("pdf: {e}"))?;
            Ok(PreviewContent::Text(cap_lines(&text, TEXT_MAX_LINES)))
        }
    }
}

/// Locate the bundled pdfium library next to the executable (CI ships
/// `pdfium.dll` / `libpdfium.so` alongside the binary).
fn pdfium() -> Result<&'static pdfium_render::prelude::Pdfium, String> {
    static PDFIUM: OnceLock<Result<pdfium_render::prelude::Pdfium, String>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            use pdfium_render::prelude::*;
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    if cfg!(windows) {
                        candidates.push(dir.join("pdfium.dll"));
                    } else {
                        candidates.push(dir.join("libpdfium.so"));
                    }
                }
            }
            for c in &candidates {
                if let Ok(bindings) = Pdfium::bind_to_library(c) {
                    return Ok(Pdfium::new(bindings));
                }
            }
            if let Ok(bindings) = Pdfium::bind_to_system_library() {
                return Ok(Pdfium::new(bindings));
            }
            Err("pdfium library not found".to_string())
        })
        .as_ref()
        .map_err(std::clone::Clone::clone)
}

fn render_pdf_pages(path: &Path) -> Result<(Vec<PdfPageImg>, bool), String> {
    use pdfium_render::prelude::*;
    let pdfium = pdfium().map_err(|e| e)?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("pdfium: {e}"))?;
    let pages = document.pages();
    let total = pages.len() as usize;
    let take = total.min(PDF_MAX_PAGES);
    let mut out: Vec<PdfPageImg> = Vec::new();
    for page in pages.iter().take(take) {
        let config = PdfRenderConfig::new()
            .set_target_width(PDF_PAGE_WIDTH)
            .clear_before_rendering(true)
            .set_clear_color(PdfColor::WHITE);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| format!("render: {e}"))?;
        let (w, h) = (bitmap.width(), bitmap.height());
        let rgba = bitmap.as_rgba_bytes();
        out.push(PdfPageImg {
            width: w as u32,
            height: h as u32,
            rgba: rgba.to_vec(),
        });
    }
    Ok((out, total > take))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv_with_quotes_and_newlines() {
        let table = delimited_table("a,b,c\n\"x,1\",\"y\"\"q\",z\n", ',');
        assert_eq!(table.columns, vec!["a", "b", "c"]);
        assert_eq!(table.rows.len(), 1);
        assert_eq!(table.rows[0][0], "x,1");
        assert_eq!(table.rows[0][1], "y\"q");
        assert_eq!(table.rows[0][2], "z");
    }

    #[test]
    fn caps_table_rows() {
        let big: String = "a,b\n".to_string() + &"1,2\n".repeat(600);
        let table = delimited_table(&big, ',');
        assert!(table.truncated);
        assert_eq!(table.rows.len(), TABLE_MAX_ROWS);
    }

    #[test]
    fn splits_mermaid_fences() {
        let md = "# Doc\n\n```mermaid\ngraph TD\nA-->B\n```\n\nafter".to_string();
        let content = markdown_doc(md);
        let PreviewContent::MarkdownDoc { segments } = content else {
            panic!("expected markdown doc");
        };
        assert_eq!(segments.len(), 3);
        assert!(matches!(&segments[1],
            MdSegment::Diagram { code } if code.contains("graph TD")));
    }

    #[test]
    fn parses_docx_document_flow() {
        let xml = concat!(
            "<w:document xmlns:w=\"x\"><w:body>",
            "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>",
            "<w:r><w:rPr><w:b/><w:sz w:val=\"48\"/></w:rPr><w:t>Title</w:t></w:r></w:p>",
            "<w:p><w:r><w:rPr><w:i/></w:rPr><w:t>italic </w:t></w:r>",
            "<w:r><w:t>plain</w:t></w:r></w:p>",
            "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"3\"/></w:numPr></w:pPr>",
            "<w:r><w:t>item</w:t></w:r></w:p>",
            "</w:body></w:document>",
        );
        let mut numbering = std::collections::HashMap::new();
        numbering.insert("3".to_string(), true);
        let blocks = parse_docx(xml, &numbering);
        assert_eq!(blocks.len(), 3);
        match &blocks[0] {
            DocBlock::Heading { level, spans } => {
                assert_eq!(*level, 1);
                assert!(spans[0].bold);
                assert_eq!(spans[0].text, "Title");
            }
            _ => panic!("expected heading"),
        }
        match &blocks[1] {
            DocBlock::Para { spans, .. } => {
                assert!(spans[0].italic);
                assert_eq!(spans[1].text, "plain");
            }
            _ => panic!("expected para"),
        }
        match &blocks[2] {
            DocBlock::ListItem { numbered, .. } => assert!(numbered),
            _ => panic!("expected list item"),
        }
    }

    #[test]
    fn parses_docx_table() {
        let xml = concat!(
            "<w:document><w:body><w:tbl><w:tr><w:tc><w:p><w:r><w:t>H1</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>H2</w:t></w:r></w:p></w:tc></w:tr>",
            "<w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc>",
            "<w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>",
        );
        let blocks = parse_docx(xml, &std::collections::HashMap::new());
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            DocBlock::Table { header, rows } => {
                assert_eq!(header, &vec!["H1".to_string(), "H2".to_string()]);
                assert_eq!(rows, &vec![vec!["a".to_string(), "b".to_string()]]);
            }
            _ => panic!("expected table"),
        }
    }

    #[test]
    fn parses_pptx_slide_shapes() {
        let xml = concat!(
            "<p:sld xmlns:p=\"x\" xmlns:a=\"y\"><p:cSld>",
            "<p:sp><p:nvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>",
            "<p:spPr><a:xfrm><a:off x=\"100\" y=\"200\"/><a:ext cx=\"300\" cy=\"400\"/></a:xfrm>",
            "<a:solidFill><a:srgbClr val=\"FF0000\"/></a:solidFill></p:spPr>",
            "<p:txBody><a:bodyPr anchor=\"ctr\"/>",
            "<a:p><a:pPr algn=\"ctr\"/><a:r><a:rPr b=\"1\" sz=\"3200\" ",
            "a:solidFill=\"x\"><a:solidFill><a:srgbClr val=\"00FF00\"/></a:solidFill></a:rPr>",
            "<a:t>Big</a:t></a:r></a:p></p:txBody></p:sp>",
            "</p:cSld></p:sld>",
        );
        let slide = parse_slide(xml, &SlideMedia::default());
        assert_eq!(slide.shapes.len(), 1);
        let s = &slide.shapes[0];
        assert_eq!((s.x, s.y, s.w, s.h), (100, 200, 300, 400));
        assert_eq!(s.fill, Some(Color32::from_rgb(0xFF, 0, 0)));
        assert!(s.anchor_center);
        assert_eq!(s.paras.len(), 1);
        assert_eq!(s.paras[0].align, 1);
        assert_eq!(s.paras[0].runs.len(), 1);
        let r = &s.paras[0].runs[0];
        assert_eq!(r.text, "Big");
        assert!(r.bold);
        assert_eq!(r.size_pt, 32.0);
        assert_eq!(r.color, Some(Color32::from_rgb(0, 0xFF, 0)));
    }

    #[test]
    fn pptx_shape_without_xfrm_gets_fallback_geometry() {
        let xml = concat!(
            "<p:sld><p:sp><p:nvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>",
            "<p:txBody><a:p><a:r><a:t>T</a:t></a:r></a:p></p:txBody></p:sp></p:sld>",
        );
        let slide = parse_slide(xml, &SlideMedia::default());
        assert_eq!(slide.shapes.len(), 1);
        assert!(slide.shapes[0].w > 0 && slide.shapes[0].h > 0);
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode_entities("a &amp; b &#x2605; &#9731;"), "a & b ★ ☃");
    }
}
