//! `~/.vibe/config.toml` access with the latest Mistral Vibe schema.
//!
//! Design rule: the file is loaded into a [`toml::Table`] and edited
//! in place, so every key this app does not know about (new options
//! shipped by the CLI, experiments, admin fields…) survives a save.
//! Typed views are only projections for the editor UI.

use std::path::PathBuf;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::vibe_home;

/// A provider preset (`[[providers]]`).
#[derive(Clone, Debug, Default)]
pub struct Provider {
    pub name: String,
    pub api_base: String,
    pub api_key_env_var: Option<String>,
    pub backend: Option<String>,
    pub api_style: Option<String>,
    pub extra_headers: Vec<(String, String)>,
}

/// A model preset (`[[models]]`).
#[derive(Clone, Debug, Default)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
    pub alias: Option<String>,
    pub enabled: Option<bool>,
}

/// An MCP server (`[[mcp_servers]]`).
#[derive(Clone, Debug, Default)]
pub struct McpServer {
    pub name: String,
    /// `http`, `streamable-http` or `stdio`.
    pub transport: String,
    pub url: Option<String>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub api_key_env: Option<String>,
    pub api_key_header: Option<String>,
    pub api_key_format: Option<String>,
    pub startup_timeout_sec: Option<u64>,
    pub tool_timeout_sec: Option<u64>,
}

/// Round-trip-safe view of `~/.vibe/config.toml`.
pub struct VibeConfigFile {
    pub path: PathBuf,
    pub table: Table,
    dirty: bool,
}

impl VibeConfigFile {
    /// Load the user-level config (`~/.vibe/config.toml`).
    pub fn load() -> Self {
        let path = vibe_home().join("config.toml");
        Self::load_at(&path).unwrap_or(Self {
            path,
            table: Table::new(),
            dirty: false,
        })
    }

    pub fn load_at(path: &std::path::Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self {
                path: path.to_path_buf(),
                table: Table::new(),
                dirty: false,
            });
        }
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let table: Table = content
            .parse::<Value>()
            .map_err(|e| anyhow::anyhow!("TOML parse error in {}: {e}", path.display()))?
            .try_into()
            .unwrap_or_default();
        Ok(Self {
            path: path.to_path_buf(),
            table,
            dirty: false,
        })
    }

    pub fn save(&mut self) -> Result<()> {
        let content = toml::to_string_pretty(&self.table)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, content)
            .with_context(|| format!("failed to write {}", self.path.display()))?;
        self.dirty = false;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    // -- primitive accessors ------------------------------------------------

    pub fn get_str(&self, key: &str) -> Option<String> {
        self.table
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.table.get(key).and_then(Value::as_bool)
    }

    pub fn get_string_list(&self, key: &str) -> Vec<String> {
        self.table
            .get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_str(&mut self, key: &str, value: &str) {
        self.table.insert(key.into(), Value::from(value));
        self.dirty = true;
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.table.insert(key.into(), Value::from(value));
        self.dirty = true;
    }

    pub fn set_string_list(&mut self, key: &str, values: Vec<String>) {
        self.table.insert(
            key.into(),
            Value::Array(values.into_iter().map(Value::from).collect()),
        );
        self.dirty = true;
    }

    pub fn remove_key(&mut self, key: &str) {
        if self.table.remove(key).is_some() {
            self.dirty = true;
        }
    }

    // -- known settings -----------------------------------------------------

    pub fn active_model(&self) -> Option<String> {
        self.get_str("active_model")
    }

    pub fn set_active_model(&mut self, alias: &str) {
        self.set_str("active_model", alias);
    }

    pub fn default_agent(&self) -> String {
        self.get_str("default_agent")
            .unwrap_or_else(|| "accept-edits".into())
    }

    pub fn set_default_agent(&mut self, agent: &str) {
        self.set_str("default_agent", agent);
    }

    pub fn theme(&self) -> String {
        self.get_str("theme").unwrap_or_else(|| "dark".into())
    }

    pub fn set_theme(&mut self, theme: &str) {
        self.set_str("theme", theme);
    }

    // -- providers -----------------------------------------------------------

    fn array_of_tables(&self, key: &str) -> Vec<Table> {
        self.table
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_table().cloned()).collect())
            .unwrap_or_default()
    }

    fn set_array_of_tables(&mut self, key: &str, tables: Vec<Table>) {
        self.table.insert(
            key.into(),
            Value::Array(tables.into_iter().map(Value::Table).collect()),
        );
        self.dirty = true;
    }

    pub fn providers(&self) -> Vec<Provider> {
        self.array_of_tables("providers")
            .iter()
            .map(provider_from_table)
            .collect()
    }

    pub fn upsert_provider(&mut self, provider: &Provider) {
        let mut providers = self.array_of_tables("providers");
        let table = provider_to_table(provider);
        match providers
            .iter()
            .position(|p| p.get("name").and_then(Value::as_str) == Some(provider.name.as_str()))
        {
            Some(idx) => providers[idx] = table,
            None => providers.push(table),
        }
        self.set_array_of_tables("providers", providers);
    }

    pub fn remove_provider(&mut self, name: &str) -> bool {
        let providers = self.array_of_tables("providers");
        let filtered: Vec<_> = providers
            .into_iter()
            .filter(|p| p.get("name").and_then(Value::as_str) != Some(name))
            .collect();
        let changed = filtered.len() != self.array_of_tables("providers").len();
        self.set_array_of_tables("providers", filtered);
        changed
    }

    // -- models ---------------------------------------------------------------

    pub fn models(&self) -> Vec<ModelConfig> {
        self.array_of_tables("models")
            .iter()
            .map(model_from_table)
            .collect()
    }

    pub fn upsert_model(&mut self, model: &ModelConfig) {
        let mut models = self.array_of_tables("models");
        let table = model_to_table(model);
        match models
            .iter()
            .position(|m| m.get("name").and_then(Value::as_str) == Some(model.name.as_str()))
        {
            Some(idx) => models[idx] = table,
            None => models.push(table),
        }
        self.set_array_of_tables("models", models);
    }

    pub fn remove_model(&mut self, name: &str) -> bool {
        let models = self.array_of_tables("models");
        let filtered: Vec<_> = models
            .into_iter()
            .filter(|m| m.get("name").and_then(Value::as_str) != Some(name))
            .collect();
        let changed = filtered.len() != self.array_of_tables("models").len();
        self.set_array_of_tables("models", filtered);
        changed
    }

    /// All model aliases known to the config (`alias` if set, else `name`).
    pub fn model_aliases(&self) -> Vec<String> {
        self.models()
            .iter()
            .map(|m| m.alias.clone().unwrap_or_else(|| m.name.clone()))
            .collect()
    }

    // -- MCP servers -----------------------------------------------------------

    pub fn mcp_servers(&self) -> Vec<McpServer> {
        self.array_of_tables("mcp_servers")
            .iter()
            .map(mcp_from_table)
            .collect()
    }

    pub fn upsert_mcp_server(&mut self, server: &McpServer) {
        let mut servers = self.array_of_tables("mcp_servers");
        let table = mcp_to_table(server);
        match servers
            .iter()
            .position(|s| s.get("name").and_then(Value::as_str) == Some(server.name.as_str()))
        {
            Some(idx) => servers[idx] = table,
            None => servers.push(table),
        }
        self.set_array_of_tables("mcp_servers", servers);
    }

    pub fn remove_mcp_server(&mut self, name: &str) -> bool {
        let servers = self.array_of_tables("mcp_servers");
        let filtered: Vec<_> = servers
            .into_iter()
            .filter(|s| s.get("name").and_then(Value::as_str) != Some(name))
            .collect();
        let changed = filtered.len() != self.array_of_tables("mcp_servers").len();
        self.set_array_of_tables("mcp_servers", filtered);
        changed
    }

    // -- tools -----------------------------------------------------------------

    pub fn enabled_tools(&self) -> Vec<String> {
        self.get_string_list("enabled_tools")
    }

    pub fn disabled_tools(&self) -> Vec<String> {
        self.get_string_list("disabled_tools")
    }

    pub fn set_enabled_tools(&mut self, tools: Vec<String>) {
        self.set_string_list("enabled_tools", tools);
    }

    pub fn set_disabled_tools(&mut self, tools: Vec<String>) {
        self.set_string_list("disabled_tools", tools);
    }

    /// Per-tool settings: `[tools.<name>] permission = "…"` entries.
    pub fn tool_permission(&self, tool: &str) -> Option<String> {
        self.table
            .get("tools")
            .and_then(Value::as_table)
            .and_then(|t| t.get(tool))
            .and_then(Value::as_table)
            .and_then(|t| t.get("permission"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    pub fn set_tool_permission(&mut self, tool: &str, permission: &str) {
        let tools = self
            .table
            .entry("tools")
            .or_insert_with(|| Value::Table(Table::new()));
        if let Value::Table(tools) = tools {
            let entry = tools
                .entry(tool.to_string())
                .or_insert_with(|| Value::Table(Table::new()));
            if let Value::Table(entry) = entry {
                entry.insert("permission".into(), Value::from(permission));
            }
        }
        self.dirty = true;
    }

    // -- skills ------------------------------------------------------------------

    pub fn skill_paths(&self) -> Vec<String> {
        self.get_string_list("skill_paths")
    }

    pub fn enabled_skills(&self) -> Vec<String> {
        self.get_string_list("enabled_skills")
    }

    pub fn disabled_skills(&self) -> Vec<String> {
        self.get_string_list("disabled_skills")
    }

    pub fn set_skill_paths(&mut self, paths: Vec<String>) {
        self.set_string_list("skill_paths", paths);
    }

    pub fn set_enabled_skills(&mut self, patterns: Vec<String>) {
        self.set_string_list("enabled_skills", patterns);
    }

    pub fn set_disabled_skills(&mut self, patterns: Vec<String>) {
        self.set_string_list("disabled_skills", patterns);
    }

    // -- session logging ------------------------------------------------------------

    /// `[session_logging]` fragment (`enabled`, `save_dir`, `auto_compact_threshold`).
    pub fn session_logging(&self) -> Option<Table> {
        self.table
            .get("session_logging")
            .and_then(Value::as_table)
            .cloned()
    }

    pub fn set_session_logging_bool(&mut self, key: &str, value: bool) {
        let entry = self
            .table
            .entry("session_logging")
            .or_insert_with(|| Value::Table(Table::new()));
        if let Value::Table(t) = entry {
            t.insert(key.into(), Value::from(value));
            self.dirty = true;
        }
    }

    pub fn set_session_logging_str(&mut self, key: &str, value: &str) {
        let entry = self
            .table
            .entry("session_logging")
            .or_insert_with(|| Value::Table(Table::new()));
        if let Value::Table(t) = entry {
            t.insert(key.into(), Value::from(value));
            self.dirty = true;
        }
    }

    pub fn set_session_logging_i64(&mut self, key: &str, value: i64) {
        let entry = self
            .table
            .entry("session_logging")
            .or_insert_with(|| Value::Table(Table::new()));
        if let Value::Table(t) = entry {
            t.insert(key.into(), Value::from(value));
            self.dirty = true;
        }
    }

    // -- telemetry / updates / trust -----------------------------------------------

    pub fn enable_telemetry(&self) -> bool {
        self.get_bool("enable_telemetry").unwrap_or(true)
    }

    pub fn enable_otel(&self) -> bool {
        self.get_bool("enable_otel").unwrap_or(false)
    }

    pub fn otel_endpoint(&self) -> Option<String> {
        self.get_str("otel_endpoint")
    }

    pub fn enable_update_checks(&self) -> bool {
        self.get_bool("enable_update_checks").unwrap_or(true)
    }

    pub fn enable_notifications(&self) -> bool {
        self.get_bool("enable_notifications").unwrap_or(true)
    }

    pub fn enable_system_trust_store(&self) -> bool {
        self.get_bool("enable_system_trust_store").unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// toml ↔ typed view conversions
// ---------------------------------------------------------------------------

fn table_str(t: &Table, key: &str) -> Option<String> {
    t.get(key).and_then(Value::as_str).map(str::to_string)
}

fn table_str_list(t: &Table, key: &str) -> Vec<String> {
    t.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn table_pairs(t: &Table, key: &str) -> Vec<(String, String)> {
    t.get(key)
        .and_then(Value::as_table)
        .map(|tbl| {
            tbl.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn pairs_value(pairs: &[(String, String)]) -> Option<Value> {
    if pairs.is_empty() {
        return None;
    }
    let mut t = Table::new();
    for (k, v) in pairs {
        t.insert(k.clone(), Value::from(v.as_str()));
    }
    Some(Value::Table(t))
}

fn provider_from_table(t: &Table) -> Provider {
    Provider {
        name: table_str(t, "name").unwrap_or_default(),
        api_base: table_str(t, "api_base").unwrap_or_default(),
        api_key_env_var: table_str(t, "api_key_env_var"),
        backend: table_str(t, "backend"),
        api_style: table_str(t, "api_style"),
        extra_headers: table_pairs(t, "extra_headers"),
    }
}

fn provider_to_table(p: &Provider) -> Table {
    let mut t = Table::new();
    t.insert("name".into(), Value::from(p.name.as_str()));
    if !p.api_base.is_empty() {
        t.insert("api_base".into(), Value::from(p.api_base.as_str()));
    }
    if let Some(v) = &p.api_key_env_var {
        t.insert("api_key_env_var".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &p.backend {
        t.insert("backend".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &p.api_style {
        t.insert("api_style".into(), Value::from(v.as_str()));
    }
    if let Some(v) = pairs_value(&p.extra_headers) {
        t.insert("extra_headers".into(), v);
    }
    t
}

fn model_from_table(t: &Table) -> ModelConfig {
    ModelConfig {
        name: table_str(t, "name").unwrap_or_default(),
        provider: table_str(t, "provider").unwrap_or_default(),
        alias: table_str(t, "alias"),
        enabled: t.get("enabled").and_then(Value::as_bool),
    }
}

fn model_to_table(m: &ModelConfig) -> Table {
    let mut t = Table::new();
    t.insert("name".into(), Value::from(m.name.as_str()));
    t.insert("provider".into(), Value::from(m.provider.as_str()));
    if let Some(alias) = &m.alias {
        t.insert("alias".into(), Value::from(alias.as_str()));
    }
    if let Some(enabled) = m.enabled {
        t.insert("enabled".into(), Value::from(enabled));
    }
    t
}

fn mcp_from_table(t: &Table) -> McpServer {
    McpServer {
        name: table_str(t, "name").unwrap_or_default(),
        transport: table_str(t, "transport").unwrap_or_else(|| "stdio".into()),
        url: table_str(t, "url"),
        command: table_str(t, "command"),
        args: table_str_list(t, "args"),
        env: table_pairs(t, "env"),
        headers: table_pairs(t, "headers"),
        api_key_env: table_str(t, "api_key_env"),
        api_key_header: table_str(t, "api_key_header"),
        api_key_format: table_str(t, "api_key_format"),
        startup_timeout_sec: t
            .get("startup_timeout_sec")
            .and_then(Value::as_integer)
            .map(|v| v.max(0) as u64),
        tool_timeout_sec: t
            .get("tool_timeout_sec")
            .and_then(Value::as_integer)
            .map(|v| v.max(0) as u64),
    }
}

fn mcp_to_table(s: &McpServer) -> Table {
    let mut t = Table::new();
    t.insert("name".into(), Value::from(s.name.as_str()));
    t.insert("transport".into(), Value::from(s.transport.as_str()));
    if let Some(v) = &s.url {
        t.insert("url".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &s.command {
        t.insert("command".into(), Value::from(v.as_str()));
    }
    if !s.args.is_empty() {
        t.insert(
            "args".into(),
            Value::Array(s.args.iter().map(|a| Value::from(a.as_str())).collect()),
        );
    }
    if let Some(v) = pairs_value(&s.env) {
        t.insert("env".into(), v);
    }
    if let Some(v) = pairs_value(&s.headers) {
        t.insert("headers".into(), v);
    }
    if let Some(v) = &s.api_key_env {
        t.insert("api_key_env".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &s.api_key_header {
        t.insert("api_key_header".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &s.api_key_format {
        t.insert("api_key_format".into(), Value::from(v.as_str()));
    }
    if let Some(v) = s.startup_timeout_sec {
        t.insert("startup_timeout_sec".into(), Value::from(v as i64));
    }
    if let Some(v) = s.tool_timeout_sec {
        t.insert("tool_timeout_sec".into(), Value::from(v as i64));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
active_model = "mistral-medium-3.5"
default_agent = "ask"
enable_telemetry = true
experiments = { voice = true }

[[providers]]
name = "openrouter"
api_base = "https://openrouter.ai/api/v1"
api_key_env_var = "OPENROUTER_API_KEY"
api_style = "openai"
backend = "generic"

[[models]]
name = "mistralai/codestral"
provider = "openrouter"
alias = "codestral-openrouter"

[[mcp_servers]]
name = "mistralai"
transport = "streamable-http"
url = "https://api.mistral.ai/mcp"
api_key_env = "MISTRAL_API_KEY"

[tools.bash]
permission = "ask"

[session_logging]
enabled = true
"#;

    #[test]
    fn round_trips_unknown_keys() {
        let mut cfg =
            VibeConfigFile::load_at(std::path::Path::new("/tmp/does-not-exist.toml")).unwrap();
        cfg.table = SAMPLE.parse::<Value>().unwrap().try_into().unwrap();
        assert_eq!(cfg.active_model().as_deref(), Some("mistral-medium-3.5"));

        // Edit known keys…
        cfg.set_active_model("codestral-openrouter");
        // …and save round-trips everything, including unknown keys.
        let out = toml::to_string_pretty(&cfg.table).unwrap();
        assert!(out.contains("experiments"));
        assert!(out.contains("codestral-openrouter"));
        assert!(out.contains("[tools.bash]"));
        assert!(out.contains("voice = true"));
    }

    #[test]
    fn providers_models_mcp_roundtrip() {
        let mut cfg = VibeConfigFile::load_at(std::path::Path::new("/tmp/nope.toml")).unwrap();
        cfg.table = SAMPLE.parse::<Value>().unwrap().try_into().unwrap();

        let providers = cfg.providers();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].api_base, "https://openrouter.ai/api/v1");
        assert_eq!(providers[0].extra_headers.len(), 0);

        let models = cfg.models();
        assert_eq!(models.len(), 1);
        assert_eq!(
            cfg.model_aliases(),
            vec!["codestral-openrouter".to_string()]
        );

        let servers = cfg.mcp_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].transport, "streamable-http");
        assert_eq!(servers[0].api_key_env.as_deref(), Some("MISTRAL_API_KEY"));

        // Upsert a new server, then remove it.
        let mut srv = McpServer::default();
        srv.name = "local".into();
        srv.transport = "stdio".into();
        srv.command = Some("node".into());
        srv.args = vec!["server.js".into()];
        cfg.upsert_mcp_server(&srv);
        assert_eq!(cfg.mcp_servers().len(), 2);
        assert!(cfg.remove_mcp_server("local"));
        assert_eq!(cfg.mcp_servers().len(), 1);

        assert_eq!(cfg.tool_permission("bash").as_deref(), Some("ask"));
        cfg.set_tool_permission("bash", "allow");
        assert_eq!(cfg.tool_permission("bash").as_deref(), Some("allow"));
    }

    #[test]
    fn telemetry_flags_default_like_cli() {
        let cfg = VibeConfigFile::load_at(std::path::Path::new("/tmp/nope2.toml")).unwrap();
        assert!(cfg.enable_telemetry());
        assert!(cfg.enable_update_checks());
        assert!(!cfg.enable_otel());
        assert!(!cfg.enable_system_trust_store());
        assert_eq!(cfg.default_agent(), "accept-edits");
    }
}
