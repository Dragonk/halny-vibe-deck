//! Server functions for the Files / Git dock: directory listing, file
//! previews (text/markdown/csv/images/xlsx/docx/pdf) and git status/diff.
//!
//! All paths are confined to the engine working directory: the remote
//! control mode must never expose the host filesystem.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// shared types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetData {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewData {
    /// text | markdown | csv | image | xlsx | doc | slides | pdf | binary | error
    pub kind: String,
    /// text content / csv source / extracted pdf text
    pub text: String,
    /// data: URL for images
    pub data_url: String,
    /// pdf: rendered pages as data URLs (empty -> text fallback)
    pub pages: Vec<String>,
    /// doc/slides: pre-rendered escaped HTML
    pub html: String,
    /// xlsx: all sheets
    pub sheets: Vec<SheetData>,
    /// text-like kinds can switch to the editor
    pub editable: bool,
    pub file_name: String,
    pub file_size: u64,
}

impl PreviewData {
    pub fn empty() -> Self {
        Self {
            kind: String::new(),
            text: String::new(),
            data_url: String::new(),
            pages: Vec::new(),
            html: String::new(),
            sheets: Vec::new(),
            editable: false,
            file_name: String::new(),
            file_size: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GitFileEntry {
    pub path: String,
    pub code: String,
    pub staged: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GitStatus {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub entries: Vec<GitFileEntry>,
    pub error: String,
}

// ---------------------------------------------------------------------------
// server implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
mod imp {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use vibedeck_engine::Engine;

    const TEXT_CAP: u64 = 512 * 1024;
    const IMAGE_CAP: u64 = 4 * 1024 * 1024;

    pub fn check(token: &str) -> Result<(), ServerFnError> {
        let engine = Engine::global();
        if engine.auth_enabled() && !engine.check_token(token) {
            return Err(ServerFnError::new("unauthorized"));
        }
        Ok(())
    }

    /// Resolve `path` inside the engine working directory. Rejects escapes.
    pub fn confine(path: &str) -> Result<PathBuf, ServerFnError> {
        let root_raw = Engine::global().workdir();
        if root_raw.trim().is_empty() {
            return Err(ServerFnError::new("no working directory configured"));
        }
        let root = fs::canonicalize(&root_raw)
            .map_err(|_| ServerFnError::new("working directory does not exist"))?;
        let rel = path.trim();
        let target = if rel.is_empty() {
            root.clone()
        } else {
            let p = Path::new(rel);
            let joined = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            };
            // lexical normalisation without requiring existence
            let mut norm = PathBuf::new();
            for comp in joined.components() {
                match comp {
                    std::path::Component::ParentDir => {
                        if !norm.pop() {
                            return Err(ServerFnError::new("path escapes working directory"));
                        }
                    }
                    std::path::Component::CurDir => {}
                    other => norm.push(other.as_os_str()),
                }
            }
            norm
        };
        if target != root && !target.starts_with(&root) {
            return Err(ServerFnError::new("path escapes working directory"));
        }
        Ok(target)
    }

    pub fn list_dir(path: &str) -> Result<Vec<FileEntry>, ServerFnError> {
        let dir = confine(path)?;
        let mut out: Vec<FileEntry> = Vec::new();
        let display_root = Engine::global().workdir();
        for entry in fs::read_dir(&dir).map_err(|e| ServerFnError::new(e.to_string()))? {
            let Ok(entry) = entry else { continue };
            let meta = entry.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" && is_dir {
                continue;
            }
            let full = entry.path();
            // store paths relative to the working directory
            let rel = strip_root(&full, &display_root);
            out.push(FileEntry {
                name,
                path: rel,
                is_dir,
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified_ms: meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            });
        }
        out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(out)
    }

    fn strip_root(full: &Path, root: &str) -> String {
        let root_p = Path::new(root);
        if let Ok(rel) = full.strip_prefix(root_p) {
            rel.to_string_lossy().into_owned()
        } else {
            full.to_string_lossy().into_owned()
        }
    }

    fn read_capped(path: &Path, cap: u64) -> Result<Vec<u8>, ServerFnError> {
        let meta = fs::metadata(path).map_err(|e| ServerFnError::new(e.to_string()))?;
        if meta.len() > cap {
            return Err(ServerFnError::new(format!(
                "file too large ({} bytes, limit {})",
                meta.len(),
                cap
            )));
        }
        fs::read(path).map_err(|e| ServerFnError::new(e.to_string()))
    }

    pub fn read_file(path: &str) -> PreviewData {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let full = match confine(path) {
            Ok(p) => p,
            Err(e) => {
                return PreviewData {
                    kind: "error".into(),
                    text: e.to_string(),
                    file_name: name,
                    ..PreviewData::empty()
                }
            }
        };
        let size = fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
        let err = |msg: String| PreviewData {
            kind: "error".into(),
            text: msg,
            file_name: name.clone(),
            file_size: size,
            ..PreviewData::empty()
        };
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let base = PreviewData::empty();
        const DOC_CAP: u64 = 25 * 1024 * 1024;
        match ext.as_str() {
            "md" | "markdown" => match read_capped(&full, TEXT_CAP) {
                Ok(bytes) => PreviewData {
                    kind: "markdown".into(),
                    text: String::from_utf8_lossy(&bytes).into_owned(),
                    editable: true,
                    file_name: name,
                    file_size: size,
                    ..base
                },
                Err(e) => err(e.to_string()),
            },
            "csv" | "tsv" => match read_capped(&full, 1024 * 1024) {
                Ok(bytes) => PreviewData {
                    kind: "csv".into(),
                    text: String::from_utf8_lossy(&bytes).into_owned(),
                    editable: true,
                    file_name: name,
                    file_size: size,
                    ..base
                },
                Err(e) => err(e.to_string()),
            },
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "svg" => {
                match read_capped(&full, IMAGE_CAP) {
                    Ok(bytes) => {
                        let mime = if ext == "svg" {
                            "image/svg+xml"
                        } else if ext == "png" {
                            "image/png"
                        } else if ext == "gif" {
                            "image/gif"
                        } else if ext == "webp" {
                            "image/webp"
                        } else if ext == "bmp" {
                            "image/bmp"
                        } else if ext == "ico" {
                            "image/x-icon"
                        } else if ext == "tiff" {
                            "image/tiff"
                        } else {
                            "image/jpeg"
                        };
                        let b64 = b64_encode(&bytes);
                        PreviewData { kind: "image".into(), data_url: format!("data:{mime};base64,{b64}"), file_name: name, file_size: size, ..base }
                    }
                    Err(e) => err(e.to_string()),
                }
            }
            "xlsx" | "xls" | "xlsm" | "ods" => {
                if size > DOC_CAP {
                    return err("file too large".into());
                }
                match crate::preview::xlsx_sheets(&full) {
                    Ok(sheets) => PreviewData { kind: "xlsx".into(), sheets, file_name: name, file_size: size, ..base },
                    Err(e) => err(e),
                }
            }
            "docx" | "odt" => {
                if size > DOC_CAP {
                    return err("file too large".into());
                }
                let html = if ext == "docx" {
                    crate::preview::docx_to_html(&full)
                } else {
                    crate::preview::odt_to_html(&full)
                };
                match html {
                    Ok(html) => PreviewData { kind: "doc".into(), html, file_name: name, file_size: size, ..base },
                    Err(e) => err(e),
                }
            }
            "pptx" | "odp" => {
                if size > DOC_CAP {
                    return err("file too large".into());
                }
                let html = if ext == "pptx" {
                    crate::preview::pptx_to_html(&full)
                } else {
                    crate::preview::odp_to_html(&full)
                };
                match html {
                    Ok(html) => PreviewData { kind: "slides".into(), html, file_name: name, file_size: size, ..base },
                    Err(e) => err(e),
                }
            }
            "pdf" => {
                if size > DOC_CAP {
                    return err("file too large".into());
                }
                match crate::preview::pdf_pages(&full) {
                    Ok(pages) if !pages.is_empty() => PreviewData { kind: "pdf".into(), pages, file_name: name, file_size: size, ..base },
                    _ => match pdf_text(&full) {
                        Ok(text) => PreviewData { kind: "pdf".into(), text, file_name: name, file_size: size, ..base },
                        Err(e) => err(e),
                    },
                }
            }
            "txt" | "log" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "json" | "toml"
            | "yaml" | "yml" | "html" | "css" | "sh" | "sql" | "go" | "c" | "h" | "cpp"
            | "java" | "kt" | "rb" | "php" | "xml" | "ini" | "conf" | "env" | "lock" | "" => {
                match read_capped(&full, TEXT_CAP) {
                    Ok(bytes) => PreviewData {
                        kind: "text".into(),
                        text: String::from_utf8_lossy(&bytes).into_owned(),
                        editable: true,
                        file_name: name,
                        file_size: size,
                        ..base
                    },
                    Err(e) => err(e.to_string()),
                }
            }
            _ => {
                if is_probably_binary(&ext) {
                    PreviewData { kind: "binary".into(), file_name: name, file_size: size, ..base }
                } else {
                    match read_capped(&full, TEXT_CAP) {
                        Ok(bytes) => PreviewData {
                            kind: "text".into(),
                            text: String::from_utf8_lossy(&bytes).into_owned(),
                            editable: true,
                            file_name: name,
                            file_size: size,
                            ..base
                        },
                        Err(e) => err(e.to_string()),
                    }
                }
            }
        }
    }

    pub fn save_file(path: &str, content: &str) -> Result<(), ServerFnError> {
        let full = confine(path)?;
        if content.len() > 1024 * 1024 {
            return Err(ServerFnError::new("content too large (limit 1 MB)".to_string()));
        }
        fs::write(&full, content).map_err(|e| ServerFnError::new(e.to_string()))
    }

    fn is_probably_binary(ext: &str) -> bool {
        matches!(
            ext,
            "exe" | "dll" | "so" | "dylib" | "bin" | "zip" | "tar" | "gz" | "7z" | "rar"
                | "woff" | "woff2" | "ttf" | "otf" | "class" | "jar" | "wasm" | "mp3" | "mp4"
                | "avi" | "mov" | "sqlite" | "db" | "pyc" | "o" | "a" | "lib" | "obj"
        )
    }

    pub fn b64_encode(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).map(|b| *b as u32).unwrap_or(0);
            let b2 = chunk.get(2).map(|b| *b as u32).unwrap_or(0);
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        out
    }

        // ---- pdf (pdf-extract) ----
    fn pdf_text(path: &Path) -> Result<String, String> {
        let text = pdf_extract::extract_text(path).map_err(|e| format!("pdf: {e}"))?;
        let mut out = text;
        if out.len() > 512 * 1024 {
            out.truncate(512 * 1024);
        }
        Ok(out)
    }

    // ---- git ----
    fn git(args: &[&str]) -> Result<String, String> {
        let cwd = Engine::global().workdir();
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&cwd)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() && !out.stderr.is_empty() {
            let err = String::from_utf8_lossy(&out.stderr).into_owned();
            if err.contains("not a git repository") || err.contains("not have a repository") {
                return Err("not a git repository".into());
            }
            return Err(err.trim().to_string());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn git_status() -> GitStatus {
        let mut st = GitStatus {
            branch: String::new(),
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            error: String::new(),
        };
        match git(&["status", "--porcelain=v1", "-b"]) {
            Ok(out) => {
                for line in out.lines() {
                    if line.starts_with("##") {
                        let header = line[2..].trim();
                        // "main...origin/main [ahead 1, behind 2]"
                        let branch = header.split("...").next().unwrap_or("").trim();
                        let branch = branch.split(' ').next().unwrap_or("").trim();
                        st.branch = branch.to_string();
                        if let Some(pos) = header.find("ahead ") {
                            st.ahead = header[pos + 6..]
                                .split(|c: char| !c.is_ascii_digit())
                                .next()
                                .unwrap_or("")
                                .parse()
                                .unwrap_or(0);
                        }
                        if let Some(pos) = header.find("behind ") {
                            st.behind = header[pos + 7..]
                                .split(|c: char| !c.is_ascii_digit())
                                .next()
                                .unwrap_or("")
                                .parse()
                                .unwrap_or(0);
                        }
                        continue;
                    }
                    if line.len() < 4 {
                        continue;
                    }
                    let x = line.as_bytes()[0];
                    let y = line.as_bytes()[1];
                    let path = line[3..].trim().to_string();
                    if path.is_empty() {
                        continue;
                    }
                    let (code, staged) = if y != b' ' {
                        (y as char, false)
                    } else if x != b' ' {
                        (x as char, true)
                    } else {
                        continue;
                    };
                    st.entries.push(GitFileEntry {
                        path: path.split(" -> ").last().unwrap_or(&path).to_string(),
                        code: code.to_string(),
                        staged,
                    });
                }
            }
            Err(e) => st.error = e,
        }
        st
    }

    pub fn git_diff(staged: bool, path: &str) -> String {
        let mut args: Vec<String> = vec!["diff".into()];
        if staged {
            args.push("--cached".into());
        }
        if !path.trim().is_empty() {
            args.push("--".into());
            args.push(path.trim().to_string());
        }
        let owned: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        git(&owned).unwrap_or_default()
    }

    /// Whole working-tree diff (staged + unstaged), used by the input
    /// bar's "branch diff" attach action (Nexus `git.diffAll`).
    pub fn git_diff_all() -> String {
        git(&["diff", "HEAD"]).unwrap_or_default()
    }

    /// Fuzzy file search under the working directory for the `@` picker
    /// (Nexus `fs.searchFiles`): BFS with depth and result caps, skipping
    /// heavy directories. An empty query returns the top-level entries.
    pub fn search_files(query: &str) -> Vec<FileEntry> {
        const MAX_DEPTH: usize = 6;
        const MAX_VISITED: usize = 4000;
        const MAX_RESULTS: usize = 60;

        let Ok(root) = confine("") else {
            return Vec::new();
        };
        let q = query.to_lowercase();
        let mut results: Vec<FileEntry> = Vec::new();
        let mut visited: usize = 0;

        // (dir, depth)
        let mut queue: std::collections::VecDeque<(std::path::PathBuf, usize)> =
            std::collections::VecDeque::new();
        queue.push_back((root, 0));

        while let Some((dir, depth)) = queue.pop_front() {
            visited += 1;
            if visited > MAX_VISITED || results.len() >= MAX_RESULTS {
                break;
            }
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries {
                if results.len() >= MAX_RESULTS {
                    break;
                }
                let Ok(entry) = entry else { continue };
                let meta = entry.metadata().ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" {
                    continue;
                }
                let is_heavy = is_heavy_dir(&name);
                let display_root = Engine::global().workdir();
                let rel = strip_root(&entry.path(), &display_root);
                if !q.is_empty() && !rel.to_lowercase().contains(&q) {
                    // still descend into matching dirs so nested hits appear
                    if is_dir && depth < MAX_DEPTH && !is_heavy {
                        queue.push_back((entry.path(), depth + 1));
                    }
                    continue;
                }
                results.push(FileEntry {
                    name,
                    path: rel,
                    is_dir,
                    size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified_ms: meta
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0),
                });
                if is_dir && depth < MAX_DEPTH && !is_heavy {
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
        results
    }

    fn is_heavy_dir(name: &str) -> bool {
        matches!(
            name,
            "node_modules" | "target" | ".venv" | "venv" | "dist" | "build" | "__pycache__" | ".next" | ".cache"
        )
    }
}

// ---------------------------------------------------------------------------
// server functions
// ---------------------------------------------------------------------------

#[server]
pub async fn api_list_dir(token: String, path: String) -> Result<Vec<FileEntry>, ServerFnError> {
    imp::check(&token)?;
    imp::list_dir(&path)
}

#[server]
pub async fn api_read_file(token: String, path: String) -> Result<PreviewData, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::read_file(&path))
}

#[server]
pub async fn api_save_file(token: String, path: String, content: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::save_file(&path, &content)
}

#[server]
pub async fn api_git_status(token: String) -> Result<GitStatus, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_status())
}

#[server]
pub async fn api_git_diff(token: String, staged: bool, path: String) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_diff(staged, &path))
}

/// Fuzzy file search under the working directory (the `@` picker).
#[server]
pub async fn api_search_files(token: String, query: String) -> Result<Vec<FileEntry>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::search_files(&query))
}

/// Whole working-tree diff for the "branch diff" attach action.
#[server]
pub async fn api_git_diff_all(token: String) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_diff_all())
}
