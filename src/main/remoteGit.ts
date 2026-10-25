import { sshRun, shq } from "./remote";
import type { RemoteConnection } from "./config";
import type { GitStatus, GitCommit } from "./git";

// v0.7.0: remote git over ssh - parses plain git output into the exact
// shapes the local simple-git handlers produce (GitStatus / GitCommit).

interface GitOut {
  code: number;
  out: string;
  err: string;
}

async function gitOut(conn: RemoteConnection, cwd: string, args: string, timeoutMs = 30000): Promise<GitOut> {
  const res = await sshRun(conn, "git -C " + shq(cwd) + " " + args + " 2>&1", timeoutMs);
  return { code: res.code, out: res.stdout, err: res.stderr };
}

async function isRepo(conn: RemoteConnection, cwd: string): Promise<boolean> {
  const chk = await sshRun(
    conn,
    "git -C " + shq(cwd) + " rev-parse --is-inside-work-tree 2>/dev/null",
    15000,
  );
  return chk.code === 0 && chk.stdout.trim() === "true";
}

export async function remoteGitStatus(conn: RemoteConnection, cwd: string): Promise<GitStatus> {
  const result: GitStatus = { repo: false, branch: null, ahead: 0, behind: 0, files: [] };
  if (!cwd) return result;
  try {
    if (!(await isRepo(conn, cwd))) return result;
    result.repo = true;
    const res = await gitOut(conn, cwd, "status --porcelain=v1 -z --branch");
    if (res.code !== 0) {
      result.error = res.out.trim() || "git status failed";
      return result;
    }
    const records = res.out.split("\x00").filter(Boolean);
    for (const rec of records) {
      if (rec.startsWith("##")) {
        const head = rec.slice(2).trim();
        result.branch = head.split("...")[0].trim() || null;
        const aheadPart = head.split("ahead ")[1];
        const behindPart = head.split("behind ")[1];
        if (aheadPart) result.ahead = parseInt(aheadPart, 10) || 0;
        if (behindPart) result.behind = parseInt(behindPart, 10) || 0;
      } else if (rec.length >= 3 && rec[2] === " ") {
        // v0.7.1: porcelain -z rename/copy entries are followed by a second
        // NUL-separated record (the old path); only XY-prefixed records
        // (2 status chars + 1 space + path) are real entries.
        const xy = rec.slice(0, 2);
        const path = rec.slice(3);
        const index = (xy[0] ?? " ").trim();
        const working = (xy[1] ?? " ").trim();
        result.files.push({
          path,
          index: index === "" ? "?" : index,
          working: working === "" ? "?" : working,
          staged: index !== "" && index !== "?" && index !== " ",
        });
      }
    }
  } catch (err) {
    result.error = err instanceof Error ? err.message : String(err);
  }
  return result;
}

export async function remoteGitLog(conn: RemoteConnection, cwd: string, max: number): Promise<GitCommit[]> {
  if (!cwd) return [];
  try {
    if (!(await isRepo(conn, cwd))) return [];
    const res = await gitOut(
      conn,
      cwd,
      "log -n " + max + " --pretty=format:%H%x1f%an%x1f%ae%x1f%aI%x1f%s",
    );
    if (res.code !== 0) return [];
    const commits: GitCommit[] = [];
    for (const line of res.out.split("\n")) {
      if (!line.trim()) continue;
      const f = line.split("\x1f");
      if (f.length < 5) continue;
      commits.push({
        hash: f[0],
        shortHash: f[0].slice(0, 7),
        message: f[4],
        author: f[1] + (f[2] ? " <" + f[2] + ">" : ""),
        date: new Date(f[3]).toISOString(),
      });
    }
    return commits;
  } catch {
    return [];
  }
}

export async function remoteGitDiffFile(
  conn: RemoteConnection,
  cwd: string,
  filePath: string,
): Promise<string | null> {
  if (!cwd || !filePath) return null;
  try {
    const res = await gitOut(conn, cwd, "diff -- " + shq(filePath));
    return res.code === 0 ? res.out : null;
  } catch {
    return null;
  }
}

export async function remoteGitDiffAll(conn: RemoteConnection, cwd: string): Promise<string | null> {
  if (!cwd) return null;
  try {
    const res = await gitOut(conn, cwd, "diff");
    return res.code === 0 ? res.out : null;
  } catch {
    return null;
  }
}

async function repoGuard(conn: RemoteConnection, cwd: string): Promise<string | null> {
  if (!cwd) return "Not a git repository";
  if (!(await isRepo(conn, cwd))) return "Not a git repository";
  return null;
}

export async function remoteGitStage(
  conn: RemoteConnection,
  cwd: string,
  paths: string[],
): Promise<string | null> {
  if (!cwd || paths.length === 0) return null;
  const guard = await repoGuard(conn, cwd);
  if (guard) return guard;
  try {
    const res = await gitOut(conn, cwd, "add -- " + paths.map((p) => shq(p)).join(" "));
    return res.code === 0 ? null : res.out.trim() || "git add failed";
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
}

export async function remoteGitUnstage(
  conn: RemoteConnection,
  cwd: string,
  paths: string[],
): Promise<string | null> {
  if (!cwd || paths.length === 0) return null;
  const guard = await repoGuard(conn, cwd);
  if (guard) return guard;
  try {
    const res = await gitOut(conn, cwd, "reset -- " + paths.map((p) => shq(p)).join(" "));
    return res.code === 0 ? null : res.out.trim() || "git reset failed";
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
}

export async function remoteGitCommit(
  conn: RemoteConnection,
  cwd: string,
  message: string,
): Promise<string | null> {
  if (!cwd || !message.trim()) return "Empty commit message";
  const guard = await repoGuard(conn, cwd);
  if (guard) return guard;
  try {
    const res = await gitOut(conn, cwd, "commit -m " + shq(message));
    return res.code === 0 ? null : res.out.trim() || "git commit failed";
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
}

export async function remoteGitPush(conn: RemoteConnection, cwd: string): Promise<string | null> {
  if (!cwd) return null;
  const guard = await repoGuard(conn, cwd);
  if (guard) return guard;
  try {
    const res = await gitOut(conn, cwd, "push", 120000);
    return res.code === 0 ? null : res.out.trim() || "git push failed";
  } catch (err) {
    return err instanceof Error ? err.message : String(err);
  }
}
