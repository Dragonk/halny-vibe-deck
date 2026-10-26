import { sshRun, shq } from "./remote";
import type { RemoteConnection } from "./config";

// v0.7.0: remote filesystem over ssh - mirrors the shapes of the local
// fs handlers exactly so the renderer needs no special-casing.

const TEXT_CAP = 4 * 1024 * 1024; // 4 MB
const BINARY_CAP = 48 * 1024 * 1024; // 48 MB

export async function remoteListDir(
  conn: RemoteConnection,
  dir: string,
): Promise<Array<{ name: string; isDir: boolean }>> {
  const res = await sshRun(conn, "ls -1ap " + shq(dir) + " 2>/dev/null", 20000);
  if (res.code !== 0) return [];
  const out: Array<{ name: string; isDir: boolean }> = [];
  for (const line of res.stdout.split("\n")) {
    const name = line.replace(/\/$/, "");
    if (!name || name === "." || name === "..") continue;
    out.push({ name, isDir: line.endsWith("/") });
  }
  return out;
}

export async function remoteStat(
  conn: RemoteConnection,
  filePath: string,
): Promise<{ exists: boolean; isDir: boolean; size: number; ext: string }> {
  const res = await sshRun(conn, "stat -c '%F|%s' " + shq(filePath) + " 2>/dev/null", 15000);
  if (res.code !== 0 || !res.stdout.trim()) {
    return { exists: false, isDir: false, size: 0, ext: "" };
  }
  const [type, sizeStr] = res.stdout.trim().split("|");
  const ext = filePath.split(".").pop()?.toLowerCase() ?? "";
  return {
    exists: true,
    isDir: type === "directory",
    size: parseInt(sizeStr, 10) || 0,
    ext,
  };
}

export async function remoteReadText(
  conn: RemoteConnection,
  filePath: string,
): Promise<{ content: string; truncated: boolean; size: number; error: string | null }> {
  try {
    const st = await remoteStat(conn, filePath);
    if (!st.exists) {
      return { content: "", truncated: false, size: 0, error: "File not found: " + filePath };
    }
    if (st.size > TEXT_CAP) {
      const res = await sshRun(
        conn,
        "head -c " + TEXT_CAP + " " + shq(filePath) + " 2>/dev/null | base64 -w0",
        60000,
      );
      const content = Buffer.from(res.stdout, "base64").toString("utf-8");
      return { content, truncated: true, size: st.size, error: null };
    }
    const res = await sshRun(conn, "base64 -w0 " + shq(filePath) + " 2>/dev/null", 60000);
    if (res.code !== 0) {
      return { content: "", truncated: false, size: 0, error: res.stderr || "Read failed" };
    }
    const content = Buffer.from(res.stdout, "base64").toString("utf-8");
    return { content, truncated: false, size: st.size, error: null };
  } catch (err) {
    return { content: "", truncated: false, size: 0, error: err instanceof Error ? err.message : String(err) };
  }
}

export async function remoteReadBinary(
  conn: RemoteConnection,
  filePath: string,
  mimeFor: (path: string) => string,
): Promise<{ base64: string; mime: string; size: number; error: string | null }> {
  try {
    const st = await remoteStat(conn, filePath);
    if (!st.exists) {
      return { base64: "", mime: "", size: 0, error: "File not found: " + filePath };
    }
    if (st.size > BINARY_CAP) {
      return { base64: "", mime: mimeFor(filePath), size: st.size, error: "File too large" };
    }
    const res = await sshRun(conn, "base64 -w0 " + shq(filePath) + " 2>/dev/null", 120000);
    if (res.code !== 0) {
      return { base64: "", mime: "", size: 0, error: res.stderr || "Read failed" };
    }
    const buf = Buffer.from(res.stdout, "base64");
    return { base64: buf.toString("base64"), mime: mimeFor(filePath), size: st.size, error: null };
  } catch (err) {
    return { base64: "", mime: "", size: 0, error: err instanceof Error ? err.message : String(err) };
  }
}

export async function remoteWriteText(
  conn: RemoteConnection,
  filePath: string,
  content: string,
): Promise<{ error: string | null }> {
  try {
    const b64 = Buffer.from(content, "utf-8").toString("base64");
    const res = await sshRun(conn, "base64 -d > " + shq(filePath), 60000, b64);
    if (res.code !== 0) return { error: res.stderr || "Write failed" };
    return { error: null };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}

export async function remoteReadFile(
  conn: RemoteConnection,
  filePath: string,
): Promise<{ type: "file" | "directory" | "error"; content: string }> {
  const st = await remoteStat(conn, filePath);
  if (!st.exists) return { type: "error", content: "Error reading: " + filePath };
  if (st.isDir) {
    const entries = await remoteListDir(conn, filePath);
    return {
      type: "directory",
      content: entries.map((e) => (e.isDir ? "[dir] " : "[file] ") + e.name).join("\n"),
    };
  }
  const res = await sshRun(
    conn,
    "head -c 100000 " + shq(filePath) + " 2>/dev/null | base64 -w0",
    60000,
  );
  if (res.code !== 0) return { type: "error", content: "Error reading: " + filePath };
  let content = Buffer.from(res.stdout, "base64").toString("utf-8");
  if (st.size > 100000) content = content + "\n\n[...truncated...]";
  return { type: "file", content };
}

export async function remoteSearchFiles(
  conn: RemoteConnection,
  rootDir: string,
  query: string,
): Promise<Array<{ path: string; isDir: boolean }>> {
  const results: Array<{ path: string; isDir: boolean }> = [];
  try {
    const res = await sshRun(
      conn,
      "cd " + shq(rootDir) + " 2>/dev/null && find . -maxdepth 4 \\( -name .git -o -name node_modules -o -name .venv -o -name venv -o -name __pycache__ -o -name dist -o -name build -o -name out -o -name target -o -name .next -o -name .cache \\) -prune -o -printf '%y|%P\\n' 2>/dev/null | head -2000",
      30000,
    );
    const q = query.toLowerCase();
    for (const line of res.stdout.split("\n")) {
      if (!line) continue;
      const sep = line.indexOf("|");
      if (sep < 0) continue;
      const y = line.slice(0, sep);
      const rel = line.slice(sep + 1);
      if (!q || rel.toLowerCase().includes(q)) {
        results.push({ path: rel, isDir: y === "d" });
      }
      if (results.length >= 50) break;
    }
  } catch {
    /* remote search unavailable */
  }
  return results;
}

// v0.13.14: remote mkdir -p for the web "New project…" flow
export async function remoteMkdir(
  conn: RemoteConnection,
  dir: string,
): Promise<{ error: string | null }> {
  try {
    await sshRun(conn, "mkdir -p " + shq(dir), 20000);
    return { error: null };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}
