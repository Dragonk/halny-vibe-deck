//! GFM-subset markdown -> HTML renderer (pure Rust, no dependencies).
//!
//! Used by the chat view and the file preview (`dangerous_inner_html`).
//! Everything that originates from the model or the filesystem is escaped:
//! raw HTML in the input is rendered as text, never executed. This is a
//! functional port of the Nexus marked pipeline, not a copy — it targets the
//! same CSS classes (`.markdown-content`, `.md-code-block`, `.mermaid-container`).

/// Render a markdown document to an HTML fragment (block level).
pub fn to_html(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0usize;
    let mut out = String::new();
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.is_empty() {
            i += 1;
            continue;
        }

        // --- fenced code / mermaid ---
        if let Some(fence) = fence_of(t) {
            let lang = t[fence.len()..].trim().to_string();
            let mut body = String::new();
            i += 1;
            while i < lines.len() && fence_of(lines[i].trim()) != Some(fence) {
                body.push_str(lines[i]);
                body.push('\n');
                i += 1;
            }
            if i < lines.len() {
                i += 1; // skip the closing fence
            }
            if lang == "mermaid" {
                out.push_str(&format!(
                    "<div class=\"mermaid-container\" data-src=\"{}\"></div>",
                    esc_attr(&body)
                ));
            } else if lang.is_empty() {
                out.push_str(&format!(
                    "<div class=\"md-code-block\"><pre><code>{}</code></pre></div>",
                    esc(&body)
                ));
            } else {
                out.push_str(&format!(
                    "<div class=\"md-code-block\"><div class=\"md-code-lang\">{}</div><pre><code class=\"language-{}\">{}</code></pre></div>",
                    esc(&lang),
                    esc_attr(&lang),
                    esc(&body)
                ));
            }
            continue;
        }

        // --- heading ---
        if is_heading(t) {
            let level = t.chars().take_while(|c| *c == '#').count();
            let content = t[level..].trim();
            out.push_str(&format!(
                "<h{lvl}>{}</h{lvl}>",
                inline(content),
                lvl = level
            ));
            i += 1;
            continue;
        }

        // --- horizontal rule ---
        if is_hr(t) {
            out.push_str("<hr />");
            i += 1;
            continue;
        }

        // --- blockquote ---
        if t.starts_with('>') {
            let mut inner = String::new();
            while i < lines.len() {
                let lt = lines[i].trim_start();
                if !lt.starts_with('>') {
                    break;
                }
                let mut s = &lt[1..];
                if s.starts_with(' ') {
                    s = &s[1..];
                }
                inner.push_str(s);
                inner.push('\n');
                i += 1;
            }
            out.push_str(&format!("<blockquote>{}</blockquote>", to_html(&inner)));
            continue;
        }

        // --- table ---
        if t.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1].trim()) {
            let header = split_row(t);
            let aligns = parse_aligns(lines[i + 1].trim());
            i += 2;
            out.push_str("<table><thead><tr>");
            for (ci, cell) in header.iter().enumerate() {
                let align = aligns.get(ci).copied().unwrap_or("");
                out.push_str(&format!(
                    "<th{}>{}</th>",
                    align_attr(align),
                    inline(cell)
                ));
            }
            out.push_str("</tr></thead><tbody>");
            while i < lines.len() {
                let rt = lines[i].trim();
                if rt.is_empty() || !rt.contains('|') || is_table_sep(rt) {
                    break;
                }
                let row = split_row(rt);
                out.push_str("<tr>");
                for (ci, cell) in row.iter().enumerate() {
                    let align = aligns.get(ci).copied().unwrap_or("");
                    out.push_str(&format!(
                        "<td{}>{}</td>",
                        align_attr(align),
                        inline(cell)
                    ));
                }
                out.push_str("</tr>");
                i += 1;
            }
            out.push_str("</tbody></table>");
            continue;
        }

        // --- list (nested by indentation, task items, ordered) ---
        if item_info(line).is_some() {
            let base = line.len() - line.trim_start().len();
            out.push_str(&parse_items(&lines, &mut i, base));
            continue;
        }

        // --- paragraph ---
        let mut para = String::new();
        while i < lines.len() {
            let l = lines[i];
            let lt = l.trim();
            if lt.is_empty() {
                break;
            }
            if fence_of(lt).is_some()
                || is_heading(lt)
                || is_hr(lt)
                || lt.starts_with('>')
                || item_info(l).is_some()
                || (lt.contains('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1].trim()))
            {
                break;
            }
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(lt);
            i += 1;
        }
        out.push_str(&format!("<p>{}</p>", inline(&para)));
    }
    out
}

/// Render inline markdown (bold, italic, strike, code, links, images, autolinks).
///
/// Round 7 (R11): path-looking prose tokens (e.g. `Dokumentacja/QUICK_START.md`
/// written without backticks) become clickable path links too. They are
/// protected behind private-use placeholders while the marker scan runs, so
/// markers inside a path (like `_` in `QUICK_START`) cannot split it.
pub fn inline(s: &str) -> String {
    let mut links: Vec<String> = Vec::new();
    let mut work = String::with_capacity(s.len());
    for tok in s.split_inclusive(|c: char| c.is_whitespace()) {
        let ws_len = tok.len() - tok.trim_end().len();
        let (body, ws) = tok.split_at(tok.len() - ws_len);
        let lead_len = body
            .find(|c: char| {
                c.is_alphanumeric() || matches!(c, '/' | '\\' | '@' | '.' | '_' | '-' | '~')
            })
            .unwrap_or(body.len());
        let lead = &body[..lead_len];
        let rest = &body[lead_len..];
        // trailing punctuation (`,`, `.`, `)`, …) is not part of the path
        let mut trail_len = 0usize;
        for (i, c) in rest.char_indices().rev() {
            if matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '"' | '\'' | '`') {
                trail_len = rest.len() - i;
            } else {
                break;
            }
        }
        let core = &rest[..rest.len() - trail_len];
        let trail = &rest[rest.len() - trail_len..];
        work.push_str(lead);
        // a quote/backtick/angle lead means this token is quoted markup or a
        // code span — Nexus keeps those literal, and so do we
        let ok_lead = !lead.contains(['"', '\'', '`', '<', '>']);
        if ok_lead
            && !core.is_empty()
            && crate::state::looks_like_inline_path(core)
            // never linkify markup-looking fragments (`<script>…</script>`)
            && !core.contains(['<', '>', '"', '\'', '`', '(', ')', '{', '}', '[', ']', '='])
        {
            let idx = links.len();
            links.push(format!(
                "<code class=\"markdown-path-link\" role=\"button\" tabindex=\"0\" title=\"{}\" data-path=\"{}\">{}</code>",
                esc_attr(core),
                esc_attr(core),
                esc(core)
            ));
            work.push('\u{E000}');
            work.push_str(&idx.to_string());
            work.push('\u{E001}');
        } else {
            work.push_str(core);
        }
        work.push_str(trail);
        work.push_str(ws);
    }

    let mut out = String::new();
    let mut rest = work.as_str();
    while !rest.is_empty() {
        let mut best: Option<(usize, &str)> = None;
        for m in ["**", "__", "~~", "`", "*", "_", "![", "[", "http://", "https://"] {
            if let Some(p) = rest.find(m) {
                let replace = match best {
                    Some((bp, bm)) => p < bp || (p == bp && m.len() > bm.len()),
                    None => true,
                };
                if replace {
                    best = Some((p, m));
                }
            }
        }
        match best {
            None => {
                out.push_str(&esc(rest));
                rest = "";
            }
            Some((p, m)) => {
                if p > 0 {
                    out.push_str(&esc(&rest[..p]));
                }
                rest = &rest[p..];
                rest = handle_marker(m, rest, &mut out);
            }
        }
    }
    for (i, l) in links.iter().enumerate() {
        out = out.replace(&format!("\u{E000}{i}\u{E001}"), l);
    }
    out
}

fn handle_marker<'a>(m: &str, rest: &'a str, out: &mut String) -> &'a str {
    match m {
        "**" | "__" => {
            if let Some(e) = rest[m.len()..].find(m) {
                let inner = &rest[m.len()..m.len() + e];
                out.push_str(&format!("<strong>{}</strong>", inline(inner)));
                &rest[m.len() + e + m.len()..]
            } else {
                out.push_str(m);
                &rest[m.len()..]
            }
        }
        "~~" => {
            if let Some(e) = rest[2..].find("~~") {
                let inner = &rest[2..2 + e];
                out.push_str(&format!("<del>{}</del>", inline(inner)));
                &rest[2 + e + 2..]
            } else {
                out.push_str("~~");
                &rest[2..]
            }
        }
        "`" => {
            if let Some(e) = rest[1..].find('`') {
                let inner = &rest[1..1 + e];
                // Clickable path links (Nexus `markdownComponents.code`):
                // inline code that looks like a file path renders as a
                // link; the document-level click bridge in chat.rs reads
                // `data-path` and opens the preview. `data-path` holds the
                // raw (unresolved) path — state::open_path resolves it.
                if crate::state::looks_like_inline_path(inner) {
                    out.push_str(&format!(
                        "<code class=\"markdown-path-link\" role=\"button\" tabindex=\"0\" title=\"{}\" data-path=\"{}\">{}</code>",
                        esc_attr(inner),
                        esc_attr(inner),
                        esc(inner)
                    ));
                } else {
                    out.push_str(&format!("<code>{}</code>", esc(inner)));
                }
                &rest[1 + e + 1..]
            } else {
                out.push('`');
                &rest[1..]
            }
        }
        "*" | "_" => {
            // intraword underscores (snake_case) stay literal
            if m == "_" && out.chars().last().map(|c| c.is_alphanumeric()).unwrap_or(false) {
                out.push('_');
                return &rest[1..];
            }
            if let Some(e) = rest[1..].find(m) {
                let inner = &rest[1..1 + e];
                if inner.trim().is_empty() || inner.contains('\n') {
                    out.push_str(m);
                    return &rest[1..];
                }
                out.push_str(&format!("<em>{}</em>", inline(inner)));
                &rest[1 + e + 1..]
            } else {
                out.push_str(m);
                &rest[1..]
            }
        }
        "![" => {
            if let Some(cb) = rest.find("](") {
                if let Some(cp) = rest[cb..].find(')') {
                    let alt = &rest[2..cb];
                    let url = &rest[cb + 2..cb + cp];
                    out.push_str(&format!(
                        "<img src=\"{}\" alt=\"{}\" />",
                        esc_attr(url),
                        esc_attr(alt)
                    ));
                    return &rest[cb + cp + 1..];
                }
            }
            out.push_str("![");
            &rest[2..]
        }
        "[" => {
            if let Some(cb) = rest.find("](") {
                if let Some(cp) = rest[cb..].find(')') {
                    let text = &rest[1..cb];
                    let url = &rest[cb + 2..cb + cp];
                    // Round 7 (R11): a relative/project link is a file path,
                    // not an external URL — render it as a clickable path
                    // link (data-path) instead of a target=_blank anchor.
                    let is_ext = url.starts_with("http://")
                        || url.starts_with("https://")
                        || url.starts_with("mailto:")
                        || url.starts_with('#');
                    if !is_ext
                        && (crate::state::looks_like_inline_path(url)
                            || (url.contains('/') && !url.contains("://")))
                    {
                        out.push_str(&format!(
                            "<a class=\"markdown-path-link\" role=\"button\" tabindex=\"0\" title=\"{}\" data-path=\"{}\">{}</a>",
                            esc_attr(url),
                            esc_attr(url),
                            esc(text)
                        ));
                        return &rest[cb + cp + 1..];
                    }
                    out.push_str(&format!(
                        "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                        esc_attr(url),
                        inline(text)
                    ));
                    return &rest[cb + cp + 1..];
                }
            }
            out.push('[');
            &rest[1..]
        }
        _ => {
            // autolink: http:// or https://
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ')' || c == '<' || c == '"')
                .unwrap_or(rest.len());
            let url = &rest[..end];
            out.push_str(&format!(
                "<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>",
                esc_attr(url),
                esc(url)
            ));
            &rest[end..]
        }
    }
}

fn parse_items(lines: &[&str], i: &mut usize, base: usize) -> String {
    let ordered = item_info(lines[*i]).map(|x| x.1).unwrap_or(false);
    let mut out = String::from(if ordered { "<ol>" } else { "<ul>" });
    while *i < lines.len() {
        let line = lines[*i];
        if line.trim().is_empty() {
            break;
        }
        let Some((ind, ord, rest0)) = item_info(line) else { break };
        if ind != base || ord != ordered {
            break;
        }
        *i += 1;
        out.push_str("<li>");
        let mut rest = rest0;
        // task list item
        if let Some(r) = rest.strip_prefix("[ ] ") {
            out.push_str("<input type=\"checkbox\" disabled /> ");
            rest = r;
        } else if let Some(r) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
            out.push_str("<input type=\"checkbox\" checked disabled /> ");
            rest = r;
        }
        out.push_str(&inline(rest));
        // nested lists / continuation lines
        loop {
            if *i >= lines.len() {
                break;
            }
            let l2 = lines[*i];
            if l2.trim().is_empty() {
                break;
            }
            if let Some((ind2, _, _)) = item_info(l2) {
                if ind2 > base {
                    out.push_str(&parse_items(lines, i, ind2));
                    continue;
                }
                break;
            }
            let ind2 = l2.len() - l2.trim_start().len();
            if ind2 > base {
                out.push(' ');
                out.push_str(&inline(l2.trim()));
                *i += 1;
                continue;
            }
            break;
        }
        out.push_str("</li>");
    }
    out.push_str(if ordered { "</ol>" } else { "</ul>" });
    out
}

fn item_info(line: &str) -> Option<(usize, bool, &str)> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    let (ord, rest) = if let Some(r) = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))
    {
        (false, r)
    } else {
        let d = t.chars().take_while(|c| c.is_ascii_digit()).count();
        if d > 0 && t[d..].starts_with(". ") {
            (true, &t[d + 2..])
        } else {
            return None;
        }
    };
    Some((indent, ord, rest))
}

fn fence_of(t: &str) -> Option<&'static str> {
    if t.starts_with("```") {
        Some("```")
    } else if t.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_heading(t: &str) -> bool {
    let h = t.chars().take_while(|c| *c == '#').count();
    h >= 1 && h <= 6 && t[h..].starts_with(' ')
}

fn is_hr(t: &str) -> bool {
    let s: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    s.len() >= 3
        && (s.chars().all(|c| c == '-') || s.chars().all(|c| c == '*') || s.chars().all(|c| c == '_'))
}

fn is_table_sep(t: &str) -> bool {
    let s = t.trim();
    let s = s.strip_prefix('|').unwrap_or(s);
    let s = s.strip_suffix('|').unwrap_or(s);
    if s.trim().is_empty() {
        return false;
    }
    s.split('|').all(|c| {
        let c = c.trim();
        let c = c.strip_prefix(':').unwrap_or(c);
        let c = c.strip_suffix(':').unwrap_or(c);
        c.len() >= 3 && c.chars().all(|ch| ch == '-')
    })
}

fn split_row(t: &str) -> Vec<String> {
    let s = t.trim();
    let s = s.strip_prefix('|').map(|x| x.trim_end()).unwrap_or(s);
    let s = s.strip_suffix('|').map(|x| x.trim_start()).unwrap_or(s);
    s.split('|').map(|c| c.trim().to_string()).collect()
}

fn parse_aligns(sep: &str) -> Vec<&'static str> {
    split_row(sep)
        .iter()
        .map(|c| {
            let c = c.trim();
            let left = c.starts_with(':');
            let right = c.ends_with(':');
            if left && right {
                "center"
            } else if right {
                "right"
            } else {
                "left"
            }
        })
        .collect()
}

fn align_attr(align: &str) -> String {
    if align.is_empty() {
        String::new()
    } else {
        format!(" style=\"text-align:{}\"", align)
    }
}

/// Escape text for an HTML text node.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape text for an HTML attribute value.
fn esc_attr(s: &str) -> String {
    esc(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(inline("<script>alert(1)</script>"), "&lt;script&gt;alert(1)&lt;/script&gt;");
    }

    #[test]
    fn renders_basic_blocks() {
        let html = to_html("# Title\n\npara **bold** and `code`\n\n- one\n- two");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<code>code</code>"));
        assert!(html.contains("<ul><li>one</li><li>two</li></ul>"));
    }

    #[test]
    fn renders_clickable_path_links() {
        // plain inline code stays plain
        let html = to_html("edit `foo` now");
        assert!(html.contains("<code>foo</code>"));
        assert!(!html.contains("markdown-path-link"));
        // path-looking inline code becomes a clickable link with data-path
        let html = to_html("see `src/main.rs` for details");
        assert!(html.contains("markdown-path-link"));
        assert!(html.contains("data-path=\"src/main.rs\""));
        assert!(html.contains("role=\"button\""));
        // quotes in plain inline code stay literal in text (esc() escapes
        // only & < >; attributes go through esc_attr)
        let html = to_html("run `a \"quoted\" cmd`");
        assert!(html.contains("<code>a \"quoted\" cmd</code>"));
        assert!(!html.contains("markdown-path-link"));
        // Nexus looksLikeInlinePath: a leading quote fails the first-char
        // check, so quoted paths do NOT become links (resolution of
        // surrounding noise happens at click time, not in markdown)
        let html = to_html("see `\"src/a.rs\"` here");
        assert!(!html.contains("markdown-path-link"));
        // @-prefixed paths link with the raw text in data-path
        let html = to_html("open `@docs/spec.md` now");
        assert!(html.contains("markdown-path-link"));
        assert!(html.contains("data-path=\"@docs/spec.md\""));
    }

    #[test]
    fn plain_prose_paths_become_links() {
        // round 7 (R11): agent prose paths (no backticks) are clickable
        let html = to_html("Sprawdź Dokumentacja/QUICK_START.md, potem wróć.");
        assert!(html.contains("data-path=\"Dokumentacja/QUICK_START.md\""));
        assert!(html.contains("markdown-path-link"));
        // trailing sentence punctuation stays outside the link
        assert!(html.contains("</code>, potem"));
        // plain words and URLs are untouched
        let html = to_html("see https://example.com/x and hello world");
        assert!(!html.contains("markdown-path-link"));
        // fenced code is NOT linkified
        let html = to_html("```\nDokumentacja/QUICK_START.md\n```");
        assert!(!html.contains("markdown-path-link"));
    }

    #[test]
    fn relative_markdown_links_become_path_links() {
        // round 7 (R11): [text](relative/path.md) opens the preview
        let html = to_html("zobacz [quick start](Dokumentacja/QUICK_START.md) tutaj");
        assert!(html.contains("data-path=\"Dokumentacja/QUICK_START.md\""));
        assert!(html.contains("markdown-path-link"));
        assert!(!html.contains("target=\"_blank\""));
        // external links stay external
        let html = to_html("[site](https://example.com)");
        assert!(html.contains("target=\"_blank\""));
        assert!(!html.contains("markdown-path-link"));
    }

    #[test]
    fn renders_code_and_mermaid() {
        let html = to_html("```rust\nfn x() {}\n```\n\n```mermaid\ngraph TD; A-->B;\n```");
        assert!(html.contains("<div class=\"md-code-lang\">rust</div>"));
        assert!(html.contains("fn x() {}"));
        assert!(html.contains("mermaid-container"));
        assert!(html.contains("data-src=\"graph TD; A--&gt;B;\n\""));
    }

    #[test]
    fn renders_table_and_links() {
        let html = to_html("| a | b |\n|---|---:|\n| 1 | 2 |");
        assert!(html.contains("<th style=\"text-align:left\">a</th>"));
        assert!(html.contains("<th style=\"text-align:right\">b</th>"));
        let l = inline("see [docs](https://x.example) now");
        assert!(l.contains("<a href=\"https://x.example\""));
    }

    #[test]
    fn nested_and_task_lists() {
        let html = to_html("- top\n  - nested\n- [x] done\n- [ ] todo");
        assert!(html.contains("<ul><li>top<ul><li>nested</li></ul></li>"));
        assert!(html.contains("checked disabled"));
        assert!(html.contains("<input type=\"checkbox\" disabled />"));
    }

    #[test]
    fn blockquote_and_hr() {
        let html = to_html("> quoted **b**\n\n---\n");
        assert!(html.contains("<blockquote><p>quoted <strong>b</strong></p></blockquote>"));
        assert!(html.contains("<hr />"));
    }
}

