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
    pub args: String,
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
    use vibedeck_core::snapshot::RemoteConnection;
    use vibedeck_engine::remote_vibe;
    use vibedeck_engine::Engine;
    use vibedeck_vibe::config::{McpServer, ModelConfig, Provider, VibeConfigFile};
    use vibedeck_vibe::files::{AgentFile, PromptFile, SkillFile};
    use vibedeck_vibe::vibe_home;

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
            mcp: cfg
                .mcp_servers()
                .into_iter()
                .map(|s| McpRow {
                    name: s.name,
                    transport: s.transport,
                    url: s.url.unwrap_or_default(),
                    command: s.command.unwrap_or_default(),
                    args: s.args.join(" "),
                })
                .collect(),
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
            cfg.upsert_mcp_server(&McpServer {
                name: row.name.trim().to_string(),
                transport: if row.transport.is_empty() {
                    "stdio".to_string()
                } else {
                    row.transport.trim().to_string()
                },
                url: opt(&row.url),
                command: opt(&row.command),
                args: row
                    .args
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect(),
                env: Vec::new(),
                headers: Vec::new(),
                api_key_env: None,
                api_key_header: None,
                api_key_format: None,
                startup_timeout_sec: None,
                tool_timeout_sec: None,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
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
    conn: Option<vibedeck_core::snapshot::RemoteConnection>,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::set_list(&scope, &cwd, conn.as_ref(), &key, values))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}
