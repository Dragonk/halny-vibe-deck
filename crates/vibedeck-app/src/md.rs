//! Minimal, dependency-free Markdown rendering for egui.
//!
//! Covers what agent answers actually contain: headings, bullets,
//! numbered lists, blockquotes, fenced code blocks (with a copy
//! button) and inline `code` / **bold** / *italic*.

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId, Ui};

/// Render `markdown` inside `ui`.
pub fn markdown(ui: &mut Ui, markdown: &str) {
    let code_bg = ui.visuals().extreme_bg_color;
    let strong = ui.visuals().strong_text_color();
    let weak = ui.visuals().weak_text_color();
    let code_color = ui.style().noninteractive().fg_stroke.color;
    let accent = Color32::from_rgb(0x4d, 0xa3, 0xff);

    let mut in_fence = false;
    let mut fence_lines: Vec<String> = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            if in_fence {
                code_block(ui, &fence_lines.join("\n"), code_bg, code_color);
                fence_lines.clear();
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            fence_lines.push(line.to_string());
            continue;
        }
        if trimmed.is_empty() {
            ui.add_space(4.0);
            continue;
        }
        if let Some(rest) = heading_level(trimmed) {
            let size = match rest.0 {
                1 => 20.0,
                2 => 17.0,
                3 => 15.0,
                _ => 14.0,
            };
            let job = styled(rest.1, strong, weak, accent, Some(size));
            ui.label(job);
            continue;
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            ui.horizontal(|ui| {
                ui.label("•");
                let job = styled(&trimmed[2..], strong, weak, accent, None);
                ui.label(job);
            });
            continue;
        }
        if trimmed.starts_with('>') {
            ui.horizontal(|ui| {
                ui.colored_label(weak, "▍");
                let job = styled(
                    trimmed.trim_start_matches('>').trim(),
                    weak,
                    weak,
                    accent,
                    None,
                );
                ui.label(job);
            });
            continue;
        }
        // Numbered list item or plain paragraph.
        let job = styled(trimmed, strong, weak, accent, None);
        ui.label(job);
    }
    if in_fence {
        code_block(ui, &fence_lines.join("\n"), code_bg, code_color);
    }
}

fn heading_level(line: &str) -> Option<(usize, &str)> {
    let mut level = 0;
    for c in line.chars() {
        if c == '#' {
            level += 1;
        } else {
            break;
        }
    }
    if (1..=6).contains(&level) && line[level..].starts_with(' ') {
        Some((level, line[level + 1..].trim()))
    } else {
        None
    }
}

fn code_block(ui: &mut Ui, code: &str, bg: Color32, _fg: Color32) {
    egui::Frame::group(ui.style())
        .fill(bg)
        .inner_margin(egui::Margin::symmetric(8.0, 6.0))
        .show(ui, |ui| {
            ui.set_min_width(200.0);
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
                ui.label(code);
            });
        })
        .response
        .on_hover_cursor(egui::CursorIcon::Text);
    if ui
        .add(egui::Button::new("📋 Copy").small())
        .on_hover_text("Copy code block")
        .clicked()
    {
        ui.ctx().copy_text(code.to_string());
    }
    ui.add_space(4.0);
}

/// Inline styling: `code`, **bold**, *italic*.
fn styled(
    text: &str,
    strong: Color32,
    weak: Color32,
    accent: Color32,
    size: Option<f32>,
) -> LayoutJob {
    const BODY: f32 = 14.0;
    let mut job = LayoutJob::default();
    let base = size.unwrap_or(BODY);
    let normal = TextFormat::simple(FontId::proportional(base), strong);
    let mut bold = normal.clone();
    bold.font_id = FontId::proportional(base + 0.5);
    bold.color = strong;
    let mut italic = normal.clone();
    italic.italics = true;
    italic.color = weak;
    let mut code = normal.clone();
    code.font_id = FontId::monospace(13.0);
    code.background = Color32::from_black_alpha(25);
    code.color = accent;

    let bytes = text.as_bytes();
    let mut i = 0;
    let mut plain = String::new();
    let flush = |plain: &mut String, job: &mut LayoutJob, fmt: &TextFormat| {
        if !plain.is_empty() {
            job.append(plain, 0.0, fmt.clone());
            plain.clear();
        }
    };
    while i < bytes.len() {
        if bytes[i] == b'`' {
            if let Some(end) = text[i + 1..].find('`') {
                flush(&mut plain, &mut job, &normal);
                job.append(&text[i + 1..i + 1 + end], 0.0, code.clone());
                i += end + 2;
                continue;
            }
        }
        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            if let Some(end) = text[i + 2..].find("**") {
                flush(&mut plain, &mut job, &normal);
                job.append(&text[i + 2..i + 2 + end], 0.0, bold.clone());
                i += end + 4;
                continue;
            }
        }
        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] != b' ' {
            if let Some(end) = text[i + 1..].find('*') {
                flush(&mut plain, &mut job, &normal);
                job.append(&text[i + 1..i + 1 + end], 0.0, italic.clone());
                i += end + 2;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        plain.push(ch);
        i += ch.len_utf8();
    }
    flush(&mut plain, &mut job, &normal);
    job
}

/// One-line plain preview (for session lists): strips markdown markers.
pub fn plain_preview(markdown: &str, max_chars: usize) -> String {
    let mut s = markdown
        .replace(['#', '*', '`', '>'], "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if s.chars().count() > max_chars {
        let cut: String = s.chars().take(max_chars).collect();
        s = format!("{cut}…");
    }
    s
}
