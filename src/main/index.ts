import { app, BrowserWindow, ipcMain, shell, dialog, Menu } from "electron";
import { join } from "path";
import fs from "fs";
import { acpManager, AcpManager } from "./acp";
import { loadConfig, saveConfig, AppConfig } from "./config";
import { menuLabels } from "./i18n";
import {
  loadVibeSessions,
  loadVibeSessionMessages,
  loadVibeSessionMetadata,
  loadVibeSubagentSessions,
  searchSessionContent,
  loadSessionLastModel,
  loadSessionLastConfig,
} from "./vibeStats";
import { checkSetup, installDependencies } from "./setup";
import {
  loadVibeConfig,
  saveVibeConfig,
  loadVibeConfigRaw,
  saveVibeConfigRaw,
  loadAgentFiles,
  saveAgentFile,
  listConfigFiles,
  createSkillFile,
  deleteSkillFile,
  writeSkillFile,
  deleteSkillSubFile,
  importSkillFromFolder,
  importSkillFromZip,
  deleteAgentFile,
  loadPromptFiles,
  savePromptFile,
  deletePromptFile,
  loadSkillFiles,
  getVibeDirPath,
  resolveVibeDir,
  VibeConfig,
  VibeAgentFileConfig,
} from "./vibeConfig";
import { gitStatus, gitLog, gitDiffFile, gitDiffAll, gitStage, gitUnstage, gitCommit, gitPush } from "./git";
import { randomUUID } from "crypto";
import { broadcastToWeb } from "./bus";
import { installIpcCapture } from "./ipc-registry";
import {
  startServe,
  stopServe,
  serveStatus,
  regenerateCertificate,
  makeSalt,
  hashPassword,
} from "./serve";
import { testRemoteConnection, installRemoteAcp, listRemoteSessions, checkRemoteStatus } from "./remote";
import {
  remoteListDir,
  remoteStat,
  remoteReadText,
  remoteReadBinary,
  remoteWriteText,
  remoteReadFile,
  remoteSearchFiles,
} from "./remoteFs";
import {
  remoteGitStatus,
  remoteGitLog,
  remoteGitDiffFile,
  remoteGitDiffAll,
  remoteGitStage,
  remoteGitUnstage,
  remoteGitCommit,
  remoteGitPush,
} from "./remoteGit";
import type { ServeConfig, RemoteConnection } from "./config";

const MIME_BY_EXT: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  bmp: "image/bmp",
  svg: "image/svg+xml",
  ico: "image/x-icon",
  tiff: "image/tiff",
  pdf: "application/pdf",
};

function mimeFor(file: string): string {
  const ext = file.split(".").pop()?.toLowerCase() ?? "";
  return MIME_BY_EXT[ext] ?? "application/octet-stream";
}

// ---------------------------------------------------------------------------
// v0.7.0: multi-remote ACP workers — one AcpManager per remote connection
// (several remote agents at once) plus the local agent. Workers persist;
// the renderer addresses them by remoteId. Killing the local ssh child
// terminates the remote vibe-acp via SIGHUP.
// ---------------------------------------------------------------------------
const remoteAcpManagers = new Map<string, AcpManager>()
let mainWinRef: BrowserWindow | null = null

function getRemoteManager(remoteId: string): AcpManager {
  let m = remoteAcpManagers.get(remoteId)
  if (!m) {
    m = new AcpManager()
    m.setRemoteTag(remoteId)
    remoteAcpManagers.set(remoteId, m)
  }
  m.setWindow(mainWinRef)
  return m
}

function getManager(remoteId?: string): AcpManager {
  if (remoteId) {
    const m = remoteAcpManagers.get(remoteId)
    if (!m || !m.isRunning()) throw new Error("Remote ACP worker not started: " + remoteId)
    return m
  }
  return acpManager
}

async function stopAllAcp(): Promise<void> {
  await acpManager.stop()
  for (const m of remoteAcpManagers.values()) await m.stop()
  remoteAcpManagers.clear()
}

function createWindow(): BrowserWindow {
  const iconPath = app.isPackaged
    ? join(process.resourcesPath, "icon.png")
    : join(__dirname, "../../resources/icon.png");

  const win = new BrowserWindow({
    width: 1400,
    height: 900,
    minWidth: 960,
    minHeight: 600,
    frame: false,
    titleBarStyle: "hidden",
    backgroundColor: "#111115",
    show: false,
    autoHideMenuBar: true,
    icon: iconPath,
    webPreferences: {
      preload: join(__dirname, "../preload/index.js"),
      sandbox: false,
      contextIsolation: true,
      nodeIntegration: false,
    },
  });

  win.on("ready-to-show", () => win.show());

  win.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url);
    return { action: "deny" };
  });

  if (process.env["NODE_ENV"] === "development") {
    win.loadURL("http://localhost:5173");
  } else {
    win.loadFile(join(__dirname, "../renderer/index.html"));
  }

  acpManager.setWindow(win);
  // v0.7.0: remote workers mirror events to the (re)created window too
  mainWinRef = win;

  // v0.6.0: mirror all renderer-bound events to web (serve) clients
  const origSend = win.webContents.send.bind(win.webContents);
  ;(win.webContents as unknown as { send: (channel: string, ...args: unknown[]) => void }).send = (
    channel: string,
    ...args: unknown[]
  ) => {
    origSend(channel, ...args);
    if (channel !== "acp:event") broadcastToWeb(channel, args);
  };

  // Native context menu for text inputs (copy / cut / paste / select-all),
  // localized to the configured app language (main/i18n.ts).
  win.webContents.on("context-menu", (_e, props) => {
    const { editFlags } = props;
    const labels = menuLabels(loadConfig().language);
    const menu = Menu.buildFromTemplate([
      { role: "cut", label: labels.cut, enabled: editFlags.canCut },
      { role: "copy", label: labels.copy, enabled: editFlags.canCopy },
      { role: "paste", label: labels.paste, enabled: editFlags.canPaste },
      { type: "separator" },
      { role: "selectAll", label: labels.selectAll },
    ]);
    menu.popup({ window: win });
  });

  return win;
}

// v0.6.0: CLI flags (--serve, --headless, --port, --user, --password, --cert, --key)
const argv = process.argv;
const hasArg = (name: string): boolean => argv.includes("--" + name);
const argVal = (name: string): string | null => {
  const i = argv.indexOf("--" + name);
  if (i < 0 || i + 1 >= argv.length) return null;
  const v = argv[i + 1];
  return v && !v.startsWith("--") ? v : null;
};
const headless = hasArg("headless");
const serveFlag = hasArg("serve");
const cliPort = argVal("port");
const cliUser = argVal("user");
const cliPass = argVal("password");
const cliCert = argVal("cert");
const cliKey = argVal("key");

app.whenReady().then(() => {
  installIpcCapture();
  const win: BrowserWindow | null = headless ? null : createWindow();
  if (!win) acpManager.setWindow(null);

  // Window controls
  ipcMain.handle("window:minimize", () => win?.minimize());
  ipcMain.handle("window:maximize", () => {
    if (!win) return;
    if (win.isMaximized()) win.unmaximize();
    else win.maximize();
  });
  ipcMain.handle("window:close", () => win?.close());

  // Config
  ipcMain.handle("config:get", () => loadConfig());
  ipcMain.handle("config:save", (_e, config: AppConfig) => {
    saveConfig(config);
  });

  // Dialogs
  ipcMain.handle("dialog:open-folder", async () => {
    if (!win) return null;
    const result = await dialog.showOpenDialog(win, {
      properties: ["openDirectory"],
    });
    return result.canceled ? null : result.filePaths[0];
  });

  ipcMain.handle("dialog:open-file", async () => {
    if (!win) return null;
    const result = await dialog.showOpenDialog(win, {
      properties: ["openFile"],
      filters: [
        {
          name: "Executable",
          extensions: process.platform === "win32" ? ["exe", "*"] : ["*"],
        },
      ],
    });
    return result.canceled ? null : result.filePaths[0];
  });

  // ACP
  ipcMain.handle("acp:init", async (_e, acpPath: string) => {
    await acpManager.start(acpPath).catch((err: Error) => {
      if (err.message === "ACP stopped") return;
      throw err;
    });
  });

  ipcMain.handle("acp:ensure-worker", async (_e, acpPath: string, _cwd: string, remoteId?: string) => {
    // v0.7.0: idempotently make sure the worker for a connection (or the
    // local agent) is running. Never primes a session — resume paths use this
    // so replaying a session does not litter the remote with empty sessions.
    const mgr = remoteId ? getRemoteManager(remoteId) : acpManager
    if (mgr.isRunning()) return
    let remote: { sshCmd: string } | undefined
    if (remoteId) {
      const cfg = loadConfig()
      const conn = cfg.remoteConnections.find((c) => c.id === remoteId)
      if (!conn) throw new Error("Remote connection not found: " + remoteId)
      remote = { sshCmd: conn.sshCmd }
    }
    try {
      await mgr.start(acpPath, remote)
    } catch (err) {
      if (remoteId) remoteAcpManagers.delete(remoteId)
      throw err
    }
  })

  ipcMain.handle("acp:start", async (_e, acpPath: string, cwd: string, remoteId?: string) => {
    // v0.7.0: start (or reuse) the worker for the connection, then prime a
    // session on it. Workers persist, so several remotes can run at once.
    const mgr = remoteId ? getRemoteManager(remoteId) : acpManager
    try {
      if (!mgr.isRunning()) {
        let remote: { sshCmd: string } | undefined
        if (remoteId) {
          const cfg = loadConfig()
          const conn = cfg.remoteConnections.find((c) => c.id === remoteId)
          if (!conn) throw new Error("Remote connection not found: " + remoteId)
          remote = { sshCmd: conn.sshCmd }
        }
        await mgr.start(acpPath, remote)
      }
    } catch (err) {
      if (remoteId) remoteAcpManagers.delete(remoteId)
      if (err instanceof Error && err.message === "ACP stopped") return
      throw err
    }
    await mgr.newSession(cwd)
  })

  ipcMain.handle("acp:new-session", async (_e, cwd: string, remoteId?: string) => {
    await getManager(remoteId).newSession(cwd)
  })

  ipcMain.handle("acp:load-session", async (_e, sessionId: string, cwd: string, remoteId?: string) => {
    await getManager(remoteId).loadSession(sessionId, cwd)
  })

  ipcMain.handle("acp:send-prompt", async (_e, text: string, acpSessionId?: string, remoteId?: string) => {
    await getManager(remoteId).sendPrompt(text, acpSessionId)
  })

  ipcMain.handle("acp:cancel-session", (_e, acpSessionId: string, remoteId?: string) => {
    getManager(remoteId).cancelSession(acpSessionId)
  })

  ipcMain.handle("acp:set-config-option", async (_e, key: string, value: string, sessionId?: string, remoteId?: string) => {
    await getManager(remoteId).setConfigOption(key, value, sessionId)
  })

  ipcMain.handle("acp:approve-tool", (_e, requestId: string, optionId: string, remoteId?: string) => {
    getManager(remoteId).approveToolRequest(requestId, optionId)
  })

  ipcMain.handle("acp:answer-question", (_e, requestId: string, answer: string, remoteId?: string) => {
    getManager(remoteId).answerUserQuestion(requestId, answer)
  })

  ipcMain.handle("acp:stop-worker", async (_e, remoteId?: string) => {
    const m = remoteId ? remoteAcpManagers.get(remoteId) : null
    if (m) {
      await m.stop()
      remoteAcpManagers.delete(remoteId!)
    }
  })

  ipcMain.handle("acp:stop", async () => {
    await stopAllAcp()
  })

  ipcMain.handle("acp:is-running", (_e, remoteId?: string) =>
    remoteId ? (remoteAcpManagers.get(remoteId)?.isRunning() ?? false) : acpManager.isRunning(),
  )

  ipcMain.handle("acp:get-context-summary", async (_e, messages: Array<{ role: string; content: string }>, remoteId?: string) => {
    return getManager(remoteId).getContextSummary(messages);
  });

  // Vibe stats
  ipcMain.handle("stats:vibe-sessions", () => loadVibeSessions());
  ipcMain.handle("stats:vibe-session-last-model", (_e, sessionId: string) =>
    loadSessionLastModel(sessionId));
  ipcMain.handle("stats:vibe-session-last-config", (_e, sessionId: string) =>
    loadSessionLastConfig(sessionId));
  ipcMain.handle("stats:search-session-content", (_e, query: string) =>
    searchSessionContent(query),
  );
  ipcMain.handle("stats:vibe-session-messages", (_e, sessionId: string) =>
    loadVibeSessionMessages(sessionId),
  );
  ipcMain.handle("stats:vibe-session-metadata", (_e, sessionId: string) =>
    loadVibeSessionMetadata(sessionId),
  );
  ipcMain.handle("stats:vibe-subagent-sessions", (_e, parentSessionId: string) =>
    loadVibeSubagentSessions(parentSessionId),
  );


  // v0.7.0: resolve a remote connection for fs/git handlers (throws when unknown)
  const remoteConnOrThrow = (id: string): RemoteConnection => {
    const conn = loadConfig().remoteConnections.find((c) => c.id === id);
    if (!conn) throw new Error("Remote connection not found: " + id);
    return conn;
  };

  // Write text files (file editor in the preview dock)
  ipcMain.handle("fs:write-text", async (_e, filePath: string, content: string, remoteId?: string) => {
    if (remoteId) return remoteWriteText(remoteConnOrThrow(remoteId), filePath, content);
    try {
      await fs.promises.writeFile(filePath, content, "utf-8");
      return { error: null };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) };
    }
  });

  // Filesystem helpers
  ipcMain.handle("fs:list-dir", (_e, dir: string, remoteId?: string) => {
    if (remoteId) return remoteListDir(remoteConnOrThrow(remoteId), dir);
    try {
      return fs
        .readdirSync(dir, { withFileTypes: true })
        .map((e) => ({ name: e.name, isDir: e.isDirectory() }));
    } catch {
      return [];
    }
  });

  // Recursive file search for @ picker (max depth 4, skips node_modules/.git/etc)
  ipcMain.handle("fs:search-files", (_e, rootDir: string, query: string, remoteId?: string) => {
    if (remoteId) return remoteSearchFiles(remoteConnOrThrow(remoteId), rootDir, query);
    const results: Array<{ path: string; isDir: boolean }> = [];
    const SKIP_DIRS = new Set([
      ".git",
      "node_modules",
      ".venv",
      "__pycache__",
      "dist",
      ".next",
      "build",
      "target",
      ".cache",
    ]);
    const q = query.toLowerCase();

    function walk(dir: string, relBase: string, depth: number): void {
      if (depth > 4) return;
      let entries: fs.Dirent[];
      try {
        entries = fs.readdirSync(dir, { withFileTypes: true });
      } catch {
        return;
      }
      for (const e of entries) {
        if (e.isDirectory() && SKIP_DIRS.has(e.name)) continue;
        const relPath = relBase ? relBase + "/" + e.name : e.name;
        if (!q || relPath.toLowerCase().includes(q) || e.name.toLowerCase().includes(q)) {
          results.push({ path: relPath, isDir: e.isDirectory() });
        }
        if (e.isDirectory()) walk(join(dir, e.name), relPath, depth + 1);
        if (results.length >= 50) return;
      }
    }

    try {
      walk(rootDir, "", 0);
    } catch {}
    return results;
  });

  // Read file contents for @ picker
  ipcMain.handle("fs:read-file", (_e, filePath: string, remoteId?: string) => {
    if (remoteId) return remoteReadFile(remoteConnOrThrow(remoteId), filePath);
    try {
      const stats = fs.statSync(filePath);
      if (stats.isDirectory()) {
        const entries = fs.readdirSync(filePath, { withFileTypes: true });
        return {
          type: "directory",
          content: entries.map((e) => (e.isDirectory() ? "[dir] " : "[file] ") + e.name).join("\n"),
        };
      } else {
        const content = fs.readFileSync(filePath, "utf-8");
        const truncated =
          content.length > 100000 ? content.slice(0, 100000) + "\n\n[...truncated...]" : content;
        return { type: "file", content: truncated };
      }
    } catch {
      return { type: "error", content: "Error reading: " + filePath };
    }
  });

  // File info for the preview panel
  ipcMain.handle("fs:stat", (_e, filePath: string, remoteId?: string) => {
    if (remoteId) return remoteStat(remoteConnOrThrow(remoteId), filePath);
    try {
      const stats = fs.statSync(filePath);
      return { exists: true, isDir: stats.isDirectory(), size: stats.size, ext: filePath.split(".").pop()?.toLowerCase() ?? "" };
    } catch {
      return { exists: false, isDir: false, size: 0, ext: "" };
    }
  });

  const TEXT_CAP = 4 * 1024 * 1024; // 4 MB
  ipcMain.handle("fs:read-text", (_e, filePath: string, remoteId?: string) => {
    if (remoteId) return remoteReadText(remoteConnOrThrow(remoteId), filePath);
    try {
      const stats = fs.statSync(filePath);
      if (stats.size > TEXT_CAP) {
        const fd = fs.openSync(filePath, "r");
        const buf = Buffer.alloc(TEXT_CAP);
        const read = fs.readSync(fd, buf, 0, TEXT_CAP, 0);
        fs.closeSync(fd);
        return { content: buf.subarray(0, read).toString("utf-8"), truncated: true, size: stats.size, error: null };
      }
      return { content: fs.readFileSync(filePath, "utf-8"), truncated: false, size: stats.size, error: null };
    } catch (err) {
      return { content: "", truncated: false, size: 0, error: err instanceof Error ? err.message : String(err) };
    }
  });

  const BINARY_CAP = 48 * 1024 * 1024; // 48 MB
  ipcMain.handle("fs:read-binary", (_e, filePath: string, remoteId?: string) => {
    if (remoteId) return remoteReadBinary(remoteConnOrThrow(remoteId), filePath, mimeFor);
    try {
      const stats = fs.statSync(filePath);
      if (stats.size > BINARY_CAP) {
        return { base64: "", mime: mimeFor(filePath), size: stats.size, error: "File too large" };
      }
      const buf = fs.readFileSync(filePath);
      return { base64: buf.toString("base64"), mime: mimeFor(filePath), size: stats.size, error: null };
    } catch (err) {
      return { base64: "", mime: "", size: 0, error: err instanceof Error ? err.message : String(err) };
    }
  });

  // Git (dock tab) - v0.7.0: routed to the remote worker over ssh
  ipcMain.handle("git:status", (_e, cwd: string, remoteId?: string) =>
    remoteId ? remoteGitStatus(remoteConnOrThrow(remoteId), cwd) : gitStatus(cwd));
  ipcMain.handle("git:log", (_e, cwd: string, max: number, remoteId?: string) =>
    remoteId ? remoteGitLog(remoteConnOrThrow(remoteId), cwd, max) : gitLog(cwd, max));
  ipcMain.handle("git:diff-file", (_e, cwd: string, filePath: string, remoteId?: string) =>
    remoteId ? remoteGitDiffFile(remoteConnOrThrow(remoteId), cwd, filePath) : gitDiffFile(cwd, filePath));
  ipcMain.handle("git:diff-all", (_e, cwd: string, remoteId?: string) =>
    remoteId ? remoteGitDiffAll(remoteConnOrThrow(remoteId), cwd) : gitDiffAll(cwd));
  ipcMain.handle("git:stage", (_e, cwd: string, paths: string[], remoteId?: string) =>
    remoteId ? remoteGitStage(remoteConnOrThrow(remoteId), cwd, paths) : gitStage(cwd, paths));
  ipcMain.handle("git:unstage", (_e, cwd: string, paths: string[], remoteId?: string) =>
    remoteId ? remoteGitUnstage(remoteConnOrThrow(remoteId), cwd, paths) : gitUnstage(cwd, paths));
  ipcMain.handle("git:commit", (_e, cwd: string, message: string, remoteId?: string) =>
    remoteId ? remoteGitCommit(remoteConnOrThrow(remoteId), cwd, message) : gitCommit(cwd, message));
  ipcMain.handle("git:push", (_e, cwd: string, remoteId?: string) =>
    remoteId ? remoteGitPush(remoteConnOrThrow(remoteId), cwd) : gitPush(cwd));

  // Shell: open path in file explorer, creating if needed
  ipcMain.handle("shell:open-path", async (_e, path: string) => {
    try {
      if (!fs.existsSync(path)) {
        fs.mkdirSync(path, { recursive: true });
      }
    } catch {}
    return shell.openPath(path);
  });

  // Setup / installation
  ipcMain.handle("setup:check", () => checkSetup());

  ipcMain.handle("setup:install", async () => {
    await installDependencies((message, type) => {
      win?.webContents.send("setup:progress", { message, type });
    });
    return checkSetup();
  });

  // Vibe config (config.toml) — global (~/.vibe) or project (<cwd>/.vibe) scope
  const resolveScopeDir = (scope: string, cwd: string): string =>
    resolveVibeDir(scope === "project" ? "project" : "global", cwd || undefined);

  ipcMain.handle("vibe:get-config", (_e, scope: string, cwd: string) => loadVibeConfig(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-config", (_e, config: VibeConfig, scope: string, cwd: string) => {
    saveVibeConfig(config, resolveScopeDir(scope, cwd));
  });
  ipcMain.handle("vibe:get-raw-config", (_e, scope: string, cwd: string) => loadVibeConfigRaw(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-raw-config", (_e, content: string, scope: string, cwd: string) => saveVibeConfigRaw(content, resolveScopeDir(scope, cwd)));

  ipcMain.handle("vibe:get-agents", (_e, scope: string, cwd: string) => loadAgentFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-agent", (_e, name: string, config: VibeAgentFileConfig, scope: string, cwd: string) => {
    saveAgentFile(name, config, resolveScopeDir(scope, cwd));
  });
  ipcMain.handle("vibe:delete-agent", (_e, name: string, scope: string, cwd: string) => {
    deleteAgentFile(name, resolveScopeDir(scope, cwd));
  });

  ipcMain.handle("vibe:get-prompts", (_e, scope: string, cwd: string) => loadPromptFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-prompt", (_e, name: string, content: string, scope: string, cwd: string) => {
    savePromptFile(name, content, resolveScopeDir(scope, cwd));
  });
  ipcMain.handle("vibe:delete-prompt", (_e, name: string, scope: string, cwd: string) => {
    deletePromptFile(name, resolveScopeDir(scope, cwd));
  });

  ipcMain.handle("vibe:get-skills", (_e, scope: string, cwd: string) => loadSkillFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:get-dir", (_e, scope: string, cwd: string) => resolveScopeDir(scope, cwd));
  ipcMain.handle("vibe:list-config-files", (_e, scope: string, cwd: string) =>
    listConfigFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:create-skill", (_e, name: string, scope: string, cwd: string) =>
    createSkillFile(name, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:delete-skill", (_e, name: string, scope: string, cwd: string) =>
    deleteSkillFile(name, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:write-skill-file",
    (_e, name: string, relPath: string, content: string, scope: string, cwd: string) =>
    writeSkillFile(name, relPath, content, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:delete-skill-file", (_e, name: string, relPath: string, scope: string, cwd: string) =>
    deleteSkillSubFile(name, relPath, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:import-skill",
    (_e, src: string, isZip: boolean, scope: string, cwd: string) =>
    isZip
      ? importSkillFromZip(src, resolveScopeDir(scope, cwd))
      : importSkillFromFolder(src, resolveScopeDir(scope, cwd)));

  // ---------- v0.6.0: serve (HTTPS web hosting) ----------
  const rendererDir = (): string => join(__dirname, "..", "renderer");
  const applyServeSection = (mutate: (serve: ServeConfig) => ServeConfig): void => {
    const cfg = loadConfig();
    cfg.serve = mutate({ ...cfg.serve });
    saveConfig(cfg);
  };

  ipcMain.handle("serve:status", () => serveStatus());
  ipcMain.handle("serve:start", async () => {
    const cfg = loadConfig();
    await startServe(cfg.serve, rendererDir());
    return serveStatus();
  });
  ipcMain.handle("serve:stop", () => {
    stopServe();
    return serveStatus();
  });
  ipcMain.handle("serve:save-config", (_e, patch: Partial<ServeConfig>) => {
    applyServeSection((serve) => ({
      ...serve,
      ...patch,
      username: serve.username,
      salt: serve.salt,
      hash: serve.hash,
    }));
    return serveStatus();
  });
  ipcMain.handle("serve:set-credentials", (_e, username: string, password: string) => {
    const u = String(username || "").trim();
    const p = String(password || "");
    if (u.length < 3) throw new Error("Username must be at least 3 characters");
    if (p.length < 6) throw new Error("Password must be at least 6 characters");
    const salt = makeSalt();
    applyServeSection((serve) => ({ ...serve, username: u, salt, hash: hashPassword(p, salt) }));
    return { ok: true };
  });
  ipcMain.handle("serve:regen-cert", async () => {
    const cfg = loadConfig();
    await regenerateCertificate(cfg.serve);
    if (serveStatus().running) {
      stopServe();
      await startServe(loadConfig().serve, rendererDir());
    }
    return { ok: true };
  });

  // ---------- v0.6.0: remote SSH connections ----------
  ipcMain.handle("remote:test", (_e, draft: RemoteConnection) => testRemoteConnection(draft));
  ipcMain.handle("remote:install-acp", (_e, id: string) => {
    const conn = loadConfig().remoteConnections.find((c) => c.id === id);
    if (!conn) throw new Error("Remote connection not found");
    return installRemoteAcp(conn);
  });
  ipcMain.handle("remote:sessions", (_e, id: string) => {
    const conn = loadConfig().remoteConnections.find((c) => c.id === id);
    if (!conn) throw new Error("Remote connection not found");
    return listRemoteSessions(conn);
  });
  ipcMain.handle("remote:status-now", async (_e, ids?: string[]) => {
    const conns = loadConfig().remoteConnections.filter(
      (c) => c.enabled && (!ids || ids.includes(c.id)),
    );
    const out: Record<string, unknown> = {};
    for (const c of conns) {
      out[c.id] = await checkRemoteStatus(c);
    }
    return out;
  });
  ipcMain.handle("remote:save", (_e, conn: RemoteConnection) => {
    const cfg = loadConfig();
    const clean: RemoteConnection = {
      ...conn,
      id: conn.id || randomUUID(),
      name: String(conn.name || "").trim(),
      sshCmd: String(conn.sshCmd || "").trim(),
      locations: (conn.locations ?? []).filter((l: string) => l && l.trim()),
      installCommand: conn.installCommand ?? "",
    };
    if (!clean.name || !clean.sshCmd) throw new Error("Name and ssh command are required");
    const i = cfg.remoteConnections.findIndex((c) => c.id === clean.id);
    if (i >= 0) cfg.remoteConnections[i] = clean;
    else cfg.remoteConnections.push(clean);
    saveConfig(cfg);
    return clean;
  });
  ipcMain.handle("remote:delete", (_e, id: string) => {
    const cfg = loadConfig();
    cfg.remoteConnections = cfg.remoteConnections.filter((c) => c.id !== id);
    saveConfig(cfg);
    return { ok: true };
  });

  // ---------- v0.6.0: CLI credentials + serve autostart ----------
  if (cliUser && cliPass) {
    const salt = makeSalt();
    applyServeSection((serve) => ({
      ...serve,
      username: cliUser,
      salt,
      hash: hashPassword(cliPass, salt),
    }));
    console.log("[serve] credentials saved for user:", cliUser);
  }
  if (cliCert && cliKey) {
    applyServeSection((serve) => ({ ...serve, certPath: cliCert, keyPath: cliKey }));
    console.log("[serve] custom certificate configured:", cliCert);
  }
  if (cliPort) {
    const port = parseInt(cliPort, 10);
    if (Number.isNaN(port) || port < 1 || port > 65535) {
      console.error("[serve] invalid port:", cliPort);
    } else {
      applyServeSection((serve) => ({ ...serve, port }));
    }
  }
  const cfgAtBoot = loadConfig();
  if (serveFlag || cfgAtBoot.serve.enabled) {
    void (async () => {
      try {
        await startServe(loadConfig().serve, rendererDir());
      } catch (err) {
        console.error(
          "[serve] failed to start:",
          err instanceof Error ? err.message : String(err),
        );
      }
    })();
  }

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0 && !headless) createWindow();
  });
});

// v0.7.0: stop the local agent and every remote worker on quit — killing
// the ssh children terminates the remote vibe-acp processes too.
app.on("before-quit", () => {
  void stopAllAcp()
})

app.on("window-all-closed", () => {
  void stopAllAcp()
  if (process.platform !== "darwin") app.quit();
});

