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

/// One entry of a zip archive listing (Nexus FilePreview ZipView).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZipEntry {
    pub name: String,
    pub size: u64,
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
    /// zip: archive listing (Nexus ZipView)
    #[serde(default)]
    pub zip_entries: Vec<ZipEntry>,

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
            zip_entries: Vec::new(),

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
    /// False when the directory is not inside a git repository (Nexus
    /// `GitStatus.repo`) — the panel shows "Not a git repository".
    #[serde(default)]
    pub repo: bool,
}

/// One commit of the git history (Nexus `GitCommit`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GitCommit {
    pub hash: String,
    pub short_hash: String,
    /// Commit subject (first line).
    pub message: String,
    /// "Name <email>".
    pub author: String,
    /// ISO date.
    pub date: String,
}

/// Parse `git status --porcelain=v1 -b` output (Nexus `gitStatus`).
/// Pure function (no process spawn) so it can be unit-tested without
/// the server build. `staged` follows Nexus: a file is staged when the
/// index column carries a change (anything but ' '/'?'), and the shown
/// status code prefers the index column.
pub fn parse_git_status(out: &str) -> GitStatus {
    let mut st = GitStatus {
        branch: String::new(),
        ahead: 0,
        behind: 0,
        entries: Vec::new(),
        error: String::new(),
        repo: true,
    };
    for line in out.lines() {
        if line.starts_with("##") {
            let header = line[2..].trim();
            // "main...origin/main [ahead 1, behind 2]" — but an unborn
            // branch reads "No commits yet on main" (simple-git / Nexus
            // still reports the branch name).
            let branch_part = if let Some(rest) = header.strip_prefix("No commits yet on ") {
                rest
            } else {
                header.split("...").next().unwrap_or("").trim()
            };
            let branch = branch_part.split(' ').next().unwrap_or("").trim();
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
        // Nexus: index !== ' ' && index !== '?' => staged. The shown code
        // prefers the index column, falling back to the working column.
        let (code, staged) = if x != b' ' {
            (x as char, x != b'?')
        } else if y != b' ' {
            (y as char, false)
        } else {
            continue;
        };
        st.entries.push(GitFileEntry {
            path: path.split(" -> ").last().unwrap_or(&path).to_string(),
            code: code.to_string(),
            staged,
        });
    }
    st
}

// ---------------------------------------------------------------------------
// remote path helpers (Nexus remoteFs.ts)
// ---------------------------------------------------------------------------

/// Resolve a browse path against the remote session root: empty = the
/// root itself, a leading `/` (chat path clicks) passes through as an
/// absolute remote path, anything else is joined; `..` is rejected.
pub fn remote_abs(root: &str, rel: &str) -> Result<String, String> {
    let raw = root.trim();
    if raw.is_empty() {
        return Err("no remote working directory".into());
    }
    // normalize one trailing slash ("/" itself stays intact)
    let root = if raw.len() > 1 && raw.ends_with('/') {
        &raw[..raw.len() - 1]
    } else {
        raw
    };
    let rel = rel.trim();
    if rel.is_empty() {
        return Ok(root.to_string());
    }
    if rel.starts_with('/') {
        return Ok(rel.to_string());
    }
    for c in rel.split('/') {
        if c == ".." {
            return Err("path escapes working directory".into());
        }
    }
    if root == "/" {
        Ok(format!("/{rel}"))
    } else {
        Ok(format!("{root}/{rel}"))
    }
}

/// Parse `find -printf '%y|%s|%T@|%f'` lines into (type, size, mtime_s, name).
pub fn parse_find_ls(out: &str) -> Vec<(char, u64, u64, String)> {
    let mut v = Vec::new();
    for line in out.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() != 4 {
            continue;
        }
        let y = parts[0].chars().next().unwrap_or('f');
        let size = parts[1].parse().unwrap_or(0);
        let mtime = parts[2].parse::<f64>().unwrap_or(0.0).max(0.0) as u64;
        v.push((y, size, mtime, parts[3].to_string()));
    }
    v
}

// ---------------------------------------------------------------------------
// server implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "server")]
mod imp {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use mistraldeck_engine::Engine;

    const TEXT_CAP: u64 = 512 * 1024;
    const IMAGE_CAP: u64 = 4 * 1024 * 1024;

    pub fn check(_token: &str) -> Result<(), ServerFnError> {
        // Same auth model as api.rs: no token enforcement inside the
        // server functions. Requests arrive either from a loopback
        // listener (trusted) or through the remote HTTPS listener, whose
        // middleware already validated the session cookie before the
        // request can reach this function. Enforcing the (vestigial)
        // token here broke every file-tree call from the local desktop app
        // whenever remote-access credentials were configured.
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

    /// Base64 of a whole file (download button; capped so a huge binary
    /// cannot blow up the response).
    pub fn download_base64(
        path: &str,
        remote_id: Option<&str>,
        _remote_root: &str,
    ) -> Result<String, ServerFnError> {
        use base64::Engine as _;
        let bytes = if let Some(rid) = remote_id.filter(|s| !s.is_empty()) {
            // Remote download: the file is base64-encoded on the host so the
            // SSH pipe stays text (same trick as the preview reader).
            let conn = Engine::global()
                .remote_connection(rid)
                .ok_or_else(|| ServerFnError::new("remote connection not found"))?;
            let cmd = format!("base64 -w0 {}", mistraldeck_engine::remote::shq(path));
            let res = mistraldeck_engine::remote::ssh_run(&conn, &cmd, 120_000, None);
            if res.code != 0 {
                return Err(ServerFnError::new(format!("ssh failed: {}", res.stderr)));
            }
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(res.stdout.trim().as_bytes())
                .map_err(|e| ServerFnError::new(e.to_string()))?
        } else {
            let p = std::path::Path::new(path);
            let meta = std::fs::metadata(p).map_err(|e| ServerFnError::new(e.to_string()))?;
            if meta.len() > 64 * 1024 * 1024 {
                return Err(ServerFnError::new("file is larger than 64 MB"));
            }
            std::fs::read(p).map_err(|e| ServerFnError::new(e.to_string()))?
        };
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn list_dir(
        path: &str,
        remote_id: Option<&str>,
        remote_root: &str,
    ) -> Result<Vec<FileEntry>, ServerFnError> {
        if let Some(rid) = remote_id.filter(|s| !s.is_empty()) {
            let conn = Engine::global()
                .remote_connection(rid)
                .ok_or_else(|| ServerFnError::new("remote connection not found"))?;
            return Ok(remote_list_dir(&conn, remote_root, path));
        }
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

    pub fn read_file(path: &str, remote_id: Option<&str>, remote_root: &str) -> PreviewData {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(rid) = remote_id.filter(|s| !s.is_empty()) {
            let conn = match Engine::global().remote_connection(rid) {
                Some(c) => c,
                None => {
                    return PreviewData {
                        kind: "error".into(),
                        text: "remote connection not found".into(),
                        file_name: name,
                        ..PreviewData::empty()
                    }
                }
            };
            return remote_read_file(&conn, remote_root, path, name);
        }
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
        preview_from_path(&full, name, size)
    }

    /// The preview pipeline shared by local and remote reads (Nexus
    /// FilePreview kinds): the sniffers run on an existing file.
    fn preview_from_path(full: &Path, name: String, size: u64) -> PreviewData {
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
            "zip" => {
                if size > DOC_CAP {
                    return err("file too large".into());
                }
                match crate::preview::zip_listing(&full) {
                    Ok(entries) => PreviewData {
                        kind: "zip".into(),
                        zip_entries: entries,
                        file_name: name,
                        file_size: size,
                        ..base
                    },
                    Err(e) => err(e),
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

    pub fn save_file(
        path: &str,
        content: &str,
        remote_id: Option<&str>,
        remote_root: &str,
    ) -> Result<(), ServerFnError> {
        if let Some(rid) = remote_id.filter(|s| !s.is_empty()) {
            let conn = Engine::global()
                .remote_connection(rid)
                .ok_or_else(|| ServerFnError::new("remote connection not found"))?;
            return remote_save_file(&conn, remote_root, path, content);
        }
        let full = confine(path)?;
        if content.len() > 1024 * 1024 {
            return Err(ServerFnError::new("content too large (limit 1 MB)".to_string()));
        }
        fs::write(&full, content).map_err(|e| ServerFnError::new(e.to_string()))
    }

    fn is_probably_binary(ext: &str) -> bool {
        matches!(
            ext,
            "exe" | "dll" | "so" | "dylib" | "bin" | "tar" | "gz" | "7z" | "rar"
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

    // ---- remote fs over SSH (Nexus remoteFs.ts) ----

    /// Nexus BINARY_CAP: the largest file pulled over SSH for a preview.
    const REMOTE_BINARY_CAP: u64 = 48 * 1024 * 1024;

    fn remote_stat(
        conn: &mistraldeck_core::snapshot::RemoteConnection,
        path: &str,
    ) -> Option<(bool, u64)> {
        let cmd = format!(
            "stat -c '%F|%s' {} 2>/dev/null",
            mistraldeck_engine::remote::shq(path)
        );
        let res = mistraldeck_engine::remote::ssh_run(conn, &cmd, 15_000, None);
        if res.code != 0 || res.stdout.trim().is_empty() {
            return None;
        }
        let mut it = res.stdout.trim().splitn(2, '|');
        let ty = it.next().unwrap_or("");
        let size = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        Some((ty == "directory", size))
    }

    fn remote_list_dir(
        conn: &mistraldeck_core::snapshot::RemoteConnection,
        root: &str,
        rel: &str,
    ) -> Vec<FileEntry> {
        let Ok(abs) = remote_abs(root, rel) else {
            return Vec::new();
        };
        let cmd = format!(
            "find {} -mindepth 1 -maxdepth 1 -printf '%y|%s|%T@|%f\\n' 2>/dev/null",
            mistraldeck_engine::remote::shq(&abs)
        );
        let res = mistraldeck_engine::remote::ssh_run(conn, &cmd, 20_000, None);
        if res.code != 0 {
            return Vec::new();
        }
        let mut out: Vec<FileEntry> = Vec::new();
        for (y, size, mtime_s, name) in parse_find_ls(&res.stdout) {
            let is_dir = y == 'd';
            if is_dir && name == ".git" {
                continue;
            }
            if name == "." || name == ".." || name.is_empty() {
                continue;
            }
            let path = if rel.trim().is_empty() {
                name.clone()
            } else {
                format!("{}/{}", rel.trim_end_matches('/'), name)
            };
            out.push(FileEntry {
                name,
                path,
                is_dir,
                size,
                modified_ms: mtime_s.saturating_mul(1000),
            });
        }
        out.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        out
    }

    fn remote_read_file(
        conn: &mistraldeck_core::snapshot::RemoteConnection,
        root: &str,
        path: &str,
        name: String,
    ) -> PreviewData {
        let err = |msg: String| PreviewData {
            kind: "error".into(),
            text: msg,
            file_name: name.clone(),
            ..PreviewData::empty()
        };
        let Ok(abs) = remote_abs(root, path) else {
            return err("no remote working directory".into());
        };
        let Some((_, size)) = remote_stat(conn, &abs) else {
            return err(format!("File not found: {abs}"));
        };
        if size > REMOTE_BINARY_CAP {
            return err("file too large (limit 48 MB)".into());
        }
        let cmd = format!(
            "base64 -w0 {} 2>/dev/null",
            mistraldeck_engine::remote::shq(&abs)
        );
        let res = mistraldeck_engine::remote::ssh_run(conn, &cmd, 120_000, None);
        if res.code != 0 {
            let msg = if res.stderr.trim().is_empty() {
                "Read failed".to_string()
            } else {
                res.stderr.trim().to_string()
            };
            return err(msg);
        }
        let bytes = crate::pwa::decode_base64(res.stdout.trim());
        // stage the bytes in a temp file and run the local preview
        // pipeline on them (all kinds: xlsx/docx/pdf/zip/images/text)
        static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = std::env::temp_dir()
            .join(format!("mistraldeck-remote-preview-{}-{seq}.bin", std::process::id()));
        if let Err(e) = fs::write(&tmp, &bytes) {
            return err(e.to_string());
        }
        let data = preview_from_path(&tmp, name, size);
        let _ = fs::remove_file(&tmp);
        data
    }

    fn remote_save_file(
        conn: &mistraldeck_core::snapshot::RemoteConnection,
        root: &str,
        path: &str,
        content: &str,
    ) -> Result<(), ServerFnError> {
        let abs = remote_abs(root, path).map_err(ServerFnError::new)?;
        if content.len() > 1024 * 1024 {
            return Err(ServerFnError::new(
                "content too large (limit 1 MB)".to_string(),
            ));
        }
        let b64 = b64_encode(content.as_bytes());
        let cmd = format!("base64 -d > {}", mistraldeck_engine::remote::shq(&abs));
        let res = mistraldeck_engine::remote::ssh_run(conn, &cmd, 60_000, Some(&b64));
        if res.code != 0 {
            let msg = if res.stderr.trim().is_empty() {
                "Write failed".to_string()
            } else {
                res.stderr.trim().to_string()
            };
            return Err(ServerFnError::new(msg));
        }
        Ok(())
    }

    fn remote_search_files(
        conn: &mistraldeck_core::snapshot::RemoteConnection,
        root: &str,
        query: &str,
    ) -> Vec<FileEntry> {
        // port 1:1 of the Nexus remoteSearchFiles find (pruned, capped)
        let cmd = format!(
            "cd {} 2>/dev/null && find . -maxdepth 4 \\( -name .git -o -name node_modules -o -name .venv -o -name venv -o -name __pycache__ -o -name dist -o -name build -o -name out -o -name target -o -name .next -o -name .cache \\) -prune -o -printf '%y|%P\\n' 2>/dev/null | head -2000",
            mistraldeck_engine::remote::shq(root)
        );
        let res = mistraldeck_engine::remote::ssh_run(conn, &cmd, 30_000, None);
        let q = query.to_lowercase();
        let mut results: Vec<FileEntry> = Vec::new();
        for line in res.stdout.lines() {
            let Some((y, rel)) = line.split_once('|') else {
                continue;
            };
            if rel.is_empty() || rel == "." {
                continue;
            }
            if !q.is_empty() && !rel.to_lowercase().contains(&q) {
                continue;
            }
            let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
            results.push(FileEntry {
                name,
                path: rel.to_string(),
                is_dir: y.trim() == "d",
                size: 0,
                modified_ms: 0,
            });
            if results.len() >= 50 {
                break;
            }
        }
        results
    }

    // ---- git ----
    // Git runs on the *session* working directory (Nexus parity), not the
    // engine workdir: local sessions spawn `git -C <cwd>` in-process,
    // remote (SSH) sessions pipe the same command through the
    // connection's ssh command (GitPanel v0.7.0 remoteId).

    enum GitCtx {
        Local(String),
        Remote(mistraldeck_core::snapshot::RemoteConnection, String),
    }

    impl GitCtx {
        /// Resolve the execution context from a session cwd + remote id.
        fn resolve(cwd: &str, remote_id: Option<&str>) -> Result<GitCtx, String> {
            if cwd.trim().is_empty() {
                return Err("no working directory".into());
            }
            match remote_id {
                None | Some("") => Ok(GitCtx::Local(cwd.to_string())),
                Some(rid) => match Engine::global().remote_connection(rid) {
                    Some(conn) => Ok(GitCtx::Remote(conn, cwd.to_string())),
                    None => Err("remote connection not found".into()),
                },
            }
        }
    }

    fn git_run(ctx: &GitCtx, args: &[String]) -> Result<String, String> {
        fn not_a_repo(err: String) -> String {
            if err.contains("not a git repository") || err.contains("not have a repository") {
                "not a git repository".to_string()
            } else {
                err
            }
        }
        match ctx {
            GitCtx::Local(cwd) => {
                let out = std::process::Command::new("git")
                    .arg("-C")
                    .arg(cwd)
                    .args(args)
                    .output()
                    .map_err(|e| e.to_string())?;
                if !out.status.success() {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    return Err(not_a_repo(err));
                }
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            }
            GitCtx::Remote(conn, cwd) => {
                let mut remote_cmd = format!("git -C {}", mistraldeck_engine::remote::shq(cwd));
                for a in args {
                    remote_cmd.push(' ');
                    remote_cmd.push_str(&mistraldeck_engine::remote::shq(a));
                }
                let res = mistraldeck_engine::remote::ssh_run(conn, &remote_cmd, 60_000, None);
                if res.code != 0 {
                    let err = if !res.stderr.trim().is_empty() {
                        res.stderr.trim().to_string()
                    } else if !res.stdout.trim().is_empty() {
                        res.stdout.trim().to_string()
                    } else {
                        format!("ssh exited with code {}", res.code)
                    };
                    return Err(not_a_repo(err));
                }
                Ok(res.stdout)
            }
        }
    }

    pub fn git_status(cwd: &str, remote_id: Option<&str>) -> GitStatus {
        let mut st = GitStatus {
            branch: String::new(),
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            error: String::new(),
            repo: false,
        };
        let ctx = match GitCtx::resolve(cwd, remote_id) {
            Ok(c) => c,
            Err(e) => {
                st.error = e;
                return st;
            }
        };
        let args = vec![
            "status".to_string(),
            "--porcelain=v1".to_string(),
            "-b".to_string(),
        ];
        match git_run(&ctx, &args) {
            Ok(out) => st = parse_git_status(&out),
            // Nexus maps a missing repository to `repo: false` (the panel
            // shows "Not a git repository", not an error banner).
            Err(e) if e == "not a git repository" => {}
            Err(e) => st.error = e,
        }
        st
    }

    pub fn git_log(cwd: &str, remote_id: Option<&str>, max: u32) -> Vec<GitCommit> {
        let Ok(ctx) = GitCtx::resolve(cwd, remote_id) else {
            return Vec::new();
        };
        let args = vec![
            "log".to_string(),
            format!("-n{max}"),
            // hash, subject, author name, author email, ISO date
            "--pretty=format:%H%x1f%s%x1f%an%x1f%ae%x1f%aI".to_string(),
        ];
        let Ok(out) = git_run(&ctx, &args) else {
            return Vec::new();
        };
        let mut commits = Vec::new();
        for line in out.lines() {
            let parts: Vec<&str> = line.split('\u{1f}').collect();
            if parts.len() < 5 {
                continue;
            }
            let hash = parts[0].to_string();
            commits.push(GitCommit {
                short_hash: hash.chars().take(7).collect(),
                hash,
                message: parts[1].to_string(),
                author: if parts[3].is_empty() {
                    parts[2].to_string()
                } else {
                    format!("{} <{}>", parts[2], parts[3])
                },
                date: parts[4].to_string(),
            });
        }
        commits
    }

    /// `git add -- <paths>`; empty path list is a no-op (Nexus gitStage).
    pub fn git_stage(cwd: &str, remote_id: Option<&str>, paths: &[String]) -> String {
        if paths.is_empty() {
            return String::new();
        }
        let ctx = match GitCtx::resolve(cwd, remote_id) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let mut args = vec!["add".to_string(), "--".to_string()];
        args.extend(paths.iter().cloned());
        git_run(&ctx, &args).map(|_| String::new()).unwrap_or_else(|e| e)
    }

    /// `git reset -- <paths>`; empty path list is a no-op (Nexus gitUnstage).
    pub fn git_unstage(cwd: &str, remote_id: Option<&str>, paths: &[String]) -> String {
        if paths.is_empty() {
            return String::new();
        }
        let ctx = match GitCtx::resolve(cwd, remote_id) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let mut args = vec!["reset".to_string(), "--".to_string()];
        args.extend(paths.iter().cloned());
        git_run(&ctx, &args).map(|_| String::new()).unwrap_or_else(|e| e)
    }

    /// `git commit -m <msg>` (Nexus gitCommit).
    pub fn git_commit(cwd: &str, remote_id: Option<&str>, message: &str) -> String {
        if message.trim().is_empty() {
            return "Empty commit message".to_string();
        }
        let ctx = match GitCtx::resolve(cwd, remote_id) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let args = vec![
            "commit".to_string(),
            "-m".to_string(),
            message.to_string(),
        ];
        git_run(&ctx, &args).map(|_| String::new()).unwrap_or_else(|e| e)
    }

    /// `git push` (Nexus gitPush).
    pub fn git_push(cwd: &str, remote_id: Option<&str>) -> String {
        let ctx = match GitCtx::resolve(cwd, remote_id) {
            Ok(c) => c,
            Err(e) => return e,
        };
        let args = vec!["push".to_string()];
        git_run(&ctx, &args).map(|_| String::new()).unwrap_or_else(|e| e)
    }

    pub fn git_diff(cwd: &str, remote_id: Option<&str>, staged: bool, path: &str) -> String {
        let Ok(ctx) = GitCtx::resolve(cwd, remote_id) else {
            return String::new();
        };
        let mut args: Vec<String> = vec!["diff".into()];
        if staged {
            args.push("--cached".into());
        }
        if !path.trim().is_empty() {
            args.push("--".into());
            args.push(path.trim().to_string());
        }
        git_run(&ctx, &args).unwrap_or_default()
    }

    /// Whole working-tree diff (staged + unstaged), used by the input
    /// bar's "branch diff" attach action (Nexus `git.diffAll`).
    pub fn git_diff_all(cwd: &str, remote_id: Option<&str>) -> String {
        let Ok(ctx) = GitCtx::resolve(cwd, remote_id) else {
            return String::new();
        };
        let args = vec!["diff".to_string(), "HEAD".to_string()];
        git_run(&ctx, &args).unwrap_or_default()
    }

    /// Fuzzy file search under the working directory for the `@` picker
    /// (Nexus `fs.searchFiles`): BFS with depth and result caps, skipping
    /// heavy directories. An empty query returns the top-level entries.
    pub fn search_files(query: &str, remote_id: Option<&str>, remote_root: &str) -> Vec<FileEntry> {
        if let Some(rid) = remote_id.filter(|s| !s.is_empty()) {
            let Some(conn) = Engine::global().remote_connection(rid) else {
                return Vec::new();
            };
            return remote_search_files(&conn, remote_root, query);
        }
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

/// List a directory. Local sessions list the engine working directory
/// (paths relative to it); remote sessions list the remote session root
/// over SSH, paths relative to that root (Nexus remoteFs.remoteListDir).
#[server]
pub async fn api_list_dir(
    token: String,
    path: String,
    remote_id: Option<String>,
    remote_root: String,
) -> Result<Vec<FileEntry>, ServerFnError> {
    imp::check(&token)?;
    imp::list_dir(&path, remote_id.as_deref(), &remote_root)
}

/// Raw bytes of one file as base64 (preview header "download", Z21).
#[server]
pub async fn api_download_base64(
    token: String,
    path: String,
    remote_id: Option<String>,
    remote_root: String,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    imp::download_base64(&path, remote_id.as_deref(), &remote_root)
}

/// Open a file with the OS default application (desktop only; the browser
/// build hides the button — the stable edition does the same).
#[server]
pub async fn api_open_externally(token: String, path: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    let target = path.trim().to_string();
    if target.is_empty() {
        return Err(ServerFnError::new("empty path"));
    }
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", &target]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(&target);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&target);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| ServerFnError::new(e.to_string()))
}

/// Create a project folder (the picker's "new folder" field).
#[server]
pub async fn api_create_dir(token: String, path: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    std::fs::create_dir_all(path.trim()).map_err(|e| ServerFnError::new(e.to_string()))
}

/// Home directory of this host (start of the "New project…" picker).
#[server]
pub async fn api_home_dir(token: String) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(dirs::home_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_else(|| "/".to_string()))
}

/// List the **directories** of an absolute path for the project picker.
///
/// Unlike [`api_list_dir`] this is not confined to the engine working
/// directory: choosing a project means browsing the whole disk (the stable
/// edition's `NewProjectModal` does the same). Only directory entries are
/// returned, sorted by name.
#[server]
pub async fn api_browse_dirs(token: String, path: String) -> Result<Vec<FileEntry>, ServerFnError> {
    imp::check(&token)?;
    let dir = if path.trim().is_empty() {
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
    } else {
        std::path::PathBuf::from(path.trim())
    };
    let mut out: Vec<FileEntry> = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| ServerFnError::new(e.to_string()))? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue; // keep the picker readable; hidden dirs are never projects
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir {
            continue;
        }
        let full = entry.path().display().to_string();
        out.push(FileEntry {
            name,
            path: full,
            is_dir: true,
            size: 0,
            modified_ms: 0,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(out)
}

/// Preview one file (text/markdown/csv/image/xlsx/doc/slides/pdf/zip).
/// Remote sessions pull the file over SSH first (Nexus remoteFs).
#[server]
pub async fn api_read_file(
    token: String,
    path: String,
    remote_id: Option<String>,
    remote_root: String,
) -> Result<PreviewData, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::read_file(&path, remote_id.as_deref(), &remote_root))
}

/// Save a text file (dock editor). Remote sessions write through SSH
/// with `base64 -d` on stdin (Nexus remoteFs.remoteWriteText).
#[server]
pub async fn api_save_file(
    token: String,
    path: String,
    content: String,
    remote_id: Option<String>,
    remote_root: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::save_file(&path, &content, remote_id.as_deref(), &remote_root)
}

#[server]
pub async fn api_git_status(
    token: String,
    cwd: String,
    remote_id: Option<String>,
) -> Result<GitStatus, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_status(&cwd, remote_id.as_deref()))
}

#[server]
pub async fn api_git_diff(
    token: String,
    cwd: String,
    remote_id: Option<String>,
    staged: bool,
    path: String,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_diff(&cwd, remote_id.as_deref(), staged, &path))
}

/// Fuzzy file search under the working directory (the `@` picker).
/// Remote sessions run the same pruned `find` on the remote root over
/// SSH (Nexus remoteFs.remoteSearchFiles, cap 50).
#[server]
pub async fn api_search_files(
    token: String,
    query: String,
    remote_id: Option<String>,
    remote_root: String,
) -> Result<Vec<FileEntry>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::search_files(&query, remote_id.as_deref(), &remote_root))
}

/// Whole working-tree diff for the "branch diff" attach action.
#[server]
pub async fn api_git_diff_all(
    token: String,
    cwd: String,
    remote_id: Option<String>,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_diff_all(&cwd, remote_id.as_deref()))
}

/// Stage paths (`git add -- …`, Nexus gitStage). Ok("") = success.
#[server]
pub async fn api_git_stage(
    token: String,
    cwd: String,
    remote_id: Option<String>,
    paths: Vec<String>,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_stage(&cwd, remote_id.as_deref(), &paths))
}

/// Unstage paths (`git reset -- …`, Nexus gitUnstage). Ok("") = success.
#[server]
pub async fn api_git_unstage(
    token: String,
    cwd: String,
    remote_id: Option<String>,
    paths: Vec<String>,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_unstage(&cwd, remote_id.as_deref(), &paths))
}

/// Commit staged changes (Nexus gitCommit). Ok("") = success.
#[server]
pub async fn api_git_commit(
    token: String,
    cwd: String,
    remote_id: Option<String>,
    message: String,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_commit(&cwd, remote_id.as_deref(), &message))
}

/// Push to the remote (Nexus gitPush). Ok("") = success.
#[server]
pub async fn api_git_push(
    token: String,
    cwd: String,
    remote_id: Option<String>,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_push(&cwd, remote_id.as_deref()))
}

/// Recent commits of the session cwd (Nexus gitLog, max 30 in the panel).
#[server]
pub async fn api_git_log(
    token: String,
    cwd: String,
    remote_id: Option<String>,
    max: u32,
) -> Result<Vec<GitCommit>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::git_log(&cwd, remote_id.as_deref(), max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_status_branch_and_counts() {
        // NB: no `\` line continuations here — they would eat the leading
        // status-column spaces of the porcelain lines.
        let out = "## main...origin/main [ahead 2, behind 1]\nM  src/a.rs\n M src/b.rs\nMM src/c.rs\n?? new.txt\nR  old.rs -> new.rs\n";
        let st = parse_git_status(out);
        assert!(st.repo);
        assert_eq!(st.branch, "main");
        assert_eq!(st.ahead, 2);
        assert_eq!(st.behind, 1);
        assert_eq!(st.entries.len(), 5);
        // index column decides staging (Nexus: index !== ' ' && !== '?')
        assert_eq!(st.entries[0].code, "M");
        assert!(st.entries[0].staged);
        assert_eq!(st.entries[1].code, "M");
        assert!(!st.entries[1].staged);
        // "MM" has a staged index change — staged like Nexus
        assert_eq!(st.entries[2].code, "M");
        assert!(st.entries[2].staged);
        assert_eq!(st.entries[3].code, "?");
        assert!(!st.entries[3].staged);
        // renames keep the post-" -> " path
        assert_eq!(st.entries[4].path, "new.rs");
        assert!(st.entries[4].staged);
    }

    #[test]
    fn parse_git_status_fresh_repo() {
        let st = parse_git_status("## No commits yet on main\n?? a.txt\n");
        assert!(st.repo);
        assert_eq!(st.branch, "main");
        assert_eq!(st.entries.len(), 1);
        assert_eq!(st.entries[0].code, "?");
    }

    #[test]
    fn parse_git_status_detached_head() {
        let st = parse_git_status("## HEAD (no branch)\n");
        assert!(st.repo);
        assert_eq!(st.branch, "HEAD");
        assert!(st.entries.is_empty());
    }

    #[test]
    fn remote_abs_join_and_escape() {
        assert_eq!(remote_abs("/home/x/proj", "").unwrap(), "/home/x/proj");
        assert_eq!(remote_abs("/home/x/proj/", "").unwrap(), "/home/x/proj");
        assert_eq!(
            remote_abs("/home/x/proj", "src/a.rs").unwrap(),
            "/home/x/proj/src/a.rs"
        );
        assert_eq!(
            remote_abs("/home/x/proj", "/etc/passwd").unwrap(),
            "/etc/passwd"
        );
        assert!(remote_abs("/home/x/proj", "../escape").is_err());
        assert!(remote_abs("", "a").is_err());
    }

    #[test]
    fn parse_find_ls_fields() {
        let out = "d|4096|1729938000.5000000|src\n-|1234|1729938001.2500000|main.rs\nbad\n";
        let v = parse_find_ls(out);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].0, 'd');
        assert_eq!(v[0].1, 4096);
        assert_eq!(v[0].2, 1729938000);
        assert_eq!(v[0].3, "src");
        assert_eq!(v[1].0, '-');
        assert_eq!(v[1].1, 1234);
        assert_eq!(v[1].3, "main.rs");
    }
}
