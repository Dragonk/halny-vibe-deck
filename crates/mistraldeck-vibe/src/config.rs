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

/// Per-million-token prices of a `[[models]]` entry (USD / Mtok),
/// used for the live cost estimate in the Statistics view.
/// Validate raw TOML text for config.toml (Nexus `saveRemoteRawConfig`
/// check): must parse and be a table; `Err` carries the parser message.
pub fn validate_config_toml(content: &str) -> Result<(), String> {
    let value: Value = toml::from_str(content).map_err(|e| e.to_string())?;
    if value.as_table().is_none() {
        return Err("config.toml must be a TOML table".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Default)]
pub struct ModelPrice {
    pub name: String,
    /// matches by `name` or `alias`
    pub alias: Option<String>,
    pub input_price: f64,
    pub output_price: f64,
    /// `None` on a model without cache support - cached tokens are then
    /// billed at the plain input price (Nexus `modelPriceFor`).
    pub cached_input_price: Option<f64>,
}

/// Read the `[[models]]` price table from `~/.vibe/config.toml`.
/// Returns an empty list when the file is missing or has no prices.
pub fn model_prices() -> Vec<ModelPrice> {
    let path: PathBuf = vibe_home().join("config.toml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = content.parse::<Value>() else {
        return Vec::new();
    };
    let Some(models) = value.get("models") else {
        return Vec::new();
    };
    // normalizeVibeConfig parity: also accept `[models.<name>]` tables.
    let entries: Vec<(Option<String>, &Value)> = match models {
        Value::Array(a) => a.iter().map(|v| (None, v)).collect(),
        Value::Table(t) => t.iter().map(|(k, v)| (Some(k.clone()), v)).collect(),
        _ => return Vec::new(),
    };
    entries
        .iter()
        .filter_map(|(key, m)| {
            let name = match m.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => key.clone()?,
            };
            let f = |k: &str| {
                m.get(k)
                    .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
                    .unwrap_or(0.0)
            };
            Some(ModelPrice {
                alias: m
                    .get("alias")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                name,
                input_price: f("input_price"),
                output_price: f("output_price"),
                cached_input_price: m
                    .get("cached_input_price")
                    .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))),
            })
        })
        .collect()
}

/// Estimate the turn cost from token counts and the price table
/// (USD, per-million-token pricing as written by the CLI).
pub fn estimate_cost(
    prices: &[ModelPrice],
    model: &str,
    prompt_tokens: f64,
    completion_tokens: f64,
    cached_tokens: f64,
) -> f64 {
    // Nexus modelPriceFor: match by name/alias case-insensitively,
    // including mutual endsWith.
    let lower = model.to_lowercase();
    let price = prices.iter().find(|p| {
        p.name == model
            || p.alias.as_deref() == Some(model)
            || p.name.to_lowercase() == lower
            || p.alias.as_deref().map(str::to_lowercase).as_deref() == Some(lower.as_str())
            || p.name.to_lowercase().ends_with(&lower)
            || lower.ends_with(&p.name.to_lowercase())
    });
    let Some(price) = price else {
        return 0.0;
    };
    // Nexus computeSessionCost: cached tokens are billed at the cached
    // price, the uncached remainder at the input price; a missing
    // cached_input_price (model without cache support) bills cache at
    // the plain input price instead of free.
    let cached_price = price.cached_input_price.unwrap_or(price.input_price);
    let uncached = (prompt_tokens - cached_tokens).max(0.0);
    (uncached * price.input_price
        + cached_tokens * cached_price
        + completion_tokens * price.output_price)
        / 1_000_000.0
}

/// Mirror of the CLI's auto_compact_threshold resolution
/// (vibe/core/config/vibe_schema.py, Nexus `resolveCompactThreshold`):
/// a per-model cap wins, then the global cap, then 80% of the declared
/// window, then 200 000. `Some(0)` = never compact; `-1` disables an
/// override.
pub fn resolve_compact_threshold(
    cap: Option<i64>,
    window: Option<i64>,
    global: Option<i64>,
) -> Option<i64> {
    if let Some(cap) = cap {
        if cap != -1 && (window.is_none() || cap < window.unwrap()) {
            return Some(cap);
        }
    }
    let global_set = global.filter(|g| *g != -1);
    let global_value = global_set.unwrap_or(200_000);
    if global_set.is_some() && (window.is_none() || global_value < window.unwrap()) {
        return Some(global_value);
    }
    if let Some(w) = window.filter(|w| *w > 0) {
        return Some((w as f64 * 0.8) as i64);
    }
    Some(global_value)
}

/// Context window (`max_context_length`) and auto-compact threshold of a
/// model from `~/.vibe/config.toml`, matched by exact `name` or `alias`
/// (Nexus UsageBar lookup). `window = None` = the model declares no
/// window and gets no gauge.
pub fn usage_model_info(model: &str) -> (Option<i64>, Option<i64>) {
    let path: PathBuf = vibe_home().join("config.toml");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (None, None);
    };
    let Ok(value) = content.parse::<Value>() else {
        return (None, None);
    };
    let global = value
        .get("auto_compact_threshold")
        .and_then(Value::as_integer);
    let found = value
        .get("models")
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter().find(|m| {
                let name = m.get("name").and_then(Value::as_str);
                let alias = m.get("alias").and_then(Value::as_str);
                name == Some(model) || alias == Some(model)
            })
        });
    let window = found
        .and_then(|m| m.get("max_context_length"))
        .and_then(Value::as_integer)
        .filter(|w| *w > 0);
    let cap = found
        .and_then(|m| m.get("auto_compact_threshold"))
        .and_then(Value::as_integer);
    (window, resolve_compact_threshold(cap, window, global))
}

/// A provider preset (`[[providers]]`).
#[derive(Clone, Debug, Default)]
pub struct Provider {
    pub name: String,
    pub api_base: String,
    pub api_key_env_var: Option<String>,
    pub backend: Option<String>,
    pub api_style: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    /// Inline API key (`api_key`).
    pub api_key: Option<String>,
    /// Provider-specific reasoning field (`reasoning_field_name`).
    pub reasoning_field_name: Option<String>,
    /// Cloud project id (`project_id`).
    pub project_id: Option<String>,
    /// Cloud region (`region`).
    pub region: Option<String>,
    /// Toggle (`enabled`); None = enabled.
    pub enabled: Option<bool>,
}

/// A model preset (`[[models]]`).
#[derive(Clone, Debug, Default)]
pub struct ModelConfig {
    pub name: String,
    pub provider: String,
    pub alias: Option<String>,
    pub enabled: Option<bool>,
    /// Human-friendly label shown in pickers (`display_name`).
    pub display_name: Option<String>,
    /// Default reasoning effort of the model (`thinking`).
    pub thinking: Option<String>,
    /// Allowed reasoning efforts (`thinking_levels`; empty/missing = the
    /// CLI default levels off/low/medium/high/max).
    pub thinking_levels: Option<Vec<String>>,
    /// Model context window (`max_context_length`), tokens.
    pub max_context_length: Option<i64>,
    /// Per-model auto-compact override (`auto_compact_threshold`);
    /// -1 disables the override, 0 = never compact.
    pub auto_compact_threshold: Option<i64>,
    /// Sampling temperature (`temperature`).
    pub temperature: Option<f64>,
    /// Nucleus sampling probability (`top_p`).
    pub top_p: Option<f64>,
    /// Hard output cap (`max_output_tokens`), tokens.
    pub max_output_tokens: Option<i64>,
    /// `input_price` ($/M tokens).
    pub input_price: Option<f64>,
    /// `output_price` ($/M tokens).
    pub output_price: Option<f64>,
    /// `cached_input_price` ($/M); None = model without cache support.
    pub cached_input_price: Option<f64>,
    /// Whether the model accepts image input (`supports_images`).
    pub supports_images: Option<bool>,
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
    pub cwd: Option<String>,
    /// Explicit `[mcp_servers.auth]` block; `static` or `oauth` (discriminated
    /// union — the other branch's keys must NOT be present). When `None`, the
    /// legacy flat auth keys are used as-is (read + write).
    pub auth: Option<McpAuth>,
}

/// MCP auth block (`[mcp_servers.auth]`): `static` (headers / API key) or
/// `oauth` (browser login, auto-refreshing tokens).
#[derive(Clone, Debug, Default)]
pub struct McpAuth {
    /// `static` or `oauth`.
    pub auth_type: String,
    pub headers: Vec<(String, String)>,
    pub api_key_env: Option<String>,
    pub api_key_header: Option<String>,
    pub api_key_format: Option<String>,
    pub scopes: Vec<String>,
    pub client_id: Option<String>,
    pub client_metadata_url: Option<String>,
    pub redirect_port: Option<i64>,
}

fn mcp_auth_from_table(t: &Table) -> McpAuth {
    McpAuth {
        auth_type: table_str(t, "type")
            .or_else(|| table_str(t, "auth_type"))
            .unwrap_or_else(|| "static".into()),
        headers: table_pairs(t, "headers"),
        api_key_env: table_str(t, "api_key_env"),
        api_key_header: table_str(t, "api_key_header"),
        api_key_format: table_str(t, "api_key_format"),
        scopes: table_str_list(t, "scopes"),
        client_id: table_str(t, "client_id"),
        client_metadata_url: table_str(t, "client_metadata_url"),
        redirect_port: t.get("redirect_port").and_then(Value::as_integer),
    }
}

fn mcp_auth_to_table(a: &McpAuth) -> Table {
    let mut t = Table::new();
    t.insert("type".into(), Value::from(a.auth_type.as_str()));
    if a.auth_type == "oauth" {
        // oauth branch: strip static-only keys (CLI discriminated union)
        if !a.scopes.is_empty() {
            t.insert(
                "scopes".into(),
                Value::from(a.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
            );
        }
        if let Some(v) = &a.client_id {
            t.insert("client_id".into(), Value::from(v.as_str()));
        }
        if let Some(v) = &a.client_metadata_url {
            t.insert("client_metadata_url".into(), Value::from(v.as_str()));
        }
        if let Some(v) = a.redirect_port {
            t.insert("redirect_port".into(), Value::from(v));
        }
    } else {
        // static branch: strip oauth-only keys
        if let Some(v) = pairs_value(&a.headers) {
            t.insert("headers".into(), v);
        }
        if let Some(v) = &a.api_key_env {
            t.insert("api_key_env".into(), Value::from(v.as_str()));
        }
        if let Some(v) = &a.api_key_header {
            t.insert("api_key_header".into(), Value::from(v.as_str()));
        }
        if let Some(v) = &a.api_key_format {
            t.insert("api_key_format".into(), Value::from(v.as_str()));
        }
    }
    t
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

    /// Build a config from raw TOML text (remote round-trip): unparseable
    /// content yields an empty table instead of an error.
    pub fn from_raw(content: &str) -> Self {
        let table: Table = content
            .parse::<Value>()
            .ok()
            .and_then(|v| v.try_into().ok())
            .unwrap_or_default();
        Self {
            path: PathBuf::new(),
            table,
            dirty: false,
        }
    }

    /// Render the table back to TOML text (remote round-trip).
    pub fn render(&self) -> String {
        toml::to_string_pretty(&self.table).unwrap_or_default()
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
        // normalizeVibeConfig parity (Nexus vibeConfig.ts): tolerate
        // `[models.<name>]` tables where the app expects `[[models]]` — a
        // hand-edited or foreign-shaped config must not hide the models
        // from the UI. Read-side guard only; the CLI schema is `[[models]]`.
        if let Some(Value::Table(entries)) = self.table.get("models") {
            let mut out = Vec::new();
            for (key, v) in entries.iter() {
                let mut entry = v.as_table().cloned().unwrap_or_default();
                entry.insert("name".into(), Value::from(key.as_str()));
                out.push(model_from_table(&entry));
            }
            return out;
        }
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

fn table_float(t: &Table, key: &str) -> Option<f64> {
    t.get(key).and_then(|v| {
        v.as_float()
            .or_else(|| v.as_integer().map(|i| i as f64))
    })
}

fn provider_from_table(t: &Table) -> Provider {
    Provider {
        name: table_str(t, "name").unwrap_or_default(),
        api_base: table_str(t, "api_base").unwrap_or_default(),
        api_key_env_var: table_str(t, "api_key_env_var"),
        backend: table_str(t, "backend"),
        api_style: table_str(t, "api_style"),
        extra_headers: table_pairs(t, "extra_headers"),
        api_key: table_str(t, "api_key"),
        reasoning_field_name: table_str(t, "reasoning_field_name"),
        project_id: table_str(t, "project_id"),
        region: table_str(t, "region"),
        enabled: t.get("enabled").and_then(Value::as_bool),
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
    if let Some(v) = &p.api_key {
        t.insert("api_key".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &p.reasoning_field_name {
        t.insert("reasoning_field_name".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &p.project_id {
        t.insert("project_id".into(), Value::from(v.as_str()));
    }
    if let Some(v) = &p.region {
        t.insert("region".into(), Value::from(v.as_str()));
    }
    if let Some(v) = p.enabled {
        t.insert("enabled".into(), Value::from(v));
    }
    t
}

fn model_from_table(t: &Table) -> ModelConfig {
    ModelConfig {
        name: table_str(t, "name").unwrap_or_default(),
        provider: table_str(t, "provider").unwrap_or_default(),
        alias: table_str(t, "alias"),
        enabled: t.get("enabled").and_then(Value::as_bool),
        display_name: table_str(t, "display_name"),
        thinking: table_str(t, "thinking"),
        thinking_levels: {
            let levels = table_str_list(t, "thinking_levels");
            if levels.is_empty() {
                None
            } else {
                Some(levels)
            }
        },
        max_context_length: t
            .get("max_context_length")
            .and_then(Value::as_integer),
        auto_compact_threshold: t
            .get("auto_compact_threshold")
            .and_then(Value::as_integer),
        temperature: table_float(t, "temperature"),
        top_p: table_float(t, "top_p"),
        max_output_tokens: t.get("max_output_tokens").and_then(Value::as_integer),
        input_price: table_float(t, "input_price"),
        output_price: table_float(t, "output_price"),
        cached_input_price: table_float(t, "cached_input_price"),
        supports_images: t.get("supports_images").and_then(Value::as_bool),
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
    if let Some(display_name) = &m.display_name {
        t.insert("display_name".into(), Value::from(display_name.as_str()));
    }
    if let Some(thinking) = &m.thinking {
        t.insert("thinking".into(), Value::from(thinking.as_str()));
    }
    if let Some(levels) = &m.thinking_levels {
        t.insert(
            "thinking_levels".into(),
            Value::from(levels.iter().map(|s| s.as_str()).collect::<Vec<_>>()),
        );
    }
    if let Some(v) = m.max_context_length {
        t.insert("max_context_length".into(), Value::from(v));
    }
    if let Some(v) = m.auto_compact_threshold {
        t.insert("auto_compact_threshold".into(), Value::from(v));
    }
    if let Some(v) = m.temperature {
        t.insert("temperature".into(), Value::from(v));
    }
    if let Some(v) = m.top_p {
        t.insert("top_p".into(), Value::from(v));
    }
    if let Some(v) = m.max_output_tokens {
        t.insert("max_output_tokens".into(), Value::from(v));
    }
    if let Some(v) = m.input_price {
        t.insert("input_price".into(), Value::from(v));
    }
    if let Some(v) = m.output_price {
        t.insert("output_price".into(), Value::from(v));
    }
    if let Some(v) = m.cached_input_price {
        t.insert("cached_input_price".into(), Value::from(v));
    }
    if let Some(v) = m.supports_images {
        t.insert("supports_images".into(), Value::from(v));
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
        cwd: table_str(t, "cwd"),
        auth: t.get("auth").and_then(Value::as_table).map(mcp_auth_from_table),
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
    if let Some(a) = &s.auth {
        // Explicit auth block: legacy flat keys must NOT mix with [auth]
        // (the CLI rejects that combination).
        t.insert("auth".into(), Value::Table(mcp_auth_to_table(a)));
    } else {
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
    }
    if let Some(v) = &s.cwd {
        t.insert("cwd".into(), Value::from(v.as_str()));
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
    use super::resolve_compact_threshold;

    #[test]
    fn from_raw_render_roundtrip() {
        let raw = "active_model = \"mistral-medium-3.5\"\n[settings]\ntheme = \"dark\"\n";
        let cfg = super::VibeConfigFile::from_raw(raw);
        assert_eq!(
            cfg.get_str("active_model").as_deref(),
            Some("mistral-medium-3.5")
        );
        let rendered = cfg.render();
        let cfg2 = super::VibeConfigFile::from_raw(&rendered);
        assert_eq!(
            cfg2.get_str("active_model").as_deref(),
            Some("mistral-medium-3.5")
        );
        assert_eq!(
            cfg2.get_str("theme").or_else(|| cfg2
                .table
                .get("settings")
                .and_then(|s| s.get("theme"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()))
                .as_deref(),
            Some("dark")
        );
        // Unparseable raw content yields an empty config, not an error.
        assert!(super::VibeConfigFile::from_raw("not toml {{{").table.is_empty());
    }

    #[test]
    fn compact_threshold_resolution() {
        // Per-model cap wins when below the window.
        assert_eq!(
            resolve_compact_threshold(Some(150_000), Some(262_144), Some(180_000)),
            Some(150_000)
        );
        // -1 disables the per-model cap: the global cap applies.
        assert_eq!(
            resolve_compact_threshold(Some(-1), Some(262_144), Some(180_000)),
            Some(180_000)
        );
        // Global -1: falls back to 80% of the window (Nexus: 209715 for 262144).
        assert_eq!(
            resolve_compact_threshold(None, Some(262_144), Some(-1)),
            Some(209_715)
        );
        // No overrides at all: 80% of the window.
        assert_eq!(
            resolve_compact_threshold(None, Some(100_000), None),
            Some(80_000)
        );
        // No window: the 200k default.
        assert_eq!(resolve_compact_threshold(None, None, None), Some(200_000));
        // 0 = never compact.
        assert_eq!(
            resolve_compact_threshold(Some(0), Some(262_144), None),
            Some(0)
        );
    }

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
    fn model_full_fields_roundtrip() {
        let mut cfg = VibeConfigFile::load_at(std::path::Path::new("/tmp/nope3.toml")).unwrap();
        cfg.upsert_model(&ModelConfig {
            name: "x".into(),
            provider: "p".into(),
            alias: Some("ax".into()),
            enabled: Some(false),
            display_name: Some("X Model".into()),
            thinking: Some("max".into()),
            thinking_levels: Some(vec!["low".into(), "max".into()]),
            max_context_length: Some(262144),
            auto_compact_threshold: Some(209715),
            temperature: Some(0.3),
            top_p: Some(0.95),
            max_output_tokens: Some(8192),
            input_price: Some(0.5),
            output_price: Some(1.5),
            cached_input_price: Some(0.1),
            supports_images: Some(true),
        });
        let out = cfg.models();
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert_eq!(m.name, "x");
        assert_eq!(m.provider, "p");
        assert_eq!(m.alias.as_deref(), Some("ax"));
        assert_eq!(m.enabled, Some(false));
        assert_eq!(m.display_name.as_deref(), Some("X Model"));
        assert_eq!(m.thinking.as_deref(), Some("max"));
        assert_eq!(
            m.thinking_levels.as_deref(),
            Some(&["low".to_string(), "max".to_string()][..])
        );
        assert_eq!(m.max_context_length, Some(262144));
        assert_eq!(m.auto_compact_threshold, Some(209715));
        assert_eq!(m.temperature, Some(0.3));
        assert_eq!(m.top_p, Some(0.95));
        assert_eq!(m.max_output_tokens, Some(8192));
        assert_eq!(m.input_price, Some(0.5));
        assert_eq!(m.output_price, Some(1.5));
        assert_eq!(m.cached_input_price, Some(0.1));
        assert_eq!(m.supports_images, Some(true));
    }

    #[test]
    fn provider_full_fields_roundtrip() {
        let mut cfg = VibeConfigFile::load_at(std::path::Path::new("/tmp/nope4.toml")).unwrap();
        cfg.upsert_provider(&Provider {
            name: "gem".into(),
            api_base: "https://example.com".into(),
            api_key_env_var: Some("GEM_KEY".into()),
            backend: Some("gemini".into()),
            api_style: Some("google".into()),
            extra_headers: Vec::new(),
            api_key: Some("sk-x".into()),
            reasoning_field_name: Some("thought".into()),
            project_id: Some("proj-1".into()),
            region: Some("eu".into()),
            enabled: Some(false),
        });
        let out = cfg.providers();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].api_key.as_deref(), Some("sk-x"));
        assert_eq!(out[0].reasoning_field_name.as_deref(), Some("thought"));
        assert_eq!(out[0].project_id.as_deref(), Some("proj-1"));
        assert_eq!(out[0].region.as_deref(), Some("eu"));
        assert_eq!(out[0].enabled, Some(false));
        assert_eq!(out[0].api_style.as_deref(), Some("google"));
        assert_eq!(out[0].api_key_env_var.as_deref(), Some("GEM_KEY"));
    }

    #[test]
    fn models_object_form_normalized() {
        // normalizeVibeConfig parity: `[models.<name>]` tables must surface
        // as models named after the key instead of vanishing.
        let mut cfg = VibeConfigFile::load_at(std::path::Path::new("/tmp/nope5.toml")).unwrap();
        cfg.table = r#"
[models.foo]
provider = "mistral"
display_name = "Foo"
temperature = 0.2
"#
        .parse::<Value>()
        .unwrap()
        .try_into()
        .unwrap();
        let models = cfg.models();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "foo");
        assert_eq!(models[0].provider, "mistral");
        assert_eq!(models[0].display_name.as_deref(), Some("Foo"));
        assert_eq!(models[0].temperature, Some(0.2));
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

    fn price(name: &str, input: f64, output: f64, cached: Option<f64>) -> ModelPrice {
        ModelPrice {
            name: name.to_string(),
            alias: None,
            input_price: input,
            output_price: output,
            cached_input_price: cached,
        }
    }

    #[test]
    fn estimate_cost_bills_cache_separately() {
        let prices = vec![price("m", 1.0, 2.0, Some(0.5))];
        // 600 uncached @1.0 + 400 cached @0.5 + 100 output @2.0 (per Mtok)
        let cost = estimate_cost(&prices, "m", 1000.0, 100.0, 400.0);
        assert!((cost - 0.001).abs() < 1e-12, "{cost}");
    }

    #[test]
    fn estimate_cost_missing_cache_price_bills_input() {
        let prices = vec![price("m", 1.0, 2.0, None)];
        // cached billed at the plain input price: 1000 @1.0 + 100 @2.0
        let cost = estimate_cost(&prices, "m", 1000.0, 100.0, 400.0);
        assert!((cost - 0.0012).abs() < 1e-12, "{cost}");
    }

    #[test]
    fn estimate_cost_matches_case_insensitive_and_suffix() {
        let prices = vec![price("mistral-large", 1.0, 2.0, None)];
        assert!(estimate_cost(&prices, "Mistral-Large", 10.0, 0.0, 0.0) > 0.0);
        assert!(estimate_cost(&prices, "some/mistral-large", 10.0, 0.0, 0.0) > 0.0);
        assert_eq!(estimate_cost(&prices, "other-model", 10.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn mcp_oauth_auth_block_roundtrip() {
        let raw = "[[mcp_servers]]\nname = \"srv\"\ntransport = \"http\"\nurl = \"https://api.example.com/mcp\"\n\n[mcp_servers.auth]\ntype = \"oauth\"\nscopes = [\"read\", \"write\"]\nclient_id = \"cid-1\"\nclient_metadata_url = \"https://auth.example.com/metadata\"\nredirect_port = 47823\n";
        let cfg = super::VibeConfigFile::from_raw(raw);
        let srv = cfg.mcp_servers();
        assert_eq!(srv.len(), 1);
        let a = srv[0].auth.as_ref().expect("auth block parsed");
        assert_eq!(a.auth_type, "oauth");
        assert_eq!(a.scopes, vec!["read".to_string(), "write".to_string()]);
        assert_eq!(a.client_id.as_deref(), Some("cid-1"));
        assert_eq!(a.redirect_port, Some(47823));
        assert!(a.headers.is_empty());
        let out = cfg.render();
        assert!(!out.is_empty(), "render must not fail");
        assert!(!out.contains("api_key_env"), "no static keys in oauth branch");
        let cfg2 = super::VibeConfigFile::from_raw(&out);
        let srv2 = cfg2.mcp_servers();
        assert_eq!(srv2.len(), 1, "roundtrip keeps the server:\n{out}");
        let a2 = srv2[0].auth.as_ref().expect("auth survives roundtrip");
        assert_eq!(a2.auth_type, "oauth");
        assert_eq!(a2.scopes.len(), 2);
        assert_eq!(a2.redirect_port, Some(47823));
        assert_eq!(srv2[0].url.as_deref(), Some("https://api.example.com/mcp"));
    }

    #[test]
    fn mcp_static_auth_block_and_legacy_flat() {
        let raw = "[[mcp_servers]]\nname = \"s1\"\ntransport = \"http\"\nurl = \"https://x/mcp\"\n\n[mcp_servers.auth]\ntype = \"static\"\napi_key_env = \"MY_KEY\"\napi_key_header = \"X-Key\"\napi_key_format = \"Bearer {token}\"\n\n[mcp_servers.auth.headers]\nX-Api-Key = \"secret\"\n";
        let cfg = super::VibeConfigFile::from_raw(raw);
        let s = &cfg.mcp_servers()[0];
        let a = s.auth.as_ref().expect("static auth parsed");
        assert_eq!(a.auth_type, "static");
        assert_eq!(a.api_key_env.as_deref(), Some("MY_KEY"));
        assert_eq!(a.api_key_header.as_deref(), Some("X-Key"));
        assert_eq!(
            a.headers,
            vec![("X-Api-Key".to_string(), "secret".to_string())]
        );
        // legacy flat keys are read as-is (auth stays None) and stay flat on re-render
        let legacy = "[[mcp_servers]]\nname = \"old\"\ntransport = \"http\"\nurl = \"https://y/mcp\"\napi_key_env = \"OLD_KEY\"\napi_key_header = \"Authorization\"\napi_key_format = \"Bearer {token}\"\nheaders = { X-Api-Key = \"k\" }\n";
        let cfg3 = super::VibeConfigFile::from_raw(legacy);
        let s3 = &cfg3.mcp_servers()[0];
        assert!(s3.auth.is_none(), "legacy config must not gain an auth block");
        assert_eq!(s3.api_key_env.as_deref(), Some("OLD_KEY"));
        assert_eq!(
            s3.headers,
            vec![("X-Api-Key".to_string(), "k".to_string())]
        );
        let out3 = cfg3.render();
        assert!(
            !out3.contains("[mcp_servers.auth]"),
            "no auth header for legacy:\n{out3}"
        );
        assert!(out3.contains("OLD_KEY"));
        let cfg4 = super::VibeConfigFile::from_raw(&out3);
        let s4 = &cfg4.mcp_servers()[0];
        assert!(s4.auth.is_none());
        assert_eq!(s4.api_key_env.as_deref(), Some("OLD_KEY"));
    }

    #[test]
    fn mcp_stdio_fields_roundtrip() {
        let raw = "[[mcp_servers]]\nname = \"loc\"\ntransport = \"stdio\"\ncommand = \"uvx\"\nargs = [\"mcp-server\", \"--flag\"]\nenv = { DEBUG = \"1\" }\ncwd = \"/tmp/wd\"\nstartup_timeout_sec = 15\ntool_timeout_sec = 120\n";
        let cfg = super::VibeConfigFile::from_raw(raw);
        let s = &cfg.mcp_servers()[0];
        assert_eq!(s.command.as_deref(), Some("uvx"));
        assert_eq!(
            s.args,
            vec!["mcp-server".to_string(), "--flag".to_string()]
        );
        assert_eq!(s.env, vec![("DEBUG".to_string(), "1".to_string())]);
        assert_eq!(s.cwd.as_deref(), Some("/tmp/wd"));
        assert_eq!(s.startup_timeout_sec, Some(15));
        assert_eq!(s.tool_timeout_sec, Some(120));
        let cfg2 = super::VibeConfigFile::from_raw(&cfg.render());
        let s2 = &cfg2.mcp_servers()[0];
        assert_eq!(s2.cwd.as_deref(), Some("/tmp/wd"));
        assert_eq!(s2.startup_timeout_sec, Some(15));
        assert_eq!(s2.tool_timeout_sec, Some(120));
        assert_eq!(s2.args.len(), 2);
    }
}
