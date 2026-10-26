import { app, BrowserWindow, ipcMain, shell, dialog, Menu } from "electron";
import { join } from "path";
import fs from "fs";
import os from "os";
import { acpManager, AcpManager, AcpApplyConfig } from "./acp";
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
  loadSessionPreview,
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
import {
  getRemoteVibeConfig,
  saveRemoteVibeConfig,
  getRemoteRawConfig,
  saveRemoteRawConfig,
  getRemoteVibeDir,
  getRemoteAgents,
  listRemoteProjectAgents,
  exposeRemoteProjectAgent,
  exposeRemoteProjectAgents,
  saveRemoteAgent,
  deleteRemoteAgent,
  getRemotePrompts,
  saveRemotePrompt,
  deleteRemotePrompt,
  listRemoteConfigFiles,
  getRemoteSkills,
  createRemoteSkill,
  deleteRemoteSkill,
  writeRemoteSkillFile,
  deleteRemoteSkillFile,
  importRemoteSkill,
  loadRemoteSessionPreview,
} from "./remoteVibe";
import { gitStatus, gitLog, gitDiffFile, gitDiffAll, gitStage, gitUnstage, gitCommit, gitPush } from "./git";
import { randomUUID } from "crypto";
import { broadcastToWeb } from "./bus";
import { installIpcCapture } from "./ipc-registry";
import {
  startServe,
  stopServe,
  serveStatus,
  isServeRunning,
  regenerateCertificate,
  makeSalt,
  hashPassword,
} from "./serve";
import { testRemoteConnection, installRemoteAcp, listRemoteSessions, checkRemoteStatus } from "./remote";
import {
  checkAppUpdate,
  checkCliUpdate,
  checkRemoteCliUpdate,
  upgradeLocalCli,
  upgradeRemoteCli,
  updateApp,
} from "./updates";
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
// v0.13.8: the Windows update helper drops nexus-update-failed.txt into
// the temp dir when a self-update could not finish; surface it once on the
// next launch so update failures are never silent again.
function showFailedUpdateNotice(): void {
  if (process.platform !== "win32") return
  try {
    const failPath = join(os.tmpdir(), "nexus-update-failed.txt")
    if (!fs.existsSync(failPath)) return
    const reason = fs.readFileSync(failPath, "utf8").trim()
    fs.unlinkSync(failPath)
    dialog.showErrorBox(
      "Update failed",
      "The application update could not be installed:\n\n" +
        reason + "\n\nDetails: " + join(os.tmpdir(), "nexus-update.log"),
    )
  } catch {
    /* diagnostics only - never block startup */
  }
}

const cliKey = argVal("key");

app.whenReady().then(() => {
  showFailedUpdateNotice();
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
    // v0.8.1: the renderer's snapshot can be older than the serve section
    // on disk (serve:set-credentials writes it straight to disk) - always
    // keep the on-disk serve section so login/password survive unrelated
    // config saves and app restarts.
    config.serve = loadConfig().serve;
    saveConfig(config);
    // v0.7.1: a removed remote connection must not leave its worker
    // (and the ssh child) running until the app quits.
    const keepIds = new Set((config.remoteConnections ?? []).map((c) => c.id));
    for (const id of [...remoteAcpManagers.keys()]) {
      if (!keepIds.has(id)) {
        const m = remoteAcpManagers.get(id);
        remoteAcpManagers.delete(id);
        void m?.stop();
      }
    }
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
    // v0.10.7: single-owner boot. Every client boot (desktop window start,
    // web/PWA F5) goes through here, so it must be idempotent: a healthy
    // worker is reused as-is and the boot only re-announces readiness plus
    // the session list to every view - no restart, no duplicate session.
    if (acpManager.isRunning()) {
      let healthy = false
      try {
        await acpManager.ping()
        healthy = true
      } catch {
        healthy = false
      }
      if (healthy) {
        acpManager.announceReady()
        await acpManager.listSessions().catch(() => {})
        return
      }
      // v0.8.0 zombie lesson: a live handle that fails the probe is dead.
      await acpManager.stop().catch(() => {})
    }
    await acpManager.start(acpPath).catch((err: Error) => {
      if (err.message === "ACP stopped") return;
      throw err;
    });
    // v0.10.7: the boot prime is owned by the main process (the renderer
    // used to prime again on InitializationComplete, duplicating a session
    // on every window start / web F5). Once per worker lifetime.
    const bootCfg = loadConfig()
    const primeCwd = bootCfg.workingDirectory || "."
    exposeProjectAgentsForCwd(primeCwd)
    await acpManager.primeBootSession(primeCwd).catch(() => {})
  });

  ipcMain.handle("acp:ensure-worker", async (_e, acpPath: string, _cwd: string, remoteId?: string) => {
    // v0.7.0: idempotently make sure the worker for a connection (or the
    // local agent) is running. Never primes a session — resume paths use this
    // so replaying a session does not litter the remote with empty sessions.
    const mgr = remoteId ? getRemoteManager(remoteId) : acpManager
    if (mgr.isRunning()) {
      // v0.8.0: a live process handle is not proof the transport works - a
      // stale ssh worker never emits exit and used to hang resume for the
      // full 180 s request timeout. Probe it; restart on failure.
      try {
        await mgr.ping()
        return
      } catch {
        await mgr.stop().catch(() => {})
        if (remoteId) remoteAcpManagers.delete(remoteId)
      }
    }
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
    let mgr = remoteId ? getRemoteManager(remoteId) : acpManager
    // v0.10.7: probe a live handle before reusing it (v0.8.0 zombie lesson).
    // acp:start always creates its session on purpose - explicit user
    // intent (remote start, draft first prompt, settings restart) - so the
    // boot prime flag is deliberately not consulted here.
    if (mgr.isRunning()) {
      try {
        await mgr.ping()
      } catch {
        await mgr.stop().catch(() => {})
        if (remoteId) {
          remoteAcpManagers.delete(remoteId)
          mgr = getRemoteManager(remoteId)
        }
      }
    }
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
    // v0.10.2: expose project agents before priming (see acp:new-session).
    if (remoteId) await exposeRemoteProjectAgents(remoteId, cwd).catch(console.error)
    else exposeProjectAgentsForCwd(cwd)
    await mgr.newSession(cwd)
  })

  ipcMain.handle("acp:new-session", async (_e, cwd: string, remoteId?: string, applyConfig?: AcpApplyConfig) => {
    // v0.10.2: the CLI reads agent files at session/new - expose the
    // project's agents first so the new session can select them.
    if (remoteId) await exposeRemoteProjectAgents(remoteId, cwd).catch(console.error)
    else exposeProjectAgentsForCwd(cwd)
    await getManager(remoteId).newSession(cwd, applyConfig)
  })

  ipcMain.handle("acp:load-session", async (_e, sessionId: string, cwd: string, remoteId?: string, applyConfig?: AcpApplyConfig) => {
    // v0.10.2: expose project agents so a resumed session can keep a
    // project-agent mode (the CLI reads agent files at session/load).
    if (remoteId) await exposeRemoteProjectAgents(remoteId, cwd).catch(console.error)
    else exposeProjectAgentsForCwd(cwd)
    await getManager(remoteId).loadSession(sessionId, cwd, applyConfig)
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
    // v0.7.1: only the LOCAL agent. Settings restarts the local agent on
    // save; remote workers must survive it (stopAllAcp runs on quit).
    await acpManager.stop()
  })

  // v0.9.0: update system - app (GitHub releases) + mistral-vibe CLI (PyPI).
  ipcMain.handle("updates:check-app", () => checkAppUpdate());
  ipcMain.handle("updates:check-cli", () => checkCliUpdate());
  ipcMain.handle("updates:check-cli-remote", (_e, remoteId: string) => {
    const conn = loadConfig().remoteConnections.find((c) => c.id === remoteId);
    if (!conn) throw new Error("Remote connection not found: " + remoteId);
    return checkRemoteCliUpdate(conn);
  });
  ipcMain.handle("updates:update-cli", async (_e, remoteId?: string | null) => {
    // The CLI binaries are being replaced on disk - every vibe-acp worker
    // (local + remote) must stop first. The renderer restores live sessions
    // once the upgrade finishes.
    await stopAllAcp();
    if (remoteId) {
      const conn = loadConfig().remoteConnections.find((c) => c.id === remoteId);
      if (!conn) throw new Error("Remote connection not found: " + remoteId);
      return upgradeRemoteCli(conn);
    }
    return upgradeLocalCli();
  });
  ipcMain.handle("updates:update-app", () => {
    // Preserve serving across the restart: when the server is running even
    // though serving is disabled in config (started manually), flip it on so
    // it comes back automatically after the update relaunch.
    const cfg = loadConfig();
    if (isServeRunning() && !(cfg.serve?.enabled)) {
      cfg.serve = { ...cfg.serve, enabled: true };
      saveConfig(cfg);
    }
    // v0.11.0: live download/install progress for the banner + Settings
    // (win.webContents.send is mirrored to web clients by the serve bridge).
    const win = mainWinRef;
    return updateApp((p) => {
      try {
        win?.webContents.send("updates:progress", p);
      } catch {
        /* window may already be tearing down */
      }
    });
  });

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
  ipcMain.handle("stats:session-preview", (_e, sessionId: string) =>
    loadSessionPreview(sessionId),
  );
  ipcMain.handle("stats:remote-session-preview", (_e, remoteId: string, sessionId: string) =>
    loadRemoteSessionPreview(remoteId, sessionId),
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

  // v0.10.2: the CLI reads agent files when a session is created, but only
  // from the global agents dir (~/.vibe/agents). Expose a project's agents
  // there (symlink, copy fallback) so session/new picks them up. Existing
  // real global agent files are never shadowed; dangling links are cleaned.
  const exposeProjectAgentsForCwd = (cwd: string): void => {
    try {
      if (!cwd) return;
      const srcDir = join(cwd, ".vibe", "agents");
      if (!fs.existsSync(srcDir)) return;
      const dstDir = join(getVibeDirPath(), "agents");
      fs.mkdirSync(dstDir, { recursive: true });
      for (const f of fs.readdirSync(dstDir)) {
        const dst = join(dstDir, f);
        try {
          if (fs.lstatSync(dst).isSymbolicLink() && !fs.existsSync(dst)) fs.rmSync(dst, { force: true });
        } catch {
          /* not a link entry - ignore */
        }
      }
      for (const f of fs.readdirSync(srcDir)) {
        if (!f.endsWith(".toml") || !/^[A-Za-z0-9._-]+\.toml$/.test(f)) continue;
        const dst = join(dstDir, f);
        let isLink = false;
        try {
          isLink = fs.lstatSync(dst).isSymbolicLink();
        } catch {
          isLink = false;
        }
        if (!isLink && fs.existsSync(dst)) continue;
        fs.rmSync(dst, { force: true });
        try {
          fs.symlinkSync(join(srcDir, f), dst);
        } catch {
          try {
            fs.copyFileSync(join(srcDir, f), dst);
          } catch {
            /* copy failed - skip this agent */
          }
        }
      }
    } catch (err) {
      console.error("[agents] project agent exposure failed:", err);
    }
  };

  ipcMain.handle("vibe:get-config", async (_e, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? getRemoteVibeConfig(remoteId ?? "")
      : scope === "project" && remoteId
        ? getRemoteVibeConfig(remoteId, cwd + "/.vibe")
        : loadVibeConfig(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-config", async (_e, config: VibeConfig, scope: string, cwd: string, remoteId?: string) => {
    if (scope === "remote") {
      await saveRemoteVibeConfig(config, remoteId ?? "");
      return;
    }
    if (scope === "project" && remoteId) {
      await saveRemoteVibeConfig(config, remoteId, cwd + "/.vibe");
      return;
    }
    saveVibeConfig(config, resolveScopeDir(scope, cwd));
  });
  ipcMain.handle("vibe:get-raw-config", async (_e, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? getRemoteRawConfig(remoteId ?? "")
      : scope === "project" && remoteId
        ? getRemoteRawConfig(remoteId, cwd + "/.vibe")
        : loadVibeConfigRaw(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-raw-config", async (_e, content: string, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? saveRemoteRawConfig(content, remoteId ?? "")
      : scope === "project" && remoteId
        ? saveRemoteRawConfig(content, remoteId, cwd + "/.vibe")
        : saveVibeConfigRaw(content, resolveScopeDir(scope, cwd)));

  ipcMain.handle("vibe:get-agents", async (_e, scope: string, cwd: string, remoteId?: string) => {
    if (scope === "remote") return getRemoteAgents(remoteId ?? "");
    // v0.10.2: project agents of a remote host (mode picker for remote sessions)
    if (remoteId) return listRemoteProjectAgents(remoteId, cwd);
    return loadAgentFiles(resolveScopeDir(scope, cwd));
  });
  // v0.10.2: make a project agent visible to the CLI by exposing its file in
  // the global agents dir (~/.vibe/agents) - the CLI cannot see project agents.
  ipcMain.handle("vibe:expose-project-agent", async (_e, name: string, cwd: string, remoteId?: string) => {
    if (!/^[A-Za-z0-9._-]+$/.test(name)) throw new Error("Invalid agent name");
    if (remoteId) return exposeRemoteProjectAgent(remoteId, cwd, name);
    const src = join(resolveScopeDir("project", cwd), "agents", name + ".toml");
    if (!fs.existsSync(src)) throw new Error("Project agent file not found: " + src);
    const dstDir = join(getVibeDirPath(), "agents");
    fs.mkdirSync(dstDir, { recursive: true });
    const dst = join(dstDir, name + ".toml");
    // Never shadow an existing real global agent file with the same name.
    let isLink = false;
    try {
      isLink = fs.lstatSync(dst).isSymbolicLink();
    } catch {
      isLink = false;
    }
    if (!isLink && fs.existsSync(dst)) throw new Error("A global agent named " + name + " already exists");
    fs.rmSync(dst, { force: true });
    try {
      fs.symlinkSync(src, dst);
    } catch {
      // Windows without developer mode cannot symlink - copy instead.
      fs.copyFileSync(src, dst);
    }
  });
  ipcMain.handle("vibe:save-agent", async (_e, name: string, config: VibeAgentFileConfig, scope: string, cwd: string, remoteId?: string) => {
    if (scope === "remote") {
      await saveRemoteAgent(name, config, remoteId ?? "");
      return;
    }
    if (scope === "project" && remoteId) {
      await saveRemoteAgent(name, config, remoteId, cwd + "/.vibe");
      return;
    }
    saveAgentFile(name, config, resolveScopeDir(scope, cwd));
  });
  ipcMain.handle("vibe:delete-agent", async (_e, name: string, scope: string, cwd: string, remoteId?: string) => {
    if (scope === "remote") {
      await deleteRemoteAgent(name, remoteId ?? "");
      return;
    }
    if (scope === "project" && remoteId) {
      await deleteRemoteAgent(name, remoteId, cwd + "/.vibe");
      return;
    }
    deleteAgentFile(name, resolveScopeDir(scope, cwd));
  });

  ipcMain.handle("vibe:get-prompts", async (_e, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? getRemotePrompts(remoteId ?? "")
      : scope === "project" && remoteId
        ? getRemotePrompts(remoteId, cwd + "/.vibe")
        : loadPromptFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:save-prompt", async (_e, name: string, content: string, scope: string, cwd: string, remoteId?: string) => {
    if (scope === "remote") {
      await saveRemotePrompt(name, content, remoteId ?? "");
      return;
    }
    if (scope === "project" && remoteId) {
      await saveRemotePrompt(name, content, remoteId, cwd + "/.vibe");
      return;
    }
    savePromptFile(name, content, resolveScopeDir(scope, cwd));
  });
  ipcMain.handle("vibe:delete-prompt", async (_e, name: string, scope: string, cwd: string, remoteId?: string) => {
    if (scope === "remote") {
      await deleteRemotePrompt(name, remoteId ?? "");
      return;
    }
    if (scope === "project" && remoteId) {
      await deleteRemotePrompt(name, remoteId, cwd + "/.vibe");
      return;
    }
    deletePromptFile(name, resolveScopeDir(scope, cwd));
  });

  ipcMain.handle("vibe:get-skills", async (_e, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? getRemoteSkills(remoteId ?? "")
      : scope === "project" && remoteId
        ? getRemoteSkills(remoteId, cwd + "/.vibe")
        : loadSkillFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:get-dir", async (_e, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? getRemoteVibeDir(remoteId ?? "")
      : scope === "project" && remoteId
        ? cwd + "/.vibe"
        : resolveScopeDir(scope, cwd));
  ipcMain.handle("vibe:list-config-files", async (_e, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? listRemoteConfigFiles(remoteId ?? "")
      : scope === "project" && remoteId
        ? listRemoteConfigFiles(remoteId, cwd + "/.vibe")
        : listConfigFiles(resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:create-skill", async (_e, name: string, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? createRemoteSkill(name, remoteId ?? "")
      : scope === "project" && remoteId
        ? createRemoteSkill(name, remoteId, cwd + "/.vibe")
        : createSkillFile(name, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:delete-skill", async (_e, name: string, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? deleteRemoteSkill(name, remoteId ?? "")
      : scope === "project" && remoteId
        ? deleteRemoteSkill(name, remoteId, cwd + "/.vibe")
        : deleteSkillFile(name, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:write-skill-file",
    async (_e, name: string, relPath: string, content: string, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? writeRemoteSkillFile(name, relPath, content, remoteId ?? "")
      : scope === "project" && remoteId
        ? writeRemoteSkillFile(name, relPath, content, remoteId, cwd + "/.vibe")
        : writeSkillFile(name, relPath, content, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:delete-skill-file", async (_e, name: string, relPath: string, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote"
      ? deleteRemoteSkillFile(name, relPath, remoteId ?? "")
      : scope === "project" && remoteId
        ? deleteRemoteSkillFile(name, relPath, remoteId, cwd + "/.vibe")
        : deleteSkillSubFile(name, relPath, resolveScopeDir(scope, cwd)));
  ipcMain.handle("vibe:import-skill",
    async (_e, src: string, isZip: boolean, scope: string, cwd: string, remoteId?: string) =>
    scope === "remote" || (scope === "project" && remoteId)
      ? importRemoteSkill()
      : isZip
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

