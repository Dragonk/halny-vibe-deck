//! Server-side preview renderers (only compiled for the server build).
//!
//! Office documents (docx/odt/pptx/odp), spreadsheets (xlsx/xls/xlsm/ods) and
//! PDF pages (pdfium, bundled next to the executable). Everything is rendered
//! here into escaped HTML (or page images as data URLs) so the client can
//! inject it with `dangerous_inner_html` — no JS parser needed on the client.

use std::fs;
use std::io::Read;
use std::path::Path;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader as XmlReader;

use crate::fsgit::SheetData;

const HTML_CAP: usize = 4 * 1024 * 1024;
const BLOCK_CAP: usize = 4000;
const SHEET_MAX: usize = 12;
const SHEET_MAX_ROWS: usize = 400;
const SHEET_MAX_COLS: usize = 64;
const PDF_MAX_PAGES: usize = 30;
const PDF_PAGE_WIDTH: i32 = 1100;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    crate::highlight::escape_html(s)
}

fn local(e: &BytesStart) -> String {
    e.local_name().as_ref().to_string()
}

fn local_end(e: &BytesEnd) -> String {
    let qname = e.name();
    let name: &str = qname.as_ref();
    name.rsplit(':').next().unwrap_or(name).to_string()
}

fn attr(e: &BytesStart, key_local: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        let key: &str = a.key.as_ref();
        if key == key_local || key.rsplit(':').next() == Some(key_local) {
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

fn is_zero_val(v: &str) -> bool {
    v == "0" || v.eq_ignore_ascii_case("false") || v == "none"
}

fn read_zip_entry(path: &Path, name: &str) -> Result<Vec<u8>, String> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut entry = z.by_name(name).map_err(|e| format!("zip {name}: {e}"))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).map(|b| *b as u32).unwrap_or(0);
        let b2 = chunk.get(2).map(|b| *b as u32).unwrap_or(0);
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Inline text with \n → <br/> and tab → spaces, escaped.
fn text_html(s: &str) -> String {
    esc(s)
        .replace('\n', "<br/>")
        .replace('\t', "&nbsp;&nbsp;&nbsp;&nbsp;")
}

// ---------------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------------

pub fn docx_to_html(path: &Path) -> Result<String, String> {
    let xml = read_zip_entry(path, "word/document.xml")?;
    let xml = String::from_utf8_lossy(&xml).into_owned();
    // Round 4 (C3): relationship table (hyperlinks + image targets).
    let rels_xml = read_zip_entry(path, "word/_rels/document.xml.rels").unwrap_or_default();
    let rels_xml = String::from_utf8_lossy(&rels_xml).into_owned();
    let rels = parse_docx_rels(&rels_xml);
    // Round 6 (R11): styleId → (outline level, is-TOC) from styles.xml, so
    // localized Word builds ("Nagwek1", "Überschrift 2", …) get real
    // heading levels, and TOC-entry paragraphs are replaced by the
    // generated TOC.
    let style_info = docx_style_info(path);
    Ok(docx_xml_full(&xml, |rid| rels.get(rid).cloned(), &style_info, |target| {
        // media targets are relative to word/
        let inner = if target.starts_with('/') {
            target.trim_start_matches('/').to_string()
        } else {
            format!("word/{target}")
        };
        read_zip_entry(path, &inner).ok().map(|bytes| {
            let mime = mime_for(&inner).to_string();
            (mime, bytes)
        })
    }))
}

fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "emf" => "image/x-emf",
        "wmf" => "image/x-wmf",
        _ => "application/octet-stream",
    }
}

/// `word/_rels/document.xml.rels` → rId → (target, kind).
fn parse_docx_rels(xml: &str) -> std::collections::HashMap<String, (String, String)> {
    let mut out = std::collections::HashMap::new();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if local(&e) == "Relationship" {
                    if let (Some(id), Some(target)) = (attr(&e, "Id"), attr(&e, "Target")) {
                        let ty = attr(&e, "Type").unwrap_or_default();
                        let kind = if ty.contains("hyperlink") {
                            "link"
                        } else if ty.contains("image") {
                            "image"
                        } else {
                            "other"
                        };
                        out.insert(id, (target, kind.to_string()));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Round 8: TOC gallery values are localized ("Table of Contents",
/// "Spis treści", "Inhaltsverzeichnis", …) — normalize to ASCII lowercase
/// alnum and match the known names.
fn is_toc_gallery(val: &str) -> bool {
    let mut norm = String::with_capacity(val.len());
    for ch in val.chars() {
        match ch {
            'ł' | 'Ł' => norm.push('l'),
            'ó' | 'Ó' => norm.push('o'),
            'ü' | 'Ü' => norm.push('u'),
            'ö' | 'Ö' => norm.push('o'),
            'ä' | 'Ä' => norm.push('a'),
            'é' | 'É' => norm.push('e'),
            'í' | 'Í' => norm.push('i'),
            'ñ' | 'Ñ' => norm.push('n'),
            _ => {
                if ch.is_ascii_alphanumeric() {
                    norm.push(ch.to_ascii_lowercase());
                }
            }
        }
    }
    matches!(
        norm.as_str(),
        "tableofcontents" | "spistreci" | "inhaltsverzeichnis" | "tabledematieres"
            | "tabladecontenidos" | "indice" | "sommaire" | "toc"
    )
}

/// Heading level from a paragraph style id/name. Round 5 (R4): Word style
/// ids are not always "Heading1" — localized builds use "Nagłówek 1",
/// "Überschrift 1" etc., and some docs use "Heading 1" with a space, so
/// normalize away everything but letters+digits and match known prefixes.
fn heading_level(style: &str) -> Option<u32> {
    let s = style.trim();
    let mut norm = String::with_capacity(s.len());
    for ch in s.chars() {
        // round 8: ASCII-fold common diacritics BEFORE dropping the rest, so
        // "Nagłówek 1" → "naglowek1" and "Überschrift 2" → "uberschrift2"
        // actually match (dropping non-ASCII blindly gave "nagwek1" /
        // "berschrift2", which the old prefix list could never hit).
        let folded = match ch {
            'ł' | 'Ł' => Some('l'),
            'ó' | 'Ó' => Some('o'),
            'ü' | 'Ü' => Some('u'),
            'ö' | 'Ö' => Some('o'),
            'ä' | 'Ä' => Some('a'),
            'é' | 'É' => Some('e'),
            'á' | 'Á' => Some('a'),
            'í' | 'Í' => Some('i'),
            'ñ' | 'Ñ' => Some('n'),
            'ś' | 'Ś' => Some('s'),
            'ć' | 'Ć' => Some('c'),
            'ź' | 'Ź' => Some('z'),
            'ż' | 'Ż' => Some('z'),
            _ => None,
        };
        if let Some(f) = folded {
            norm.push(f.to_ascii_lowercase());
        } else if ch.is_ascii_alphanumeric() {
            norm.push(ch.to_ascii_lowercase());
        }
    }
    if matches!(norm.as_str(), "title" | "tytul" | "titulo" | "titre") {
        return Some(1);
    }
    for prefix in ["heading", "naglowek", "uberschrift", "ueberschrift", "berschrift", "titre", "titolo", "encabezado"] {
        if let Some(rest) = norm.strip_prefix(prefix) {
            if let Ok(n) = rest.parse::<u32>() {
                if (1..=9).contains(&n) {
                    return Some(n.min(6));
                }
            }
        }
    }
    None
}

/// One run: rich formatting (round 4, C3: underline/color/highlight/size/
/// super-subscript) plus inline images.
#[derive(Default, Clone)]
struct Run {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    link: Option<String>,
    color: String,
    highlight: String,
    size_half_pt: Option<f64>,
    vert: String,
    text: String,
    /// data-URI images collected for this run (drawing inside the run)
    images: Vec<String>,
}

impl Run {
    fn html(&self) -> String {
        let mut t = text_html(&self.text);
        for img in &self.images {
            t.push_str(&format!("<img class=\"doc-img\" src=\"{img}\" alt=\"image\"/>"));
        }
        if t.is_empty() {
            return t;
        }
        if !self.vert.is_empty() {
            t = format!(
                "<{}>{}</{}>",
                if self.vert == "superscript" { "sup" } else { "sub" },
                t,
                if self.vert == "superscript" { "sup" } else { "sub" }
            );
        }
        if !self.size_half_pt.is_none() {
            if let Some(sz) = self.size_half_pt {
                t = format!("<span style=\"font-size:{:.1}pt;\">{}</span>", sz / 2.0, t);
            }
        }
        if !self.color.is_empty() {
            t = format!("<span style=\"color:{};\">{}</span>", self.color, t);
        }
        if !self.highlight.is_empty() && self.highlight != "none" {
            t = format!(
                "<span style=\"background-color:{};\">{}</span>",
                highlight_color(&self.highlight),
                t
            );
        }
        if self.underline {
            t = format!("<u>{t}</u>");
        }
        if self.strike {
            t = format!("<s>{t}</s>");
        }
        if self.bold {
            t = format!("<b>{t}</b>");
        }
        if self.italic {
            t = format!("<i>{t}</i>");
        }
        if let Some(href) = &self.link {
            t = format!("<a href=\"{}\" class=\"doc-link\">{}</a>", esc(href), t);
        }
        t
    }
}

fn highlight_color(name: &str) -> String {
    match name {
        "yellow" => "#ffff00".into(),
        "green" => "#00ff00".into(),
        "cyan" => "#00ffff".into(),
        "magenta" => "#ff00ff".into(),
        "blue" => "#0000ff".into(),
        "red" => "#ff0000".into(),
        "black" => "#000000".into(),
        "white" => "#ffffff".into(),
        "darkBlue" => "#00008b".into(),
        "darkRed" => "#8b0000".into(),
        "darkGreen" => "#006400".into(),
        "gray" | "grey" => "#808080".into(),
        other => format!("#{}", other),
    }
}

/// Page geometry from the document's last `w:sectPr` (twips → px at 96dpi).
/// Used to render the preview as real pages (round 7, R5). Defaults to A4
/// with Word's default 1"/1.25" margins.
fn docx_page_style(xml: &str) -> String {
    let twip = 96.0 / 1440.0;
    let mut w = 11906.0f64;
    let mut h = 16838.0f64;
    let mut mar = (1440.0f64, 1800.0f64, 1440.0f64, 1800.0f64); // t r b l
    // scan every sectPr block; the last one wins (body-level section)
    let mut rest = xml;
    while let Some(pos) = rest.find("<w:sectPr") {
        // round 8: "<w:sectPrChange" also starts with "<w:sectPr" — require
        // the tag to actually be sectPr (next char '>' or whitespace)
        let after = rest[pos + "<w:sectPr".len()..].chars().next();
        if !matches!(after, Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('/')) {
            rest = &rest[pos + 1..];
            continue;
        }
        let end = rest[pos..]
            .find("</w:sectPr>")
            .map(|e| pos + e + "</w:sectPr>".len())
            .or_else(|| rest[pos..].find("/>").map(|e| pos + e + 2))
            .unwrap_or(rest.len());
        let block = &rest[pos..end.min(rest.len())];
        if let Some(p) = block.find("<w:pgSz") {
            let tag_end = block[p..].find('>').map(|e| p + e).unwrap_or(block.len());
            let tag = &block[p..tag_end];
            if let Some(v) = attr_str(tag, "w:w") {
                w = v.parse().unwrap_or(w);
            }
            if let Some(v) = attr_str(tag, "w:h") {
                h = v.parse().unwrap_or(h);
            }
        }
        if let Some(p) = block.find("<w:pgMar") {
            let tag_end = block[p..].find('>').map(|e| p + e).unwrap_or(block.len());
            let tag = &block[p..tag_end];
            let g = |k: &str, d: f64| attr_str(tag, k).and_then(|v| v.parse().ok()).unwrap_or(d);
            mar = (
                g("w:top", mar.0),
                g("w:right", mar.1),
                g("w:bottom", mar.2),
                g("w:left", mar.3),
            );
        }
        rest = &rest[end.min(rest.len())..];
    }
    // round 8: emit the page HEIGHT too — without a min-height the "pages"
    // were just width-constrained sheets, not page-shaped cards
    format!(
        "width:{:.0}px;min-height:{:.0}px;padding:{:.0}px {:.0}px {:.0}px {:.0}px;",
        w * twip,
        h * twip,
        mar.0 * twip,
        mar.1 * twip,
        mar.2 * twip,
        mar.3 * twip
    )
}

/// Read `w:key="value"` from a raw XML tag string.
fn attr_str(tag: &str, key: &str) -> Option<String> {
    let pat = format!("{key}=\"");
    let start = tag.find(&pat)? + pat.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// docx parser with images, TOC, and full run/paragraph formatting
/// (round 4, C3). `resolve_rel(rId) -> Option<(target, kind)>`,
/// `resolve_media(target) -> Option<(mime, bytes)>`.
/// Round 7 (R5): the body renders as `.doc-page` sheets (real page breaks
/// from `w:br w:type="page"` / `w:lastRenderedPageBreak` split the sheets;
/// page size/margins come from `w:sectPr`), and the generated TOC is
/// inserted at the TOP of the first page.
fn docx_xml_full(
    xml: &str,
    resolve_rel: impl Fn(&str) -> Option<(String, String)>,
    style_info: &std::collections::HashMap<String, (Option<u32>, bool)>,
    resolve_media: impl Fn(&str) -> Option<(String, Vec<u8>)>,
) -> String {
    let page_style = docx_page_style(xml);
    let page_open_tag = format!("<div class=\"doc-page\" style=\"{page_style}\">");
    let mut out = String::from("<div class=\"doc-render doc-paged\"><div class=\"doc-pages\">");
    out.push_str(&page_open_tag);
    // round 8: start offset of the currently open sheet — used to drop an
    // empty trailing page when the document ends with a page break
    let mut last_page_open = out.len() - page_open_tag.len();
    // insertion point for the generated TOC (top of the first page)
    let first_page_start = out.len();
    let mut page_break_pending = false;
    let mut list_open = false;
    let mut list_num = false;

    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    // paragraph state
    let mut para_style = String::new();
    let mut para_outline: Option<u32> = None;
    let mut para_list = false;
    let mut para_num = false;
    let mut para_html = String::new();
    let mut para_css = String::new();
    let mut in_para = false;

    let mut run = Run::default();
    let mut in_run = false;
    let mut link_id: Option<String> = None;
    let mut capture_text = false;
    let mut capture_instr = false;

    // drawing / blip capture
    let mut in_drawing = 0usize;
    let mut blip_rid = String::new();

    // TOC (round 4, C3): headings collected for a generated TOC when the
    // document carries a TOC field/SDT.
    let mut headings: Vec<(u32, String)> = Vec::new();
    let mut toc_marker = false;
    // round 11: insert the generated TOC where the TOC field/SDT actually
    // sits in the document (not forced to the top of the first page)
    let mut toc_insert_pos: Option<usize> = None;
    let mut heading_counter = 0usize;

    let mut in_table = false;
    let mut in_cell = false;
    let mut cell_html = String::new();
    let mut row_html = String::new();

    let mut blocks = 0usize;

    // round 11: a page break in the MIDDLE of a paragraph (e.g. heading +
    // break + image in one w:p) must split there — otherwise the image
    // renders on the old page and an empty sheet appears between them
    macro_rules! page_break_here {
        () => {
            if page_break_pending {
                if in_para && !in_cell && !para_html.is_empty() {
                    if list_open {
                        out.push_str(if list_num { "</ol>" } else { "</ul>" });
                        list_open = false;
                    }
                    out.push_str(&format!("<p style=\"{para_css}\">{para_html}</p>"));
                    out.push_str("</div>");
                    last_page_open = out.len();
                    out.push_str(&page_open_tag);
                    para_html.clear();
                    para_css.clear();
                    page_break_pending = false;
                } else if in_para && !in_cell {
                    // break-only paragraph: close the page after it
                } else {
                    // inside a table cell: defer to the next flush_para
                }
            }
        };
    }

    macro_rules! flush_para {
        () => {
            if in_para {
                if in_cell {
                    if !para_html.is_empty() {
                        if !cell_html.is_empty() {
                            cell_html.push_str("<br/>");
                        }
                        cell_html.push_str(&para_html);
                    }
                } else {
                    blocks += 1;
                    // round 6 (R11): TOC-entry paragraphs are replaced by the
                    // generated TOC at the top
                    let (style_level, is_toc_style) = style_info
                        .get(&para_style)
                        .cloned()
                        .unwrap_or((None, false));
                    let skip_para = is_toc_style && toc_marker;
                    if skip_para {
                        in_para = false;
                    }
                    // round 5 (R4) + round 6 (R11): direct w:outlineLvl wins,
                    // then the styles.xml level, then the style-name parse
                    let level = para_outline
                        .or(style_level)
                        .or_else(|| heading_level(&para_style));
                    if !skip_para {
                    if let Some(level) = level {
                        heading_counter += 1;
                        // remember plain text of the heading for the TOC
                        let plain: String = strip_tags(&para_html);
                        headings.push((level, plain.clone()));
                        if list_open {
                            out.push_str(if list_num { "</ol>" } else { "</ul>" });
                            list_open = false;
                        }
                        out.push_str(&format!(
                            "<h{level} id=\"doc-h-{heading_counter}\" style=\"{para_css}\">{para_html}</h{level}>"
                        ));
                    } else if para_list {
                        if !list_open {
                            out.push_str(if para_num { "<ol class=\"doc-list-num\">" } else { "<ul>" });
                            list_open = true;
                            list_num = para_num;
                        } else if list_num != para_num {
                            out.push_str(if list_num { "</ol>" } else { "</ul>" });
                            out.push_str(if para_num { "<ol class=\"doc-list-num\">" } else { "<ul>" });
                            list_num = para_num;
                        }
                        out.push_str(&format!(
                            "<li style=\"{para_css}\">{para_html}</li>"
                        ));
                    } else {
                        if list_open {
                            out.push_str(if list_num { "</ol>" } else { "</ul>" });
                            list_open = false;
                        }
                        if para_html.is_empty() {
                            out.push_str("<p class=\"doc-spacer\">&nbsp;</p>");
                        } else {
                            out.push_str(&format!("<p style=\"{para_css}\">{para_html}</p>"));
                        }
                    }
                    }
                }
                in_para = false;
                // round 7 (R5): an explicit page break at the end of this
                // paragraph closes the current sheet; the next content
                // opens a fresh one
                if page_break_pending && !in_cell {
                    out.push_str("</div>");
                    last_page_open = out.len();
                    out.push_str(&page_open_tag);
                    page_break_pending = false;
                }
            }
        };
    }

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "p" => {
                        para_style.clear();
                        para_outline = None;
                        para_list = false;
                        para_num = false;
                        para_html.clear();
                        para_css.clear();
                        in_para = true;
                    }
                    "r" => {
                        run = Run::default();
                        in_run = true;
                    }
                    "b" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.bold = true;
                        }
                    }
                    "i" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.italic = true;
                        }
                    }
                    "strike" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.strike = true;
                        }
                    }
                    "u" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.underline = true;
                        }
                    }
                    "color" if in_run => {
                        if let Some(c) = attr(&e, "val") {
                            if c != "auto" {
                                run.color = format!("#{}", c.trim_start_matches('#'));
                            }
                        }
                    }
                    "highlight" if in_run => {
                        if let Some(h) = attr(&e, "val") {
                            run.highlight = h;
                        }
                    }
                    "sz" if in_run => {
                        if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<f64>().ok()) {
                            run.size_half_pt = Some(v);
                        }
                    }
                    "vertAlign" if in_run => {
                        if let Some(v) = attr(&e, "val") {
                            run.vert = v;
                        }
                    }
                    "t" => capture_text = true,
                    "instrText" => capture_instr = true,
                    "tab" if in_run => run.text.push('\t'),
                    "br" if in_run => run.text.push('\n'),
                    "pStyle" if in_para => {
                        para_style = attr(&e, "val").unwrap_or_default();
                    }
                    "outlineLvl" if in_para => {
                        // round 5 (R4): w:outlineLvl is the reliable level
                        // (0-based) — Word sets it on every heading style
                        if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<u32>().ok()) {
                            para_outline = Some((v + 1).min(6));
                        }
                    }
                    "numPr" if in_para => {
                        para_list = true;
                    }
                    "numId" if in_para => {
                        if attr(&e, "val").as_deref() == Some("0") {
                            para_list = false;
                        }
                    }
                    "ilvl" if in_para => {}
                    "ind" if in_para => {
                        // round 4 (C3): paragraph indentation (twips → pt)
                        let left = attr(&e, "left").and_then(|v| v.parse::<f64>().ok());
                        let hanging = attr(&e, "hanging").and_then(|v| v.parse::<f64>().ok());
                        let first = attr(&e, "firstLine").and_then(|v| v.parse::<f64>().ok());
                        if let Some(l) = left {
                            let pt = l / 20.0;
                            if pt > 0.0 {
                                para_css.push_str(&format!("margin-left:{pt:.1}pt;"));
                            }
                            if let Some(h) = hanging {
                                if h > 0.0 {
                                    para_css.push_str(&format!("text-indent:-{:.1}pt;", h / 20.0));
                                }
                            }
                        }
                        if let Some(f) = first {
                            if f != 0.0 {
                                para_css.push_str(&format!("text-indent:{:.1}pt;", f / 20.0));
                            }
                        }
                    }
                    "jc" if in_para => {
                        match attr(&e, "val").as_deref() {
                            Some("center") => para_css.push_str("text-align:center;"),
                            Some("right") => para_css.push_str("text-align:right;"),
                            Some("both") => para_css.push_str("text-align:justify;"),
                            _ => {}
                        }
                    }
                    "spacing" if in_para => {
                        if let Some(b) = attr(&e, "before").and_then(|v| v.parse::<f64>().ok()) {
                            if b > 0.0 {
                                para_css.push_str(&format!("margin-top:{:.1}pt;", b / 20.0));
                            }
                        }
                        if let Some(a) = attr(&e, "after").and_then(|v| v.parse::<f64>().ok()) {
                            if a > 0.0 {
                                para_css.push_str(&format!("margin-bottom:{:.1}pt;", a / 20.0));
                            }
                        }
                    }
                    "hyperlink" => {
                        link_id = attr(&e, "id");
                    }
                    "drawing" => {
                        in_drawing += 1;
                        blip_rid.clear();
                    }
                    "blip" => {
                        if let Some(rid) = attr(&e, "embed").or_else(|| attr(&e, "link")) {
                            blip_rid = rid;
                        }
                    }
                    "imagedata" => {
                        if let Some(rid) = attr(&e, "id") {
                            blip_rid = rid;
                        }
                    }
                    "sdt" => {}
                    "docPartGallery" => {
                        if let Some(v) = attr(&e, "val") {
                            if is_toc_gallery(&v) {
                                toc_marker = true;
                                if toc_insert_pos.is_none() { toc_insert_pos = Some(out.len()); }
                            }
                        }
                    }
                    // round 8: a TOC inserted as a simple field
                    // (<w:fldSimple w:instr="TOC \o ...">) carries the
                    // instruction in an ATTRIBUTE — it never reaches the
                    // instrText arm below, so the TOC was silently skipped
                    "fldSimple" => {
                        if let Some(v) = attr(&e, "instr") {
                            if v.contains("TOC") {
                                toc_marker = true;
                                if toc_insert_pos.is_none() { toc_insert_pos = Some(out.len()); }
                            }
                        }
                    }
                    "tbl" if !in_cell => {
                        flush_para!();
                        if list_open {
                            out.push_str(if list_num { "</ol>" } else { "</ul>" });
                            list_open = false;
                        }
                        in_table = true;
                        out.push_str("<table class=\"doc-table\"><tbody>");
                    }
                    "tr" if in_table => row_html.clear(),
                    "tc" if in_table => {
                        in_cell = true;
                        cell_html.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "p" => {
                        flush_para!();
                        out.push_str("<p class=\"doc-spacer\">&nbsp;</p>");
                    }
                    "t" => {}
                    "instrText" => capture_instr = true,
                    "br" if in_run => {
                        // round 7 (R5): explicit page break starts a new sheet
                        if attr(&e, "type").as_deref() == Some("page") {
                            page_break_pending = true;
                            page_break_here!();
                        }
                        run.text.push('\n');
                    }
                    "lastRenderedPageBreak" if in_run => {
                        // Word's soft page break marker — render as a page too
                        page_break_pending = true;
                        page_break_here!();
                    }
                    "tab" if in_run => run.text.push('\t'),
                    "b" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.bold = true;
                        }
                    }
                    "i" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.italic = true;
                        }
                    }
                    "strike" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.strike = true;
                        }
                    }
                    "u" if in_run => {
                        if !attr(&e, "val").map(|v| is_zero_val(&v)).unwrap_or(false) {
                            run.underline = true;
                        }
                    }
                    "color" if in_run => {
                        if let Some(c) = attr(&e, "val") {
                            if c != "auto" {
                                run.color = format!("#{}", c.trim_start_matches('#'));
                            }
                        }
                    }
                    "highlight" if in_run => {
                        if let Some(h) = attr(&e, "val") {
                            run.highlight = h;
                        }
                    }
                    "sz" if in_run => {
                        if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<f64>().ok()) {
                            run.size_half_pt = Some(v);
                        }
                    }
                    "vertAlign" if in_run => {
                        if let Some(v) = attr(&e, "val") {
                            run.vert = v;
                        }
                    }
                    "pStyle" if in_para => {
                        para_style = attr(&e, "val").unwrap_or_default();
                    }
                    "outlineLvl" if in_para => {
                        if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<u32>().ok()) {
                            para_outline = Some((v + 1).min(6));
                        }
                    }
                    "numPr" if in_para => {
                        para_list = true;
                    }
                    "ind" if in_para => {
                        let left = attr(&e, "left").and_then(|v| v.parse::<f64>().ok());
                        if let Some(l) = left {
                            if l > 0.0 {
                                para_css.push_str(&format!("margin-left:{:.1}pt;", l / 20.0));
                            }
                        }
                    }
                    "jc" if in_para => {
                        match attr(&e, "val").as_deref() {
                            Some("center") => para_css.push_str("text-align:center;"),
                            Some("right") => para_css.push_str("text-align:right;"),
                            Some("both") => para_css.push_str("text-align:justify;"),
                            _ => {}
                        }
                    }
                    "blip" => {
                        if let Some(rid) = attr(&e, "embed").or_else(|| attr(&e, "link")) {
                            blip_rid = rid;
                        }
                    }
                    "imagedata" => {
                        if let Some(rid) = attr(&e, "id") {
                            blip_rid = rid;
                        }
                    }
                    "docPartGallery" => {
                        if let Some(v) = attr(&e, "val") {
                            if is_toc_gallery(&v) {
                                toc_marker = true;
                                if toc_insert_pos.is_none() { toc_insert_pos = Some(out.len()); }
                            }
                        }
                    }
                    "fldSimple" => {
                        if let Some(v) = attr(&e, "instr") {
                            if v.contains("TOC") {
                                toc_marker = true;
                                if toc_insert_pos.is_none() { toc_insert_pos = Some(out.len()); }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if capture_text && in_run {
                    run.text.push_str(&t.xml10_content());
                } else if capture_instr {
                    let instr = t.xml10_content();
                    if instr.contains("TOC") {
                        toc_marker = true;
                        if toc_insert_pos.is_none() { toc_insert_pos = Some(out.len()); }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = local_end(&e);
                match name.as_str() {
                    "t" => capture_text = false,
                    "instrText" => capture_instr = false,
                    "r" => {
                        if in_run {
                            if let Some(rid) = &link_id {
                                if let Some((target, kind)) = resolve_rel(rid) {
                                    if kind == "link" && !target.is_empty() {
                                        run.link = Some(target);
                                    }
                                }
                            }
                            let html = run.html();
                            if !html.is_empty() {
                                para_html.push_str(&html);
                            }
                            in_run = false;
                        }
                    }
                    "hyperlink" => link_id = None,
                    "drawing" => {
                        in_drawing = in_drawing.saturating_sub(1);
                        if in_drawing == 0 && !blip_rid.is_empty() {
                            // resolve the image through the rels table
                            if let Some((target, kind)) = resolve_rel(&blip_rid) {
                                if kind == "image" {
                                    if let Some((mime, bytes)) = resolve_media(&target) {
                                        run.images
                                            .push(format!("data:{mime};base64,{}", b64(&bytes)));
                                    }
                                }
                            }
                            blip_rid.clear();
                        }
                    }
                    "p" => flush_para!(),
                    "tc" if in_table => {
                        out_row_cell(&mut row_html, &cell_html);
                        in_cell = false;
                    }
                    "tr" if in_table => {
                        out.push_str(&format!("<tr>{row_html}</tr>"));
                    }
                    "tbl" if in_table => {
                        out.push_str("</tbody></table>");
                        in_table = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        if out.len() > HTML_CAP || blocks > BLOCK_CAP {
            break;
        }
    }
    if list_open {
        out.push_str(if list_num { "</ol>" } else { "</ul>" });
    }

    // Round 8: a page break as the very last content opened a fresh sheet
    // that stayed empty — drop it instead of rendering a blank trailing page.
    if out[last_page_open..].is_empty() && last_page_open > 0 {
        out.truncate(last_page_open);
    }

    // Round 7 (R5): close the last sheet and the page stack.
    out.push_str("</div></div></div>");

    // Round 4 (C3) + round 7 (R5): generate a real table of contents from the
    // headings when the document carries a TOC field/SDT (Word stores it as
    // a field that only updates inside Word — render it ourselves), and
    // insert it at the TOP of the first page (it used to land at the very
    // end of the document, where nobody ever saw it).
    if toc_marker && headings.len() >= 2 {
        let mut toc = String::from("<div class=\"doc-toc\"><div class=\"doc-toc-title\">Table of contents</div><ul>");
        let mut last_level = 1u32;
        for (i, (level, text)) in headings.iter().enumerate() {
            let lvl = *level;
            let pad = "&nbsp;&nbsp;".repeat((lvl.saturating_sub(last_level)).min(4) as usize);
            let _ = pad;
            let indent = (lvl.saturating_sub(1)) * 14;
            toc.push_str(&format!(
                "<li style=\"margin-left:{indent}px;\"><a href=\"#doc-h-{}\">{}</a></li>",
                i + 1,
                esc(text)
            ));
            last_level = lvl;
        }
        toc.push_str("</ul></div>");
        let at = toc_insert_pos.unwrap_or(first_page_start);
        out.insert_str(at, &toc);
    }
    out
}

/// Plain text of an html fragment (for TOC entries).
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// Backward-compatible wrapper used by the unit tests (no rels/media).
fn docx_xml(xml: &str) -> String {
    docx_xml_full(xml, |_| None, &std::collections::HashMap::new(), |_| None)
}

/// `word/styles.xml` → styleId → (outline level 1-based, is TOC style).
/// Follows one `basedOn` hop when the style itself has no outlineLvl.
fn docx_style_info(path: &Path) -> std::collections::HashMap<String, (Option<u32>, bool)> {
    let mut out = std::collections::HashMap::new();
    let Ok(xml) = read_zip_entry(path, "word/styles.xml") else {
        return out;
    };
    let xml = String::from_utf8_lossy(&xml).into_owned();
    // first pass: collect raw style defs
    struct Def {
        id: String,
        name: String,
        outline: Option<u32>,
        based_on: Option<String>,
    }
    let mut defs: Vec<Def> = Vec::new();
    {
        let mut reader = XmlReader::from_str(&xml);
        let mut buf = Vec::new();
        let mut cur: Option<Def> = None;
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let name = local(&e);
                    match name.as_str() {
                        "style" => {
                            cur = Some(Def {
                                id: attr(&e, "styleId").unwrap_or_default(),
                                name: String::new(),
                                outline: None,
                                based_on: None,
                            });
                        }
                        "name" => {
                            if let Some(d) = cur.as_mut() {
                                d.name = attr(&e, "val").unwrap_or_default();
                            }
                        }
                        "outlineLvl" => {
                            if let Some(d) = cur.as_mut() {
                                d.outline = attr(&e, "val").and_then(|v| v.parse::<u32>().ok());
                            }
                        }
                        "basedOn" => {
                            if let Some(d) = cur.as_mut() {
                                d.based_on = attr(&e, "val");
                            }
                        }
                        _ => {}
                    }
                }
                Ok(Event::End(e)) => {
                    if local_end(&e) == "style" {
                        if let Some(d) = cur.take() {
                            if !d.id.is_empty() {
                                defs.push(d);
                            }
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
    }
    for d in &defs {
        let lvl = d.outline.map(|v| (v + 1).min(6)).or_else(|| {
            // fall back to the name ("heading 2" → 2), then to basedOn
            let n = d.name.trim().to_ascii_lowercase();
            let norm: String = n.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            for prefix in ["heading", "naglowek", "nagłówek", "nagwek", "überschrift", "ueberschrift", "titre", "titolo", "encabezado"] {
                if let Some(rest) = norm.strip_prefix(prefix) {
                    if let Ok(n) = rest.parse::<u32>() {
                        if (1..=9).contains(&n) {
                            return Some(n.min(6));
                        }
                    }
                }
            }
            d.based_on
                .as_ref()
                .and_then(|b| defs.iter().find(|x| &x.id == b))
                .and_then(|b| b.outline.map(|v| (v + 1).min(6)))
        });
        let norm_name: String = d
            .name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        let is_toc = norm_name.contains("toc") || norm_name.contains("spistreci") || norm_name.contains("tableofcontents");
        out.insert(d.id.clone(), (lvl, is_toc));
    }
    out
}

fn out_row_cell(row_html: &mut String, cell_html: &str) {
    row_html.push_str(&format!("<td>{cell_html}</td>"));
}

// ---------------------------------------------------------------------------
// ODT
// ---------------------------------------------------------------------------

pub fn odt_to_html(path: &Path) -> Result<String, String> {
    let xml = read_zip_entry(path, "content.xml")?;
    let xml = String::from_utf8_lossy(&xml).into_owned();
    Ok(odf_body(&xml, "office:text", false))
}

pub fn odp_to_html(path: &Path) -> Result<String, String> {
    let xml = read_zip_entry(path, "content.xml")?;
    let xml = String::from_utf8_lossy(&xml).into_owned();
    Ok(odf_slides(&xml))
}

/// ODT text body → document html (headings, paragraphs, lists, tables).
fn odf_body(xml: &str, _body: &str, _slides: bool) -> String {
    let mut out = String::from("<div class=\"doc-render\">");
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut in_body = false;
    let mut capture: Option<u8> = None; // 1 = heading, 2 = paragraph
    let mut cap_text = String::new();
    let mut heading_level = 1u32;

    let mut list_depth = 0usize;
    let mut in_li = false;
    let mut table_depth = 0usize;
    let mut in_cell = false;
    let mut cell_html = String::new();
    let mut row_html = String::new();
    let mut blocks = 0usize;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "body" => in_body = true,
                    "h" if in_body => {
                        capture = Some(1);
                        cap_text.clear();
                        heading_level = attr(&e, "outline-level")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1)
                            .min(6);
                    }
                    "p" if in_body => {
                        capture = Some(2);
                        cap_text.clear();
                    }
                    "list" if in_body => {
                        list_depth += 1;
                        out.push_str("<ul>");
                    }
                    "list-item" if in_body => in_li = true,
                    "table" if in_body => {
                        table_depth += 1;
                        out.push_str("<table class=\"doc-table\"><tbody>");
                    }
                    "table-row" if table_depth > 0 => row_html.clear(),
                    "table-cell" if table_depth > 0 => {
                        in_cell = true;
                        cell_html.clear();
                    }
                    "s" if capture.is_some() => {
                        let n = attr(&e, "count")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        for _ in 0..n.min(32) {
                            cap_text.push(' ');
                        }
                    }
                    "tab" if capture.is_some() => cap_text.push('\t'),
                    "line-break" if capture.is_some() => cap_text.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if capture.is_some() {
                    cap_text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(e)) => {
                let name = local_end(&e);
                match name.as_str() {
                    "body" => in_body = false,
                    "h" if capture == Some(1) => {
                        blocks += 1;
                        out.push_str(&format!(
                            "<h{0}>{1}</h{0}>",
                            heading_level,
                            text_html(&cap_text)
                        ));
                        capture = None;
                    }
                    "p" if capture == Some(2) => {
                        blocks += 1;
                        if in_cell {
                            if !cap_text.is_empty() {
                                if !cell_html.is_empty() {
                                    cell_html.push_str("<br/>");
                                }
                                cell_html.push_str(&text_html(&cap_text));
                            }
                        } else if in_li && list_depth > 0 {
                            out.push_str(&format!("<li>{}</li>", text_html(&cap_text)));
                        } else {
                            if cap_text.is_empty() {
                                out.push_str("<p class=\"doc-spacer\">&nbsp;</p>");
                            } else {
                                out.push_str(&format!("<p>{}</p>", text_html(&cap_text)));
                            }
                        }
                        capture = None;
                    }
                    "list" if list_depth > 0 => {
                        list_depth -= 1;
                        out.push_str("</ul>");
                    }
                    "list-item" => in_li = false,
                    "table-cell" if table_depth > 0 => {
                        out_row_cell(&mut row_html, &cell_html);
                        in_cell = false;
                    }
                    "table-row" if table_depth > 0 => {
                        out.push_str(&format!("<tr>{row_html}</tr>"));
                    }
                    "table" if table_depth > 0 => {
                        table_depth -= 1;
                        out.push_str("</tbody></table>");
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        if out.len() > HTML_CAP || blocks > BLOCK_CAP {
            break;
        }
    }
    out.push_str("</div>");
    out
}

/// ODP presentation → slide cards.
fn odf_slides(xml: &str) -> String {
    let mut out = String::from("<div class=\"slides-view\">");
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut in_page = false;
    let mut page_num = 0usize;
    let mut title = String::new();
    let mut bullets: Vec<String> = Vec::new();
    let mut capture: Option<u8> = None; // 1 = h, 2 = p
    let mut cap_text = String::new();

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "page" => {
                        in_page = true;
                        page_num += 1;
                        title.clear();
                        bullets.clear();
                    }
                    "h" if in_page => {
                        capture = Some(1);
                        cap_text.clear();
                    }
                    "p" if in_page => {
                        capture = Some(2);
                        cap_text.clear();
                    }
                    "s" if capture.is_some() => {
                        let n = attr(&e, "count")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(1);
                        for _ in 0..n.min(32) {
                            cap_text.push(' ');
                        }
                    }
                    "line-break" if capture.is_some() => cap_text.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if capture.is_some() {
                    cap_text.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(e)) => {
                let name = local_end(&e);
                match name.as_str() {
                    "h" if capture == Some(1) => {
                        if title.is_empty() {
                            title = cap_text.trim().to_string();
                        } else if !cap_text.trim().is_empty() {
                            bullets.push(cap_text.trim().to_string());
                        }
                        capture = None;
                    }
                    "p" if capture == Some(2) => {
                        if !cap_text.trim().is_empty() {
                            if title.is_empty() {
                                title = cap_text.trim().to_string();
                            } else {
                                bullets.push(cap_text.trim().to_string());
                            }
                        }
                        capture = None;
                    }
                    "page" if in_page => {
                        in_page = false;
                        out.push_str(&slide_card(page_num, &title, &bullets));
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        if out.len() > HTML_CAP {
            break;
        }
    }
    out.push_str("</div>");
    out
}

fn slide_card(num: usize, title: &str, bullets: &[String]) -> String {
    let mut s = String::from("<div class=\"slide-card\"><div class=\"slide-num\">");
    s.push_str(&esc(&format!("{num}")));
    s.push_str("</div>");
    if !title.is_empty() {
        s.push_str(&format!("<h3 class=\"slide-title\">{}</h3>", esc(title)));
    }
    if !bullets.is_empty() {
        s.push_str("<ul>");
        for b in bullets.iter().take(30) {
            s.push_str(&format!("<li>{}</li>", esc(b)));
        }
        s.push_str("</ul>");
    }
    s.push_str("</div>");
    s
}

// ---------------------------------------------------------------------------
// PPTX
// ---------------------------------------------------------------------------

pub fn pptx_to_html(path: &Path) -> Result<String, String> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut slides: Vec<(u32, String)> = Vec::new();
    for i in 0..z.len() {
        let Ok(mut entry) = z.by_index(i) else {
            continue;
        };
        let Ok(name) = entry.name().map(|n| n.to_string()) else {
            continue;
        };
        let Some(num) = name
            .strip_prefix("ppt/slides/slide")
            .and_then(|r| r.strip_suffix(".xml"))
        else {
            continue;
        };
        let Ok(n) = num.parse::<u32>() else {
            continue;
        };
        let mut xml = String::new();
        entry.read_to_string(&mut xml).map_err(|e| e.to_string())?;
        slides.push((n, xml));
    }
    slides.sort_by_key(|(n, _)| *n);
    if slides.is_empty() {
        return Err("no slides found".into());
    }
    let total = slides.len();
    let mut out = String::from("<div class=\"slides-view\">");
    for (idx, (_n, xml)) in slides.iter().take(80).enumerate() {
        let paras = pptx_slide_paras(xml);
        let title = paras.iter().find(|p| !p.is_empty()).cloned().unwrap_or_default();
        let bullets: Vec<String> = paras
            .into_iter()
            .filter(|p| !p.is_empty() && p.as_str() != title)
            .take(30)
            .collect();
        out.push_str(&slide_card(idx + 1, &title, &bullets));
        if out.len() > HTML_CAP {
            break;
        }
    }
    out.push_str("</div>");
    let _ = total;
    Ok(out)
}

/// Paragraphs of one slide (a:t runs joined, a:br → newline).
fn pptx_slide_paras(xml: &str) -> Vec<String> {
    let mut paras: Vec<String> = Vec::new();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut in_p = false;
    let mut in_t = false;
    let mut cur = String::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "p" if !in_p => {
                        in_p = true;
                        cur.clear();
                    }
                    "p" => {} // a:p can't nest; ignore
                    "t" => in_t = true,
                    "br" if in_p => cur.push('\n'),
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                if in_t && in_p {
                    cur.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(e)) => {
                let name = local_end(&e);
                match name.as_str() {
                    "t" => in_t = false,
                    "p" if in_p => {
                        in_p = false;
                        paras.push(cur.clone());
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        if paras.len() > 200 {
            break;
        }
    }
    paras
}

// ---------------------------------------------------------------------------
// spreadsheets
// ---------------------------------------------------------------------------

fn cell_string(data: &calamine::Data) -> String {
    match data {
        calamine::Data::Empty => String::new(),
        calamine::Data::String(s) => s.clone(),
        calamine::Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        calamine::Data::Int(i) => i.to_string(),
        calamine::Data::DateTime(d) => d.to_string(),
        calamine::Data::DateTimeIso(s) => s.clone(),
        calamine::Data::DurationIso(s) => s.clone(),
        calamine::Data::Bool(b) => b.to_string(),
        calamine::Data::Error(e) => format!("{e:?}"),
    }
}

pub fn xlsx_sheets(path: &Path) -> Result<Vec<SheetData>, String> {
    use calamine::Reader;
    let mut workbook =
        calamine::open_workbook_auto(path).map_err(|e| format!("workbook: {e}"))?;
    let names = workbook.sheet_names().to_vec();
    // Round 4 (C4) + round 5 (R6): workbook cell styles (xl/styles.xml +
    // theme palette) so the preview keeps bold/italic/colors/fills AND
    // number formats instead of a plain table.
    let style_table: Vec<XlsxStyle> = xlsx_style_table(path).unwrap_or_default();
    let mut out: Vec<SheetData> = Vec::new();
    for name in names {
        if out.len() >= SHEET_MAX {
            break;
        }
        let range = match workbook.worksheet_range(&name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let geom = xlsx_sheet_geom(path, &name).unwrap_or_default();
        let rows: Vec<Vec<String>> = range
            .rows()
            .take(SHEET_MAX_ROWS)
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .take(SHEET_MAX_COLS)
                    .enumerate()
                    .map(|(c, data)| {
                        let numfmt = geom
                            .styles
                            .get(r)
                            .and_then(|rr| rr.get(c))
                            .copied()
                            .flatten()
                            .and_then(|i| style_table.get(i))
                            .map(|st| st.numfmt.as_str())
                            .unwrap_or("");
                        format_cell(data, numfmt)
                    })
                    .collect()
            })
            .collect();
        let styles: Vec<Vec<Option<String>>> = rows
            .iter()
            .enumerate()
            .map(|(r, row)| {
                row.iter()
                    .enumerate()
                    .map(|(c, _)| {
                        geom.styles
                            .get(r)
                            .and_then(|rr| rr.get(c))
                            .copied()
                            .flatten()
                            .and_then(|i| style_table.get(i))
                            .map(|st| st.css.clone())
                    })
                    .collect()
            })
            .collect();
        out.push(SheetData { name, rows, styles, merges: geom.merges, col_widths: geom.col_widths });
    }
    if out.is_empty() {
        return Err("no sheets with data".into());
    }
    Ok(out)
}

/// One cellXfs entry: inline CSS + the number-format code (for display).
struct XlsxStyle {
    css: String,
    numfmt: String,
}

/// Apply a number-format code to a calamine value (round 5, R6): percent,
/// fixed decimals, thousands separator, currency prefix/suffix. Date
/// formats are left to calamine (it already converts them to DateTime).
fn format_cell(data: &calamine::Data, numfmt: &str) -> String {
    let f = match data {
        calamine::Data::Float(f) => Some(*f),
        calamine::Data::Int(i) => Some(*i as f64),
        _ => None,
    };
    let Some(v) = f else {
        return cell_string(data);
    };
    let code = numfmt.trim_matches('"').to_string();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return cell_string(data);
    }
    // builtin ids
    let code = match code.as_str() {
        "9" => "0%".to_string(),
        "10" => "0.00%".to_string(),
        "1" => "0".to_string(),
        "2" => "0.00".to_string(),
        "3" => "#,##0".to_string(),
        "4" => "#,##0.00".to_string(),
        "11" => "0.00E+00".to_string(),
        "37" => "#,##0;-#,##0".to_string(),
        "39" => "#,##0.00;-#,##0.00".to_string(),
        "44" | "42" | "43" => code,
        other => other.to_string(),
    };
    let is_pct = code.contains('%');
    let has_thousands = code.contains("#,##0");
    // decimal places from the pattern after '.'
    let decimals = code
        .split('.')
        .nth(1)
        .and_then(|frac| {
            let digits: String = frac.chars().take_while(|c| *c == '0' || *c == '#').collect();
            if digits.is_empty() { None } else { Some(digits.chars().filter(|c| *c == '0').count()) }
        })
        .unwrap_or(if has_thousands || code.contains("0.") { 2 } else { 0 });
    let val = if is_pct { v * 100.0 } else { v };
    let mut num = if decimals > 0 {
        format!("{:.*}", decimals, val)
    } else {
        format!("{:.0}", val)
    };
    if has_thousands {
        // insert thousand separators into the integer part
        let (int_part, frac_part) = match num.split_once('.') {
            Some((i, f)) => (i.to_string(), format!(".{f}")),
            None => (num.clone(), String::new()),
        };
        let neg = int_part.starts_with('-');
        let digits = int_part.trim_start_matches('-');
        let mut grouped = String::new();
        for (i, ch) in digits.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        let grouped: String = grouped.chars().rev().collect();
        num = format!("{}{}{}", if neg { "-" } else { "" }, grouped, frac_part);
    }
    // currency / literal prefix-suffix from the code (strip quoted literals)
    let prefix: String = code.chars().take_while(|c| !c.is_ascii_digit() && *c != '0' && *c != '#' && *c != '.' && *c != '%' && *c != ',').collect::<String>()
        .trim_matches(|c: char| c == '"' || c == '\\' || c == ' ')
        .to_string();
    let suffix = if is_pct { "%" } else { "" };
    let prefix = prefix.replace('"', "");
    // round 5 (R6): numeric patterns (decimals/thousands/percent/currency)
    // always format the value — not only when a literal prefix exists
    if is_pct || has_thousands || decimals > 0 || !prefix.is_empty() {
        return format!("{}{}{}", prefix, num, suffix);
    }
    cell_string(data)
}

// ---------------------------------------------------------------------------
// XLSX styles (xl/styles.xml + per-cell `s` attribute) — round 4 (C4)
// ---------------------------------------------------------------------------

/// `cellXfs` index → style (inline CSS + number format). Round 5 (R6):
/// theme colors (xl/theme/theme1.xml), indexed colors and tint are
/// resolved, so formatted tables keep their real background/font colors.
fn xlsx_style_table(path: &Path) -> Option<Vec<XlsxStyle>> {
    let xml = read_zip_entry(path, "xl/styles.xml").ok()?;
    let xml = String::from_utf8_lossy(&xml).into_owned();
    let theme = xlsx_theme_palette(path).unwrap_or_default();
    let mut indexed: Vec<String> = xlsx_indexed_colors().into_iter().map(|c| c).collect();
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();

    let mut numfmts: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut fonts: Vec<(bool, bool, String, f64)> = Vec::new(); // (bold, italic, color, size)
    let mut fills: Vec<String> = Vec::new(); // css background-color
    let mut cellxfs: Vec<XlsxStyle> = Vec::new();

    let mut section = String::new();
    let mut cur_font = (false, false, String::new(), 11.0);
    let mut cur_fill = String::new();
    let mut in_font = false;
    let mut in_fill = false;
    let mut in_indexed = false;

    // resolve a color element's rgb/theme/indexed/tint into a css color
    fn resolve_color(e: &BytesStart, theme: &[String], indexed: &[String]) -> String {
        let rgb = attr(e, "rgb");
        let theme_i = attr(e, "theme").and_then(|v| v.parse::<usize>().ok());
        let indexed_i = attr(e, "indexed").and_then(|v| v.parse::<usize>().ok());
        let tint = attr(e, "tint").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let base = if let Some(rgb) = rgb {
            xlsx_color(&rgb)
        } else if let Some(t) = theme_i {
            // OOXML theme attr: 0/1 are swapped dk1/lt1, then the rest in order
            let idx = match t {
                0 => 1,
                1 => 0,
                n => n,
            };
            theme.get(idx).cloned().unwrap_or_default()
        } else if let Some(i) = indexed_i {
            indexed.get(i).cloned().unwrap_or_default()
        } else {
            String::new()
        };
        apply_tint(&base, tint)
    }

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "numFmts" => section = "numfmts".into(),
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attr(&e, "numFmtId").and_then(|v| v.parse::<u32>().ok()),
                            attr(&e, "formatCode"),
                        ) {
                            numfmts.insert(id, code);
                        }
                    }
                    "fonts" => section = "fonts".into(),
                    "font" => {
                        in_font = true;
                        cur_font.0 = false;
                        cur_font.1 = false;
                        cur_font.2 = String::new();
                        cur_font.3 = 11.0;
                    }
                    "b" if in_font => cur_font.0 = true,
                    "i" if in_font => cur_font.1 = true,
                    "sz" if in_font => {
                        if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<f64>().ok()) {
                            cur_font.3 = v;
                        }
                    }
                    "color" if in_font => {
                        let c = resolve_color(&e, &theme, &indexed);
                        if !c.is_empty() {
                            cur_font.2 = c;
                        }
                    }
                    "fills" => section = "fills".into(),
                    "fill" => {
                        in_fill = true;
                        cur_fill = String::new();
                    }
                    "fgColor" if in_fill => {
                        let c = resolve_color(&e, &theme, &indexed);
                        if !c.is_empty() {
                            cur_fill = c;
                        }
                    }
                    "indexedColors" => {
                        in_indexed = true;
                        section = "indexed".into();
                    }
                    "indexedColor" if in_indexed => {
                        let c = resolve_color(&e, &theme, &indexed);
                        if !c.is_empty() {
                            indexed.push(c);
                        }
                    }
                    "cellXfs" => section = "cellxfs".into(),
                    "xf" if section == "cellxfs" => {
                        let font_id = attr(&e, "fontId").and_then(|v| v.parse::<usize>().ok());
                        let fill_id = attr(&e, "fillId").and_then(|v| v.parse::<usize>().ok());
                        let numfmt_id = attr(&e, "numFmtId")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(0);
                        let numfmt = numfmts
                            .get(&numfmt_id)
                            .cloned()
                            .unwrap_or_else(|| builtin_numfmt(numfmt_id));
                        let mut css = String::new();
                        if let Some(f) = font_id.and_then(|i| fonts.get(i)) {
                            if f.0 {
                                css.push_str("font-weight:bold;");
                            }
                            if f.1 {
                                css.push_str("font-style:italic;");
                            }
                            if !f.2.is_empty() {
                                css.push_str(&format!("color:{};", f.2));
                            }
                            if (f.3 - 11.0).abs() > 0.01 {
                                css.push_str(&format!("font-size:{:.0}pt;", f.3));
                            }
                        }
                        if let Some(fl) = fill_id.and_then(|i| fills.get(i)) {
                            if !fl.is_empty() && fl != "#FFFFFF" {
                                css.push_str(&format!("background-color:{};", fl));
                            }
                        }
                        cellxfs.push(XlsxStyle { css, numfmt });
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attr(&e, "numFmtId").and_then(|v| v.parse::<u32>().ok()),
                            attr(&e, "formatCode"),
                        ) {
                            numfmts.insert(id, code);
                        }
                    }
                    "b" if in_font => cur_font.0 = true,
                    "i" if in_font => cur_font.1 = true,
                    "sz" if in_font => {
                        if let Some(v) = attr(&e, "val").and_then(|v| v.parse::<f64>().ok()) {
                            cur_font.3 = v;
                        }
                    }
                    "color" if in_font => {
                        let c = resolve_color(&e, &theme, &indexed);
                        if !c.is_empty() {
                            cur_font.2 = c;
                        }
                    }
                    "fgColor" if in_fill => {
                        let c = resolve_color(&e, &theme, &indexed);
                        if !c.is_empty() {
                            cur_fill = c;
                        }
                    }
                    "indexedColor" if in_indexed => {
                        let c = resolve_color(&e, &theme, &indexed);
                        if !c.is_empty() {
                            indexed.push(c);
                        }
                    }
                    "xf" if section == "cellxfs" => {
                        let font_id = attr(&e, "fontId").and_then(|v| v.parse::<usize>().ok());
                        let fill_id = attr(&e, "fillId").and_then(|v| v.parse::<usize>().ok());
                        let numfmt_id = attr(&e, "numFmtId")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(0);
                        let numfmt = numfmts
                            .get(&numfmt_id)
                            .cloned()
                            .unwrap_or_else(|| builtin_numfmt(numfmt_id));
                        let mut css = String::new();
                        if let Some(f) = font_id.and_then(|i| fonts.get(i)) {
                            if f.0 {
                                css.push_str("font-weight:bold;");
                            }
                            if f.1 {
                                css.push_str("font-style:italic;");
                            }
                            if !f.2.is_empty() {
                                css.push_str(&format!("color:{};", f.2));
                            }
                            if (f.3 - 11.0).abs() > 0.01 {
                                css.push_str(&format!("font-size:{:.0}pt;", f.3));
                            }
                        }
                        if let Some(fl) = fill_id.and_then(|i| fills.get(i)) {
                            if !fl.is_empty() && fl != "#FFFFFF" {
                                css.push_str(&format!("background-color:{};", fl));
                            }
                        }
                        cellxfs.push(XlsxStyle { css, numfmt });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = local_end(&e);
                match name.as_str() {
                    "font" if in_font => {
                        fonts.push(cur_font.clone());
                        in_font = false;
                    }
                    "fill" if in_fill => {
                        fills.push(cur_fill.clone());
                        in_fill = false;
                    }
                    "indexedColors" => in_indexed = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    Some(cellxfs)
}

/// Builtin number-format ids → format code.
fn builtin_numfmt(id: u32) -> String {
    match id {
        1 => "0".into(),
        2 => "0.00".into(),
        3 => "#,##0".into(),
        4 => "#,##0.00".into(),
        9 => "0%".into(),
        10 => "0.00%".into(),
        11 => "0.00E+00".into(),
        14 => "m/d/yyyy".into(),
        37 => "#,##0;-#,##0".into(),
        38 => "#,##0;[Red]-#,##0".into(),
        39 => "#,##0.00;-#,##0.00".into(),
        40 => "#,##0.00;[Red]-#,##0.00".into(),
        44 => "$#,##0.00".into(),
        49 => "@".into(),
        _ => String::new(),
    }
}

/// The standard indexed color palette (styles.xml `indexedColors` default).
fn xlsx_indexed_colors() -> Vec<String> {
    let raw = [
        "FF000000", "FFFFFFFF", "FFFF0000", "FF00FF00", "FF0000FF", "FFFFFF00", "FFFF00FF",
        "FF00FFFF", "FF000000", "FFFFFFFF", "FFFF0000", "FF00FF00", "FF0000FF", "FFFFFF00",
        "FFFF00FF", "FF00FFFF", "FF800000", "FF008000", "FF000080", "FF808000", "FF800080",
        "FF008080", "FFC0C0C0", "FF808080", "FF9999FF", "FF993366", "FFFFFFCC", "FFCCFFFF",
        "FF660066", "FFFF8080", "FF0066CC", "FFCCCCFF", "FF000080", "FFFF00FF", "FFFFFF00",
        "FF00FFFF", "FF800080", "FF800000", "FF008080", "FF0000FF", "FF00CCFF", "FFCCFFFF",
        "FFCCFFCC", "FFFFFF99", "FF99CCFF", "FFFF99CC", "FFCC99FF", "FFFFCC99", "FF3366FF",
        "FF33CCCC", "FF99CC00", "FFFFCC00", "FFFF9900", "FFFF6600", "FF666699", "FF969696",
        "FF003366", "FF339966", "FF003300", "FF333300", "FF993300", "FF993366", "FF333399",
        "FF333333",
    ];
    raw.iter().map(|c| xlsx_color(c)).collect()
}

/// `xl/theme/theme1.xml` clrScheme → css colors in theme-index order.
fn xlsx_theme_palette(path: &Path) -> Option<Vec<String>> {
    let xml = read_zip_entry(path, "xl/theme/theme1.xml").ok()?;
    let xml = String::from_utf8_lossy(&xml).into_owned();
    let mut out: Vec<String> = Vec::new();
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    let mut in_scheme = false;
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = local(&e);
                if name == "clrScheme" {
                    in_scheme = true;
                } else if in_scheme && name.ends_with("Clr") {
                    // dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink
                    if let Some(srgb) = attr(&e, "srgbClr").or_else(|| attr(&e, "val")) {
                        // sysClr has val="windowText" etc — map the two system ones
                        if srgb.starts_with("FF") || srgb.len() == 6 {
                            out.push(xlsx_color(&srgb));
                        } else if let Some(sys) = attr(&e, "lastClr") {
                            out.push(xlsx_color(&sys));
                        }
                    } else if let Some(last) = attr(&e, "lastClr") {
                        out.push(xlsx_color(&last));
                    }
                }
            }
            Ok(Event::End(e)) => {
                if local_end(&e) == "clrScheme" {
                    in_scheme = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Apply an OOXML tint to a #RRGGBB color (tint>0 lightens, <0 darkens).
fn apply_tint(hex: &str, tint: f64) -> String {
    if tint == 0.0 || hex.len() != 7 {
        return hex.to_string();
    }
    let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(0);
    let apply = |c: u8| -> u8 {
        if tint > 0.0 {
            (c as f64 + (255.0 - c as f64) * tint).round() as u8
        } else {
            (c as f64 * (1.0 + tint)).round() as u8
        }
    };
    format!("#{:02X}{:02X}{:02X}", apply(r), apply(g), apply(b))
}

/// `FFRRGGBB`/`RRGGBB` ARGB → `#RRGGBB` css color.
fn xlsx_color(rgb: &str) -> String {
    let h = rgb.trim_start_matches('#');
    let (r, g, b) = if h.len() == 8 {
        (&h[2..4], &h[4..6], &h[6..8])
    } else if h.len() == 6 {
        (&h[0..2], &h[2..4], &h[4..6])
    } else {
        return String::new();
    };
    format!("#{}{}{}", r, g, b)
}

/// Geometry of one sheet: per-cell style indices + merged ranges + column
/// widths (round 6, R11 — formatted tables keep their merges/widths).
#[derive(Default)]
struct SheetGeom {
    styles: Vec<Vec<Option<usize>>>,
    merges: Vec<[usize; 4]>,
    col_widths: Vec<f64>,
}

/// Sheet name → `xl/worksheets/sheetN.xml`, then per-cell style indices.
fn xlsx_sheet_geom(path: &Path, sheet_name: &str) -> Option<SheetGeom> {
    let wb = read_zip_entry(path, "xl/workbook.xml").ok()?;
    let wb = String::from_utf8_lossy(&wb).into_owned();
    let rels = read_zip_entry(path, "xl/_rels/workbook.xml.rels").ok()?;
    let rels = String::from_utf8_lossy(&rels).into_owned();

    // rId for the sheet name
    let mut rid = String::new();
    {
        let mut reader = XmlReader::from_str(&wb);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    if local(&e) == "sheet" {
                        let name = attr(&e, "name").unwrap_or_default();
                        if name == sheet_name {
                            rid = attr(&e, "id")
                                .or_else(|| attr(&e, "r:id"))
                                .unwrap_or_default();
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
    }
    if rid.is_empty() {
        return None;
    }
    // rid → target
    let mut target = String::new();
    {
        let mut reader = XmlReader::from_str(&rels);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    if local(&e) == "Relationship" {
                        if attr(&e, "Id").as_deref() == Some(rid.as_str()) {
                            target = attr(&e, "Target").unwrap_or_default();
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
        }
    }
    if target.is_empty() {
        return None;
    }
    let sheet_path = if target.starts_with('/') {
        target.trim_start_matches('/').to_string()
    } else {
        format!("xl/{target}")
    };
    let xml = read_zip_entry(path, &sheet_path).ok()?;
    let xml = String::from_utf8_lossy(&xml).into_owned();

    let mut geom = SheetGeom::default();
    let mut in_cols = false;
    let mut reader = XmlReader::from_str(&xml);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = local(&e);
                match name.as_str() {
                    "c" => {
                        if let (Some(r), Some(s)) = (attr(&e, "r"), attr(&e, "s")) {
                            if let (Some((row, col)), Some(si)) = (cell_ref(&r), s.parse::<usize>().ok()) {
                                while geom.styles.len() <= row {
                                    geom.styles.push(Vec::new());
                                }
                                while geom.styles[row].len() <= col {
                                    geom.styles[row].push(None);
                                }
                                geom.styles[row][col] = Some(si);
                            }
                        }
                    }
                    "mergeCell" => {
                        if let Some(r) = attr(&e, "ref") {
                            // "A1:F1" or "A1"
                            let mut parts = r.split(':');
                            let a = parts.next().unwrap_or("");
                            let b = parts.next().unwrap_or(a);
                            if let (Some((r1, c1)), Some((r2, c2))) = (cell_ref(a), cell_ref(b)) {
                                geom.merges.push([r1, c1, r2, c2]);
                            }
                        }
                    }
                    "cols" => in_cols = true,
                    "col" if in_cols => {
                        let min = attr(&e, "min").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
                        let max = attr(&e, "max").and_then(|v| v.parse::<usize>().ok()).unwrap_or(min);
                        let w = attr(&e, "width").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                        for c in min..=max {
                            while geom.col_widths.len() < c {
                                geom.col_widths.push(0.0);
                            }
                            geom.col_widths[c - 1] = w;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                if local_end(&e) == "cols" {
                    in_cols = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    Some(geom)
}

/// `B12` → (row=11, col=1) zero-based.
fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let mut col = 0usize;
    let mut row = 0usize;
    let mut seen_digit = false;
    for ch in r.chars() {
        if ch.is_ascii_alphabetic() && !seen_digit {
            col = col * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize + 1);
        } else if ch.is_ascii_digit() {
            seen_digit = true;
            row = row * 10 + (ch as usize - '0' as usize);
        } else {
            return None;
        }
    }
    if col == 0 || row == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

// ---------------------------------------------------------------------------
// ZIP listing (Nexus FilePreview ZipView): entries sorted by name,
// directory records skipped.
pub fn zip_listing(path: &Path) -> Result<Vec<crate::fsgit::ZipEntry>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for i in 0..z.len() {
        let entry = z.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        out.push(crate::fsgit::ZipEntry {
            name: entry.name().map(|n| n.to_string()).unwrap_or_default(),
            size: entry.size(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

// PDF (pdfium, bundled next to the executable)
// ---------------------------------------------------------------------------

pub fn pdf_pages(path: &Path) -> Result<Vec<String>, String> {
    let pdfium = pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("pdfium: {e}"))?;
    let pages = document.pages();
    let total = pages.len() as usize;
    let take = total.min(PDF_MAX_PAGES);
    let mut out: Vec<String> = Vec::new();
    for page in pages.iter().take(take) {
        let config = pdfium_render::prelude::PdfRenderConfig::new()
            .set_target_width(PDF_PAGE_WIDTH)
            .clear_before_rendering(true)
            .set_clear_color(pdfium_render::prelude::PdfColor::WHITE);
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| format!("render: {e}"))?;
        let (w, h) = (bitmap.width(), bitmap.height());
        let rgba = bitmap.as_rgba_bytes();
        let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba.to_vec())
            .ok_or_else(|| "bad bitmap".to_string())?;
        let mut png: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        out.push(format!("data:image/png;base64,{}", b64(png.as_slice())));
    }
    Ok(out)
}

fn pdfium() -> Result<&'static pdfium_render::prelude::Pdfium, String> {
    static PDFIUM: std::sync::OnceLock<Result<pdfium_render::prelude::Pdfium, String>> =
        std::sync::OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            use pdfium_render::prelude::*;
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_heading_and_bold() {
        let xml = "<w:body><w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>\
<w:r><w:t>Hi</w:t></w:r></w:p>\
<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r></w:p></w:body>";
        let html = docx_xml(xml);
        // headings carry a TOC anchor id + style attr now (round 4, C3)
        assert!(html.contains("<h1 id=\"doc-h-1\"") && html.contains(">Hi</h1>"), "{html}");
        assert!(html.contains("<b>bold</b>"), "{html}");
    }

    #[test]
    fn docx_table_and_list() {
        let xml = "<w:body><w:tbl><w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc>\
<w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
<w:p><w:pPr><w:numPr/></w:pPr><w:r><w:t>item</w:t></w:r></w:p></w:body>";
        let html = docx_xml(xml);
        assert!(html.contains("<table class=\"doc-table\">"), "{html}");
        assert!(html.contains("<td>a</td>"), "{html}");
        assert!(html.contains("<ul>") && html.contains("<li") && html.contains(">item</li>"), "{html}");
    }

    #[test]
    fn odt_paragraphs_and_lists() {
        let xml = "<office:body><office:text><text:h text:outline-level=\"2\">T</text:h>\
<text:p>hello</text:p><text:list><text:list-item><text:p>x</text:p></text:list-item></text:list>\
</office:text></office:body>";
        let html = odf_body(xml, "", false);
        assert!(html.contains("<h2>T</h2>"), "{html}");
        assert!(html.contains("<p>hello</p>"), "{html}");
        assert!(html.contains("<ul><li>x</li></ul>"), "{html}");
    }

    #[test]
    fn pptx_slide_paragraphs() {
        let xml = "<p:sld><p:sp><p:txBody><a:p><a:r><a:t>Title</a:t></a:r></a:p>\
<a:p><a:r><a:t>One</a:t></a:r></a:p><a:p><a:r><a:t>Two</a:t></a:r></a:p></p:txBody></p:sp></p:sld>";
        let paras = pptx_slide_paras(xml);
        assert_eq!(paras, vec!["Title", "One", "Two"]);
    }

    #[test]
    fn odt_table_cells() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-content xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\"><office:body><office:text><text:h text:outline-level=\"1\">ODT Title</text:h><text:p>Hello from ODT</text:p><text:list><text:list-item><text:p>bullet a</text:p></text:list-item><text:list-item><text:p>bullet b</text:p></text:list-item></text:list><table:table table:name=\"T1\"><table:table-row><table:table-cell><text:p>c1</text:p></table:table-cell><table:table-cell><text:p>c2</text:p></table:table-cell></table:table-row></table:table></office:text></office:body></office:document-content>";
        let html = odf_body(xml, "", false);
        assert!(html.contains("<td>c1</td>") && html.contains("<td>c2</td>"), "{html}");
    }

    #[test]
    fn docx_realworld_table() {
        let xml = "<w:body><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Value</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body>";
        let html = docx_xml(xml);
        assert!(html.contains("<td>Name</td>") && html.contains("<td>Value</td>"), "{html}");
    }

    #[test]
    fn zip_listing_skips_dirs_and_sorts() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join("vd-zip-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.zip");
        let f = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opt = zip::write::SimpleFileOptions::default();
        w.add_directory("sub/", opt).unwrap();
        w.start_file("b.txt", opt).unwrap();
        w.write_all(b"hello world").unwrap();
        w.start_file("sub/a.txt", opt).unwrap();
        w.write_all(b"aaa").unwrap();
        drop(w);
        let out = zip_listing(&path).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "b.txt");
        assert_eq!(out[0].size, 11);
        assert_eq!(out[1].name, "sub/a.txt");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn heading_level_localized_styles() {
        // round 8: ASCII-folding makes the localized prefixes reachable
        assert_eq!(heading_level("Nagłówek 2"), Some(2));
        assert_eq!(heading_level("Überschrift 3"), Some(3));
        assert_eq!(heading_level("Heading 1"), Some(1));
        assert_eq!(heading_level("Tytuł"), Some(1));
        assert_eq!(heading_level("Normal"), None);
    }

    #[test]
    fn docx_toc_from_fld_simple() {
        // round 8: a TOC inserted as <w:fldSimple w:instr="TOC …"> carries the
        // instruction in an attribute — it must trigger the generated TOC
        let xml = "<w:body>\
<w:p><w:r><w:fldSimple w:instr=\"TOC \\o 1-3\"><w:r><w:t>x</w:t></w:r></w:fldSimple></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>One</w:t></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>Two</w:t></w:r></w:p>\
</w:body>";
        let html = docx_xml(xml);
        assert!(html.contains("doc-toc"), "{html}");
        assert!(html.contains(">One</a>") && html.contains(">Two</a>"), "{html}");
        // TOC sits at the TOP of the first page, not at the end
        let toc_pos = html.find("doc-toc").unwrap();
        let h1_pos = html.find("<h1").unwrap();
        assert!(toc_pos < h1_pos, "{html}");
    }

    #[test]
    fn docx_toc_from_localized_gallery() {
        // round 8: Polish Word writes w:val="Spis treści"
        let xml = "<w:body>\
<w:sdt><w:sdtPr><w:docPartGallery w:val=\"Spis treści\"/></w:sdtPr></w:sdt>\
<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>A</w:t></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t>B</w:t></w:r></w:p>\
</w:body>";
        let html = docx_xml(xml);
        assert!(html.contains("doc-toc"), "{html}");
    }

    #[test]
    fn docx_mid_paragraph_page_break_splits_in_place() {
        // round 11: heading + page break + image in ONE paragraph must not
        // leave an empty sheet between them
        let xml = "<w:body>\
<w:p><w:r><w:t>Heading</w:t></w:r>\
<w:r><w:br w:type=\"page\"/></w:r>\
<w:r><w:t>after break</w:t></w:r></w:p>\
</w:body>";
        let html = docx_xml(xml);
        assert_eq!(html.matches("class=\"doc-page\"").count(), 2, "{html}");
        // the text after the break lands on the SECOND page, not before it
        let second = html.rsplit("class=\"doc-page\"").next().unwrap();
        assert!(second.contains("after break"), "{html}");
        assert!(!second.contains("Heading"), "{html}");
    }

    #[test]
    fn docx_toc_inserted_at_field_position() {
        // round 11: the generated TOC goes where the TOC field sits, not
        // forced to the very top
        let xml = "<w:body>\
<w:p><w:r><w:t>Cover page</w:t></w:r></w:p>\
<w:sdt><w:sdtPr><w:docPartGallery w:val=\"Table of Contents\"/></w:sdtPr></w:sdt>\
<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>A</w:t></w:r></w:p>\
<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t>B</w:t></w:r></w:p>\
</w:body>";
        let html = docx_xml(xml);
        assert!(html.contains("doc-toc"), "{html}");
        let cover = html.find("Cover page").unwrap();
        let toc = html.find("doc-toc").unwrap();
        assert!(cover < toc, "TOC must come after the cover paragraph: {html}");
    }

    #[test]
    fn docx_page_has_min_height_and_no_empty_trailing_page() {
        // round 8: pages are page-shaped (min-height from w:pgSz w:h)
        let style = docx_page_style("<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
<w:pgMar w:top=\"1440\" w:right=\"1800\" w:bottom=\"1440\" w:left=\"1800\"/></w:sectPr>");
        assert!(style.contains("min-height:"), "{style}");
        assert!(style.contains("width:794px"), "{style}");
        // a page break as the very last content must not leave a blank sheet
        let xml = "<w:body>\
<w:p><w:r><w:t>page one</w:t></w:r></w:p>\
<w:p><w:r><w:br w:type=\"page\"/><w:t>x</w:t></w:r></w:p>\
</w:body>";
        let html = docx_xml(xml);
        assert_eq!(html.matches("class=\"doc-page\"").count(), 2, "{html}");
    }
}
