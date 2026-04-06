import { app, BrowserWindow, ipcMain, shell, dialog, Menu } from 'electron'
import { join } from 'path'
import fs from 'fs'
import { acpManager } from './acp'
import { loadConfig, saveConfig, AppConfig } from './config'
import { loadVibeSessions, loadVibeSessionMessages, loadVibeSessionMetadata, loadVibeSubagentSessions } from './vibeStats'
import { checkSetup, installDependencies } from './setup'
import {
  loadVibeConfig,
  saveVibeConfig,
  loadAgentFiles,
  saveAgentFile,
  deleteAgentFile,
  loadPromptFiles,
  savePromptFile,
  deletePromptFile,
  loadSkillFiles,
  getVibeDirPath,
  VibeConfig,
  VibeAgentConfig,
  VibeMcpServerConfig,
  VibeProviderConfig,
  VibeModelConfig,
  VibeSkill
} from './vibeConfig'

function createWindow(): BrowserWindow {
  const iconPath = join(__dirname, '../renderer/icon.png')

  const win = new BrowserWindow({
    width: 1200,
    height: 800,
    minWidth: 800,
    minHeight: 600,
    frame: false,
    titleBarStyle: 'hidden',
    backgroundColor: '#111115',
    show: false,
    autoHideMenuBar: true,
    icon: iconPath,
    webPreferences: {
      preload: join(__dirname, '../preload/index.js'),
      sandbox: false,
      contextIsolation: true,
      nodeIntegration: false
    }
  })

  win.on('ready-to-show', () => win.show())

  win.webContents.setWindowOpenHandler(({ url }) => {
    shell.openExternal(url)
    return { action: 'deny' }
  })

  if (process.env['NODE_ENV'] === 'development') {
    win.loadURL('http://localhost:5173')
  } else {
    win.loadFile(join(__dirname, '../renderer/index.html'))
  }

  acpManager.setWindow(win)

  // Native context menu for text inputs (copy / cut / paste / select-all)
  win.webContents.on('context-menu', (_e, props) => {
    const { editFlags } = props
    const menu = Menu.buildFromTemplate([
      { role: 'cut', enabled: editFlags.canCut },
      { role: 'copy', enabled: editFlags.canCopy },
      { role: 'paste', enabled: editFlags.canPaste },
      { type: 'separator' },
      { role: 'selectAll' }
    ])
    menu.popup({ window: win })
  })

  return win
}

app.whenReady().then(() => {
  const win = createWindow()

  // Window controls
  ipcMain.handle('window:minimize', () => win.minimize())
  ipcMain.handle('window:maximize', () => {
    if (win.isMaximized()) win.unmaximize()
    else win.maximize()
  })
  ipcMain.handle('window:close', () => win.close())

  // Config
  ipcMain.handle('config:get', () => loadConfig())
  ipcMain.handle('config:save', (_e, config: AppConfig) => {
    saveConfig(config)
  })

  // Dialog: open folder
  ipcMain.handle('dialog:open-folder', async () => {
    const result = await dialog.showOpenDialog(win, {
      properties: ['openDirectory']
    })
    return result.canceled ? null : result.filePaths[0]
  })

  // Dialog: open file (for ACP binary)
  ipcMain.handle('dialog:open-file', async () => {
    const result = await dialog.showOpenDialog(win, {
      properties: ['openFile'],
      filters: [
        { name: 'Executable', extensions: process.platform === 'win32' ? ['exe', '*'] : ['*'] }
      ]
    })
    return result.canceled ? null : result.filePaths[0]
  })

  // ACP
  ipcMain.handle('acp:init', async (_e, acpPath: string) => {
    // Initialize ACP without creating a session (session/list is called automatically)
    await acpManager.start(acpPath).catch((err: Error) => {
      if (err.message === 'ACP stopped') return
      throw err
    })
  })

  ipcMain.handle('acp:start', async (_e, acpPath: string, cwd: string, model: string) => {
    // start() stops any running instance first; ignore stale-request rejections from that stop
    await acpManager.start(acpPath).catch((err: Error) => {
      if (err.message === 'ACP stopped') return  // expected when restarting
      throw err
    })
    await acpManager.newSession(cwd, model)
  })

  ipcMain.handle('acp:new-session', async (_e, cwd: string, model: string) => {
    await acpManager.newSession(cwd, model)
  })

  ipcMain.handle('acp:resume-session', async (_e, sessionId: string, cwd: string, model: string) => {
    await acpManager.resumeSession(sessionId, cwd, model)
  })

  ipcMain.handle('acp:load-session', async (_e, sessionId: string, cwd: string, model: string) => {
    await acpManager.loadSession(sessionId, cwd, model)
  })

  ipcMain.handle('acp:send-prompt', async (_e, text: string, acpSessionId?: string) => {
    await acpManager.sendPrompt(text, acpSessionId)
  })

  ipcMain.handle('acp:cancel-session', (_e, acpSessionId: string) => {
    acpManager.cancelSession(acpSessionId)
  })

  ipcMain.handle('acp:set-mode', async (_e, mode: string, sessionId?: string) => {
    await acpManager.setConfigOption('mode', mode, sessionId)
  })

  ipcMain.handle('acp:set-model', async (_e, model: string, sessionId?: string) => {
    await acpManager.setConfigOption('model', model, sessionId)
  })

  ipcMain.handle('acp:approve-tool', (_e, requestId: string, optionId: string) => {
    acpManager.approveToolRequest(requestId, optionId)
  })
  ipcMain.handle('acp:answer-question', (_e, requestId: string, answer: string) => {
    acpManager.answerUserQuestion(requestId, answer)
  })

  ipcMain.handle('acp:stop', async () => {
    await acpManager.stop()
  })

  ipcMain.handle('acp:is-running', () => acpManager.isRunning())

  ipcMain.handle('acp:get-context-summary', async (_e, messages: Array<{ role: string; content: string }>) => {
    return acpManager.getContextSummary(messages)
  })

  // Vibe stats
  ipcMain.handle('stats:vibe-sessions', () => loadVibeSessions())
  ipcMain.handle('stats:vibe-session-messages', (_e, sessionId: string) =>
    loadVibeSessionMessages(sessionId)
  )
  ipcMain.handle('stats:vibe-session-metadata', (_e, sessionId: string) =>
    loadVibeSessionMetadata(sessionId)
  )
  ipcMain.handle('stats:vibe-subagent-sessions', (_e, parentSessionId: string) =>
    loadVibeSubagentSessions(parentSessionId)
  )

  // Filesystem helpers
  ipcMain.handle('fs:list-dir', (_e, dir: string) => {
    try {
      return fs
        .readdirSync(dir, { withFileTypes: true })
        .map((e) => ({ name: e.name, isDir: e.isDirectory() }))
    } catch {
      return []
    }
  })

  // Recursive file search for @ picker (max depth 4, skips node_modules/.git/etc)
  ipcMain.handle('fs:search-files', (_e, rootDir: string, query: string): Array<{ path: string; isDir: boolean }> => {
    const results: Array<{ path: string; isDir: boolean }> = []
    const SKIP_DIRS = new Set(['.git', 'node_modules', '.venv', '__pycache__', 'dist', '.next', 'build', 'target'])
    const q = query.toLowerCase()

    function walk(dir: string, relBase: string, depth: number): void {
      if (depth > 4) return
      let entries: fs.Dirent[]
      try {
        entries = fs.readdirSync(dir, { withFileTypes: true })
      } catch {
        return
      }
      for (const e of entries) {
        if (e.isDirectory() && SKIP_DIRS.has(e.name)) continue
        const relPath = relBase ? `${relBase}/${e.name}` : e.name
        if (!q || relPath.toLowerCase().includes(q) || e.name.toLowerCase().includes(q)) {
          results.push({ path: relPath, isDir: e.isDirectory() })
        }
        if (e.isDirectory()) walk(join(dir, e.name), relPath, depth + 1)
        if (results.length >= 50) return
      }
    }

    try {
      walk(rootDir, '', 0)
    } catch {}
    return results
  })

  // Read file contents for @ picker
  ipcMain.handle('fs:read-file', (_e, filePath: string) => {
    try {
      const stats = fs.statSync(filePath)
      if (stats.isDirectory()) {
        // Return directory listing
        const entries = fs.readdirSync(filePath, { withFileTypes: true })
        return { type: 'directory', content: entries.map(e => `${e.isDirectory() ? '📁' : '📄'} ${e.name}`).join('\n') }
      } else {
        // Return file contents (limit to 100KB)
        const content = fs.readFileSync(filePath, 'utf-8')
        const truncated = content.length > 100000 ? content.slice(0, 100000) + '\n\n[...truncated...]' : content
        return { type: 'file', content: truncated }
      }
    } catch (err) {
      return { type: 'error', content: `Error reading: ${filePath}` }
    }
  })

  // Shell: open path in file explorer, creating if needed
  ipcMain.handle('shell:open-path', async (_e, path: string) => {
    try {
      if (!fs.existsSync(path)) {
        fs.mkdirSync(path, { recursive: true })
      }
    } catch {}
    return shell.openPath(path)
  })

  // Setup / installation
  ipcMain.handle('setup:check', () => checkSetup())

  ipcMain.handle('setup:install', async () => {
    await installDependencies((message, type) => {
      win.webContents.send('setup:progress', { message, type })
    })
    // Return fresh status so renderer can get the discovered path
    return checkSetup()
  })

  // Vibe config
  ipcMain.handle('vibe:get-config', () => loadVibeConfig())

  ipcMain.handle('vibe:save-config', (_e, config: VibeConfig) => {
    saveVibeConfig(config)
  })

  ipcMain.handle('vibe:get-agents', () => loadAgentFiles())

  ipcMain.handle('vibe:save-agent', (_e, name: string, config: VibeAgentConfig) => {
    saveAgentFile(name, config)
  })

  ipcMain.handle('vibe:delete-agent', (_e, name: string) => {
    deleteAgentFile(name)
  })

  ipcMain.handle('vibe:get-prompts', () => loadPromptFiles())

  ipcMain.handle('vibe:save-prompt', (_e, name: string, content: string) => {
    savePromptFile(name, content)
  })

  ipcMain.handle('vibe:delete-prompt', (_e, name: string) => {
    deletePromptFile(name)
  })

  ipcMain.handle('vibe:get-skills', () => loadSkillFiles())

  ipcMain.handle('vibe:get-dir', () => getVibeDirPath())

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow()
  })
})

app.on('window-all-closed', () => {
  acpManager.stop()
  if (process.platform !== 'darwin') app.quit()
})
