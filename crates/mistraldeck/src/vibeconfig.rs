//! Server functions for the Vibe CLI configuration layer: `~/.vibe/config.toml`
//! (providers, models, MCP servers, tool/skill lists, raw TOML) plus the
//! `agents/`, `prompts/` and `skills/` files. Token-gated like the rest of
//! the remote API; file writes are confined to those three directories.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ProviderRow {
    pub name: String,
    pub api_base: String,
    pub api_key_env_var: String,
    pub backend: String,
    pub api_style: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelRow {
    pub name: String,
    pub provider: String,
    pub alias: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct McpRow {
    pub name: String,
    pub transport: String,
    pub url: String,
    pub command: String,
    /// JSON array string, e.g. ["arg1", "arg2"].
    pub args: String,
    /// JSON object string, e.g. {"DEBUG": "1"} (stdio).
    pub env: String,
    /// JSON object string of static-auth headers.
    pub headers: String,
    pub api_key_env: String,
    pub api_key_header: String,
    pub api_key_format: String,
    /// "" (none) | "static" | "oauth".
    pub auth_type: String,
    /// Comma-separated scopes (oauth).
    pub scopes: String,
    pub client_id: String,
    pub client_metadata_url: String,
    /// "" = default 47823 (oauth).
    pub redirect_port: String,
    pub cwd: String,
    /// "" = default 10.
    pub startup_timeout_sec: String,
    /// "" = default 60.
    pub tool_timeout_sec: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct VibeConfigData {
    pub raw_toml: String,
    pub active_model: String,
    pub default_agent: String,
    pub providers: Vec<ProviderRow>,
    pub models: Vec<ModelRow>,
    pub mcp: Vec<McpRow>,
    pub enabled_tools: Vec<String>,
    pub disabled_tools: Vec<String>,
    pub enabled_skills: Vec<String>,
    pub disabled_skills: Vec<String>,
    pub skill_paths: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct FileCard {
    /// agents | prompts | skills
    pub kind: String,
    pub display_name: String,
    /// path relative to ~/.vibe (e.g. `agents/foo.toml`)
    pub path: String,
    pub content: String,
    pub meta: String,
}

#[cfg(feature = "server")]
fn opt(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.trim().to_string())
    }
}

#[cfg(feature = "server")]
mod imp {
    use super::*;
    use std::fs;
    use mistraldeck_core::snapshot::RemoteConnection;
    use mistraldeck_engine::remote_vibe;
    use mistraldeck_engine::Engine;
    use mistraldeck_vibe::config::{
        McpAuth, McpServer, ModelConfig, Provider, VibeConfigFile,
    };
    use mistraldeck_vibe::files::{AgentFile, PromptFile, SkillFile};
    use mistraldeck_vibe::vibe_home;

    pub fn check(token: &str) -> Result<(), ServerFnError> {
        let engine = Engine::global();
        if engine.auth_enabled() && !engine.check_token(token) {
            return Err(ServerFnError::new("unauthorized"));
        }
        Ok(())
    }

    /// Effective config target for one settings operation (Nexus scope
    /// picker): a local directory (`~/.vibe` or `<cwd>/.vibe`) or a remote
    /// host's `.vibe` reached over SSH.
    enum Ctx {
        Local(std::path::PathBuf),
        Remote {
            conn: RemoteConnection,
            dir: String,
        },
    }

    fn resolve_ctx(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
    ) -> Result<Ctx, ServerFnError> {
        match conn {
            Some(c) => Ok(Ctx::Remote {
                conn: c.clone(),
                dir: remote_vibe::scope_dir(if scope == "project" && !cwd.trim().is_empty() {
                    Some(cwd)
                } else {
                    None
                }),
            }),
            None => Ok(Ctx::Local(base_dir(scope, cwd))),
        }
    }

    pub fn load(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
    ) -> Result<VibeConfigData, ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let (raw_toml, cfg) = match &ctx {
            Ctx::Local(dir) => {
                let path = dir.join("config.toml");
                let cfg = VibeConfigFile::load_at(&path)
                    .map_err(|e| ServerFnError::new(format!("config load failed: {e}")))?;
                let raw_toml = fs::read_to_string(&path).unwrap_or_default();
                (raw_toml, cfg)
            }
            Ctx::Remote { conn, dir } => {
                let raw_toml = remote_vibe::get_remote_vibe_config_checked(conn, dir)
                    .map_err(ServerFnError::new)?;
                (raw_toml.clone(), VibeConfigFile::from_raw(&raw_toml))
            }
        };
        Ok(data_from(&cfg, raw_toml))
    }

    /// Engine row -> UI row. Maps an explicit `[auth]` block; legacy flat
    /// auth keys (no auth block) surface as a "static" row (Nexus
    /// `mcpLegacyAuth`).
    fn mcp_row_from(s: McpServer) -> McpRow {
        let pairs_json = |v: Vec<(String, String)>| {
            serde_json::to_string(&v.into_iter().collect::<std::collections::BTreeMap<String, String>>())
                .unwrap_or_else(|_| "{}".into())
        };
        let (auth_type, headers, api_env, api_hdr, api_fmt, scopes, cid, cmu, port) =
            match &s.auth {
                Some(a) => (
                    if a.auth_type.is_empty() {
                        "static".to_string()
                    } else {
                        a.auth_type.clone()
                    },
                    pairs_json(a.headers.clone()),
                    a.api_key_env.clone().unwrap_or_default(),
                    a.api_key_header.clone().unwrap_or_default(),
                    a.api_key_format.clone().unwrap_or_default(),
                    a.scopes.join(", "),
                    a.client_id.clone().unwrap_or_default(),
                    a.client_metadata_url.clone().unwrap_or_default(),
                    a.redirect_port.map(|p| p.to_string()).unwrap_or_default(),
                ),
                None => {
                    let legacy = !s.headers.is_empty()
                        || s.api_key_env.is_some()
                        || s.api_key_header.is_some()
                        || s.api_key_format.is_some();
                    (
                        if legacy { "static".to_string() } else { String::new() },
                        pairs_json(s.headers.clone()),
                        s.api_key_env.clone().unwrap_or_default(),
                        s.api_key_header.clone().unwrap_or_default(),
                        s.api_key_format.clone().unwrap_or_default(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    )
                }
            };
        McpRow {
            name: s.name,
            transport: s.transport,
            url: s.url.unwrap_or_default(),
            command: s.command.unwrap_or_default(),
            args: serde_json::to_string(&s.args).unwrap_or_else(|_| "[]".into()),
            env: pairs_json(s.env),
            headers,
            api_key_env: api_env,
            api_key_header: api_hdr,
            api_key_format: api_fmt,
            auth_type,
            scopes,
            client_id: cid,
            client_metadata_url: cmu,
            redirect_port: port,
            cwd: s.cwd.unwrap_or_default(),
            startup_timeout_sec: s.startup_timeout_sec.map(|v| v.to_string()).unwrap_or_default(),
            tool_timeout_sec: s.tool_timeout_sec.map(|v| v.to_string()).unwrap_or_default(),
        }
    }

    fn data_from(cfg: &VibeConfigFile, raw_toml: String) -> VibeConfigData {
        VibeConfigData {
            raw_toml,
            active_model: cfg.active_model().unwrap_or_default(),
            default_agent: cfg.default_agent(),
            providers: cfg
                .providers()
                .into_iter()
                .map(|p| ProviderRow {
                    name: p.name,
                    api_base: p.api_base,
                    api_key_env_var: p.api_key_env_var.unwrap_or_default(),
                    backend: p.backend.unwrap_or_default(),
                    api_style: p.api_style.unwrap_or_default(),
                })
                .collect(),
            models: cfg
                .models()
                .into_iter()
                .map(|m| ModelRow {
                    name: m.name,
                    provider: m.provider,
                    alias: m.alias.unwrap_or_default(),
                    enabled: m.enabled.unwrap_or(true),
                })
                .collect(),
            mcp: cfg.mcp_servers().into_iter().map(mcp_row_from).collect(),
            enabled_tools: cfg.enabled_tools(),
            disabled_tools: cfg.disabled_tools(),
            enabled_skills: cfg.enabled_skills(),
            disabled_skills: cfg.disabled_skills(),
            skill_paths: cfg.skill_paths(),
        }
    }

    /// Read-modify-write the scope's config.toml: locally through
    /// `VibeConfigFile`, remotely via the SSH round-trip
    /// (read raw → parse → mutate → validate → write raw).
    fn with_cfg(
        ctx: &Ctx,
        f: impl FnOnce(&mut VibeConfigFile),
    ) -> Result<(), ServerFnError> {
        match ctx {
            Ctx::Local(dir) => {
                let path = dir.join("config.toml");
                let mut cfg = VibeConfigFile::load_at(&path)
                    .map_err(|e| ServerFnError::new(format!("config load failed: {e}")))?;
                f(&mut cfg);
                cfg.save()
                    .map_err(|e| ServerFnError::new(format!("save failed: {e}")))?;
                Ok(())
            }
            Ctx::Remote { conn, dir } => {
                let raw = remote_vibe::get_remote_vibe_config_checked(conn, dir)
                    .map_err(ServerFnError::new)?;
                let mut cfg = VibeConfigFile::from_raw(&raw);
                f(&mut cfg);
                let rendered = cfg.render();
                if let Some(e) = remote_vibe::save_remote_raw_config(conn, dir, &rendered) {
                    return Err(ServerFnError::new(e));
                }
                Ok(())
            }
        }
    }

    /// Base directory for vibe config files: `~/.vibe` (global scope) or
    /// `<cwd>/.vibe` (project scope; empty cwd falls back to the engine
    /// working directory).
    pub fn base_dir(scope: &str, cwd: &str) -> std::path::PathBuf {
        if scope == "project" {
            let base = if cwd.trim().is_empty() {
                Engine::global().workdir().to_string()
            } else {
                cwd.trim().to_string()
            };
            std::path::PathBuf::from(base).join(".vibe")
        } else {
            vibe_home()
        }
    }

    /// Confine a user-supplied file path to {agents,prompts,skills}/ under the
    /// selected scope's base directory.
    fn confine(rel: &str, base: &std::path::Path) -> Result<std::path::PathBuf, ServerFnError> {
        let rel = rel.trim().trim_start_matches('/');
        if rel.contains("..") || rel.is_empty() {
            return Err(ServerFnError::new("invalid path"));
        }
        let mut parts = rel.splitn(2, '/');
        let dir = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default();
        if !matches!(dir, "agents" | "prompts" | "skills") || rest.is_empty() {
            return Err(ServerFnError::new("path must be under agents/, prompts/ or skills/"));
        }
        Ok(base.join(dir).join(rest))
    }

    /// Generic directory scan for the project scope (no load_all helpers there).
    fn scan_kind(base: &std::path::Path, kind: &str) -> Vec<FileCard> {
        let ext = match kind {
            "agents" => "toml",
            "prompts" => "md",
            _ => return scan_skills(base),
        };
        let dir = base.join(kind);
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_file() || p.extension().and_then(|e| e.to_str()) != Some(ext) {
                    continue;
                }
                let name = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                let content = fs::read_to_string(&p).unwrap_or_default();
                let meta = if kind == "agents" {
                    agent_meta(&content)
                } else {
                    String::new()
                };
                out.push(FileCard {
                    kind: kind.to_string(),
                    display_name: name.clone(),
                    path: format!("{kind}/{name}.{ext}"),
                    content,
                    meta,
                });
            }
        }
        out.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        out
    }

    fn scan_skills(base: &std::path::Path) -> Vec<FileCard> {
        let dir = base.join("skills");
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let skill_md = p.join("SKILL.md");
                if !skill_md.is_file() {
                    continue;
                }
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                out.push(FileCard {
                    kind: "skills".to_string(),
                    display_name: name.clone(),
                    path: format!("skills/{name}/SKILL.md"),
                    content: fs::read_to_string(&skill_md).unwrap_or_default(),
                    meta: String::new(),
                });
            }
        }
        out.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        out
    }

    /// Best-effort meta line for a raw agent TOML (project scope has no loader).
    fn agent_meta(content: &str) -> String {
        let mut model = String::new();
        if let Ok(table) = content.parse::<toml::Table>() {
            model = table
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
        }
        if model.is_empty() {
            String::new()
        } else {
            format!("model: {model}")
        }
    }

    pub fn list_files(
        kind: &str,
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
    ) -> Vec<FileCard> {
        let ctx = match resolve_ctx(scope, cwd, conn) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        match &ctx {
            Ctx::Local(base) => {
                if scope == "project" {
                    return scan_kind(base, kind);
                }
                match kind {
            "agents" => AgentFile::load_all()
                .into_iter()
                .map(|a| {
                    let path = format!("agents/{}.toml", a.name);
                    let content =
                        fs::read_to_string(vibe_home().join(&path)).unwrap_or_default();
                    let meta = format!(
                        "model: {} · tools: {}",
                        a.active_model.as_deref().unwrap_or("—"),
                        a.enabled_tools.join(", ")
                    );
                    FileCard {
                        kind: "agents".into(),
                        display_name: a.name,
                        path,
                        content,
                        meta,
                    }
                })
                .collect(),
            "prompts" => PromptFile::load_all()
                .into_iter()
                .map(|p| FileCard {
                    kind: "prompts".into(),
                    display_name: p.name.clone(),
                    path: format!("prompts/{}.md", p.name),
                    content: p.content,
                    meta: String::new(),
                })
                .collect(),
            "skills" => SkillFile::load_all()
                .into_iter()
                .map(|s| FileCard {
                    kind: "skills".into(),
                    display_name: s.name.clone(),
                    path: format!("skills/{}/SKILL.md", s.name),
                    content: s.content,
                    meta: String::new(),
                })
                .collect(),
            _ => Vec::new(),
                }
            }
            Ctx::Remote { conn, dir } => remote_files(conn, dir, kind),
        }
    }

    /// Map the C4 remote vibe getters onto the flat FileCard model used by
    /// the Settings file editor (agents/prompts/skills tabs).
    fn remote_files(conn: &RemoteConnection, dir: &str, kind: &str) -> Vec<FileCard> {
        match kind {
            "agents" => remote_vibe::get_remote_agents(conn, dir)
                .into_iter()
                .map(|a| {
                    let meta = format!(
                        "model: {} · {}",
                        a.active_model.clone().unwrap_or_else(|| "—".to_string()),
                        a.agent_type.clone().unwrap_or_else(|| "agent".to_string())
                    );
                    FileCard {
                        kind: "agents".to_string(),
                        display_name: a.name.clone(),
                        path: format!("agents/{}.toml", a.name),
                        content: a.content,
                        meta,
                    }
                })
                .collect(),
            "prompts" => remote_vibe::get_remote_prompts(conn, dir)
                .into_iter()
                .map(|p| FileCard {
                    kind: "prompts".to_string(),
                    display_name: p.name.clone(),
                    path: format!("prompts/{}.md", p.name),
                    content: p.content,
                    meta: String::new(),
                })
                .collect(),
            "skills" => remote_vibe::get_remote_skills(conn, dir)
                .into_iter()
                .map(|s| FileCard {
                    kind: "skills".to_string(),
                    display_name: s.name.clone(),
                    path: format!("skills/{}/SKILL.md", s.name),
                    content: s.content,
                    meta: format!("{} files", s.files.len()),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn write_file(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        path: &str,
        content: &str,
    ) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        match &ctx {
            Ctx::Local(base) => {
                let target = confine(path, base)?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| ServerFnError::new(format!("mkdir failed: {e}")))?;
                }
                if path.ends_with(".toml") {
                    // validate before overwriting the agent file
                    content
                        .parse::<toml::Table>()
                        .map_err(|e| ServerFnError::new(format!("invalid TOML: {e}")))?;
                }
                fs::write(&target, content)
                    .map_err(|e| ServerFnError::new(format!("write failed: {e}")))
            }
            Ctx::Remote { conn, dir } => remote_write_entry(conn, dir, path, content),
        }
    }

    /// Write one agents/prompts/skills entry on the remote host (SSH).
    fn remote_write_entry(
        conn: &RemoteConnection,
        dir: &str,
        path: &str,
        content: &str,
    ) -> Result<(), ServerFnError> {
        let rel = path.trim().trim_start_matches('/');
        if rel.contains("..") || rel.is_empty() {
            return Err(ServerFnError::new("invalid path"));
        }
        let mut parts = rel.splitn(2, '/');
        let dirk = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default();
        if rest.is_empty() {
            return Err(ServerFnError::new("invalid path"));
        }
        match dirk {
            "agents" => remote_vibe::save_remote_agent_raw(conn, dir, rest.trim_end_matches(".toml"), content)
                .map_err(ServerFnError::new),
            "prompts" => remote_vibe::save_remote_prompt(conn, dir, rest.trim_end_matches(".md"), content)
                .map_err(ServerFnError::new),
            "skills" => {
                // skills/<name>/<rel> (the Settings editor writes SKILL.md)
                let mut it = rest.splitn(2, '/');
                let name = it.next().unwrap_or_default();
                let rel = it.next().unwrap_or_default();
                if name.is_empty() || rel.is_empty() {
                    return Err(ServerFnError::new("invalid path"));
                }
                remote_vibe::write_remote_skill_file(conn, dir, name, rel, content)
                    .map_or(Ok(()), |e| Err(ServerFnError::new(e)))
            }
            _ => Err(ServerFnError::new(
                "path must be under agents/, prompts/ or skills/",
            )),
        }
    }

    pub fn delete_file(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        path: &str,
    ) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        match &ctx {
            Ctx::Local(base) => {
                let target = confine(path, base)?;
                if target.exists() {
                    fs::remove_file(&target)
                        .map_err(|e| ServerFnError::new(format!("delete failed: {e}")))?;
                }
                Ok(())
            }
            Ctx::Remote { conn, dir } => remote_delete_entry(conn, dir, path),
        }
    }

    /// Delete one agents/prompts/skills entry on the remote host (SSH).
    fn remote_delete_entry(
        conn: &RemoteConnection,
        dir: &str,
        path: &str,
    ) -> Result<(), ServerFnError> {
        let rel = path.trim().trim_start_matches('/');
        if rel.contains("..") || rel.is_empty() {
            return Err(ServerFnError::new("invalid path"));
        }
        let mut parts = rel.splitn(2, '/');
        let dirk = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or_default();
        if rest.is_empty() {
            return Err(ServerFnError::new("invalid path"));
        }
        match dirk {
            "agents" => {
                remote_vibe::delete_remote_agent(conn, dir, rest.trim_end_matches(".toml"));
                Ok(())
            }
            "prompts" => {
                remote_vibe::delete_remote_prompt(conn, dir, rest.trim_end_matches(".md"));
                Ok(())
            }
            "skills" => {
                let mut it = rest.splitn(2, '/');
                let name = it.next().unwrap_or_default();
                let rel = it.next().unwrap_or_default();
                if name.is_empty() || rel.is_empty() {
                    return Err(ServerFnError::new("invalid path"));
                }
                remote_vibe::delete_remote_skill_file(conn, dir, name, rel)
                    .map_or(Ok(()), |e| Err(ServerFnError::new(e)))
            }
            _ => Err(ServerFnError::new(
                "path must be under agents/, prompts/ or skills/",
            )),
        }
    }

    pub fn save_raw(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        raw: &str,
    ) -> Result<(), ServerFnError> {
        raw.parse::<toml::Table>()
            .map_err(|e| ServerFnError::new(format!("invalid TOML: {e}")))?;
        let ctx = resolve_ctx(scope, cwd, conn)?;
        match &ctx {
            Ctx::Local(dir) => {
                let path = dir.join("config.toml");
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&path, raw)
                    .map_err(|e| ServerFnError::new(format!("write failed: {e}")))
            }
            Ctx::Remote { conn, dir } => remote_vibe::save_remote_raw_config(conn, dir, raw)
                .map_or(Ok(()), |e| Err(ServerFnError::new(e))),
        }
    }

    pub fn upsert_provider(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        row: ProviderRow,
    ) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| {
            cfg.upsert_provider(&Provider {
                name: row.name.trim().to_string(),
                api_base: row.api_base.trim().to_string(),
                api_key_env_var: opt(&row.api_key_env_var),
                backend: opt(&row.backend),
                api_style: opt(&row.api_style),
                extra_headers: Vec::new(),
            });
        })
    }

    pub fn remove_provider(scope: &str, cwd: &str, conn: Option<&RemoteConnection>, name: &str) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| {
            cfg.remove_provider(name);
        })
    }

    pub fn upsert_model(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        row: ModelRow,
    ) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        // Preserve the per-model fields not edited by the settings row
        // (display_name/thinking/thinking_levels and the usage-bar fields
        // max_context_length/auto_compact_threshold) — `upsert_model`
        // replaces the whole `[[models]]` entry.
        let (display_name, thinking, thinking_levels, max_context_length, auto_compact_threshold) = {
            let cfg = match &ctx {
                Ctx::Local(dir) => VibeConfigFile::load_at(&dir.join("config.toml"))
                    .map_err(|e| ServerFnError::new(format!("config load failed: {e}")))?,
                Ctx::Remote { conn, dir } => {
                    let raw = remote_vibe::get_remote_vibe_config_checked(conn, dir)
                        .map_err(ServerFnError::new)?;
                    VibeConfigFile::from_raw(&raw)
                }
            };
            cfg.models()
                .into_iter()
                .find(|m| m.name == row.name.trim())
                .map(|m| {
                    (
                        m.display_name,
                        m.thinking,
                        m.thinking_levels,
                        m.max_context_length,
                        m.auto_compact_threshold,
                    )
                })
                .unwrap_or((None, None, None, None, None))
        };
        with_cfg(&ctx, |cfg| {
            cfg.upsert_model(&ModelConfig {
                name: row.name.trim().to_string(),
                provider: row.provider.trim().to_string(),
                alias: opt(&row.alias),
                enabled: Some(row.enabled),
                display_name,
                thinking,
                thinking_levels,
                max_context_length,
                auto_compact_threshold,
            });
        })
    }

    pub fn remove_model(scope: &str, cwd: &str, conn: Option<&RemoteConnection>, name: &str) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| {
            cfg.remove_model(name);
        })
    }

    pub fn upsert_mcp(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        row: McpRow,
    ) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| {
            let transport = if row.transport.is_empty() {
                "stdio".to_string()
            } else {
                row.transport.trim().to_string()
            };
            let is_stdio = transport == "stdio";
            // Invalid JSON in a textarea keeps the previously stored values
            // (Nexus silently ignores a failed JSON.parse too).
            let prev = cfg
                .mcp_servers()
                .into_iter()
                .find(|s| s.name == row.name.trim())
                .unwrap_or_default();
            let parse_pairs = |txt: &str, fallback: Vec<(String, String)>| {
                serde_json::from_str::<std::collections::BTreeMap<String, String>>(txt)
                    .map(|m| m.into_iter().collect())
                    .unwrap_or(fallback)
            };
            let args = serde_json::from_str::<Vec<String>>(row.args.trim()).unwrap_or(prev.args);
            let env = parse_pairs(row.env.trim(), prev.env);
            let headers = parse_pairs(row.headers.trim(), prev.headers);
            // Writing an explicit auth block clears the legacy flat keys
            // (Nexus `mcpAuthUpdate`/`mcpOauthUpdate`); stdio has no auth.
            let auth = if is_stdio {
                None
            } else {
                match row.auth_type.as_str() {
                    "oauth" => Some(McpAuth {
                        auth_type: "oauth".to_string(),
                        scopes: row
                            .scopes
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        client_id: opt(&row.client_id),
                        client_metadata_url: opt(&row.client_metadata_url),
                        redirect_port: row.redirect_port.trim().parse().ok(),
                        ..Default::default()
                    }),
                    "static" => Some(McpAuth {
                        auth_type: "static".to_string(),
                        headers,
                        api_key_env: opt(&row.api_key_env),
                        api_key_header: Some(if row.api_key_header.trim().is_empty() {
                            "Authorization".to_string()
                        } else {
                            row.api_key_header.trim().to_string()
                        }),
                        api_key_format: Some(if row.api_key_format.trim().is_empty() {
                            "Bearer {token}".to_string()
                        } else {
                            row.api_key_format.trim().to_string()
                        }),
                        ..Default::default()
                    }),
                    _ => None,
                }
            };
            cfg.upsert_mcp_server(&McpServer {
                name: row.name.trim().to_string(),
                transport,
                // transport switch clears the other branch's keys (Nexus)
                url: if is_stdio { None } else { opt(&row.url) },
                command: if is_stdio { opt(&row.command) } else { None },
                args: if is_stdio { args } else { Vec::new() },
                env: if is_stdio { env } else { Vec::new() },
                headers: Vec::new(),
                api_key_env: None,
                api_key_header: None,
                api_key_format: None,
                startup_timeout_sec: row.startup_timeout_sec.trim().parse().ok(),
                tool_timeout_sec: row.tool_timeout_sec.trim().parse().ok(),
                cwd: if is_stdio { opt(&row.cwd) } else { None },
                auth,
            });
        })
    }

    pub fn remove_mcp(scope: &str, cwd: &str, conn: Option<&RemoteConnection>, name: &str) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| {
            cfg.remove_mcp_server(name);
        })
    }

    pub fn set_active_model(scope: &str, cwd: &str, conn: Option<&RemoteConnection>, alias: &str) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| {
            cfg.set_active_model(alias);
        })
    }

    pub fn set_list(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        key: &str,
        values: Vec<String>,
    ) -> Result<(), ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let values: Vec<String> = values
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        match key {
            "enabled_tools" | "disabled_tools" | "enabled_skills" | "disabled_skills" => {}
            _ => return Err(ServerFnError::new("unknown list key")),
        }
        with_cfg(&ctx, |cfg| match key {
            "enabled_tools" => cfg.set_enabled_tools(values),
            "disabled_tools" => cfg.set_disabled_tools(values),
            "enabled_skills" => cfg.set_enabled_skills(values),
            _ => cfg.set_disabled_skills(values),
        })
    }
}

#[server]
pub async fn api_vibe_config(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<VibeConfigData, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::load(&scope, &cwd, conn.as_ref()))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_save_vibe_config_raw(
    token: String,
    raw: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::save_raw(&scope, &cwd, conn.as_ref(), &raw))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_list_vibe_files(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    kind: String,
) -> Result<Vec<FileCard>, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::list_files(&kind, &scope, &cwd, conn.as_ref()))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))
}

#[server]
pub async fn api_save_vibe_file(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    path: String,
    content: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::write_file(&scope, &cwd, conn.as_ref(), &path, &content)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_delete_vibe_file(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    path: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::delete_file(&scope, &cwd, conn.as_ref(), &path))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_upsert_provider(
    token: String,
    row: ProviderRow,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::upsert_provider(&scope, &cwd, conn.as_ref(), row))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_remove_provider(
    token: String,
    name: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::remove_provider(&scope, &cwd, conn.as_ref(), &name)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_upsert_model(
    token: String,
    row: ModelRow,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::upsert_model(&scope, &cwd, conn.as_ref(), row))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_remove_model(
    token: String,
    name: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::remove_model(&scope, &cwd, conn.as_ref(), &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_upsert_mcp(
    token: String,
    row: McpRow,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::upsert_mcp(&scope, &cwd, conn.as_ref(), row))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_remove_mcp(
    token: String,
    name: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::remove_mcp(&scope, &cwd, conn.as_ref(), &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_set_active_model(
    token: String,
    alias: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::set_active_model(&scope, &cwd, conn.as_ref(), &alias)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_set_vibe_list(
    token: String,
    key: String,
    values: Vec<String>,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::set_list(&scope, &cwd, conn.as_ref(), &key, values))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[cfg(all(test, feature = "server"))]
mod mcp_smoke {
    use super::imp;
    use super::McpRow;

    fn read_server(name: &str) -> mistraldeck_vibe::config::McpServer {
        let path = mistraldeck_vibe::vibe_home().join("config.toml");
        let toml = std::fs::read_to_string(&path).expect("config.toml readable");
        let cfg = mistraldeck_vibe::config::VibeConfigFile::from_raw(&toml);
        cfg.mcp_servers()
            .into_iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("server {name} not found"))
    }

    fn upsert(row: McpRow) {
        imp::upsert_mcp("global", "", None, row)
            .unwrap_or_else(|e| panic!("upsert failed: {e}"));
    }

    #[test]
    fn upsert_mcp_scenarios() {
        // Isolated VIBE_HOME so the smoke test never touches the real ~/.vibe.
        let d = std::env::temp_dir().join(format!("md-smoke7b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        std::env::set_var("VIBE_HOME", &d);

        // 1) http + static auth -> explicit [auth] block, legacy flat keys cleared
        upsert(McpRow {
            name: "srv-http".into(),
            transport: "http".into(),
            url: "https://api.example.com/mcp".into(),
            auth_type: "static".into(),
            headers: r#"{"X-Api-Key":"secret"}"#.into(),
            api_key_env: "MY_KEY".into(),
            ..Default::default()
        });
        let s = read_server("srv-http");
        assert_eq!(s.url.as_deref(), Some("https://api.example.com/mcp"));
        assert!(s.command.is_none(), "http clears stdio command");
        let a = s.auth.as_ref().expect("auth block written");
        assert_eq!(a.auth_type, "static");
        assert_eq!(a.api_key_env.as_deref(), Some("MY_KEY"));
        assert_eq!(a.api_key_header.as_deref(), Some("Authorization"));
        assert_eq!(a.api_key_format.as_deref(), Some("Bearer {token}"));
        assert_eq!(
            a.headers,
            vec![("X-Api-Key".to_string(), "secret".to_string())]
        );
        // legacy flat keys cleared at server level
        assert!(s.api_key_env.is_none());
        assert!(s.headers.is_empty());

        // 2) oauth: csv scopes, redirect_port, static keys dropped
        upsert(McpRow {
            name: "srv-oauth".into(),
            transport: "http".into(),
            url: "https://mcp.example.com".into(),
            auth_type: "oauth".into(),
            scopes: "read, write".into(),
            client_id: "cid".into(),
            client_metadata_url: "https://auth.example.com/metadata".into(),
            redirect_port: "9999".into(),
            ..Default::default()
        });
        let s = read_server("srv-oauth");
        let a = s.auth.as_ref().expect("oauth auth written");
        assert_eq!(a.auth_type, "oauth");
        assert_eq!(a.scopes, vec!["read".to_string(), "write".to_string()]);
        assert_eq!(a.client_id.as_deref(), Some("cid"));
        assert_eq!(a.client_metadata_url.as_deref(), Some("https://auth.example.com/metadata"));
        assert_eq!(a.redirect_port, Some(9999));
        assert!(a.api_key_env.is_none(), "oauth branch drops static keys");
        assert!(a.headers.is_empty());

        // 3) stdio: JSON args/env, cwd, timeouts
        upsert(McpRow {
            name: "srv-stdio".into(),
            transport: "stdio".into(),
            command: "uvx".into(),
            args: r#"["mcp-server","--flag"]"#.into(),
            env: r#"{"DEBUG":"1"}"#.into(),
            cwd: "/tmp/wd".into(),
            startup_timeout_sec: "15".into(),
            tool_timeout_sec: "120".into(),
            ..Default::default()
        });
        let s = read_server("srv-stdio");
        assert_eq!(s.command.as_deref(), Some("uvx"));
        assert_eq!(
            s.args,
            vec!["mcp-server".to_string(), "--flag".to_string()]
        );
        assert_eq!(s.env, vec![("DEBUG".to_string(), "1".to_string())]);
        assert_eq!(s.cwd.as_deref(), Some("/tmp/wd"));
        assert_eq!(s.startup_timeout_sec, Some(15));
        assert_eq!(s.tool_timeout_sec, Some(120));
        assert!(s.auth.is_none());
        assert!(s.url.is_none());

        // 4) invalid JSON keeps the previously stored values (Nexus JSON.parse semantics)
        upsert(McpRow {
            name: "srv-stdio".into(),
            transport: "stdio".into(),
            command: "uvx".into(),
            args: "not json".into(),
            ..Default::default()
        });
        let s = read_server("srv-stdio");
        assert_eq!(s.args.len(), 2, "prev args kept on invalid JSON");
        assert_eq!(s.env.len(), 1, "prev env kept on empty env");

        // 5) transport switch http -> stdio clears url + auth
        upsert(McpRow {
            name: "srv-http".into(),
            transport: "stdio".into(),
            command: "sh".into(),
            ..Default::default()
        });
        let s = read_server("srv-http");
        assert!(s.auth.is_none(), "auth cleared on transport switch");
        assert!(s.url.is_none(), "url cleared on transport switch");
        assert_eq!(s.command.as_deref(), Some("sh"));

        std::env::remove_var("VIBE_HOME");
        let _ = std::fs::remove_dir_all(&d);
    }
}
