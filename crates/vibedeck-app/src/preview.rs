//! File preview loaders for the workspace dock.
//!
//! Everything runs on a background thread and produces a
//! [`PreviewContent`] the dock can render:
//!
//! - Markdown (`.md`) → friendly rendered preview
//! - CSV / TSV → table
//! - Excel (`.xlsx` / `.xls` / `.xlsm` / `.ods`) → first sheet as a table
//! - Word (`.docx`), PowerPoint (`.pptx`) → extracted text
//! - PDF → extracted text
//! - Images (png / jpg / gif / bmp / webp / tiff / ico) → inline image
//! - Text-ish formats (source code, config, logs…) → plain text
//! - Anything else → a "no preview" note (binary / unsupported)

use std::io::Read as _;
use std::path::Path;

use crate::dock::{InfoKind, PreviewContent};

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
        "md" | "markdown" => text_or_info(path, |t| {
            PreviewContent::Markdown(cap_lines(&t, TEXT_MAX_LINES))
        }),
        "csv" => text_or_info(path, |t| delimited_table(&t, ',').into_content()),
        "tsv" => text_or_info(path, |t| delimited_table(&t, '\t').into_content()),
        "xlsx" | "xlsm" | "xls" | "ods" => spreadsheet(path, size),
        "docx" => docx(path, size),
        "pptx" => pptx(path, size),
        "pdf" => pdf(path, size),
        _ if IMAGE_EXTS.contains(&ext.as_str()) => {
            if size > IMAGE_MAX_BYTES {
                Ok(PreviewContent::Info(InfoKind::TooLarge))
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
            Ok(PreviewContent::Info(InfoKind::Binary))
        }
        _ if TEXT_EXTS.contains(&ext.as_str()) => text_or_info(path, |t| {
            PreviewContent::Text(cap_lines(&t, TEXT_MAX_LINES))
        }),
        _ => {
            // Unknown or no extension (Makefile, LICENSE…):
            // sniff — try UTF-8, fall back to "binary".
            match read_text(path) {
                Some(t) => Ok(PreviewContent::Text(cap_lines(&t, TEXT_MAX_LINES))),
                None => Ok(PreviewContent::Info(InfoKind::Binary)),
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
        return Ok(PreviewContent::Info(InfoKind::TooLarge));
    }
    match read_text(path) {
        Some(t) => Ok(f(t)),
        None => Ok(PreviewContent::Info(InfoKind::Binary)),
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

// -- tables ---------------------------------------------------------------------

/// A parsed delimited table (CSV/TSV or spreadsheet sheet).
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

// -- spreadsheets (calamine) ------------------------------------------------------

fn spreadsheet(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(InfoKind::TooLarge));
    }
    use calamine::Reader;
    let mut workbook =
        calamine::open_workbook_auto(path).map_err(|e| format!("workbook: {e}"))?;
    let names = workbook.sheet_names();
    let Some(first) = names.first().cloned() else {
        return Ok(PreviewContent::Info(InfoKind::Unsupported));
    };
    let range = workbook
        .worksheet_range(&first)
        .map_err(|e| format!("sheet: {e}"))?;

    let mut all: Vec<Vec<String>> = Vec::new();
    let mut truncated = false;
    for (i, row) in range.rows().enumerate() {
        if i >= TABLE_MAX_ROWS {
            truncated = true;
            break;
        }
        all.push(
            row.iter()
                .take(TABLE_MAX_COLS)
                .map(|c| cell_string(c))
                .collect(),
        );
    }
    let note = if names.len() > 1 {
        format!("{}: {}", names.len(), names.join(", "))
    } else {
        first
    };
    let mut it = all.into_iter();
    let columns = it.next().unwrap_or_default();
    let body: Vec<Vec<String>> = it
        .map(|mut r| {
            while r.len() < columns.len() {
                r.push(String::new());
            }
            r
        })
        .collect();
    Ok(Table {
        columns,
        rows: body,
        truncated,
        note,
    }
    .into_content())
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

// -- docx / pptx (zip + xml) --------------------------------------------------------

fn zip_read_text(path: &Path, entry: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let mut z = archive
        .by_name(entry)
        .map_err(|e| format!("{entry}: {e}"))?;
    z.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn docx(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(InfoKind::TooLarge));
    }
    let xml = zip_read_text(path, "word/document.xml")?;
    let text = extract_tagged_text(&xml, "w:t", "w:p");
    Ok(PreviewContent::Text(cap_lines(&text, TEXT_MAX_LINES)))
}

fn pptx(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(InfoKind::TooLarge));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut slides: Vec<(u64, String)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?;
        let Some(rest) = name
            .as_ref()
            .strip_prefix("ppt/slides/slide")
            .and_then(|r| r.strip_suffix(".xml"))
        else {
            continue;
        };
        let Ok(num) = rest.parse::<u64>() else {
            continue;
        };
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut entry, &mut xml).map_err(|e| e.to_string())?;
        slides.push((num, extract_tagged_text(&xml, "a:t", "a:p")));
    }
    slides.sort_by_key(|(n, _)| *n);
    if slides.is_empty() {
        return Ok(PreviewContent::Info(InfoKind::Unsupported));
    }
    let mut out = String::new();
    for (i, (num, text)) in slides.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!("── Slide {num} ──\n"));
        out.push_str(text);
    }
    Ok(PreviewContent::Text(cap_lines(&out, TEXT_MAX_LINES)))
}

/// Extract text content between `<tag…>…</tag>` runs, breaking paragraphs on
/// `</para_tag>` — a tiny, dependency-free XML text walker good enough for
/// docx (`w:t` / `w:p`) and pptx (`a:t` / `a:p`).
fn extract_tagged_text(xml: &str, text_tag: &str, para_tag: &str) -> String {
    let open = format!("<{text_tag}");
    let close = format!("</{text_tag}>");
    let para_close = format!("</{para_tag}>");
    let mut out = String::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i..];
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            // self-closing tag — no text
            rest = &after[gt + 1..];
            continue;
        }
        let seg = &after[gt + 1..];
        let Some(end) = seg.find(&close) else { break };
        out.push_str(&decode_entities(&seg[..end]));
        rest = &seg[end + close.len()..];
        // What comes before the next text run? If a paragraph closed, break
        // the line; otherwise separate runs with a space.
        let next_run = rest.find(&open).map(|p| &rest[..p]).unwrap_or(rest);
        if next_run.contains(&para_close) {
            out.push('\n');
        } else {
            out.push(' ');
        }
    }
    out
}

/// Decode the handful of XML entities docx/pptx documents actually use.
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

// -- pdf -------------------------------------------------------------------------

fn pdf(path: &Path, size: u64) -> Result<PreviewContent, String> {
    if size > DOC_MAX_BYTES {
        return Ok(PreviewContent::Info(InfoKind::TooLarge));
    }
    let text = pdf_extract::extract_text(path).map_err(|e| format!("pdf: {e}"))?;
    Ok(PreviewContent::Text(cap_lines(&text, TEXT_MAX_LINES)))
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
        let big: String = "a,b\n".to_string()
            + &"1,2\n".repeat(600);
        let table = delimited_table(&big, ',');
        assert!(table.truncated);
        assert_eq!(table.rows.len(), TABLE_MAX_ROWS);
    }

    #[test]
    fn extracts_docx_text() {
        let xml = r#"<w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t>world</w:t></w:r></w:p><w:p><w:r><w:t>Second &amp; &lt;line&gt;</w:t></w:r></w:p>"#;
        let text = extract_tagged_text(xml, "w:t", "w:p");
        assert_eq!(text, "Hello world\nSecond & <line>\n");
    }

    #[test]
    fn extracts_pptx_text() {
        let xml = r#"<a:p><a:r><a:t>Slide one</a:t></a:r></a:p><a:p><a:r><a:t>More</a:t></a:r></a:p>"#;
        let text = extract_tagged_text(xml, "a:t", "a:p");
        assert_eq!(text, "Slide one\nMore\n");
    }
}
