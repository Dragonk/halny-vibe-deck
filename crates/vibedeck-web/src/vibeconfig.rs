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

    pub fn load() -> VibeConfigData {
        let cfg = VibeConfigFile::load();
        let raw_toml = fs::read_to_string(cfg.path.clone()).unwrap_or_default();
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

    fn save_cfg(mut cfg: VibeConfigFile) -> Result<(), ServerFnError> {
        cfg.save()
            .map_err(|e| ServerFnError::new(format!("save failed: {e}")))
    }

    /// Base directory for vibe config files: `~/.vibe` (global scope) or
    /// `<working_directory>/.vibe` (project scope).
    pub fn base_dir(scope: &str) -> std::path::PathBuf {
        if scope == "project" {
            std::path::PathBuf::from(Engine::global().workdir()).join(".vibe")
        } else {
            vibe_home()
        }
    }

    /// Confine a user-supplied file path to {agents,prompts,skills}/ under the
    /// selected scope's base directory.
    fn confine(rel: &str, scope: &str) -> Result<std::path::PathBuf, ServerFnError> {
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
        Ok(base_dir(scope).join(dir).join(rest))
    }

    /// Generic directory scan for the project scope (no load_all helpers there).
    fn scan_kind(scope: &str, kind: &str) -> Vec<FileCard> {
        let ext = match kind {
            "agents" => "toml",
            "prompts" => "md",
            _ => return scan_skills(scope),
        };
        let dir = base_dir(scope).join(kind);
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

    fn scan_skills(scope: &str) -> Vec<FileCard> {
        let dir = base_dir(scope).join("skills");
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

    pub fn list_files(kind: &str, scope: &str) -> Vec<FileCard> {
        if scope == "project" {
            return scan_kind(scope, kind);
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

    pub fn write_file(scope: &str, path: &str, content: &str) -> Result<(), ServerFnError> {
        let target = confine(path, scope)?;
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

    pub fn delete_file(scope: &str, path: &str) -> Result<(), ServerFnError> {
        let target = confine(path, scope)?;
        if target.exists() {
            fs::remove_file(&target)
                .map_err(|e| ServerFnError::new(format!("delete failed: {e}")))?;
        }
        Ok(())
    }

    pub fn save_raw(raw: &str) -> Result<(), ServerFnError> {
        raw.parse::<toml::Table>()
            .map_err(|e| ServerFnError::new(format!("invalid TOML: {e}")))?;
        let cfg = VibeConfigFile::load();
        fs::write(&cfg.path, raw)
            .map_err(|e| ServerFnError::new(format!("write failed: {e}")))
    }

    pub fn upsert_provider(row: ProviderRow) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        cfg.upsert_provider(&Provider {
            name: row.name.trim().to_string(),
            api_base: row.api_base.trim().to_string(),
            api_key_env_var: opt(&row.api_key_env_var),
            backend: opt(&row.backend),
            api_style: opt(&row.api_style),
            extra_headers: Vec::new(),
        });
        save_cfg(cfg)
    }

    pub fn remove_provider(name: &str) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        cfg.remove_provider(name);
        save_cfg(cfg)
    }

    pub fn upsert_model(row: ModelRow) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        // Preserve the per-model fields added for the session chips
        // (display_name/thinking/thinking_levels) — the settings editor
        // row does not edit them, but `upsert_model` replaces the whole
        // `[[models]]` entry.
        let (display_name, thinking, thinking_levels) = cfg
            .models()
            .into_iter()
            .find(|m| m.name == row.name.trim())
            .map(|m| (m.display_name, m.thinking, m.thinking_levels))
            .unwrap_or((None, None, None));
        cfg.upsert_model(&ModelConfig {
            name: row.name.trim().to_string(),
            provider: row.provider.trim().to_string(),
            alias: opt(&row.alias),
            enabled: Some(row.enabled),
            display_name,
            thinking,
            thinking_levels,
        });
        save_cfg(cfg)
    }

    pub fn remove_model(name: &str) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        cfg.remove_model(name);
        save_cfg(cfg)
    }

    pub fn upsert_mcp(row: McpRow) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
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
        save_cfg(cfg)
    }

    pub fn remove_mcp(name: &str) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        cfg.remove_mcp_server(name);
        save_cfg(cfg)
    }

    pub fn set_active_model(alias: &str) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        cfg.set_active_model(alias);
        save_cfg(cfg)
    }

    pub fn set_list(key: &str, values: Vec<String>) -> Result<(), ServerFnError> {
        let mut cfg = VibeConfigFile::load();
        let values: Vec<String> = values
            .into_iter()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect();
        match key {
            "enabled_tools" => cfg.set_enabled_tools(values),
            "disabled_tools" => cfg.set_disabled_tools(values),
            "enabled_skills" => cfg.set_enabled_skills(values),
            "disabled_skills" => cfg.set_disabled_skills(values),
            _ => return Err(ServerFnError::new("unknown list key")),
        }
        save_cfg(cfg)
    }
}

#[server]
pub async fn api_vibe_config(token: String) -> Result<VibeConfigData, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::load())
}

#[server]
pub async fn api_save_vibe_config_raw(token: String, raw: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::save_raw(&raw)
}

#[server]
pub async fn api_list_vibe_files(token: String, scope: String, kind: String) -> Result<Vec<FileCard>, ServerFnError> {
    imp::check(&token)?;
    Ok(imp::list_files(&kind, &scope))
}

#[server]
pub async fn api_save_vibe_file(token: String, scope: String, path: String, content: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::write_file(&scope, &path, &content)
}

#[server]
pub async fn api_delete_vibe_file(token: String, scope: String, path: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::delete_file(&scope, &path)
}

#[server]
pub async fn api_upsert_provider(token: String, row: ProviderRow) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::upsert_provider(row)
}

#[server]
pub async fn api_remove_provider(token: String, name: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::remove_provider(&name)
}

#[server]
pub async fn api_upsert_model(token: String, row: ModelRow) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::upsert_model(row)
}

#[server]
pub async fn api_remove_model(token: String, name: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::remove_model(&name)
}

#[server]
pub async fn api_upsert_mcp(token: String, row: McpRow) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::upsert_mcp(row)
}

#[server]
pub async fn api_remove_mcp(token: String, name: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::remove_mcp(&name)
}

#[server]
pub async fn api_set_active_model(token: String, alias: String) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::set_active_model(&alias)
}

#[server]
pub async fn api_set_vibe_list(token: String, key: String, values: Vec<String>) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    imp::set_list(&key, values)
}
