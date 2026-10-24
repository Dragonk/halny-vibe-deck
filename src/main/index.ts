import { app, BrowserWindow, ipcMain, shell, dialog, Menu } from "electron";
import { join } from "path";
import fs from "fs";
import { acpManager } from "./acp";
import { loadConfig, saveConfig, AppConfig } from "./config";
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

  // Native context menu for text inputs (copy / cut / paste / select-all)
  win.webContents.on("context-menu", (_e, props) => {
    const { editFlags } = props;
    const menu = Menu.buildFromTemplate([
      { role: "cut", enabled: editFlags.canCut },
      { role: "copy", enabled: editFlags.canCopy },
      { role: "paste", enabled: editFlags.canPaste },
      { type: "separator" },
      { role: "selectAll" },
    ]);
    menu.popup({ window: win });
  });

  return win;
}

app.whenReady().then(() => {
  const win = createWindow();

  // Window controls
  ipcMain.handle("window:minimize", () => win.minimize());
  ipcMain.handle("window:maximize", () => {
    if (win.isMaximized()) win.unmaximize();
    else win.maximize();
  });
  ipcMain.handle("window:close", () => win.close());

  // Config
  ipcMain.handle("config:get", () => loadConfig());
  ipcMain.handle("config:save", (_e, config: AppConfig) => {
    saveConfig(config);
  });

  // Dialogs
  ipcMain.handle("dialog:open-folder", async () => {
    const result = await dialog.showOpenDialog(win, {
      properties: ["openDirectory"],
    });
    return result.canceled ? null : result.filePaths[0];
  });

  ipcMain.handle("dialog:open-file", async () => {
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

  ipcMain.handle("acp:start", async (_e, acpPath: string, cwd: string) => {
    await acpManager.start(acpPath).catch((err: Error) => {
      if (err.message === "ACP stopped") return;
      throw err;
    });
    await acpManager.newSession(cwd);
  });

  ipcMain.handle("acp:new-session", async (_e, cwd: string) => {
    await acpManager.newSession(cwd);
  });

  ipcMain.handle("acp:load-session", async (_e, sessionId: string, cwd: string) => {
    await acpManager.loadSession(sessionId, cwd);
  });

  ipcMain.handle("acp:send-prompt", async (_e, text: string, acpSessionId?: string) => {
    await acpManager.sendPrompt(text, acpSessionId);
  });

  ipcMain.handle("acp:cancel-session", (_e, acpSessionId: string) => {
    acpManager.cancelSession(acpSessionId);
  });

  ipcMain.handle("acp:set-config-option", async (_e, key: string, value: string, sessionId?: string) => {
    await acpManager.setConfigOption(key, value, sessionId);
  });

  ipcMain.handle("acp:approve-tool", (_e, requestId: string, optionId: string) => {
    acpManager.approveToolRequest(requestId, optionId);
  });

  ipcMain.handle("acp:answer-question", (_e, requestId: string, answer: string) => {
    acpManager.answerUserQuestion(requestId, answer);
  });

  ipcMain.handle("acp:stop", async () => {
    await acpManager.stop();
  });

  ipcMain.handle("acp:is-running", () => acpManager.isRunning());

  ipcMain.handle("acp:get-context-summary", async (_e, messages: Array<{ role: string; content: string }>) => {
    return acpManager.getContextSummary(messages);
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


  // Write text files (file editor in the preview dock)
  ipcMain.handle("fs:write-text", async (_e, filePath: string, content: string) => {
    try {
      await fs.promises.writeFile(filePath, content, "utf-8");
      return { error: null };
    } catch (err) {
      return { error: err instanceof Error ? err.message : String(err) };
    }
  });

  // Filesystem helpers
  ipcMain.handle("fs:list-dir", (_e, dir: string) => {
    try {
      return fs
        .readdirSync(dir, { withFileTypes: true })
        .map((e) => ({ name: e.name, isDir: e.isDirectory() }));
    } catch {
      return [];
    }
  });

  // Recursive file search for @ picker (max depth 4, skips node_modules/.git/etc)
  ipcMain.handle("fs:search-files", (_e, rootDir: string, query: string): Array<{ path: string; isDir: boolean }> => {
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
  ipcMain.handle("fs:read-file", (_e, filePath: string) => {
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
  ipcMain.handle("fs:stat", (_e, filePath: string) => {
    try {
      const stats = fs.statSync(filePath);
      return { exists: true, isDir: stats.isDirectory(), size: stats.size, ext: filePath.split(".").pop()?.toLowerCase() ?? "" };
    } catch {
      return { exists: false, isDir: false, size: 0, ext: "" };
    }
  });

  const TEXT_CAP = 4 * 1024 * 1024; // 4 MB
  ipcMain.handle("fs:read-text", (_e, filePath: string) => {
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
  ipcMain.handle("fs:read-binary", (_e, filePath: string) => {
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

  // Git (dock tab)
  ipcMain.handle("git:status", (_e, cwd: string) => gitStatus(cwd));
  ipcMain.handle("git:log", (_e, cwd: string, max: number) => gitLog(cwd, max));
  ipcMain.handle("git:diff-file", (_e, cwd: string, filePath: string) => gitDiffFile(cwd, filePath));
  ipcMain.handle("git:diff-all", (_e, cwd: string) => gitDiffAll(cwd));
  ipcMain.handle("git:stage", (_e, cwd: string, paths: string[]) => gitStage(cwd, paths));
  ipcMain.handle("git:unstage", (_e, cwd: string, paths: string[]) => gitUnstage(cwd, paths));
  ipcMain.handle("git:commit", (_e, cwd: string, message: string) => gitCommit(cwd, message));
  ipcMain.handle("git:push", (_e, cwd: string) => gitPush(cwd));

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
      win.webContents.send("setup:progress", { message, type });
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

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on("window-all-closed", () => {
  acpManager.stop();
  if (process.platform !== "darwin") app.quit();
});

