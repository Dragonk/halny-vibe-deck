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
    pub api_key: String,
    pub reasoning_field_name: String,
    pub project_id: String,
    pub region: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ModelRow {
    pub name: String,
    pub provider: String,
    pub alias: String,
    pub enabled: bool,
    pub display_name: String,
    pub supports_images: bool,
    /// Optional number fields: "" = not set (Nexus writes only touched keys).
    pub temperature: String,
    pub top_p: String,
    pub max_output_tokens: String,
    pub input_price: String,
    pub output_price: String,
    pub cached_input_price: String,
    /// thinking preset: "" = off.
    pub thinking: String,
    /// Comma-separated thinking levels.
    pub thinking_levels: String,
    pub max_context_length: String,
    pub auto_compact_threshold: String,
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

/// One `[tools.<name>] permission = "…"` entry (Z15).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ToolPermRow {
    pub name: String,
    pub permission: String,
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
    /// Per-tool permissions from `[tools.<name>]` (UI: Global Tool Permissions).
    #[serde(default)]
    pub tools: Vec<ToolPermRow>,
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
    /// Structured agent profile (agents tab configurator), when available.
    #[serde(default)]
    pub agent: Option<AgentView>,
}

/// One agent profile as the Settings "Agents" configurator sees it
/// (stable SettingsView agent editor parity).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentView {
    pub name: String,
    pub agent_type: String,
    pub display_name: String,
    pub description: String,
    pub active_model: String,
    pub system_prompt_id: String,
    pub safety: String,
    pub instructions: String,
    pub enabled_tools: String,
    pub disabled_tools: String,
    /// `(tool, permission)` pairs of the agent's `[tools.*]` table.
    #[serde(default)]
    pub tool_permissions: Vec<(String, String)>,
}

#[cfg(feature = "server")]
impl AgentView {
    pub fn from_agent(a: &mistraldeck_vibe::files::AgentFile) -> Self {
        Self {
            name: a.name.clone(),
            agent_type: a
                .agent_type
                .clone()
                .unwrap_or_else(|| "agent".to_string()),
            display_name: a.display_name.clone().unwrap_or_default(),
            description: a.description.clone().unwrap_or_default(),
            active_model: a.active_model.clone().unwrap_or_default(),
            system_prompt_id: a.system_prompt_id.clone().unwrap_or_default(),
            safety: a.safety.clone().unwrap_or_else(|| "neutral".to_string()),
            instructions: a.instructions.clone().unwrap_or_default(),
            enabled_tools: a.enabled_tools.join(", "),
            disabled_tools: a.disabled_tools.join(", "),
            tool_permissions: a.tool_permissions.clone(),
        }
    }
}

/// One patch field set for `api_save_agent` (all optional; empty string
/// clears the value, matching the CLI "never write empty" rule).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AgentPatch {
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub active_model: Option<String>,
    #[serde(default)]
    pub system_prompt_id: Option<String>,
    #[serde(default)]
    pub safety: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub enabled_tools: Option<String>,
    #[serde(default)]
    pub disabled_tools: Option<String>,
    /// Replace the agent's whole `[tools.*]` table (`None` = keep).
    #[serde(default)]
    pub tool_permissions: Option<Vec<(String, String)>>,
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
fn num_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<f64>().ok()
    }
}

#[cfg(feature = "server")]
fn num_i64(s: &str) -> Option<i64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<i64>().ok()
    }
}

/// VIBE_HOME is a process-global env var: the smoke tests that set it must
/// not race each other inside the same test binary.
#[cfg(all(test, feature = "server"))]
static SMOKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(feature = "server")]
fn csv_list(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
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

    pub fn check(_token: &str) -> Result<(), ServerFnError> {
        // Same auth model as api.rs: no token enforcement inside the
        // server functions — the remote HTTPS listener's middleware has
        // already gated the request, and the loopback listener is trusted.
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
                    api_key: p.api_key.unwrap_or_default(),
                    reasoning_field_name: p.reasoning_field_name.unwrap_or_default(),
                    project_id: p.project_id.unwrap_or_default(),
                    region: p.region.unwrap_or_default(),
                    enabled: p.enabled.unwrap_or(true),
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
                    display_name: m.display_name.unwrap_or_default(),
                    supports_images: m.supports_images.unwrap_or(false),
                    temperature: m.temperature.map(|v| v.to_string()).unwrap_or_default(),
                    top_p: m.top_p.map(|v| v.to_string()).unwrap_or_default(),
                    max_output_tokens: m
                        .max_output_tokens
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    input_price: m.input_price.map(|v| v.to_string()).unwrap_or_default(),
                    output_price: m.output_price.map(|v| v.to_string()).unwrap_or_default(),
                    cached_input_price: m
                        .cached_input_price
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    thinking: m.thinking.unwrap_or_default(),
                    thinking_levels: m
                        .thinking_levels
                        .map(|v| v.join(", "))
                        .unwrap_or_default(),
                    max_context_length: m
                        .max_context_length
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    auto_compact_threshold: m
                        .auto_compact_threshold
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                })
                .collect(),
            mcp: cfg.mcp_servers().into_iter().map(mcp_row_from).collect(),
            enabled_tools: cfg.enabled_tools(),
            disabled_tools: cfg.disabled_tools(),
            tools: cfg
                .tools()
                .into_iter()
                .map(|(name, permission)| ToolPermRow { name, permission })
                .collect(),
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
    /// Reject skill names that could escape `skills/` (Z16).
fn safe_skill_name(name: &str) -> Result<String, ServerFnError> {
    let n = name.trim();
    if n.is_empty()
        || n.contains('/')
        || n.contains('\\')
        || n == "."
        || n == ".."
        || n.starts_with('.')
    {
        return Err(ServerFnError::new("invalid skill name"));
    }
    Ok(n.to_string())
}

/// Recursively collect one skill's files as editable cards.
fn collect_skill_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    skill: &str,
    out: &mut Vec<FileCard>,
) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_files(root, &path, skill, out);
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map(|p| p.display().to_string().replace('\\', "/"))
            .unwrap_or_default();
        if rel.is_empty() {
            continue;
        }
        // Skips binaries by size: a skill file is text.
        let Ok(meta) = path.metadata() else { continue };
        if meta.len() > 512 * 1024 {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else { continue };
        out.push(FileCard {
            kind: "skills".to_string(),
            display_name: rel.clone(),
            path: format!("skills/{skill}/{rel}"),
            content,
            meta: format!("{} B", meta.len()),
            agent: None,
        });
    }
}

fn copy_dir(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

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
                let agent = if kind == "agents" {
                    mistraldeck_vibe::files::parse_agent_content(&name, &content)
                        .map(|a| AgentView::from_agent(&a))
                } else {
                    None
                };
                out.push(FileCard {
                    kind: kind.to_string(),
                    display_name: name.clone(),
                    path: format!("{kind}/{name}.{ext}"),
                    content,
                    meta,
                    agent,
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
                    agent: None,
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
                        display_name: a.name.clone(),
                        path,
                        content,
                        meta,
                        agent: Some(AgentView::from_agent(&a)),
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
                    agent: None,
                })
                .collect(),
            "skills" => SkillFile::load_all()
                .into_iter()
                .map(|s| {
                    // Round 4 (A4): the master list shows the file count
                    // (stable parity: `skill.files?.length ?? 0`).
                    let dir = vibe_home().join("skills").join(&s.name);
                    let mut files = Vec::new();
                    collect_skill_files(&dir, &dir, &s.name, &mut files);
                    FileCard {
                        kind: "skills".into(),
                        display_name: s.name.clone(),
                        path: format!("skills/{}/SKILL.md", s.name),
                        content: s.content,
                        meta: files.len().to_string(),
                        agent: None,
                    }
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
                    let agent = mistraldeck_vibe::files::parse_agent_content(&a.name, &a.content)
                        .map(|x| AgentView::from_agent(&x));
                    FileCard {
                        kind: "agents".to_string(),
                        display_name: a.name.clone(),
                        path: format!("agents/{}.toml", a.name),
                        content: a.content,
                        meta,
                        agent,
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
                    agent: None,
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
                    agent: None,
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
                api_key: opt(&row.api_key),
                reasoning_field_name: opt(&row.reasoning_field_name),
                project_id: opt(&row.project_id),
                region: opt(&row.region),
                enabled: Some(row.enabled),
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
        // The settings row carries the full model schema (stage 7C); keys
        // outside the `[[models]]` entry survive the load-modify-save.
        with_cfg(&ctx, |cfg| {
            cfg.upsert_model(&ModelConfig {
                name: row.name.trim().to_string(),
                provider: row.provider.trim().to_string(),
                alias: opt(&row.alias),
                enabled: Some(row.enabled),
                display_name: opt(&row.display_name),
                thinking: opt(&row.thinking),
                thinking_levels: csv_list(&row.thinking_levels),
                max_context_length: num_i64(&row.max_context_length),
                auto_compact_threshold: num_i64(&row.auto_compact_threshold),
                temperature: num_f64(&row.temperature),
                top_p: num_f64(&row.top_p),
                max_output_tokens: num_i64(&row.max_output_tokens),
                input_price: num_f64(&row.input_price),
                output_price: num_f64(&row.output_price),
                cached_input_price: num_f64(&row.cached_input_price),
                supports_images: Some(row.supports_images),
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

    /// Absolute path of the route's `skills/` folder (Z16 "open folder").
    pub fn skills_dir(scope: &str, cwd: &str, conn: Option<&RemoteConnection>) -> Result<String, ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        match ctx {
            Ctx::Local(base) => Ok(base.join("skills").display().to_string()),
            Ctx::Remote { conn, dir } => Ok(format!("{}:{}/skills", conn.name, dir)),
        }
    }

    /// Every file of one skill (`skills/<name>/**`) — Z16 file tree.
    pub fn list_skill_files(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        name: &str,
    ) -> Result<Vec<FileCard>, ServerFnError> {
        let name = safe_skill_name(name)?;
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let Ctx::Local(base) = ctx else {
            // Remote skills list their SKILL.md only (the SSH helpers do not
            // enumerate nested files); editing still works.
            return Ok(list_files("skills", scope, cwd, conn)
                .into_iter()
                .filter(|f| f.display_name == name)
                .collect());
        };
        let dir = base.join("skills").join(&name);
        let mut out: Vec<FileCard> = Vec::new();
        collect_skill_files(&dir, &dir, &name, &mut out);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Create `skills/<name>/SKILL.md` (Z16).
    pub fn create_skill(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        name: &str,
    ) -> Result<(), ServerFnError> {
        let name = safe_skill_name(name)?;
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let Ctx::Local(base) = ctx else {
            return Err(ServerFnError::new(
                "creating skills on an SSH host is not supported yet",
            ));
        };
        let dir = base.join("skills").join(&name);
        fs::create_dir_all(&dir).map_err(|e| ServerFnError::new(e.to_string()))?;
        let file = dir.join("SKILL.md");
        if !file.exists() {
            let body = format!("# {name}\n\nDescribe when this skill applies.\n");
            fs::write(&file, body).map_err(|e| ServerFnError::new(e.to_string()))?;
        }
        Ok(())
    }

    /// Rename a skill folder (round 7, R8): `skills/<old>` → `skills/<new>`,
    /// and the `name:` field in SKILL.md's frontmatter is updated to match.
    pub fn rename_skill(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), ServerFnError> {
        let old = safe_skill_name(old_name)?;
        let new = safe_skill_name(new_name)?;
        if old == new {
            return Ok(());
        }
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let Ctx::Local(base) = ctx else {
            return Err(ServerFnError::new(
                "renaming skills on an SSH host is not supported yet",
            ));
        };
        let src = base.join("skills").join(&old);
        let dst = base.join("skills").join(&new);
        if !src.is_dir() {
            return Err(ServerFnError::new("skill not found"));
        }
        if dst.exists() {
            return Err(ServerFnError::new("a skill with that name already exists"));
        }
        fs::rename(&src, &dst).map_err(|e| ServerFnError::new(e.to_string()))?;
        // keep SKILL.md frontmatter in sync with the folder name
        let md = dst.join("SKILL.md");
        if let Ok(text) = fs::read_to_string(&md) {
            let updated = if text.starts_with("---\n") {
                if let Some(end) = text[4..].find("\n---") {
                    let fm = &text[..end + 4];
                    let body = &text[end + 4..];
                    let mut replaced = false;
                    let new_fm = fm
                        .lines()
                        .map(|l| {
                            if l.starts_with("name:") && !replaced {
                                replaced = true;
                                format!("name: {new}")
                            } else {
                                l.to_string()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let new_fm = if !replaced {
                        format!("{new_fm}\nname: {new}")
                    } else {
                        new_fm
                    };
                    format!("{new_fm}{body}")
                } else {
                    text
                }
            } else {
                text
            };
            let _ = fs::write(&md, updated);
        }
        Ok(())
    }

    /// Delete a whole skill folder (Z16).
    pub fn delete_skill(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        name: &str,
    ) -> Result<(), ServerFnError> {
        let name = safe_skill_name(name)?;
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let Ctx::Local(base) = ctx else {
            return Err(ServerFnError::new(
                "deleting skills on an SSH host is not supported yet",
            ));
        };
        let dir = base.join("skills").join(&name);
        fs::remove_dir_all(&dir).map_err(|e| ServerFnError::new(e.to_string()))
    }

    /// Import a local folder as a skill (Z16): every file is copied under
    /// `skills/<name>/` (the folder name becomes the skill name unless
    /// `name` is given).
    pub fn import_skill_folder(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        source: &str,
        name: &str,
    ) -> Result<String, ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let Ctx::Local(base) = ctx else {
            return Err(ServerFnError::new(
                "importing skills on an SSH host is not supported yet",
            ));
        };
        let src = std::path::Path::new(source.trim());
        if !src.is_dir() {
            return Err(ServerFnError::new("source is not a directory"));
        }
        let derived = if name.trim().is_empty() {
            src.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string()
        } else {
            name.trim().to_string()
        };
        let skill = safe_skill_name(&derived)?;
        let dest = base.join("skills").join(&skill);
        copy_dir(src, &dest).map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(skill)
    }

    /// Import a skill from a `.zip` archive (Z16) — extracted under
    /// `skills/<name>/`.
    pub fn import_skill_zip(
        scope: &str,
        cwd: &str,
        conn: Option<&RemoteConnection>,
        zip_path: &str,
        name: &str,
    ) -> Result<String, ServerFnError> {
        let ctx = resolve_ctx(scope, cwd, conn)?;
        let Ctx::Local(base) = ctx else {
            return Err(ServerFnError::new(
                "importing skills on an SSH host is not supported yet",
            ));
        };
        let path = std::path::Path::new(zip_path.trim());
        let file = fs::File::open(path).map_err(|e| ServerFnError::new(e.to_string()))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| ServerFnError::new(e.to_string()))?;
        let derived = if name.trim().is_empty() {
            path.file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("skill")
                .to_string()
        } else {
            name.trim().to_string()
        };
        let skill = safe_skill_name(&derived)?;
        let dest = base.join("skills").join(&skill);
        fs::create_dir_all(&dest).map_err(|e| ServerFnError::new(e.to_string()))?;
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            let Some(rel) = entry.enclosed_name() else { continue };
            let target = dest.join(rel);
            if entry.is_dir() {
                let _ = fs::create_dir_all(&target);
                continue;
            }
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let mut out = fs::File::create(&target)
                .map_err(|e| ServerFnError::new(e.to_string()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| ServerFnError::new(e.to_string()))?;
        }
        Ok(skill)
    }

    /// Write one `[tools.<name>] permission = "…"` entry (Z15).
    pub fn set_tool_permission(
        scope: &str,
        cwd: &str,
        conn: Option<&mistraldeck_core::snapshot::RemoteConnection>,
        tool: &str,
        permission: &str,
    ) -> Result<(), ServerFnError> {
        let name = tool.trim();
        let perm = permission.trim();
        if name.is_empty() {
            return Err(ServerFnError::new("empty tool name"));
        }
        if !matches!(perm, "ask" | "allow" | "deny") {
            return Err(ServerFnError::new("permission must be ask/allow/deny"));
        }
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| cfg.set_tool_permission(name, perm))
    }

    /// Remove one `[tools.<name>]` entry (Z15 tool list, stable parity).
    pub fn remove_tool_permission(
        scope: &str,
        cwd: &str,
        conn: Option<&mistraldeck_core::snapshot::RemoteConnection>,
        tool: &str,
    ) -> Result<(), ServerFnError> {
        let name = tool.trim();
        if name.is_empty() {
            return Err(ServerFnError::new("empty tool name"));
        }
        let ctx = resolve_ctx(scope, cwd, conn)?;
        with_cfg(&ctx, |cfg| cfg.remove_tool_permission(name))
    }

    /// Apply a structured patch to one agent profile and persist the TOML
    /// (Settings "Agents" configurator, stable parity). Local and SSH.
    pub fn save_agent(
        scope: &str,
        cwd: &str,
        conn: Option<&mistraldeck_core::snapshot::RemoteConnection>,
        name: &str,
        patch: &AgentPatch,
    ) -> Result<(), ServerFnError> {
        use mistraldeck_vibe::files::{agent_file_toml, parse_agent_content, AgentFile};
        let name = name.trim();
        if name.is_empty() {
            return Err(ServerFnError::new("empty agent name"));
        }
        let apply = |a: &mut AgentFile| {
            if let Some(v) = &patch.agent_type {
                a.agent_type = if v.is_empty() { None } else { Some(v.clone()) };
            }
            if let Some(v) = &patch.display_name {
                a.display_name = if v.trim().is_empty() { None } else { Some(v.trim().to_string()) };
            }
            if let Some(v) = &patch.description {
                a.description = if v.trim().is_empty() { None } else { Some(v.trim().to_string()) };
            }
            if let Some(v) = &patch.active_model {
                a.active_model = if v.trim().is_empty() { None } else { Some(v.trim().to_string()) };
            }
            if let Some(v) = &patch.system_prompt_id {
                a.system_prompt_id = if v.trim().is_empty() { None } else { Some(v.trim().to_string()) };
            }
            if let Some(v) = &patch.safety {
                a.safety = if v.trim().is_empty() { None } else { Some(v.trim().to_string()) };
            }
            if let Some(v) = &patch.instructions {
                a.instructions = if v.trim().is_empty() { None } else { Some(v.trim().to_string()) };
            }
            if let Some(v) = &patch.enabled_tools {
                a.enabled_tools = v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
            }
            if let Some(v) = &patch.disabled_tools {
                a.disabled_tools = v.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
            }
            if let Some(list) = &patch.tool_permissions {
                a.tool_permissions = list.clone();
            }
        };
        let ctx = resolve_ctx(scope, cwd, conn)?;
        match &ctx {
            Ctx::Local(base) => {
                let agents = base.join("agents");
                let path = agents.join(format!("{name}.toml"));
                let content = fs::read_to_string(&path).unwrap_or_default();
                let mut a = parse_agent_content(name, &content).unwrap_or_else(|| AgentFile {
                    name: name.to_string(),
                    agent_type: None,
                    active_model: None,
                    system_prompt_id: None,
                    display_name: None,
                    description: None,
                    enabled_tools: Vec::new(),
                    disabled_tools: Vec::new(),
                    tool_permissions: Vec::new(),
                    safety: None,
                    instructions: None,
                });
                apply(&mut a);
                fs::create_dir_all(&agents)
                    .map_err(|e| ServerFnError::new(format!("mkdir failed: {e}")))?;
                fs::write(&path, agent_file_toml(&a))
                    .map_err(|e| ServerFnError::new(format!("write failed: {e}")))?;
                Ok(())
            }
            Ctx::Remote { conn, dir } => {
                let existing = remote_vibe::get_remote_agents(conn, dir)
                    .into_iter()
                    .find(|a| a.name == name);
                let content = existing.map(|a| a.content).unwrap_or_default();
                let mut a = parse_agent_content(name, &content).unwrap_or_else(|| AgentFile {
                    name: name.to_string(),
                    agent_type: None,
                    active_model: None,
                    system_prompt_id: None,
                    display_name: None,
                    description: None,
                    enabled_tools: Vec::new(),
                    disabled_tools: Vec::new(),
                    tool_permissions: Vec::new(),
                    safety: None,
                    instructions: None,
                });
                apply(&mut a);
                remote_vibe::save_remote_agent_raw(conn, dir, name, &agent_file_toml(&a))
                    .map_err(ServerFnError::new)
            }
        }
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

/// Route's `skills/` folder (Z16 "open skills folder").
#[server]
pub async fn api_skills_dir(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    imp::skills_dir(&scope, &cwd, conn.as_ref())
}

/// All files of one skill (Z16 detail pane).
#[server]
pub async fn api_list_skill_files(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    name: String,
) -> Result<Vec<FileCard>, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::list_skill_files(&scope, &cwd, conn.as_ref(), &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_create_skill(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    name: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::create_skill(&scope, &cwd, conn.as_ref(), &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Rename a skill (round 7, R8).
#[server]
pub async fn api_rename_skill(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    old_name: String,
    new_name: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::rename_skill(&scope, &cwd, conn.as_ref(), &old_name, &new_name)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

#[server]
pub async fn api_delete_skill(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    name: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || imp::delete_skill(&scope, &cwd, conn.as_ref(), &name))
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Import a folder as a skill (Z16); returns the created skill name.
#[server]
pub async fn api_import_skill_folder(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    source: String,
    name: String,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::import_skill_folder(&scope, &cwd, conn.as_ref(), &source, &name)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Import a `.zip` as a skill (Z16); returns the created skill name.
#[server]
pub async fn api_import_skill_zip(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    zip_path: String,
    name: String,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::import_skill_zip(&scope, &cwd, conn.as_ref(), &zip_path, &name)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Import a skill from an uploaded `.zip` (browser/webview file picker —
/// round 6, R4: no typing a path; the bytes arrive as base64). The upload
/// lands in a temp file, the existing zip importer runs, the temp is removed.
#[server]
pub async fn api_import_skill_zip_data(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    file_name: String,
    data_base64: String,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data_base64.trim())
            .map_err(|e| ServerFnError::new(format!("bad base64: {e}")))?;
        let stem = std::path::Path::new(&file_name)
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("skill")
            .to_string();
        let tmp = std::env::temp_dir().join(format!("vd-skill-{}.zip", mistraldeck_core::types::now_ms()));
        std::fs::write(&tmp, &bytes).map_err(|e| ServerFnError::new(e.to_string()))?;
        let res = imp::import_skill_zip(&scope, &cwd, conn.as_ref(), tmp.to_str().unwrap_or(""), &stem);
        let _ = std::fs::remove_file(&tmp);
        res
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Import a skill from an uploaded FOLDER (webkitdirectory picker — round 6,
/// R4). `files` = (relative path, base64 content); they are materialized in a
/// temp dir and the existing folder importer copies them over.
#[server]
pub async fn api_import_skill_folder_data(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    folder_name: String,
    files: Vec<(String, String)>,
) -> Result<String, ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        use base64::Engine as _;
        if files.is_empty() {
            return Err(ServerFnError::new("empty folder"));
        }
        let tmp = std::env::temp_dir().join(format!("vd-skill-{}", mistraldeck_core::types::now_ms()));
        std::fs::create_dir_all(&tmp).map_err(|e| ServerFnError::new(e.to_string()))?;
        for (rel, b64) in &files {
            let rel = rel.trim_start_matches('/');
            if rel.is_empty() || rel.contains("..") {
                continue;
            }
            let dest = tmp.join(rel);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| ServerFnError::new(format!("bad base64: {e}")))?;
            std::fs::write(&dest, &bytes).map_err(|e| ServerFnError::new(e.to_string()))?;
        }
        let name = if folder_name.trim().is_empty() {
            String::new()
        } else {
            folder_name.trim().to_string()
        };
        let res = imp::import_skill_folder(&scope, &cwd, conn.as_ref(), tmp.to_str().unwrap_or(""), &name);
        let _ = std::fs::remove_dir_all(&tmp);
        res
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Set one tool permission (`[tools.<name>] permission`) — Z15.
#[server]
pub async fn api_set_tool_permission(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    tool: String,
    permission: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::set_tool_permission(&scope, &cwd, conn.as_ref(), &tool, &permission)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Remove one tool permission entry — Z15 tool list, stable parity.
#[server]
pub async fn api_remove_tool_permission(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    tool: String,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::remove_tool_permission(&scope, &cwd, conn.as_ref(), &tool)
    })
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
}

/// Apply a structured patch to one agent profile (Agents configurator).
#[server]
pub async fn api_save_agent(
    token: String,
    scope: String,
    cwd: String,
    conn: Option<mistraldeck_core::snapshot::RemoteConnection>,
    name: String,
    patch: AgentPatch,
) -> Result<(), ServerFnError> {
    imp::check(&token)?;
    tokio::task::spawn_blocking(move || {
        imp::save_agent(&scope, &cwd, conn.as_ref(), &name, &patch)
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
        let _guard = super::SMOKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

#[cfg(all(test, feature = "server"))]
mod model_smoke {
    use super::imp;
    use super::ModelRow;

    fn read_model(name: &str) -> mistraldeck_vibe::config::ModelConfig {
        let path = mistraldeck_vibe::vibe_home().join("config.toml");
        let toml = std::fs::read_to_string(&path).expect("config.toml readable");
        let cfg = mistraldeck_vibe::config::VibeConfigFile::from_raw(&toml);
        cfg.models()
            .into_iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("model {name} not found"))
    }

    fn upsert(row: ModelRow) {
        imp::upsert_model("global", "", None, row)
            .unwrap_or_else(|e| panic!("upsert failed: {e}"));
    }

    #[test]
    fn upsert_model_full_fields() {
        let _guard = super::SMOKE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Isolated VIBE_HOME so the smoke test never touches the real ~/.vibe.
        let d = std::env::temp_dir().join(format!("md-smoke7c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("temp dir");
        std::env::set_var("VIBE_HOME", &d);

        upsert(ModelRow {
            name: "test-model".into(),
            provider: "mistral".into(),
            alias: "tm".into(),
            enabled: true,
            display_name: "Test Model".into(),
            supports_images: true,
            temperature: "0.4".into(),
            top_p: "0.9".into(),
            max_output_tokens: "4096".into(),
            input_price: "1.25".into(),
            output_price: "3.5".into(),
            cached_input_price: "0.25".into(),
            thinking: "high".into(),
            thinking_levels: "low, high, max".into(),
            max_context_length: "131072".into(),
            auto_compact_threshold: "104857".into(),
        });
        let m = read_model("test-model");
        assert_eq!(m.display_name.as_deref(), Some("Test Model"));
        assert_eq!(m.supports_images, Some(true));
        assert_eq!(m.temperature, Some(0.4));
        assert_eq!(m.top_p, Some(0.9));
        assert_eq!(m.max_output_tokens, Some(4096));
        assert_eq!(m.input_price, Some(1.25));
        assert_eq!(m.output_price, Some(3.5));
        assert_eq!(m.cached_input_price, Some(0.25));
        assert_eq!(m.thinking.as_deref(), Some("high"));
        assert_eq!(
            m.thinking_levels.as_deref(),
            Some(&["low".to_string(), "high".to_string(), "max".to_string()][..])
        );
        assert_eq!(m.max_context_length, Some(131072));
        assert_eq!(m.auto_compact_threshold, Some(104857));

        std::env::remove_var("VIBE_HOME");
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Z15/Z16 regression tests: tool permissions and the skills lifecycle run
/// against a throwaway **project** scope (never the real `~/.vibe`).
#[cfg(all(test, feature = "server"))]
mod tools_and_skills {
    use super::imp;

    fn tmp_project(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vd-skills-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp project");
        dir
    }

    #[test]
    fn tool_permission_roundtrip() {
        let dir = tmp_project("tools");
        let cwd = dir.display().to_string();
        imp::set_tool_permission("project", &cwd, None, "bash", "allow").expect("write");
        let cfg = imp::load("project", &cwd, None).expect("load");
        let row = cfg
            .tools
            .iter()
            .find(|t| t.name == "bash")
            .expect("permission persisted");
        assert_eq!(row.permission, "allow");
        // Rejects garbage instead of writing an invalid permission.
        assert!(imp::set_tool_permission("project", &cwd, None, "bash", "whatever").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skills_create_list_import_delete() {
        let dir = tmp_project("skills");
        let cwd = dir.display().to_string();
        imp::create_skill("project", &cwd, None, "demo").expect("create");
        let files = imp::list_skill_files("project", &cwd, None, "demo").expect("list");
        assert_eq!(files.len(), 1, "SKILL.md created");
        assert!(files[0].path.ends_with("SKILL.md"));

        // Import a folder: every file is copied under skills/<name>/.
        let src = dir.join("importme");
        std::fs::create_dir_all(src.join("sub")).expect("src");
        std::fs::write(src.join("SKILL.md"), "# imported\n").expect("write");
        std::fs::write(src.join("sub/extra.md"), "extra\n").expect("write");
        let name = imp::import_skill_folder(
            "project",
            &cwd,
            None,
            src.display().to_string().as_str(),
            "",
        )
        .expect("import");
        assert_eq!(name, "importme");
        let imported = imp::list_skill_files("project", &cwd, None, "importme").expect("list");
        assert_eq!(imported.len(), 2, "nested file imported");

        // Path traversal is refused.
        assert!(imp::delete_skill("project", &cwd, None, "../escape").is_err());

        imp::delete_skill("project", &cwd, None, "demo").expect("delete");
        assert!(imp::list_skill_files("project", &cwd, None, "demo")
            .expect("list after delete")
            .is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
