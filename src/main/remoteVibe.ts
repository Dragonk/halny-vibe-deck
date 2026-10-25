// v0.8.1: remote (SSH) vibe configuration. Mirrors the vibeConfig.ts
// operations for the remote settings scope, executing them over sshRun
// on the configured remote connection. Paths target "<home>/.vibe".
import { loadConfig, type RemoteConnection } from "./config";
import { shq, sshRun } from "./remote";
import { parse, stringify } from "smol-toml";
import { agentFileData, type VibeAgentFileConfig } from "./vibeConfig";
import { buildSessionPreview, type SessionPreview } from "./vibeStats";
import type {
  VibeConfig,
  VibePrompt,
  VibeSkill,
} from "./vibeConfig";

const NL = String.fromCharCode(10);
const HOME_VIBE = '"$HOME"/.vibe';

async function connOrThrow(remoteId: string): Promise<RemoteConnection> {
  const conn = (loadConfig().remoteConnections ?? []).find(
    (c) => c.id === remoteId && c.enabled,
  );
  if (!conn) throw new Error("Remote connection not found or disabled");
  return conn;
}

async function runSoft(
  remoteId: string,
  cmd: string,
  timeoutMs = 15000,
  stdin?: string,
): Promise<string> {
  const conn = await connOrThrow(remoteId);
  const r = await sshRun(conn, cmd, timeoutMs, stdin);
  return r.code === 0 ? r.stdout : "";
}

function toB64(s: string): string {
  return Buffer.from(s, "utf-8").toString("base64");
}

async function remoteWrite(
  remoteId: string,
  dir: string,
  path: string,
  content: string,
): Promise<void> {
  const conn = await connOrThrow(remoteId);
  const r = await sshRun(
    conn,
    "mkdir -p " + dir + " && base64 -d > " + path,
    15000,
    toB64(content),
  );
  if (r.code !== 0) throw new Error(r.stderr || "Remote write failed");
}

// ---------------------------------------------------------------------------
// config.toml
// ---------------------------------------------------------------------------

export async function getRemoteVibeConfig(remoteId: string): Promise<VibeConfig> {
  const out = await runSoft(remoteId, "cat " + HOME_VIBE + "/config.toml 2>/dev/null");
  if (!out.trim()) return {};
  try {
    return parse(out) as VibeConfig;
  } catch {
    return {};
  }
}

export async function saveRemoteVibeConfig(config: VibeConfig, remoteId: string): Promise<void> {
  await remoteWrite(
    remoteId,
    HOME_VIBE,
    HOME_VIBE + "/config.toml",
    stringify(config as Record<string, unknown>),
  );
}

export async function getRemoteRawConfig(
  remoteId: string,
): Promise<{ content: string; error: string | null }> {
  const out = await runSoft(remoteId, "cat " + HOME_VIBE + "/config.toml 2>/dev/null");
  return { content: out, error: null };
}

export async function saveRemoteRawConfig(content: string, remoteId: string): Promise<string | null> {
  try {
    const parsed = parse(content);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return "config.toml must be a TOML table";
    }
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
  await remoteWrite(remoteId, HOME_VIBE, HOME_VIBE + "/config.toml", content);
  return null;
}

export async function getRemoteVibeDir(remoteId: string): Promise<string> {
  const out = await runSoft(remoteId, "echo \"$HOME\"");
  return (out.trim() || "~") + "/.vibe";
}

// ---------------------------------------------------------------------------
// Agents (<dir>/agents/*.toml)
// ---------------------------------------------------------------------------

function agentFileName(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "_") + ".toml";
}

export async function getRemoteAgents(remoteId: string): Promise<VibeAgentFileConfig[]> {
  const out = await runSoft(
    remoteId,
    "for f in \"$HOME\"/.vibe/agents/*.toml; do [ -f \"$f\" ] || continue; " +
      "echo \"@@@F:$f\"; cat \"$f\"; echo; done",
  );
  const agents: VibeAgentFileConfig[] = [];
  for (const block of out.split("@@@F:")) {
    const head = block.indexOf(NL);
    if (head < 0) continue;
    const file = block.slice(0, head).trim();
    const body = block.slice(head + 1);
    let raw: Record<string, unknown>;
    try {
      raw = parse(body) as Record<string, unknown>;
    } catch {
      continue;
    }
    const base = file.split("/").pop() ?? file;
    const stem = base.endsWith(".toml") ? base.slice(0, -5) : base;
    const rawTools = (raw["tools"] as Record<string, unknown> | undefined) ?? {};
    const tools: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(rawTools)) {
      tools["tools." + k] = v;
    }
    agents.push({
      name: stem,
      agent_type: raw["agent_type"] as VibeAgentFileConfig["agent_type"],
      active_model: raw["active_model"] as string | undefined,
      system_prompt_id: raw["system_prompt_id"] as string | undefined,
      display_name: raw["display_name"] as string | undefined,
      description: raw["description"] as string | undefined,
      instructions: raw["instructions"] as string | undefined,
      tools,
      disabled_tools: raw["disabled_tools"] as string[] | undefined,
      enabled_tools: raw["enabled_tools"] as string[] | undefined,
    });
  }
  return agents;
}

export async function saveRemoteAgent(
  name: string,
  config: VibeAgentFileConfig,
  remoteId: string,
): Promise<void> {
  const data = agentFileData(config);
  await remoteWrite(
    remoteId,
    HOME_VIBE + "/agents",
    HOME_VIBE + "/agents/" + shq(agentFileName(name)),
    stringify(data),
  );
}

export async function deleteRemoteAgent(name: string, remoteId: string): Promise<void> {
  await runSoft(
    remoteId,
    "rm -f \"$HOME\"/.vibe/agents/" + shq(agentFileName(name)),
  );
}

// ---------------------------------------------------------------------------
// Prompts (<dir>/prompts/*.md)
// ---------------------------------------------------------------------------

function promptFileName(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "_") + ".md";
}

export async function getRemotePrompts(remoteId: string): Promise<VibePrompt[]> {
  const out = await runSoft(
    remoteId,
    "for f in \"$HOME\"/.vibe/prompts/*; do [ -f \"$f\" ] || continue; " +
      "echo \"@@@P:$f\"; cat \"$f\"; echo \"@@@E\"; done",
  );
  const prompts: VibePrompt[] = [];
  for (const block of out.split("@@@P:")) {
    const head = block.indexOf(NL);
    if (head < 0) continue;
    const file = block.slice(0, head).trim();
    const rest = block.slice(head + 1);
    const end = rest.lastIndexOf("@@@E");
    const content = end >= 0 ? rest.slice(0, end) : rest;
    const base = file.split("/").pop() ?? file;
    if (base.endsWith(".md")) {
      prompts.push({ name: base.slice(0, -3), content });
    } else if (base.endsWith(".txt")) {
      prompts.push({ name: base.slice(0, -4), content });
    }
  }
  return prompts;
}

export async function saveRemotePrompt(name: string, content: string, remoteId: string): Promise<void> {
  await remoteWrite(
    remoteId,
    HOME_VIBE + "/prompts",
    HOME_VIBE + "/prompts/" + shq(promptFileName(name)),
    content,
  );
}

export async function deleteRemotePrompt(name: string, remoteId: string): Promise<void> {
  await runSoft(
    remoteId,
    "rm -f \"$HOME\"/.vibe/prompts/" + shq(promptFileName(name)),
  );
}

// ---------------------------------------------------------------------------
// Config file listing (raw tab tree)
// ---------------------------------------------------------------------------

export async function listRemoteConfigFiles(
  remoteId: string,
): Promise<Array<{ path: string; full: string }>> {
  const out = await runSoft(
    remoteId,
    "printf \"@@@H%s\" \"$HOME\"; cd \"$HOME\"/.vibe 2>/dev/null && " +
      "find . -maxdepth 3 -type f ! -path \"./logs*\" ! -path \"./sessions*\" " +
      "! -path \"./node_modules*\" ! -path \"./.git*\" ! -path \"./tmp*\" | sort",
  );
  if (!out.startsWith("@@@H")) return [];
  const nlIdx = out.indexOf(NL);
  if (nlIdx < 0) return [];
  const home = out.slice(4, nlIdx);
  const files: Array<{ path: string; full: string }> = [];
  for (const line of out.slice(nlIdx + 1).split(NL)) {
    const rel = line.trim();
    if (!rel) continue;
    const cleanRel = rel.startsWith("./") ? rel.slice(2) : rel;
    files.push({ path: cleanRel, full: home + "/.vibe/" + cleanRel });
  }
  return files;
}

// ---------------------------------------------------------------------------
// Skills (<dir>/skills/<name>/SKILL.md)
// ---------------------------------------------------------------------------

function sanitizeSkillName(name: string): string {
  return name.replace(/[^a-zA-Z0-9-_ .]/g, "").replace(/^[.]+/, "").trim();
}

function sanitizeSkillRel(relPath: string): string {
  return relPath.split("..").join("").replace(/^[/]+/, "");
}

export async function getRemoteSkills(remoteId: string): Promise<VibeSkill[]> {
  const cmd =
    "printf \"@@@H%s\" \"$HOME\"; cd \"$HOME\"/.vibe/skills 2>/dev/null || exit 0; " +
    "find . -maxdepth 4 | sort | while read -r p; do q=${p#./}; " +
    "if [ -d \"$p\" ]; then echo \"D $q\"; else echo \"F $q\"; fi; done; " +
    "for f in */SKILL.md; do [ -f \"$f\" ] || continue; echo \"@@@S:$f\"; " +
    "cat \"$f\"; echo \"@@@E\"; done";
  const out = await runSoft(remoteId, cmd, 20000);
  if (!out.startsWith("@@@H")) return [];
  const nlIdx = out.indexOf(NL);
  if (nlIdx < 0) return [];
  const home = out.slice(4, nlIdx);
  const body = out.slice(nlIdx + 1);
  const listPart = body.split("@@@S:")[0];
  const contents = new Map<string, string>();
  for (const block of body.split("@@@S:")) {
    const head = block.indexOf(NL);
    if (head < 0) continue;
    const path = block.slice(0, head).trim();
    const rest = block.slice(head + 1);
    const end = rest.lastIndexOf("@@@E");
    contents.set(path, end >= 0 ? rest.slice(0, end).trimEnd() : rest.trimEnd());
  }
  const skills = new Map<string, VibeSkill>();
  for (const line of listPart.split(NL)) {
    let isDir = false;
    let rel = "";
    if (line.startsWith("D ")) {
      isDir = true;
      rel = line.slice(2);
    } else if (line.startsWith("F ")) {
      rel = line.slice(2);
    } else {
      continue;
    }
    const segs = rel.split("/");
    if (segs.length < 2 || !segs[0]) continue;
    const name = segs[0];
    if (!skills.has(name)) {
      skills.set(name, {
        name,
        content: contents.get(name + "/SKILL.md") ?? "",
        dir: home + "/.vibe/skills/" + name,
        files: [],
      });
    }
    const skill = skills.get(name)!;
    skill.files.push({
      path: segs.slice(1).join("/"),
      full: home + "/.vibe/skills/" + rel,
      isDir,
    });
  }
  for (const s of skills.values()) {
    s.files.sort((a, b) =>
      Number(b.isDir) - Number(a.isDir) ||
      (a.path < b.path ? -1 : a.path > b.path ? 1 : 0),
    );
  }
  return [...skills.values()];
}

export async function createRemoteSkill(name: string, remoteId: string): Promise<string | null> {
  const clean = sanitizeSkillName(name);
  if (!clean) return "Invalid skill name";
  const exists = await runSoft(
    remoteId,
    "test -d \"$HOME\"/.vibe/skills/" + shq(clean) + " && echo yes",
  );
  if (exists.trim()) return "A skill with this name already exists";
  const starter =
    "---" + NL +
    "name: " + clean + NL +
    "description: TODO" + NL +
    "---" + NL + NL +
    "# " + clean + NL;
  await remoteWrite(
    remoteId,
    "\"$HOME\"/.vibe/skills/" + shq(clean),
    "\"$HOME\"/.vibe/skills/" + shq(clean) + "/SKILL.md",
    starter,
  );
  return null;
}

export async function deleteRemoteSkill(name: string, remoteId: string): Promise<string | null> {
  const clean = sanitizeSkillName(name);
  if (!clean) return "Invalid skill name";
  await runSoft(
    remoteId,
    "rm -rf \"$HOME\"/.vibe/skills/" + shq(clean),
  );
  return null;
}

export async function writeRemoteSkillFile(
  name: string,
  relPath: string,
  content: string,
  remoteId: string,
): Promise<string | null> {
  const clean = sanitizeSkillName(name);
  if (!clean) return "Invalid skill name";
  const rel = sanitizeSkillRel(relPath);
  if (!rel) return "Invalid file path";
  const target = clean + "/" + rel;
  await remoteWrite(
    remoteId,
    "\"$HOME\"/.vibe/skills/" + shq(clean),
    "\"$HOME\"/.vibe/skills/" + shq(target),
    content,
  );
  return null;
}

export async function deleteRemoteSkillFile(
  name: string,
  relPath: string,
  remoteId: string,
): Promise<string | null> {
  const clean = sanitizeSkillName(name);
  if (!clean) return "Invalid skill name";
  const rel = sanitizeSkillRel(relPath);
  if (!rel) return "Invalid file path";
  if (rel.toLowerCase() === "skill.md") {
    return "SKILL.md cannot be removed \u2014 delete the skill instead";
  }
  await runSoft(
    remoteId,
    "rm -rf \"$HOME\"/.vibe/skills/" + shq(clean + "/" + rel),
  );
  return null;
}

export async function importRemoteSkill(): Promise<string | null> {
  return "Importing skills is not supported for remote connections";
}

// v0.9.0: lazy session preview over SSH - reads the unified session store on
// the remote host without starting vibe-acp there (lazy resume).
export async function loadRemoteSessionPreview(
  remoteId: string,
  sessionId: string,
): Promise<SessionPreview | null> {
  const sid = shq(sessionId);
  const L = String.fromCharCode(10);
  const phase1 =
    'D="$HOME"/.vibe/logs/session/unified/' + sid + L +
    'if [ ! -f "$D/CURRENT" ]; then' + L +
    '  for d in "$HOME"/.vibe/logs/session/unified/*/; do' + L +
    '    if [ -f "$d/CURRENT" ] && grep -q ' + sid + ' "$d/meta.json" 2>/dev/null; then D="${d%/}"; break; fi' + L +
    '  done' + L +
    'fi' + L +
    'if [ ! -f "$D/CURRENT" ]; then echo "@@@NONE"; exit 0; fi' + L +
    'echo "@@@DIR:$D"' + L +
    'GEN=$(ls -1 "$D/generations" 2>/dev/null | sort -n | tail -1)' + L +
    'if [ -z "$GEN" ] || [ ! -f "$D/generations/$GEN/manifest.json" ]; then echo "@@@NONE"; exit 0; fi' + L +
    'echo "@@@MANIFEST"' + L +
    'cat "$D/generations/$GEN/manifest.json"' + L +
    'echo' + L +
    'echo "@@@INDEX"' + L +
    'cat "$HOME"/.vibe/logs/session/unified/.session-index.json 2>/dev/null';
  const out = await runSoft(remoteId, phase1, 25000);
  if (!out || out.indexOf('@@@NONE') >= 0) return null;
  const dirPos = out.indexOf('@@@DIR:');
  const manifestPos = out.indexOf('@@@MANIFEST');
  const indexPos = out.indexOf('@@@INDEX');
  if (dirPos < 0 || manifestPos < 0 || indexPos < 0) return null;
  const dirLine = out.slice(dirPos + 7);
  const dir = dirLine.slice(0, dirLine.indexOf(L)).trim();
  let manifest: Record<string, unknown>;
  try {
    manifest = JSON.parse(out.slice(manifestPos + 11, indexPos).trim()) as Record<string, unknown>;
  } catch {
    return null;
  }
  const ps = (manifest['projection_state'] as Record<string, unknown> | undefined) ?? {};
  const shasAll = Array.isArray(ps['chunks']) ? (ps['chunks'] as string[]) : [];
  const shas = shasAll.length > 48 ? shasAll.slice(shasAll.length - 48) : shasAll;
  if (shas.length === 0 || !dir) return null;
  const cmds: string[] = [];
  for (const sha of shas) {
    if (!/^[0-9a-f]+$/.test(sha)) continue;
    cmds.push('echo "@@@CHUNK:' + sha + '"; cat ' + shq(dir + '/chunks/' + sha + '.json') + '; echo');
  }
  const out2 = await runSoft(remoteId, cmds.join(L), 60000);
  const entries: Record<string, unknown>[] = [];
  for (const block of out2.split('@@@CHUNK:')) {
    const nl = block.indexOf(L);
    if (nl < 0) continue;
    try {
      const data = JSON.parse(block.slice(nl + 1).trim()) as unknown;
      if (Array.isArray(data)) entries.push(...(data as Record<string, unknown>[]));
    } catch {}
  }
  let indexEntry: Record<string, unknown> | null = null;
  try {
    const idxRaw = out.slice(indexPos + 9).trim();
    const idx = idxRaw ? (JSON.parse(idxRaw) as Record<string, unknown>) : null;
    const ents = idx && Array.isArray(idx['entries']) ? (idx['entries'] as Array<Record<string, unknown>>) : [];
    indexEntry =
      ents.find((e) => {
        const ident = (e['identity'] as Record<string, unknown> | undefined) ?? {};
        return ident['session_id'] === sessionId || e['session_id'] === sessionId;
      }) ?? null;
  } catch {}
  if (entries.length === 0 && !indexEntry) return null;
  return buildSessionPreview(entries, indexEntry);
}
