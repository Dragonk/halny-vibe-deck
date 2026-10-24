//! Dependency-free syntax highlighter (wasm-safe).
//!
//! Produces HTML with `<span class="tok-*">` wrappers, consumed through
//! `dangerous_inner_html` by:
//! - the raw TOML editor in Settings (`components/code_editor.rs`),
//! - code file previews in the dock.
//! Token colors come from the `--tok-*` CSS variables defined per theme in
//! `assets/main.css` (dark + light).

/// Largest source we highlight; bigger files render as plain escaped text.
const MAX_HIGHLIGHT_BYTES: usize = 200_000;

/// Escape text for embedding inside `dangerous_inner_html`.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn span(class: &str, text: &str, out: &mut String) {
    out.push_str("<span class=\"tok-");
    out.push_str(class);
    out.push_str("\">");
    out.push_str(&escape_html(text));
    out.push_str("</span>");
}

/// Line-number gutter HTML (one number per line, newline-separated).
pub fn gutter_html(src: &str) -> String {
    let lines = src.bytes().filter(|&c| c == b'\n').count() + 1;
    let mut out = String::new();
    for i in 1..=lines {
        out.push_str(&i.to_string());
        out.push('\n');
    }
    out
}

fn is_toml_delim(c: u8) -> bool {
    matches!(
        c,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b'#'
            | b'='
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b','
            | b'"'
            | b'\''
    )
}

fn is_numberish(w: &str) -> bool {
    let b = w.as_bytes();
    if b.is_empty() {
        return false;
    }
    let first = b[0];
    if !(first.is_ascii_digit() || first == b'+' || first == b'-') {
        return false;
    }
    w.bytes().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'+' | b'-')
    })
}

/// Scan a quoted string starting at `i` (b[i] is the quote). Handles
/// escapes and TOML/Python-style multiline (`"""` / `'''`).
fn scan_string(b: &[u8], mut i: usize) -> usize {
    let q = b[i];
    if i + 2 < b.len() && b[i + 1] == q && b[i + 2] == q {
        i += 3;
        while i + 2 < b.len() && !(b[i] == q && b[i + 1] == q && b[i + 2] == q) {
            i += 1;
        }
        return (i + 3).min(b.len());
    }
    i += 1;
    while i < b.len() && b[i] != q && b[i] != b'\n' {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
        }
        i += 1;
    }
    if i < b.len() && b[i] == q {
        i += 1;
    }
    i
}

/// Highlight TOML: comments, table headers, keys, strings, numbers, bools.
pub fn highlight_toml(src: &str) -> String {
    if src.len() > MAX_HIGHLIGHT_BYTES {
        return escape_html(src);
    }
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len() * 2);
    let mut i = 0usize;
    let mut line_start = true;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                out.push('\n');
                i += 1;
                line_start = true;
            }
            b' ' | b'\t' | b'\r' => {
                out.push(c as char);
                i += 1;
            }
            b'#' => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                span("com", &src[start..i], &mut out);
                line_start = false;
            }
            b'[' if line_start => {
                let start = i;
                while i < b.len() && b[i] != b']' && b[i] != b'\n' {
                    i += 1;
                }
                if i < b.len() && b[i] == b']' {
                    i += 1;
                    if i < b.len() && b[i] == b']' {
                        i += 1;
                    }
                    span("table", &src[start..i], &mut out);
                } else {
                    out.push_str(&escape_html(&src[start..i]));
                }
                line_start = false;
            }
            b'"' | b'\'' => {
                let start = i;
                i = scan_string(b, i);
                span("str", &src[start..i], &mut out);
                line_start = false;
            }
            _ => {
                let start = i;
                while i < b.len() && !is_toml_delim(b[i]) {
                    i += 1;
                }
                if i == start {
                    // lone delimiter char: [ ] { } , =
                    out.push_str(&escape_html(&src[i..i + 1]));
                    i += 1;
                } else {
                    let word = &src[start..i];
                    let mut j = i;
                    while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                        j += 1;
                    }
                    let class = if j < b.len() && b[j] == b'=' {
                        "key"
                    } else if word == "true" || word == "false" {
                        "bool"
                    } else if is_numberish(word) {
                        "num"
                    } else {
                        ""
                    };
                    if class.is_empty() {
                        out.push_str(&escape_html(word));
                    } else {
                        span(class, word, &mut out);
                    }
                }
                line_start = false;
            }
        }
    }
    out.push('\n');
    out
}

struct LangCfg {
    line: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    kw: &'static [&'static str],
}

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while",
];
const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try",
    "while", "with", "yield",
];
const JS_KW: &[&str] = &[
    "async", "await", "break", "case", "catch", "class", "const", "continue", "default",
    "delete", "do", "else", "export", "extends", "false", "finally", "for", "function", "if",
    "import", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super",
    "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while",
    "yield",
];
const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac",
    "function", "return", "export", "local", "source", "echo", "cd",
];
const GO_KW: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
    "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
    "return", "select", "struct", "switch", "type", "var",
];
const C_KW: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union",
    "unsigned", "void", "volatile", "while", "class", "namespace", "template", "public",
    "private", "protected", "virtual", "new", "delete", "nullptr", "true", "false", "include",
];

fn lang_cfg(ext: &str) -> Option<LangCfg> {
    Some(match ext {
        "rs" => LangCfg { line: &["//"], block: Some(("/*", "*/")), kw: RUST_KW },
        "py" | "pyw" => LangCfg { line: &["#"], block: None, kw: PY_KW },
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => {
            LangCfg { line: &["//"], block: Some(("/*", "*/")), kw: JS_KW }
        }
        "json" => LangCfg { line: &[], block: None, kw: &[] },
        "sh" | "bash" | "zsh" | "ksh" => LangCfg { line: &["#"], block: None, kw: SH_KW },
        "yaml" | "yml" => LangCfg { line: &["#"], block: None, kw: &[] },
        "go" => LangCfg { line: &["//"], block: Some(("/*", "*/")), kw: GO_KW },
        "c" | "h" | "cpp" | "hpp" | "cc" | "hh" | "cxx" => {
            LangCfg { line: &["//"], block: Some(("/*", "*/")), kw: C_KW }
        }
        "css" | "scss" => LangCfg { line: &["//"], block: Some(("/*", "*/")), kw: &[] },
        "sql" => LangCfg { line: &["--"], block: Some(("/*", "*/")), kw: &[] },
        "lua" | "rb" => LangCfg { line: &["--"], block: None, kw: &[] },
        _ => return None,
    })
}

fn starts_with(b: &[u8], i: usize, pat: &str) -> bool {
    b.len() >= i + pat.len() && &b[i..i + pat.len()] == pat.as_bytes()
}

/// Generic highlighter for common languages: comments, strings, numbers,
/// keywords. JSON/YAML string keys get the `key` color. Unknown extensions
/// return escaped plain text.
pub fn highlight_code(src: &str, ext: &str) -> String {
    let Some(cfg) = lang_cfg(ext) else {
        return escape_html(src);
    };
    if src.len() > MAX_HIGHLIGHT_BYTES {
        return escape_html(src);
    }
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len() * 2);
    let mut i = 0usize;
    let mut plain = 0usize; // start of the current un-classed run
    while i < b.len() {
        if let Some(marker) = cfg.line.iter().find(|m| starts_with(b, i, m)) {
            let _ = marker;
            if plain < i {
                out.push_str(&escape_html(&src[plain..i]));
            }
            let start = i;
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            span("com", &src[start..i], &mut out);
            plain = i;
            continue;
        }
        if let Some((bs, be)) = cfg.block {
            if starts_with(b, i, bs) {
                if plain < i {
                    out.push_str(&escape_html(&src[plain..i]));
                }
                let start = i;
                i += bs.len();
                while i < b.len() && !starts_with(b, i, be) {
                    i += 1;
                }
                i = (i + be.len()).min(b.len());
                span("com", &src[start..i], &mut out);
                plain = i;
                continue;
            }
        }
        let c = b[i];
        if c == b'"' || c == b'\'' || c == b'`' {
            if plain < i {
                out.push_str(&escape_html(&src[plain..i]));
            }
            let start = i;
            i = scan_string(b, i);
            let mut j = i;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            let is_key = (ext == "json" || ext == "yaml" || ext == "yml")
                && j < b.len()
                && b[j] == b':';
            if is_key {
                span("key", &src[start..i], &mut out);
            } else {
                span("str", &src[start..i], &mut out);
            }
            plain = i;
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            if plain < i {
                out.push_str(&escape_html(&src[plain..i]));
            }
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'.' | b'_'))
            {
                i += 1;
            }
            span("num", &src[start..i], &mut out);
            plain = i;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            if cfg.kw.contains(&&src[start..i]) {
                if plain < i {
                    out.push_str(&escape_html(&src[plain..i]));
                }
                span("kw", &src[start..i], &mut out);
                plain = i;
            }
            // not a keyword: the plain run keeps growing
            continue;
        }
        i += 1;
    }
    if plain < i {
        out.push_str(&escape_html(&src[plain..i]));
    }
    out
}

/// Highlight by file name: `.toml` uses the TOML highlighter, everything
/// else is dispatched by extension.
pub fn highlight_auto(name: &str, src: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "toml" => highlight_toml(src),
        _ => highlight_code(src, &ext),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_html() {
        assert_eq!(escape_html("<a \"b\">&'</a>"), "&lt;a &quot;b&quot;&gt;&amp;&#39;&lt;/a&gt;");
    }

    #[test]
    fn toml_tokens() {
        let src = "# note\n[models.gpt]\nname = \"gpt\"\ncount = 3\nflag = true\n";
        let html = highlight_toml(src);
        assert!(html.contains("tok-com"));
        assert!(html.contains("tok-table"));
        assert!(html.contains("tok-key"));
        assert!(html.contains("tok-str"));
        assert!(html.contains("tok-num"));
        assert!(html.contains("tok-bool"));
    }

    #[test]
    fn toml_no_panic_on_odd_input() {
        // unterminated strings and stray brackets must terminate quickly.
        let html = highlight_toml("[[x\na=\"unterminated\nb='''abc\n[[\n]]\n");
        assert!(!html.is_empty());
    }

    #[test]
    fn code_keywords_and_comments() {
        let html = highlight_code("// hi\nfn main() { let x = 1.5; }", "rs");
        assert!(html.contains("tok-com"));
        assert!(html.contains("tok-kw"));
        assert!(html.contains("tok-num"));
    }

    #[test]
    fn json_keys() {
        let html = highlight_code("{\"a\": 1, \"b\": \"x\"}", "json");
        assert!(html.contains("tok-key"));
        assert!(html.contains("tok-str"));
        assert!(html.contains("tok-num"));
    }

    #[test]
    fn unknown_ext_plain() {
        assert_eq!(highlight_auto("notes.txt", "<hi>"), "&lt;hi&gt;");
    }

    #[test]
    fn gutter_counts_lines() {
        assert_eq!(gutter_html("a\nb"), "1\n2\n");
    }
}
