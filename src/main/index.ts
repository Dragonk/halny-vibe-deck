import { app, BrowserWindow, ipcMain, shell, dialog, Menu } from "electron";
import { join, dirname } from "path";
import * as pathMod from "path";
import fs from "fs";
import os from "os";
import dgram from "node:dgram";
import { monitorEventLoopDelay } from "node:perf_hooks";
import { acpManager, AcpManager, AcpApplyConfig } from "./acp";
import { applyPortableUpdate, cleanupStaleUpdateDirs } from "./updateCmd";
import {
  loadQueue,
  listQueue,
  peekNextFor,
  addToQueue,
  removeFromQueue,
  clearQueue,
  queueSessionIds,
} from "./promptQueue";
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
import { checkSetupAsync, installDependencies } from "./setup";
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
import { testRemoteConnection, installRemoteAcp, listRemoteSessions, checkRemoteStatus, sshRun, shq } from "./remote";
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
  remoteWriteBinary,
  remoteMkdir,
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

// v1.0.24: every client (window + all web/PWA views) sees the same queue.
function broadcastQueue(): void {
  mainWinRef?.webContents.send("queue:changed", listQueue());
  broadcastToWeb("queue:changed", [listQueue()]);
}

// v1.0.24: the server-side queue flush - one session at a time; a
// turn in flight makes deliverQueued wait, and the turn-end hook
// (onTurnComplete) retries the delivery.
const flushingSessions = new Set<string>();
async function flushQueueFor(acpSessionId: string): Promise<void> {
  if (flushingSessions.has(acpSessionId)) return;
  flushingSessions.add(acpSessionId);
  try {
    for (;;) {
      const head = peekNextFor(acpSessionId);
      if (!head) break;
      const mgr = head.remoteId ? getRemoteManager(head.remoteId) : acpManager;
      const delivered = await mgr.deliverQueued(head);
      if (!delivered) break; // a turn in flight or a hard failure - the turn-end hook retries
      broadcastQueue();
    }
  } catch (err) {
    console.error("[queue] flush failed:", err);
  } finally {
    flushingSessions.delete(acpSessionId);
  }
}

function getRemoteManager(remoteId: string): AcpManager {
  let m = remoteAcpManagers.get(remoteId)
  if (!m) {
    m = new AcpManager()
    m.setRemoteTag(remoteId)
    remoteAcpManagers.set(remoteId, m)
  }
  m.setWindow(mainWinRef)
  // v1.0.24: every manager (local + remote) flushes the server queue
  // when its turn ends.
  m.onTurnComplete = (sid) => {
    void flushQueueFor(sid)
  }
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
// v0.13.12: support both "--name value" and "--name=value". The "=" form is
// required when the Electron binary would otherwise swallow a positional
// value as its app-path argument (e.g. packaged AppImage, electron cli).
const hasArg = (name: string): boolean =>
  argv.includes("--" + name) || argv.some((a) => a.startsWith("--" + name + "="));
const argVal = (name: string): string | null => {
  const prefix = "--" + name + "=";
  for (const a of argv) {
    if (a.startsWith(prefix)) {
      const v = a.slice(prefix.length);
      return v.length > 0 ? v : null;
    }
  }
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
  // v1.0.29: never raise a dialog from a headless server instance
  if (process.platform !== "win32" || headless) return
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

// v1.0.28: best-effort sd_notify(3) over $NOTIFY_SOCKET. With Type=notify +
// WatchdogSec a wedged main loop (port open, nothing served) stops the
// WATCHDOG=1 pings and systemd restarts the unit instead of a silent 521.
// Node typings do not know the "unix_dgram" socket type, but it works in
// Electron (same trick systemd-cat style helpers use) - declare the tiny
// surface we need and cast.
interface UnixDgramSocketLike {
  on(event: "error", listener: (err: Error) => void): void;
  send(
    msg: Buffer,
    offset: number,
    length: number,
    path: string,
    callback: (err: Error | null, bytes: number) => void
  ): void;
  close(): void;
}

function sdNotify(message: string): void {
  const socketPath = process.env["NOTIFY_SOCKET"];
  if (!socketPath) return;
  try {
    const target = socketPath.startsWith("@")
      ? "\0" + socketPath.slice(1)
      : socketPath;
    const client = dgram.createSocket(
      "unix_dgram" as unknown as dgram.SocketType
    ) as unknown as UnixDgramSocketLike;
    client.on("error", () => {
      try {
        client.close();
      } catch {
        /* already closed */
      }
    });
    client.send(Buffer.from(message, "utf8"), 0, message.length, target, () => {
      try {
        client.close();
      } catch {
        /* already closed */
      }
    });
  } catch {
    /* best effort - never block app startup */
  }
}

function startSystemdNotify(): void {
  sdNotify("READY=1");
  const usec = Number.parseInt(process.env["WATCHDOG_USEC"] ?? "0", 10);
  if (!Number.isFinite(usec) || usec <= 0) return;
  const intervalMs = Math.max(Math.floor(usec / 3_000_000), 5) * 1000;
  const timer = setInterval(() => sdNotify("WATCHDOG=1"), intervalMs);
  timer.unref();
  console.log(`[systemd] notify active, watchdog ping every ${intervalMs}ms`);
}

// v1.0.29: postmortem evidence for watchdog restarts. If anything ever
// stalls the loop again, the journal gets a line with the measured lag
// instead of a silent wedge.
function startLoopDiagnostics(): void {
  try {
    const hist = monitorEventLoopDelay({ resolution: 50 });
    hist.enable();
    const timer = setInterval(() => {
      const maxMs = hist.max / 1e6;
      if (maxMs > 2000) {
        console.log(
          `[loop] event-loop lag in the last 30s: mean ${(hist.mean / 1e6).toFixed(0)}ms, max ${maxMs.toFixed(0)}ms`,
        );
      }
      hist.reset();
    }, 30000);
    timer.unref();
  } catch {
    /* diagnostics only - never block startup */
  }
}

app.whenReady().then(() => {
  startSystemdNotify();
  startLoopDiagnostics();
  // v0.14.0 rebrand: one-time migration of user data from the legacy
  // "mistral-vibe-nexus" directory so settings survive the rename.
  try {
    const userDataDir = app.getPath("userData");
    if (!fs.existsSync(userDataDir)) {
      const legacyDir = join(app.getPath("appData"), "mistral-vibe-nexus");
      if (fs.existsSync(legacyDir)) {
        console.log("[rebrand] migrating user data from", legacyDir);
        fs.cpSync(legacyDir, userDataDir, { recursive: true });
      }
    }
  } catch (err) {
    console.error("[rebrand] user data migration failed", err);
  }
  // v1.0.24: load the persisted prompt queue; deliver queued prompts a
  // few seconds after boot (workers are already up) - queued work
  // survives even a full app restart with zero clients.
  loadQueue();
  acpManager.onTurnComplete = (sid) => {
    void flushQueueFor(sid);
  };
  setTimeout(() => {
    for (const sid of queueSessionIds()) void flushQueueFor(sid);
  }, 8000);

  // v0.13.10: headless self-update apply mode. The OLD app spawns the NEW
  // executable with --apply-update right before it exits (updates.ts);
  // this instance swaps the files and relaunches - no window is created.
  if (process.argv.includes("--apply-update")) {
    const arg = (name: string): string | null => {
      const i = process.argv.indexOf(name);
      return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : null;
    };
    const pid = parseInt(arg("--pid") ?? "0", 10);
    const srcDir = arg("--src");
    const appDir = arg("--appdir");
    const exeName = arg("--exe");
    const tmpDir = arg("--tmpd");
    if (!pid || !srcDir || !appDir || !exeName) {
      app.exit(1);
      return;
    }
    void applyPortableUpdate({ pid, srcDir, appDir, exeName, tmpDir: tmpDir ?? "" })
      .catch(() => {
        /* failure details are in the log and the fail marker */
      })
      .finally(() => app.exit(0));
    return;
  }
  showFailedUpdateNotice();
  cleanupStaleUpdateDirs();
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
    // v1.0.7: no boot prime - starting the app must NOT open any session.
    // The user starts a session consciously ("+" in a project, the new
    // project picker) or resumes an existing one; the first prompt on a
    // draft creates the ACP session (InputBar first-message flow).
  });

  ipcMain.handle("acp:ensure-worker", async (_e, acpPath: string, _cwd: string, remoteId?: string) => {
    // v0.7.0: idempotently make sure the worker for a connection (or the
    // local agent) is running. Never primes a session — resume paths use this
    // so replaying a session does not litter the remote with empty sessions.
    const mgr = remoteId ? getRemoteManager(remoteId) : acpManager
    if (mgr.isRunning()) {
      // v0.13.12: never probe (and possibly restart) a worker while a
      // turn is running - the CLI may not answer session/list within the
      // 8 s ping window mid-turn, and a restart would kill a healthy long
      // turn. The prompt request timeout and the stop button own the
      // dead-turn detection.
      if (mgr.isTurnInFlight()) return
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

  ipcMain.handle("acp:send-prompt", async (_e, text: string, acpSessionId?: string, remoteId?: string, images?: Array<{ data: string; mimeType: string; uri?: string }>, userMsg?: { id: string; text: string; attachments?: Array<{ path: string; isDir: boolean }> }) => {
    // v1.0.24: the announce lives inside sendPrompt (one central place -
    // direct and queued deliveries both announce).
    await getManager(remoteId).sendPrompt(text, acpSessionId, images, userMsg)
  })

  // v1.0.13: which ACP sessions currently run a turn - a freshly booted
  // client (phone PWA reopened) marks them busy/live instead of showing
  // them as idle ("stalled") while the turn keeps running server-side.
  // ---- v1.0.24: the SERVER-SIDE prompt queue ---------------------------------
  // A queued prompt lives on the server: every client (PC, phone, second
  // browser) sees the same queue, and delivery happens server-side even
  // when every client is closed - the queued work is never lost.
  ipcMain.handle("queue:list", () => listQueue());
  ipcMain.handle("queue:add", async (_e, item) => {
    const acpId = (item && item.acpSessionId) as string;
    addToQueue(item);
    broadcastQueue();
    // v1.0.24: an idle session delivers immediately (the client-side
    // stale-busy check already filtered real turns; this also covers a
    // client that queued against a stale busy flag).
    if (acpId) void flushQueueFor(acpId);
    return { ok: true };
  });
  ipcMain.handle("queue:remove", (_e, id: string) => {
    removeFromQueue(id);
    broadcastQueue();
    return { ok: true };
  });
  ipcMain.handle("queue:clear", () => {
    clearQueue();
    broadcastQueue();
    return { ok: true };
  });

  // v1.0.25: re-pull the CLI's session list into the sidebar - a client
  // returning from the background imports sessions created on other
  // devices while it was away (the manager emits SessionsList).
  ipcMain.handle("acp:refresh-sessions", async (_e, remoteId?: string) => {
    const mgr = remoteId ? getRemoteManager(remoteId) : acpManager;
    await mgr.listSessions().catch(() => {});
    return { ok: true };
  });

  ipcMain.handle("acp:busy-sessions", () => {
    const out: Array<{ acpSessionId: string; remoteId: string | null }> = []
    for (const sid of acpManager.busySessionIds()) out.push({ acpSessionId: sid, remoteId: null })
    for (const [id, m] of remoteAcpManagers) {
      for (const sid of m.busySessionIds()) out.push({ acpSessionId: sid, remoteId: id })
    }
    return out
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
  // v1.0.5: updates must never kill a worker mid-turn - vibe-acp replaces
  // binaries on disk, so the worker dies with the turn unfinished and the
  // user sees an aborted session. Refuse the update instead until the
  // running turn(s) finish.
  const anyTurnInFlight = (): boolean =>
    acpManager.isTurnInFlight() ||
    [...remoteAcpManagers.values()].some((m) => m.isTurnInFlight());

  ipcMain.handle("updates:update-cli", async (_e, remoteId?: string | null) => {
    // The CLI binaries are being replaced on disk - every vibe-acp worker
    // (local + remote) must stop first. The renderer restores live sessions
    // once the upgrade finishes.
    if (anyTurnInFlight()) {
      throw new Error("A turn is in progress - wait for it to finish or stop it, then update");
    }
    await stopAllAcp();
    if (remoteId) {
      const conn = loadConfig().remoteConnections.find((c) => c.id === remoteId);
      if (!conn) throw new Error("Remote connection not found: " + remoteId);
      return upgradeRemoteCli(conn);
    }
    return upgradeLocalCli();
  });
  ipcMain.handle("updates:update-app", () => {
    // v1.0.5: same mid-turn guard as the CLI update - the update quits (or
    // taskkills) this process, which takes every vibe-acp worker down with
    // it. Refuse while a turn is running so the session is not aborted.
    if (anyTurnInFlight()) {
      throw new Error("A turn is in progress - wait for it to finish or stop it, then update");
    }
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
        if (win) win.webContents.send("updates:progress", p);
        // v0.13.15: headless mode has no window - progress must still reach
        // the web clients (the window mirror does not exist without one).
        else broadcastToWeb("updates:progress", [p]);
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

  // v0.13.14: write raw bytes (base64) - web client file/image attachments
  // upload their picked file to the session's host this way.
  ipcMain.handle("fs:write-binary", async (_e, filePath: string, base64: string, remoteId?: string) => {
    if (remoteId) return remoteWriteBinary(remoteConnOrThrow(remoteId), filePath, base64);
    try {
      // v1.0.10: the attachment dir (/tmp/nexus-attachments) may not exist
      // (fresh boot, wiped tmp) - without mkdir every upload silently
      // failed with ENOENT and the file never reached the conversation.
      await fs.promises.mkdir(pathMod.dirname(filePath), { recursive: true });
      await fs.promises.writeFile(filePath, Buffer.from(base64, "base64"));
      return { error: null };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) };
    }
  });

  // v0.13.14: create a project directory (web mode "New project…" dialog)
  // v1.0.9: file-list context menu actions - rename / delete / save-as,
  // all remote-aware (the tree also browses SSH hosts).
  ipcMain.handle("fs:rename", async (_e, filePath: string, newName: string, remoteId?: string) => {
    const clean = String(newName ?? "").trim();
    if (!clean || clean.includes("/") || clean.includes("\\") || clean === "." || clean === "..") {
      return { error: "Invalid name" };
    }
    const target = join(dirname(filePath), clean);
    if (remoteId) {
      const r = await sshRun(remoteConnOrThrow(remoteId), "mv " + shq(filePath) + " " + shq(target), 20000);
      return { error: r.code === 0 ? null : (r.stderr || "mv failed").slice(0, 300) };
    }
    try {
      fs.renameSync(filePath, target);
      return { error: null };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) };
    }
  });

  ipcMain.handle("fs:delete", async (_e, filePath: string, remoteId?: string) => {
    if (remoteId) {
      const r = await sshRun(remoteConnOrThrow(remoteId), "rm -rf " + shq(filePath), 30000);
      return { error: r.code === 0 ? null : (r.stderr || "rm failed").slice(0, 300) };
    }
    try {
      fs.rmSync(filePath, { recursive: true, force: true });
      return { error: null };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) };
    }
  });

  // v1.0.9: download a (possibly remote) file into a local path chosen by
  // the user - the "Pobierz" action of the file-list context menu.
  ipcMain.handle("fs:save-as", async (_e, filePath: string, remoteId?: string) => {
    const name = filePath.split(/[\\/]/).pop() || "file";
    const win = BrowserWindow.getAllWindows()[0] ?? null;
    const res = await dialog.showSaveDialog(win, { defaultPath: join(os.homedir(), "Downloads", name) });
    if (res.canceled || !res.filePath) return { error: null, saved: false };
    const data = remoteId
      ? await remoteReadBinary(remoteConnOrThrow(remoteId), filePath, mimeFor)
      : await (async () => {
          try {
            const b = fs.readFileSync(filePath);
            return { base64: b.toString("base64"), mime: "application/octet-stream", size: b.length, error: null };
          } catch (err) {
            return { base64: "", mime: "", size: 0, error: err instanceof Error ? err.message : String(err) };
          }
        })();
    if (data.error) return { error: data.error, saved: false };
    try {
      fs.writeFileSync(res.filePath, Buffer.from(data.base64, "base64"));
      return { error: null, saved: true };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err), saved: false };
    }
  });

  ipcMain.handle("fs:mkdir", async (_e, filePath: string, remoteId?: string) => {
    if (remoteId) return remoteMkdir(remoteConnOrThrow(remoteId), filePath);
    try {
      await fs.promises.mkdir(filePath, { recursive: true });
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

  // v1.0.7: home directory of the host (or of the SSH remote) - the
  // default start location of the new-project picker, so the user does
  // not have to climb out of the filesystem root.
  ipcMain.handle("fs:home-dir", async (_e, remoteId?: string) => {
    if (remoteId) {
      try {
        const r = await sshRun(remoteConnOrThrow(remoteId), "echo $HOME", 10000);
        return r.code === 0 && r.stdout.trim() ? r.stdout.trim() : "/";
      } catch {
        return "/";
      }
    }
    return os.homedir();
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
  ipcMain.handle("setup:check", () => checkSetupAsync());

  ipcMain.handle("setup:install", async () => {
    await installDependencies((message, type) => {
      win?.webContents.send("setup:progress", { message, type });
    });
    return checkSetupAsync();
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

