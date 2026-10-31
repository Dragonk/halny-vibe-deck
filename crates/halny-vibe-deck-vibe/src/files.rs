//! Agent profiles, prompts and skills stored under `~/.vibe/`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use toml::{Table, Value};

use crate::vibe_home;

fn agents_dir() -> PathBuf {
    vibe_home().join("agents")
}

fn prompts_dir() -> PathBuf {
    vibe_home().join("prompts")
}

fn skills_dir() -> PathBuf {
    vibe_home().join("skills")
}

/// A custom agent profile (`~/.vibe/agents/<name>.toml`).
#[derive(Clone, Debug, Default)]
pub struct AgentFile {
    pub name: String,
    /// `Some("subagent")` for subagents; `None` for top-level agents.
    pub agent_type: Option<String>,
    pub active_model: Option<String>,
    pub system_prompt_id: Option<String>,
    /// Human-friendly label shown in pickers (`display_name`).
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub enabled_tools: Vec<String>,
    pub disabled_tools: Vec<String>,
    /// `(tool_name, permission)` pairs from `[tools.<name>]` blocks.
    pub tool_permissions: Vec<(String, String)>,
    /// Approval safety profile (`safety = "…"`), stable parity.
    pub safety: Option<String>,
    /// Extra system instructions appended to the system prompt
    /// (`instructions` in the CLI's AgentProfile).
    pub instructions: Option<String>,
}

impl AgentFile {
    /// Load the global profiles (`~/.vibe/agents/*.toml`).
    pub fn load_all() -> Vec<AgentFile> {
        Self::load_all_in(&agents_dir())
    }

    /// Load profiles from any agents dir (e.g. the project-level
    /// `<cwd>/.vibe/agents/` — the source of the "project agents" that
    /// join the mode chip dropdown, Nexus `vibe:get-agents("project")`).
    pub fn load_all_in(dir: &std::path::Path) -> Vec<AgentFile> {
        let mut agents = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return agents;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(table) = content.parse::<Table>() else {
                continue;
            };
            agents.push(parse_agent(name, &table));
        }
        agents
    }

    pub fn save(&self) -> Result<()> {
        let dir = agents_dir();
        std::fs::create_dir_all(&dir)?;
        let table = agent_file_data(self);
        let path = dir.join(format!("{}.toml", self.name));
        std::fs::write(&path, toml::to_string_pretty(&table)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn delete(name: &str) -> Result<()> {
        let path = agents_dir().join(format!("{name}.toml"));
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("failed to delete {}", path.display()))?;
        }
        Ok(())
    }
}

/// TOML table for an agent profile (Nexus `agentFileData`): shared by the
/// local `AgentFile::save` and the remote agent writer. Empty values are
/// never written (the CLI rejects empty strings on these fields).
pub fn agent_file_data(a: &AgentFile) -> Table {
    let mut table = Table::new();
    if let Some(model) = &a.active_model {
        table.insert("active_model".into(), Value::from(model.as_str()));
    }
    if let Some(prompt) = &a.system_prompt_id {
        table.insert("system_prompt_id".into(), Value::from(prompt.as_str()));
    }
    if let Some(display_name) = &a.display_name {
        if !display_name.is_empty() {
            table.insert("display_name".into(), Value::from(display_name.as_str()));
        }
    }
    if let Some(description) = &a.description {
        if !description.is_empty() {
            table.insert("description".into(), Value::from(description.as_str()));
        }
    }
    if !a.enabled_tools.is_empty() {
        table.insert(
            "enabled_tools".into(),
            Value::Array(
                a.enabled_tools
                    .iter()
                    .map(|t| Value::from(t.as_str()))
                    .collect(),
            ),
        );
    }
    if !a.disabled_tools.is_empty() {
        table.insert(
            "disabled_tools".into(),
            Value::Array(
                a.disabled_tools
                    .iter()
                    .map(|t| Value::from(t.as_str()))
                    .collect(),
            ),
        );
    }
    if !a.tool_permissions.is_empty() {
        let mut tools = Table::new();
        for (tool, permission) in &a.tool_permissions {
            let mut t = Table::new();
            t.insert("permission".into(), Value::from(permission.as_str()));
            tools.insert(tool.clone(), Value::Table(t));
        }
        table.insert("tools".into(), Value::Table(tools));
    }
    // `agent_type` is written only for subagents (CLI convention).
    if a.agent_type.as_deref() == Some("subagent") {
        table.insert("agent_type".into(), Value::from("subagent"));
    }
    if let Some(safety) = &a.safety {
        if !safety.is_empty() {
            table.insert("safety".into(), Value::from(safety.as_str()));
        }
    }
    if let Some(instructions) = &a.instructions {
        if !instructions.is_empty() {
            table.insert("instructions".into(), Value::from(instructions.as_str()));
        }
    }
    table
}

/// Serialized agent profile TOML (remote writer, Nexus `stringify(data)`).
pub fn agent_file_toml(a: &AgentFile) -> String {
    toml::to_string_pretty(&agent_file_data(a)).unwrap_or_default()
}

/// Parse one agent profile body (remote `@@@F` blocks); `None` on invalid
/// TOML (the block is skipped).
pub fn parse_agent_content(name: &str, content: &str) -> Option<AgentFile> {
    let table: Table = content.parse().ok()?;
    Some(parse_agent(name, &table))
}

fn parse_agent(name: &str, table: &Table) -> AgentFile {
    let agent_type = table
        .get("agent_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let safety = table
        .get("safety")
        .and_then(Value::as_str)
        .map(str::to_string);
    let instructions = table
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut tool_permissions = Vec::new();
    if let Some(Value::Table(tools)) = table.get("tools") {
        for (tool, entry) in tools {
            if let Some(permission) = entry
                .as_table()
                .and_then(|t| t.get("permission"))
                .and_then(Value::as_str)
            {
                tool_permissions.push((tool.clone(), permission.to_string()));
            }
        }
    }
    AgentFile {
        name: name.to_string(),
        agent_type,
        safety,
        active_model: table
            .get("active_model")
            .and_then(Value::as_str)
            .map(str::to_string),
        system_prompt_id: table
            .get("system_prompt_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        display_name: table
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        description: table
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        enabled_tools: table
            .get("enabled_tools")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        disabled_tools: table
            .get("disabled_tools")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        tool_permissions,
        instructions,
    }
}

/// A custom system prompt (`~/.vibe/prompts/<name>.md`).
#[derive(Clone, Debug, Default)]
pub struct PromptFile {
    pub name: String,
    pub content: String,
}

impl PromptFile {
    pub fn load_all() -> Vec<PromptFile> {
        let dir = prompts_dir();
        let mut prompts = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return prompts;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            prompts.push(PromptFile {
                name: path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string(),
                content,
            });
        }
        prompts
    }

    pub fn save(&self) -> Result<()> {
        let dir = prompts_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.md", self.name));
        std::fs::write(&path, &self.content)?;
        Ok(())
    }

    pub fn delete(name: &str) -> Result<()> {
        let path = prompts_dir().join(format!("{name}.md"));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

/// A skill discovered under `~/.vibe/skills/`.
///
/// Skills follow the directory convention (`skills/<name>/SKILL.md`),
/// with a fallback to flat `.md` files for older setups.
#[derive(Clone, Debug, Default)]
pub struct SkillFile {
    pub name: String,
    pub content: String,
}

impl SkillFile {
    pub fn load_all() -> Vec<SkillFile> {
        let dir = skills_dir();
        let mut skills = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return skills;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.is_file() {
                    if let (Some(name), Ok(content)) = (
                        path.file_name().and_then(|s| s.to_str()),
                        std::fs::read_to_string(&skill_md),
                    ) {
                        skills.push(SkillFile {
                            name: name.to_string(),
                            content,
                        });
                    }
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let (Some(name), Ok(content)) = (
                    path.file_stem().and_then(|s| s.to_str()),
                    std::fs::read_to_string(&path),
                ) {
                    skills.push(SkillFile {
                        name: name.to_string(),
                        content,
                    });
                }
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agent_table() {
        let table: Table = r#"
agent_type = "subagent"
active_model = "mistral-small-latest"

[tools.bash]
permission = "ask"
"#
        .parse()
        .unwrap();
        let agent = parse_agent("reviewer", &table);
        assert_eq!(agent.name, "reviewer");
        assert_eq!(agent.agent_type.as_deref(), Some("subagent"));
        assert_eq!(agent.active_model.as_deref(), Some("mistral-small-latest"));
        assert_eq!(
            agent.tool_permissions,
            vec![("bash".to_string(), "ask".to_string())]
        );
    }
}
