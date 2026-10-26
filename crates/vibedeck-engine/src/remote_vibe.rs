//! Remote (SSH) vibe configuration — port of Nexus `remoteVibe.ts` (v0.8.1):
//! mirror the local `~/.vibe` configuration layer (config.toml, agents/,
//! prompts/, skills/ and config file listing) on a remote host over the
//! ssh pipe. The global scope targets `<home>/.vibe` (the `"$HOME"/.vibe`
//! marker expands on the remote host); an explicit dir (a remote project's
//! `<cwd>/.vibe`) scopes operations to a project on the remote host.
//!
//! All functions are blocking (they spawn subprocesses); call them from
//! `tokio::task::spawn_blocking` in server contexts.

use std::collections::HashMap;

use vibedeck_core::snapshot::{
    RemoteAgentFile, RemoteConfigFileEntry, RemoteConnection, RemotePromptFile, RemoteSkill,
    RemoteSkillFileEntry, ToolPermissionEntry,
};
use vibedeck_vibe::files::{agent_file_toml, parse_agent_content, AgentFile};
use vibedeck_vibe::validate_config_toml;

use crate::remote::{shq, ssh_run};

/// Global vibe dir marker — `"$HOME"` expands raw inside the remote command
/// (Nexus `HOME_VIBE`).
const HOME_VIBE: &str = "\"$HOME\"/.vibe";

/// Map the API scope arg to a remote dir: `None` (or empty) = the global
/// `~/.vibe` marker, `Some(dir)` = an explicit (project) dir.
pub fn scope_dir(dir: Option<&str>) -> String {
    match dir {
        Some(d) if !d.is_empty() => d.to_string(),
        _ => HOME_VIBE.to_string(),
    }
}

/// Shell-safe form of a vibe dir (Nexus `dirShell`): the global marker
/// stays raw (expands `$HOME` remotely); explicit project dirs are quoted.
fn dir_shell(dir: &str) -> String {
    if dir == HOME_VIBE {
        dir.to_string()
    } else {
        shq(dir)
    }
}

/// Base64-encode bytes for `base64 -d` on the remote stdin (standard
/// alphabet, padded; the engine has no base64 dependency — Nexus `toB64`).
fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Run a command, returning stdout on success and "" otherwise (Nexus `runSoft`).
fn run_soft(conn: &RemoteConnection, cmd: &str, timeout_ms: u64) -> String {
    let r = ssh_run(conn, cmd, timeout_ms, None);
    if r.code == 0 {
        r.stdout
    } else {
        String::new()
    }
}

/// Write a file remotely: `mkdir -p <dir> && base64 -d > <path>` with the
/// content on stdin (Nexus `remoteWrite`).
fn remote_write(conn: &RemoteConnection, dir: &str, path: &str, content: &str) -> Result<(), String> {
    let cmd = format!("mkdir -p {dir} && base64 -d > {path}");
    let r = ssh_run(conn, &cmd, 15_000, Some(&b64_encode(content.as_bytes())));
    if r.code != 0 {
        let e = r.stderr.trim();
        return Err(if e.is_empty() { "Remote write failed".to_string() } else { e.to_string() });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// config.toml
// ---------------------------------------------------------------------------

/// Raw remote config.toml content ("" when missing — Nexus serves the raw
/// tab and the Settings forms from this; normalization happens client-side).
pub fn get_remote_vibe_config(conn: &RemoteConnection, dir: &str) -> String {
    let cmd = format!("cat {}/config.toml 2>/dev/null", dir_shell(dir));
    run_soft(conn, &cmd, 15_000)
}

/// Checked remote config.toml read (the Settings forms + raw tab over SSH):
/// `Ok("")` when the file is missing, `Err` on transport or read failure.
pub fn get_remote_vibe_config_checked(
    conn: &RemoteConnection,
    dir: &str,
) -> Result<String, String> {
    let d = dir_shell(dir);
    let cmd = format!(
        "if [ -f {d}/config.toml ]; then cat {d}/config.toml; else printf __VD_NO_CONFIG__; fi"
    );
    let r = ssh_run(conn, &cmd, 15_000, None);
    if r.code != 0 {
        let e = r.stderr.trim();
        return Err(if e.is_empty() {
            "remote config read failed".to_string()
        } else {
            e.to_string()
        });
    }
    if r.stdout.trim() == "__VD_NO_CONFIG__" {
        Ok(String::new())
    } else {
        Ok(r.stdout)
    }
}

/// Validate + save raw remote config.toml (Nexus `saveRemoteRawConfig`):
/// `None` on success, `Some(error)` on invalid TOML or a failed write.
pub fn save_remote_raw_config(
    conn: &RemoteConnection,
    dir: &str,
    content: &str,
) -> Option<String> {
    if let Err(e) = validate_config_toml(content) {
        return Some(e);
    }
    let d = dir_shell(dir);
    remote_write(conn, &d, &format!("{d}/config.toml"), content).err()
}

/// The remote `~/.vibe` dir (Nexus `getRemoteVibeDir`).
pub fn get_remote_vibe_dir(conn: &RemoteConnection) -> String {
    let out = run_soft(conn, "echo \"$HOME\"", 15_000);
    let h = out.trim();
    if h.is_empty() {
        "~/.vibe".to_string()
    } else {
        format!("{h}/.vibe")
    }
}

// ---------------------------------------------------------------------------
// Agents (<dir>/agents/*.toml)
// ---------------------------------------------------------------------------

/// Agent file name: non `[a-zA-Z0-9_-]` chars become `_` (Nexus `agentFileName`).
fn agent_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    format!("{cleaned}.toml")
}

fn to_remote_agent(a: AgentFile) -> RemoteAgentFile {
    RemoteAgentFile {
        name: a.name,
        agent_type: a.agent_type,
        active_model: a.active_model,
        system_prompt_id: a.system_prompt_id,
        display_name: a.display_name,
        description: a.description,
        enabled_tools: a.enabled_tools,
        disabled_tools: a.disabled_tools,
        tool_permissions: a
            .tool_permissions
            .into_iter()
            .map(|(tool, permission)| ToolPermissionEntry { tool, permission })
            .collect(),
        content: String::new(),
    }
}

fn from_remote_agent(name: &str, a: &RemoteAgentFile) -> AgentFile {
    AgentFile {
        name: name.to_string(),
        agent_type: a.agent_type.clone(),
        active_model: a.active_model.clone(),
        system_prompt_id: a.system_prompt_id.clone(),
        display_name: a.display_name.clone(),
        description: a.description.clone(),
        enabled_tools: a.enabled_tools.clone(),
        disabled_tools: a.disabled_tools.clone(),
        tool_permissions: a
            .tool_permissions
            .iter()
            .map(|t| (t.tool.clone(), t.permission.clone()))
            .collect(),
    }
}

/// Parse `@@@F:<file>\n<toml>` blocks (Nexus `parseRemoteAgentBlocks`);
/// unparseable blocks are skipped.
fn parse_remote_agent_blocks(out: &str) -> Vec<RemoteAgentFile> {
    let mut agents = Vec::new();
    for block in out.split("@@@F:") {
        let Some(nl) = block.find('\n') else { continue };
        let file = block[..nl].trim();
        let body = &block[nl + 1..];
        let base = file.rsplit('/').next().unwrap_or(file);
        let stem = base.strip_suffix(".toml").unwrap_or(base);
        if let Some(a) = parse_agent_content(stem, body) {
            let mut r = to_remote_agent(a);
            r.content = body.trim_end().to_string();
            agents.push(r);
        }
    }
    agents
}

/// Global agents of a remote host (`<dir>/agents/*.toml`).
pub fn get_remote_agents(conn: &RemoteConnection, dir: &str) -> Vec<RemoteAgentFile> {
    let d = dir_shell(dir);
    let cmd = format!(
        "for f in {d}/agents/*.toml; do [ -f \"$f\" ] || continue; echo \"@@@F:$f\"; cat \"$f\"; echo; done"
    );
    parse_remote_agent_blocks(&run_soft(conn, &cmd, 15_000))
}

/// Project agents (`<cwd>/.vibe/agents`) on the remote host — the CLI only
/// reports global agents, but the mode picker offers these too (Nexus
/// `listRemoteProjectAgents`, v0.10.2).
pub fn list_remote_project_agents(conn: &RemoteConnection, cwd: &str) -> Vec<RemoteAgentFile> {
    get_remote_agents(conn, &format!("{cwd}/.vibe"))
}

/// Expose a remote project agent to the remote CLI's global agents dir
/// (`~/.vibe/agents`) so `set_config_option` accepts it (Nexus
/// `exposeRemoteProjectAgent`, v0.10.2; symlink with a copy fallback).
pub fn expose_remote_project_agent(
    conn: &RemoteConnection,
    cwd: &str,
    name: &str,
) -> Result<(), String> {
    let src = shq(&format!("{cwd}/.vibe/agents/{name}.toml"));
    let file = shq(&format!("{name}.toml"));
    let cmd = format!(
        "mkdir -p {HOME_VIBE}/agents && ln -sfn {src} {HOME_VIBE}/agents/{file} || cp {src} {HOME_VIBE}/agents/{file}"
    );
    let r = ssh_run(conn, &cmd, 15_000, None);
    if r.code != 0 {
        let e = r.stderr.trim();
        Err(if e.is_empty() { "Remote agent expose failed".to_string() } else { e.to_string() })
    } else {
        Ok(())
    }
}

/// Expose every agent of a remote project before a session is created there
/// (Nexus `exposeRemoteProjectAgents`, v0.10.2): prune dead symlinks, then
/// symlink each project agent into `~/.vibe/agents` (copy fallback, never
/// overwriting real files).
pub fn expose_remote_project_agents(conn: &RemoteConnection, cwd: &str) -> Result<(), String> {
    let parts = [
        format!("D={HOME_VIBE}/agents"),
        "mkdir -p \"$D\"".to_string(),
        "for l in \"$D\"/*; do [ -L \"$l\" ] && [ ! -e \"$l\" ] && rm -f \"$l\"; done".to_string(),
        format!(
            "for f in {}/.vibe/agents/*.toml; do [ -f \"$f\" ] || continue; b=$(basename \"$f\"); \
             [ -e \"$D\"/\"$b\" ] && [ ! -L \"$D\"/\"$b\" ] && continue; \
             ln -sfn \"$f\" \"$D\"/\"$b\" || cp \"$f\" \"$D\"/\"$b\"; done",
            shq(cwd)
        ),
    ];
    let cmd = parts.join("; ");
    let r = ssh_run(conn, &cmd, 15_000, None);
    if r.code != 0 {
        let e = r.stderr.trim();
        Err(if e.is_empty() {
            "Remote project agent expose failed".to_string()
        } else {
            e.to_string()
        })
    } else {
        Ok(())
    }
}

/// Save an agent profile remotely (Nexus `saveRemoteAgent`).
pub fn save_remote_agent(
    conn: &RemoteConnection,
    dir: &str,
    name: &str,
    agent: &RemoteAgentFile,
) -> Result<(), String> {
    let data = agent_file_toml(&from_remote_agent(name, agent));
    let d = dir_shell(dir);
    remote_write(conn, &format!("{d}/agents"), &format!("{d}/agents/{}", shq(&agent_file_name(name))), &data)
}

/// Save an agent profile from raw TOML (Settings raw editor over SSH): the
/// content is validated first, then written verbatim.
pub fn save_remote_agent_raw(
    conn: &RemoteConnection,
    dir: &str,
    name: &str,
    content: &str,
) -> Result<(), String> {
    if let Err(e) = validate_config_toml(content) {
        return Err(e);
    }
    let d = dir_shell(dir);
    remote_write(
        conn,
        &format!("{d}/agents"),
        &format!("{d}/agents/{}", shq(&agent_file_name(name))),
        content,
    )
}

/// Delete an agent profile remotely (Nexus `deleteRemoteAgent`; rm errors
/// are ignored, as in Nexus).
pub fn delete_remote_agent(conn: &RemoteConnection, dir: &str, name: &str) {
    let d = dir_shell(dir);
    let _ = run_soft(
        conn,
        &format!("rm -f {d}/agents/{}", shq(&agent_file_name(name))),
        15_000,
    );
}

// ---------------------------------------------------------------------------
// Prompts (<dir>/prompts/*)
// ---------------------------------------------------------------------------

/// Prompt file name: non `[a-zA-Z0-9_-]` chars become `_` (Nexus `promptFileName`).
fn prompt_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    format!("{cleaned}.md")
}

/// Parse `@@@P:<file>\n<content>@@@E` blocks; `.md` and `.txt` names are
/// stripped of their extension (Nexus `getRemotePrompts`).
fn parse_prompt_blocks(out: &str) -> Vec<RemotePromptFile> {
    let mut prompts = Vec::new();
    for block in out.split("@@@P:") {
        let Some(nl) = block.find('\n') else { continue };
        let file = block[..nl].trim();
        let rest = &block[nl + 1..];
        let content = match rest.rfind("@@@E") {
            Some(e) => &rest[..e],
            None => rest,
        };
        let base = file.rsplit('/').next().unwrap_or(file);
        let name = base.strip_suffix(".md").or_else(|| base.strip_suffix(".txt"));
        if let Some(name) = name {
            prompts.push(RemotePromptFile {
                name: name.to_string(),
                content: content.to_string(),
            });
        }
    }
    prompts
}

/// Global prompts of a remote host (`<dir>/prompts/*`).
pub fn get_remote_prompts(conn: &RemoteConnection, dir: &str) -> Vec<RemotePromptFile> {
    let d = dir_shell(dir);
    let cmd = format!(
        "for f in {d}/prompts/*; do [ -f \"$f\" ] || continue; echo \"@@@P:$f\"; cat \"$f\"; echo \"@@@E\"; done"
    );
    parse_prompt_blocks(&run_soft(conn, &cmd, 15_000))
}

/// Save a prompt remotely (Nexus `saveRemotePrompt`).
pub fn save_remote_prompt(
    conn: &RemoteConnection,
    dir: &str,
    name: &str,
    content: &str,
) -> Result<(), String> {
    let d = dir_shell(dir);
    remote_write(conn, &format!("{d}/prompts"), &format!("{d}/prompts/{}", shq(&prompt_file_name(name))), content)
}

/// Delete a prompt remotely (rm errors ignored, as in Nexus).
pub fn delete_remote_prompt(conn: &RemoteConnection, dir: &str, name: &str) {
    let d = dir_shell(dir);
    let _ = run_soft(
        conn,
        &format!("rm -f {d}/prompts/{}", shq(&prompt_file_name(name))),
        15_000,
    );
}

// ---------------------------------------------------------------------------
// Config file listing (raw tab tree)
// ---------------------------------------------------------------------------

/// Parse the `@@@H<home>` + `find` output into (path, full) entries.
fn parse_remote_config_files(out: &str, dir: &str) -> Vec<RemoteConfigFileEntry> {
    let Some(rest) = out.strip_prefix("@@@H") else { return Vec::new() };
    let Some(nl) = rest.find('\n') else { return Vec::new() };
    let home = &rest[..nl];
    let root = if dir == HOME_VIBE { format!("{home}/.vibe") } else { dir.to_string() };
    let mut files = Vec::new();
    for line in rest[nl + 1..].lines() {
        let rel = line.trim();
        if rel.is_empty() {
            continue;
        }
        let clean = rel.strip_prefix("./").unwrap_or(rel);
        files.push(RemoteConfigFileEntry {
            path: clean.to_string(),
            full: format!("{root}/{clean}"),
        });
    }
    files
}

/// List remote config files for the raw tab tree (Nexus `listRemoteConfigFiles`):
/// maxdepth 3, skipping logs/sessions/node_modules/.git/tmp.
pub fn list_remote_config_files(conn: &RemoteConnection, dir: &str) -> Vec<RemoteConfigFileEntry> {
    let d = dir_shell(dir);
    let cmd = [
        "printf \"@@@H%s\" \"$HOME\"".to_string(),
        format!(
            "cd {d} 2>/dev/null && \
             find . -maxdepth 3 -type f ! -path \"./logs*\" ! -path \"./sessions*\" \
             ! -path \"./node_modules*\" ! -path \"./.git*\" ! -path \"./tmp*\" | sort"
        ),
    ]
    .join(";");
    parse_remote_config_files(&run_soft(conn, &cmd, 15_000), dir)
}

// ---------------------------------------------------------------------------
// Skills (<dir>/skills/<name>/SKILL.md)
// ---------------------------------------------------------------------------

/// Skill dir name: keep `[a-zA-Z0-9-_ .]`, strip leading dots, trim (Nexus
/// `sanitizeSkillName`).
fn sanitize_skill_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ' | '.'))
        .collect();
    while s.starts_with('.') {
        s.remove(0);
    }
    s.trim().to_string()
}

/// Skill-relative path: drop `..` segments, strip leading slashes (Nexus
/// `sanitizeSkillRel`).
fn sanitize_skill_rel(rel: &str) -> String {
    rel.replace("..", "").trim_start_matches('/').to_string()
}

/// Parse the skills listing output (`@@@H` + tree lines + `@@@S` contents,
/// Nexus `getRemoteSkills`): one entry per top-level dir, SKILL.md content
/// attached, files sorted dirs-first then by path.
fn parse_remote_skills(out: &str, dir: &str) -> Vec<RemoteSkill> {
    let Some(rest) = out.strip_prefix("@@@H") else { return Vec::new() };
    let Some(nl) = rest.find('\n') else { return Vec::new() };
    let home = &rest[..nl];
    let root = if dir == HOME_VIBE { format!("{home}/.vibe") } else { dir.to_string() };
    let body = &rest[nl + 1..];
    let list_part = body.split("@@@S:").next().unwrap_or("");
    let mut contents: HashMap<String, String> = HashMap::new();
    for block in body.split("@@@S:").skip(1) {
        let Some(nl2) = block.find('\n') else { continue };
        let path = block[..nl2].trim().to_string();
        let rest2 = &block[nl2 + 1..];
        let content = match rest2.rfind("@@@E") {
            Some(e) => rest2[..e].trim_end().to_string(),
            None => rest2.trim_end().to_string(),
        };
        contents.insert(path, content);
    }
    let mut skills: Vec<RemoteSkill> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for line in list_part.lines() {
        let (is_dir, rel) = if let Some(r) = line.strip_prefix("D ") {
            (true, r)
        } else if let Some(r) = line.strip_prefix("F ") {
            (false, r)
        } else {
            continue;
        };
        let segs: Vec<&str> = rel.split('/').collect();
        if segs.len() < 2 || segs[0].is_empty() {
            continue;
        }
        let name = segs[0].to_string();
        let idx = match index.get(&name) {
            Some(i) => *i,
            None => {
                let content = contents.get(&format!("{name}/SKILL.md")).cloned().unwrap_or_default();
                skills.push(RemoteSkill {
                    name: name.clone(),
                    content,
                    dir: format!("{root}/skills/{name}"),
                    files: Vec::new(),
                });
                index.insert(name.clone(), skills.len() - 1);
                skills.len() - 1
            }
        };
        skills[idx].files.push(RemoteSkillFileEntry {
            path: segs[1..].join("/"),
            full: format!("{root}/skills/{rel}"),
            is_dir,
        });
    }
    for s in &mut skills {
        s.files.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.path.cmp(&b.path)));
    }
    skills
}

/// Remote skills of a scope: tree + SKILL.md contents (Nexus `getRemoteSkills`).
pub fn get_remote_skills(conn: &RemoteConnection, dir: &str) -> Vec<RemoteSkill> {
    let d = dir_shell(dir);
    let cmd = [
        "printf \"@@@H%s\" \"$HOME\"".to_string(),
        format!("cd {d}/skills 2>/dev/null || exit 0"),
        "find . -maxdepth 4 | sort | while read -r p; do q=${{p#./}}; \
         if [ -d \"$p\" ]; then echo \"D $q\"; else echo \"F $q\"; fi; done"
            .to_string(),
        "for f in */SKILL.md; do [ -f \"$f\" ] || continue; echo \"@@@S:$f\"; cat \"$f\"; echo \"@@@E\"; done"
            .to_string(),
    ]
    .join("; ");
    parse_remote_skills(&run_soft(conn, &cmd, 20_000), dir)
}

/// Create a skill with a starter SKILL.md (Nexus `createRemoteSkill`):
/// `None` on success, `Some(error)` otherwise.
pub fn create_remote_skill(conn: &RemoteConnection, dir: &str, name: &str) -> Option<String> {
    let clean = sanitize_skill_name(name);
    if clean.is_empty() {
        return Some("Invalid skill name".to_string());
    }
    let d = dir_shell(dir);
    let exists = run_soft(
        conn,
        &format!("test -d {d}/skills/{} && echo yes", shq(&clean)),
        15_000,
    );
    if !exists.trim().is_empty() {
        return Some("A skill with this name already exists".to_string());
    }
    let starter = format!("---\nname: {clean}\ndescription: TODO\n---\n\n# {clean}\n");
    let base = format!("{d}/skills/{}", shq(&clean));
    remote_write(conn, &base, &format!("{base}/SKILL.md"), &starter).err()
}

/// Delete a skill dir remotely (Nexus `deleteRemoteSkill`).
pub fn delete_remote_skill(conn: &RemoteConnection, dir: &str, name: &str) -> Option<String> {
    let clean = sanitize_skill_name(name);
    if clean.is_empty() {
        return Some("Invalid skill name".to_string());
    }
    let d = dir_shell(dir);
    let _ = run_soft(
        conn,
        &format!("rm -rf {d}/skills/{}", shq(&clean)),
        15_000,
    );
    None
}

/// Write a file inside a remote skill dir (Nexus `writeRemoteSkillFile`).
pub fn write_remote_skill_file(
    conn: &RemoteConnection,
    dir: &str,
    name: &str,
    rel_path: &str,
    content: &str,
) -> Option<String> {
    let clean = sanitize_skill_name(name);
    if clean.is_empty() {
        return Some("Invalid skill name".to_string());
    }
    let rel = sanitize_skill_rel(rel_path);
    if rel.is_empty() {
        return Some("Invalid file path".to_string());
    }
    let d = dir_shell(dir);
    let skill_dir = format!("{d}/skills/{}", shq(&clean));
    let target = format!("{d}/skills/{}", shq(&format!("{clean}/{rel}")));
    remote_write(conn, &skill_dir, &target, content).err()
}

/// Delete a file inside a remote skill dir (Nexus `deleteRemoteSkillFile`);
/// SKILL.md itself is protected.
pub fn delete_remote_skill_file(
    conn: &RemoteConnection,
    dir: &str,
    name: &str,
    rel_path: &str,
) -> Option<String> {
    let clean = sanitize_skill_name(name);
    if clean.is_empty() {
        return Some("Invalid skill name".to_string());
    }
    let rel = sanitize_skill_rel(rel_path);
    if rel.is_empty() {
        return Some("Invalid file path".to_string());
    }
    if rel.eq_ignore_ascii_case("skill.md") {
        return Some("SKILL.md cannot be removed — delete the skill instead".to_string());
    }
    let d = dir_shell(dir);
    let _ = run_soft(
        conn,
        &format!("rm -rf {d}/skills/{}", shq(&format!("{clean}/{rel}"))),
        15_000,
    );
    None
}

// Importing skills is not supported for remote connections (Nexus
// `importRemoteSkill` returns a fixed error; the UI surfaces it directly).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_encode_basic() {
        assert_eq!(b64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(b64_encode(b"hi"), "aGk=");
        assert_eq!(b64_encode(b"abcde"), "YWJjZGU=");
        assert_eq!(b64_encode(b""), "");
    }

    #[test]
    fn agent_and_prompt_file_names() {
        assert_eq!(agent_file_name("my agent!"), "my_agent_.toml");
        assert_eq!(agent_file_name("a/b c"), "a_b_c.toml");
        assert_eq!(prompt_file_name("hello world"), "hello_world.md");
    }

    #[test]
    fn agent_blocks_parse() {
        let out = "@@@F:/home/u/.vibe/agents/reviewer.toml\nagent_type = \"subagent\"\nactive_model = \"mistral-small-latest\"\n\n[tools.bash]\npermission = \"ask\"\n\n@@@F:/home/u/.vibe/agents/broken.toml\nnot toml ][\n";
        let agents = parse_remote_agent_blocks(out);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "reviewer");
        assert_eq!(agents[0].agent_type.as_deref(), Some("subagent"));
        assert_eq!(agents[0].active_model.as_deref(), Some("mistral-small-latest"));
        assert_eq!(agents[0].tool_permissions.len(), 1);
        assert_eq!(agents[0].tool_permissions[0].tool, "bash");
        assert_eq!(agents[0].tool_permissions[0].permission, "ask");
    }

    #[test]
    fn prompt_blocks_parse() {
        let out = "@@@P:/home/u/.vibe/prompts/hello.md\nHello world\n@@@E\n@@@P:/home/u/.vibe/prompts/notes.txt\n- line\n@@@E\n@@@P:/home/u/.vibe/prompts/skip.yaml\nx\n@@@E\n";
        let prompts = parse_prompt_blocks(out);
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].name, "hello");
        assert_eq!(prompts[0].content, "Hello world\n");
        assert_eq!(prompts[1].name, "notes");
    }

    #[test]
    fn skill_name_and_rel_sanitize() {
        assert_eq!(sanitize_skill_name("..My Skill"), "My Skill");
        assert_eq!(sanitize_skill_name("a/b"), "ab");
        assert_eq!(sanitize_skill_rel("../x/y"), "x/y");
        assert_eq!(sanitize_skill_rel("a/../b"), "a//b");
    }

    #[test]
    fn skills_parse() {
        let out = "@@@H/home/u\nD alpha\nF alpha/SKILL.md\nF alpha/extra.md\n@@@S:alpha/SKILL.md\n# Alpha\n@@@E\n";
        let skills = parse_remote_skills(out, HOME_VIBE);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "alpha");
        assert_eq!(skills[0].content, "# Alpha");
        assert_eq!(skills[0].dir, "/home/u/.vibe/skills/alpha");
        // "D alpha" itself is skipped (segs.len() < 2, Nexus rule) - only
        // entries INSIDE the skill dir become file entries.
        assert_eq!(skills[0].files.len(), 2);
        assert!(!skills[0].files[0].is_dir);
        assert_eq!(skills[0].files[0].path, "SKILL.md");
        assert_eq!(skills[0].files[1].path, "extra.md");
        assert_eq!(skills[0].files[1].full, "/home/u/.vibe/skills/alpha/extra.md");
        // No @@@H marker -> empty.
        assert!(parse_remote_skills("nothing", HOME_VIBE).is_empty());
    }

    #[test]
    fn config_files_parse() {
        let out = "@@@H/home/u\n./agents/x.toml\nprompts/hello.md\n";
        let files = parse_remote_config_files(out, HOME_VIBE);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "agents/x.toml");
        assert_eq!(files[0].full, "/home/u/.vibe/agents/x.toml");
        assert_eq!(files[1].full, "/home/u/.vibe/prompts/hello.md");
        // Home echo without a newline (cd failed) -> empty.
        assert!(parse_remote_config_files("@@@H/home/u", HOME_VIBE).is_empty());
    }
}
