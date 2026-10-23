//! Markdown renderer (chat messages + file previews), a small but
//! complete subset of GitHub-flavored markdown:
//! - ATX headings (`#`…`######`),
//! - fenced code blocks with a language chip + copy button (wrapping),
//! - lists (nested by indentation, `-`/`*`/`+` bullets, `1.` numbers,
//!   task-list checkboxes `[ ]` / `[x]`),
//! - blockquotes, GFM tables with alignment, horizontal rules,
//! - inline: `code`, **bold**, *italic*, ~~strike~~, [links](url)
//!   (first link opens on click, hover lists all URLs).
//!
//! All text wraps to the available width: every `LayoutJob` gets an
//! explicit `TextWrapping` and the labels use `.wrap()`, so long lines
//! never break the chat layout.

use egui::text::LayoutJob;
use egui::{Align, Color32, FontId, Stroke, TextFormat, Ui};

const BODY: f32 = 14.0;
const CODE: f32 = 12.5;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn markdown(ui: &mut Ui, src: &str) {
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0usize;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if t.is_empty() {
            i += 1;
            continue;
        }
        // fenced code block
        if t.starts_with("```") || t.starts_with("~~~") {
            let fence: String = t.chars().take(3).collect();
            let lang = t[3..].trim().to_string();
            let mut code = String::new();
            i += 1;
            // a closing fence is a line consisting only of fence characters,
            // at least as long as the opening one — an inner ```bash or a
            // language chip never closes the block prematurely
            let fence_char = fence.chars().next().unwrap_or('`');
            let is_close = |l: &str| {
                let t = l.trim();
                t.len() >= fence.len() && t.chars().all(|c| c == fence_char)
            };
            while i < lines.len() && !is_close(lines[i]) {
                code.push_str(lines[i]);
                code.push('\n');
                i += 1;
            }
            i += 1; // closing fence (if any)
            code_block(ui, &lang, &code);
            continue;
        }
        // heading
        if t.starts_with('#') {
            let hashes = t.chars().take_while(|c| *c == '#').count().min(6);
            let text = t[hashes..].trim();
            i += 1;
            heading(ui, hashes.min(6), text);
            continue;
        }
        // horizontal rule
        if is_hr(t) {
            i += 1;
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            continue;
        }
        // blockquote block
        if t.starts_with('>') {
            let mut quote = String::new();
            while i < lines.len() {
                let l = lines[i].trim_start();
                if let Some(q) = l.strip_prefix('>') {
                    quote.push_str(q.trim_start());
                    quote.push('\n');
                    i += 1;
                } else {
                    break;
                }
            }
            blockquote(ui, &quote);
            continue;
        }
        // table: header row + `|---|` separator
        if t.starts_with('|')
            && i + 1 < lines.len()
            && lines[i + 1].trim_start().starts_with('|')
            && is_table_sep(lines[i + 1])
        {
            i = table(ui, &lines, i);
            continue;
        }
        // list block
        if list_level(t).is_some() {
            i = list_block(ui, &lines, i);
            continue;
        }
        // paragraph: consume until a blank line or a new block element
        let mut para = String::new();
        while i < lines.len() {
            let l = lines[i].trim_start();
            if l.is_empty()
                || l.starts_with("```")
                || l.starts_with("~~~")
                || l.starts_with('#')
                || l.starts_with('>')
                || l.starts_with('|')
                || list_level(l).is_some()
                || is_hr(l)
            {
                break;
            }
            if !para.is_empty() {
                para.push('\n');
            }
            para.push_str(lines[i].trim());
            i += 1;
        }
        if !para.is_empty() {
            paragraph(ui, &para);
        }
    }
}

fn is_hr(t: &str) -> bool {
    let body = t.trim();
    (body.starts_with("---") || body.starts_with("***") || body.starts_with("___"))
        && body
            .chars()
            .filter(|c| *c != ' ')
            .all(|c| c == '-' || c == '*' || c == '_')
        && body.len() >= 3
}

/// Does this line look like `| --- | :---: |`?
fn is_table_sep(line: &str) -> bool {
    let mut has_dash = false;
    let mut ok = true;
    for c in line.chars() {
        match c {
            '|' | ':' | ' ' | '\t' => {}
            '-' => has_dash = true,
            _ => ok = false,
        }
    }
    ok && has_dash
}

// ---------------------------------------------------------------------------
// Block elements
// ---------------------------------------------------------------------------

fn heading(ui: &mut Ui, level: usize, text: &str) {
    let (size, top, bottom) = match level {
        1 => (22.0, 10.0, 4.0),
        2 => (18.0, 10.0, 4.0),
        3 => (16.0, 8.0, 3.0),
        4 => (15.0, 8.0, 3.0),
        5 => (14.0, 6.0, 2.0),
        _ => (13.0, 6.0, 2.0),
    };
    ui.add_space(top);
    let (job, links) = inline_job(ui, text, size, true);
    let label = ui.add(egui::Label::new(job).wrap().selectable(false));
    apply_links(ui, label, &links);
    ui.add_space(bottom);
}

fn code_block(ui: &mut Ui, lang: &str, code: &str) {
    ui.add_space(4.0);
    egui::Frame::default()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(egui::Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            if !lang.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(lang)
                            .small()
                            .monospace()
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| {
                        if ui
                            .add(
                                egui::Button::new(egui::RichText::new("copy").small())
                                    .fill(Color32::TRANSPARENT),
                            )
                            .on_hover_text("copy")
                            .clicked()
                        {
                            ui.output_mut(|o| o.copied_text = code.trim_end().to_string());
                        }
                    });
                });
            }
            let mut job = LayoutJob::default();
            job.append(
                code.trim_end(),
                0.0,
                TextFormat::simple(FontId::monospace(CODE), ui.visuals().text_color()),
            );
            // break anywhere: long paths/commands wrap mid-token instead
            // of overflowing the chat layout (Label::wrap sets max_width)
            job.wrap.break_anywhere = true;
            ui.add(egui::Label::new(job).wrap().selectable(true));
        });
    ui.add_space(4.0);
}

/// Indentation level (every 2 spaces = one level) + the item marker kind.
fn list_level(line: &str) -> Option<(usize, ListKind)> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let kind = if t.starts_with("- [ ] ")
        || t.starts_with("* [ ] ")
        || t.starts_with("+ [ ] ")
    {
        ListKind::Task(false)
    } else if t.starts_with("- [x] ") || t.starts_with("* [x] ") || t.starts_with("+ [x] ") {
        ListKind::Task(true)
    } else if t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t == "-"
        || t == "*"
        || t == "+"
    {
        ListKind::Bullet
    } else {
        // numbered: "12. text"
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && t[digits..].starts_with(". ") {
            ListKind::Numbered
        } else {
            return None;
        }
    };
    Some((indent / 2, kind))
}

#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Bullet,
    Numbered,
    Task(bool),
}

fn list_block(ui: &mut Ui, lines: &[&str], mut i: usize) -> usize {
    ui.add_space(2.0);
    let mut number = 0usize;
    while i < lines.len() {
        let Some((level, kind)) = list_level(lines[i]) else { break };
        if kind == ListKind::Numbered {
            number += 1;
        }
        let t = lines[i].trim_start();
        let content = strip_list_marker(t);
        let bullet = match kind {
            ListKind::Bullet => "•".to_string(),
            ListKind::Numbered => format!("{number}."),
            ListKind::Task(done) => if done { "✓" } else { "□" }.to_string(),
        };
        let pad = (level * 14) as f32;
        let done = matches!(kind, ListKind::Task(true));
        let col = if done {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().text_color()
        };
        ui.horizontal(|ui| {
            ui.add_space(pad);
            ui.label(egui::RichText::new(format!("{bullet} ")).size(BODY).strong().color(col));
            let (job, links) = inline_job(ui, content, BODY, !done);
            let label = ui.add(egui::Label::new(job).wrap());
            apply_links(ui, label, &links);
        });
        i += 1;
    }
    ui.add_space(2.0);
    i
}

fn strip_list_marker(t: &str) -> &str {
    for p in ["- [ ] ", "- [x] ", "* [ ] ", "* [x] ", "+ [ ] ", "+ [x] "] {
        if let Some(rest) = t.strip_prefix(p) {
            return rest;
        }
    }
    for p in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(p) {
            return rest;
        }
    }
    if t == "-" || t == "*" || t == "+" {
        return "";
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && t[digits..].starts_with(". ") {
        return &t[digits + 2..];
    }
    t
}

fn blockquote(ui: &mut Ui, quote: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("|")
                .size(BODY)
                .strong()
                .color(ui.visuals().weak_text_color()),
        );
        for line in quote.trim().lines() {
            let (mut job, links) = inline_job(ui, line, BODY, true);
            for s in &mut job.sections {
                s.format.color = s.format.color.gamma_multiply(0.85);
                s.format.italics = true;
            }
            let label = ui.add(egui::Label::new(job).wrap());
            apply_links(ui, label, &links);
        }
    });
    ui.add_space(4.0);
}

fn table(ui: &mut Ui, lines: &[&str], mut i: usize) -> usize {
    let header: Vec<String> = split_row(lines[i]);
    let aligns = parse_aligns(lines[i + 1]);
    let ncols = header.len().max(aligns.len());
    i += 2;
    let mut rows: Vec<Vec<String>> = Vec::new();
    while i < lines.len() && lines[i].trim_start().starts_with('|') {
        rows.push(split_row(lines[i]));
        i += 1;
    }
    ui.add_space(4.0);
    egui::Frame::default()
        .stroke(egui::Stroke::new(
            1.0_f32,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(6.0))
        .show(ui, |ui| {
            let total = ui.available_width() - 4.0;
            let col_w = (total / ncols.max(1) as f32).max(30.0);
            let row_ui = |ui: &mut Ui, cells: &[String], header_row: bool| {
                ui.horizontal(|ui| {
                    for (c, cell) in cells.iter().enumerate() {
                        let align = aligns.get(c).copied().unwrap_or(Align::Min);
                        let (mut job, links) = inline_job(ui, cell, 13.0, true);
                        if header_row {
                            for s in &mut job.sections {
                                s.format.color = ui.visuals().strong_text_color();
                            }
                        }
                        job.halign = align;
                        ui.vertical(|ui| {
                            ui.set_width(col_w);
                            let label = ui.add(egui::Label::new(job).wrap());
                            apply_links(ui, label, &links);
                        });
                    }
                });
            };
            row_ui(ui, &header, true);
            ui.separator();
            for row in &rows {
                row_ui(ui, row, false);
            }
        });
    ui.add_space(4.0);
    i
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(|c| c.trim().to_string()).collect()
}

fn parse_aligns(line: &str) -> Vec<Align> {
    split_row(line)
        .iter()
        .map(|c| {
            let l = c.starts_with(':');
            let r = c.ends_with(':');
            if l && r {
                Align::Center
            } else if r {
                Align::Max
            } else {
                Align::Min
            }
        })
        .collect()
}

fn paragraph(ui: &mut Ui, text: &str) {
    ui.add_space(3.0);
    for line in text.split('\n') {
        let (job, links) = inline_job(ui, line, BODY, true);
        let label = ui.add(egui::Label::new(job).wrap());
        apply_links(ui, label, &links);
    }
    ui.add_space(3.0);
}

/// Open the first link on click; list all URLs on hover.
fn apply_links(ui: &mut Ui, label: egui::Response, links: &[(String, String)]) {
    if links.is_empty() {
        return;
    }
    let mut resp = label;
    if links.len() == 1 {
        resp = resp.on_hover_text(&links[0].1);
    } else {
        let list: Vec<String> = links.iter().map(|(_, url)| url.clone()).collect();
        resp = resp.on_hover_text(list.join("\n"));
    }
    if resp.clicked() {
        let url = links[0].1.clone();
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

// ---------------------------------------------------------------------------
// Inline parsing
// ---------------------------------------------------------------------------

/// Build one laid-out line with inline formatting.
/// Returns the job and the list of `(text, url)` links it contains.
pub fn inline_job(
    ui: &Ui,
    text: &str,
    size: f32,
    default_color: bool,
) -> (LayoutJob, Vec<(String, String)>) {
    let mut job = LayoutJob::default();
    let mut links: Vec<(String, String)> = Vec::new();
    let base = if default_color {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let strong = ui.visuals().strong_text_color();
    let accent = ui.visuals().hyperlink_color;
    let code_color = accent;

    let mut bold = false;
    let mut italic = false;
    let mut strike = false;
    let mut rest = text;
    let mut pending = String::new();

    let fmt_now = |bold: bool, italic: bool, strike: bool, code: bool| -> TextFormat {
        let (font, color) = if code {
            (FontId::monospace(size), code_color)
        } else if bold {
            // egui has no bold weight: slightly larger + strong color
            (FontId::proportional(size + 0.5), strong)
        } else {
            (FontId::proportional(size), base)
        };
        TextFormat {
            font_id: font,
            color: if strike { color.gamma_multiply(0.6) } else { color },
            italics: italic,
            strikethrough: if strike {
                Stroke::new(1.0_f32, color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        }
    };

    while !rest.is_empty() {
        // inline code spans take priority (no formatting inside)
        if let Some((s, e)) = find_code_span(rest) {
            let (before, span) = rest.split_at(s);
            let span = &span[1..e - s];
            if !pending.is_empty() {
                job.append(&std::mem::take(&mut pending), 0.0, fmt_now(bold, italic, strike, false));
            }
            if !before.is_empty() {
                pending.push_str(before);
                job.append(&std::mem::take(&mut pending), 0.0, fmt_now(bold, italic, strike, false));
            }
            job.append(span, 0.0, fmt_now(false, false, false, true));
            rest = &rest[e + 1..];
            continue;
        }
        // links [text](url)
        if rest.starts_with('[') {
            if let Some((label_text, url, consumed)) = find_link(rest) {
                if !pending.is_empty() {
                    job.append(&std::mem::take(&mut pending), 0.0, fmt_now(bold, italic, strike, false));
                }
                job.append(
                    &label_text,
                    0.0,
                    TextFormat {
                        font_id: FontId::proportional(size),
                        color: accent,
                        underline: Stroke::new(1.0_f32, accent),
                        ..Default::default()
                    },
                );
                links.push((label_text, url));
                rest = &rest[consumed..];
                continue;
            }
        }
        // images ![alt](url) → compact chip
        if rest.starts_with("![") {
            if let Some((label_text, _url, consumed)) = find_image(rest) {
                if !pending.is_empty() {
                    job.append(&std::mem::take(&mut pending), 0.0, fmt_now(bold, italic, strike, false));
                }
                job.append(
                    &format!("[img] {label_text}"),
                    0.0,
                    TextFormat {
                        font_id: FontId::proportional(size),
                        color: accent,
                        ..Default::default()
                    },
                );
                rest = &rest[consumed..];
                continue;
            }
        }
        // bold / italic / strike toggles
        let (tok, is_bold, is_italic, is_strike) = if rest.starts_with("**") {
            ("**", true, false, false)
        } else if rest.starts_with("~~") {
            ("~~", false, false, true)
        } else if rest.starts_with('*') {
            ("*", false, true, false)
        } else {
            ("", false, false, false)
        };
        if !tok.is_empty() && has_closing(rest, tok) {
            if !pending.is_empty() {
                job.append(&std::mem::take(&mut pending), 0.0, fmt_now(bold, italic, strike, false));
            }
            if is_bold {
                bold = !bold;
            }
            if is_strike {
                strike = !strike;
            }
            if is_italic {
                italic = !italic;
            }
            rest = &rest[tok.len()..];
            continue;
        }
        // regular char
        let ch = rest.chars().next().unwrap();
        pending.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    if !pending.is_empty() {
        job.append(&pending, 0.0, fmt_now(bold, italic, strike, false));
    }
    // long unbroken tokens (paths, URLs) wrap mid-token instead of overflowing
    job.wrap.break_anywhere = true;
    (job, links)
}

/// Find a complete `` `code` `` span anywhere in `s` (backtick indexes).
fn find_code_span(s: &str) -> Option<(usize, usize)> {
    let start = s.find('`')?;
    let end = s[start + 1..].find('`').map(|e| start + 1 + e)?;
    if end == start + 1 {
        return None; // empty code span
    }
    Some((start, end))
}

/// `[text](url)` → (text, url, consumed_bytes)
fn find_link(s: &str) -> Option<(String, String, usize)> {
    let close = s.find(']')?;
    let text = &s[1..close];
    if !s[close + 1..].starts_with('(') {
        return None;
    }
    let url_start = close + 2;
    let url_end = s[url_start..].find(')').map(|e| url_start + e)?;
    let url = &s[url_start..url_end];
    Some((text.to_string(), url.to_string(), url_end + 1))
}

/// `![alt](url)` → (alt, url, consumed_bytes)
fn find_image(s: &str) -> Option<(String, String, usize)> {
    let close = s.find(']')?;
    let text = &s[2..close];
    if !s[close + 1..].starts_with('(') {
        return None;
    }
    let url_start = close + 2;
    let url_end = s[url_start..].find(')').map(|e| url_start + e)?;
    Some((text.to_string(), s[url_start..url_end].to_string(), url_end + 1))
}

fn has_closing(s: &str, token: &str) -> bool {
    s[token.len()..].contains(token)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_row_split() {
        let cells = split_row("| a | b |");
        assert_eq!(cells, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn table_aligns() {
        let a = parse_aligns("| :--- | :---: | ---: |");
        assert_eq!(a, vec![Align::Min, Align::Center, Align::Max]);
    }

    #[test]
    fn sep_detect() {
        assert!(is_table_sep("| --- | --- |"));
        assert!(!is_table_sep("| a | b |"));
    }

    #[test]
    fn hr_detect() {
        assert!(is_hr("---"));
        assert!(is_hr("***"));
        assert!(!is_hr("- item"));
        assert!(!is_hr("-"));
    }

    #[test]
    fn list_level_detect() {
        assert!(matches!(list_level("- a"), Some((0, ListKind::Bullet))));
        assert!(matches!(list_level("  1. a"), Some((1, ListKind::Numbered))));
        assert!(matches!(list_level("- [x] a"), Some((0, ListKind::Task(true)))));
        assert!(list_level("plain text").is_none());
    }

    #[test]
    fn list_marker_strip() {
        assert_eq!(strip_list_marker("- [ ] task"), "task");
        assert_eq!(strip_list_marker("3. num"), "num");
        assert_eq!(strip_list_marker("- bullet"), "bullet");
    }

    #[test]
    fn link_parse() {
        let (text, url, used) = find_link("[label](https://x.y)").unwrap();
        assert_eq!(text, "label");
        assert_eq!(url, "https://x.y");
        assert_eq!(used, "[label](https://x.y)".len());
        assert!(find_link("[unclosed").is_none());
    }

    #[test]
    fn code_span_parse() {
        assert_eq!(find_code_span("a `bc` d"), Some((2, 5)));
        assert_eq!(find_code_span("``"), None);
    }
}
