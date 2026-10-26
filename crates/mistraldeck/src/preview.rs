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

#[derive(Default, Clone)]
struct Run {
    bold: bool,
    italic: bool,
    strike: bool,
    link: bool,
    text: String,
}

impl Run {
    fn html(&self) -> String {
        let mut t = text_html(&self.text);
        if t.is_empty() {
            return t;
        }
        if self.link {
            t = format!("<u>{t}</u>");
        }
        if self.bold {
            t = format!("<b>{t}</b>");
        }
        if self.italic {
            t = format!("<i>{t}</i>");
        }
        if self.strike {
            t = format!("<s>{t}</s>");
        }
        t
    }
}

pub fn docx_to_html(path: &Path) -> Result<String, String> {
    let xml = read_zip_entry(path, "word/document.xml")?;
    let xml = String::from_utf8_lossy(&xml).into_owned();
    Ok(docx_xml(&xml))
}

fn heading_level(style: &str) -> Option<u32> {
    let s = style.trim();
    if s.eq_ignore_ascii_case("title") {
        return Some(1);
    }
    let lower = s.to_ascii_lowercase();
    let rest = lower.strip_prefix("heading")?;
    let n: u32 = rest.trim().parse().ok()?;
    if (1..=9).contains(&n) {
        Some(n.min(6))
    } else {
        None
    }
}

fn docx_xml(xml: &str) -> String {
    let mut out = String::from("<div class=\"doc-render\">");
    let mut list_open = false;

    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut para_style = String::new();
    let mut para_list = false;
    let mut para_html = String::new();
    let mut in_para = false;

    let mut run = Run::default();
    let mut in_run = false;
    let mut link_depth = 0usize;
    let mut capture_text = false;

    let mut in_table = false;
    let mut in_cell = false;
    let mut cell_html = String::new();
    let mut row_html = String::new();

    let mut blocks = 0usize;

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
                    if let Some(level) = heading_level(&para_style) {
                        out.push_str(&format!("<h{level}>{para_html}</h{level}>"));
                    } else if para_list {
                        if !list_open {
                            out.push_str("<ul>");
                            list_open = true;
                        }
                        out.push_str(&format!("<li>{para_html}</li>"));
                    } else {
                        if list_open {
                            out.push_str("</ul>");
                            list_open = false;
                        }
                        if para_html.is_empty() {
                            out.push_str("<p class=\"doc-spacer\">&nbsp;</p>");
                        } else {
                            out.push_str(&format!("<p>{para_html}</p>"));
                        }
                    }
                }
                in_para = false;
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
                        para_list = false;
                        para_html.clear();
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
                    "t" => capture_text = true,
                    "tab" if in_run => run.text.push('\t'),
                    "br" if in_run => run.text.push('\n'),
                    "pStyle" if in_para => {
                        para_style = attr(&e, "val").unwrap_or_default();
                    }
                    "numPr" if in_para => para_list = true,
                    "numId" if in_para => {
                        if attr(&e, "val").as_deref() == Some("0") {
                            para_list = false;
                        }
                    }
                    "hyperlink" => link_depth += 1,
                    "tbl" if !in_cell => {
                        flush_para!();
                        if list_open {
                            out.push_str("</ul>");
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
                    "br" if in_run => run.text.push('\n'),
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
                    "pStyle" if in_para => {
                        para_style = attr(&e, "val").unwrap_or_default();
                    }
                    "numPr" if in_para => para_list = true,
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
                        if in_run {
                            let html = run.html();
                            if !html.is_empty() {
                                para_html.push_str(&html);
                            }
                            in_run = false;
                        }
                    }
                    "hyperlink" => link_depth = link_depth.saturating_sub(1),
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
        out.push_str("</ul>");
    }
    out.push_str("</div>");
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
    let mut out: Vec<SheetData> = Vec::new();
    for name in names {
        if out.len() >= SHEET_MAX {
            break;
        }
        let range = match workbook.worksheet_range(&name) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let rows: Vec<Vec<String>> = range
            .rows()
            .take(SHEET_MAX_ROWS)
            .map(|row| {
                row.iter()
                    .take(SHEET_MAX_COLS)
                    .map(cell_string)
                    .collect()
            })
            .collect();
        out.push(SheetData { name, rows });
    }
    if out.is_empty() {
        return Err("no sheets with data".into());
    }
    Ok(out)
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
        assert!(html.contains("<h1>Hi</h1>"), "{html}");
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
        assert!(html.contains("<ul><li>item</li></ul>"), "{html}");
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
}
