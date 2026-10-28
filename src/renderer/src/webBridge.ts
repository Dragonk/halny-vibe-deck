import type { AppAPI, AcpApplyConfig } from '../../preload/index'

/**
 * Web bridge: when the renderer runs in a browser (serve mode), preload
 * does not exist. This module rebuilds window.api on top of the /ws
 * websocket: every method call becomes an RPC invoke that the serve
 * server dispatches to the same ipcMain handlers.
 */

interface WsMsg {
  t: 'invoke' | 'res' | 'ev'
  id?: string
  ch?: string
  args?: unknown[]
  ok?: boolean
  value?: unknown
  error?: string
}

let ws: WebSocket | null = null
let nextId = 1
let wsReady = false
const pending = new Map<string, { resolve: (v: unknown) => void; reject: (e: Error) => void }>()
const eventSubs = new Map<string, Set<(...args: unknown[]) => void>>()
// v1.0.32: invokes that arrive during the reconnect window used to fail
// instantly with 'Connection to the app server was lost' (e.g. resuming a
// session right as a mobile link blips). Park them briefly and flush on
// reopen - a short blip becomes invisible instead of failing the resume.
const DEFER_MS = 10_000
const DEFER_MAX = 50
interface DeferredInvoke {
  ch: string
  args: unknown[]
  resolve: (v: unknown) => void
  reject: (e: Error) => void
  at: number
}
let deferredInvokes: DeferredInvoke[] = []

function sendInvoke(ch: string, args: unknown[], resolve: (v: unknown) => void, reject: (e: Error) => void): void {
  const id = String(nextId++)
  pending.set(id, { resolve, reject })
  ws?.send(JSON.stringify({ t: 'invoke', id, ch, args } satisfies WsMsg))
}

function flushDeferred(): void {
  if (!ws || !wsReady) return
  const list = deferredInvokes
  deferredInvokes = []
  for (const d of list) sendInvoke(d.ch, d.args, d.resolve, d.reject)
}

function sweepDeferred(): void {
  const now = Date.now()
  const keep: DeferredInvoke[] = []
  for (const d of deferredInvokes) {
    if (now - d.at < DEFER_MS) keep.push(d)
    else d.reject(new Error('Connection to the app server was lost'))
  }
  deferredInvokes = keep
}

function dispatchEvent(ch: string, args: unknown[]): void {
  const subs = eventSubs.get(ch)
  if (!subs) return
  for (const cb of subs) {
    try {
      cb(...args)
    } catch {
      /* listener errors must not kill the bridge */
    }
  }
}

export function subscribeWebEvent(ch: string, cb: (...args: unknown[]) => void): () => void {
  let set = eventSubs.get(ch)
  if (!set) {
    set = new Set()
    eventSubs.set(ch, set)
  }
  set.add(cb)
  return () => {
    set.delete(cb)
  }
}

function invoke(ch: string, args: unknown[]): Promise<unknown> {
  return new Promise((resolve, reject) => {
    if (!ws || !wsReady) {
      // v1.0.32: park the call for the reconnect window instead of
      // failing straight away; sweepDeferred() rejects it if the socket
      // never comes back.
      if (deferredInvokes.length >= DEFER_MAX) {
        reject(new Error('Connection to the app server was lost'))
        return
      }
      deferredInvokes.push({ ch, args, resolve, reject, at: Date.now() })
      return
    }
    sendInvoke(ch, args, resolve, reject)
  })
}

function connectOnce(): Promise<void> {
  return new Promise((resolve, reject) => {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws'
    const sock = new WebSocket(`${proto}://${location.host}/ws`)
    let settled = false
    sock.onopen = (): void => {
      ws = sock
      wsReady = true
      settled = true
      flushDeferred()
      // v1.0.13: a RE-connect (network blip, server restart) means this
      // client's view may be stale - let the app re-sync the live/busy
      // session state.
      try { window.dispatchEvent(new Event('nexus-ws-open')) } catch {}
      resolve()
    }
    sock.onerror = (): void => {
      if (!settled) {
        settled = true
        reject(new Error('Cannot connect to the app server websocket'))
      }
    }
    sock.onclose = (): void => {
      const wasCurrent = ws === sock
      wsReady = false
      if (wasCurrent) ws = null
      for (const [, p] of pending) p.reject(new Error('Connection closed'))
      pending.clear()
      // v1.0.32: parked invokes get the reconnect window; fail them only
      // if the socket is still down DEFER_MS later.
      if (wasCurrent) window.setTimeout(sweepDeferred, DEFER_MS)
      // auto-reconnect with backoff
      window.setTimeout(() => {
        void connectOnce().catch(() => {
          /* keep retrying silently */
        })
      }, 3000)
    }
    sock.onmessage = (ev: MessageEvent): void => {
      let msg: WsMsg
      try {
        msg = JSON.parse(String(ev.data)) as WsMsg
      } catch {
        return
      }
      if (msg.t === 'res' && msg.id) {
        const p = pending.get(msg.id)
        if (!p) return
        pending.delete(msg.id)
        if (msg.ok) p.resolve(msg.value)
        else p.reject(new Error(msg.error ?? 'Request failed'))
      } else if (msg.t === 'ev' && msg.ch) {
        dispatchEvent(msg.ch, msg.args ?? [])
      }
    }
  })
}

export function isWebMode(): boolean {
  return Boolean((window as unknown as { __NEXUS_WEB__?: boolean }).__NEXUS_WEB__)
}

export async function initWebApi(): Promise<void> {
  await connectOnce()
  const impl = buildApi()
  ;(window as unknown as { api: AppAPI }).api = impl
  ;(window as unknown as { __NEXUS_WEB__: boolean }).__NEXUS_WEB__ = true
}

function buildApi(): AppAPI {
  const scopeArgs = (scope?: string, cwd?: string, remoteId?: string): unknown[] => [scope ?? 'global', cwd ?? '', remoteId ?? '']
  return {
    window: {
      minimize: (): Promise<void> => invoke('window:minimize', []).then(() => undefined),
      maximize: (): Promise<void> => invoke('window:maximize', []).then(() => undefined),
      close: (): Promise<void> => invoke('window:close', []).then(() => undefined)
    },
    config: {
      get: (): Promise<unknown> => invoke('config:get', []),
      save: (config: unknown): Promise<void> => invoke('config:save', [config]).then(() => undefined)
    },
    dialog: {
      openFolder: (): Promise<string | null> => Promise.resolve(null),
      openFile: (): Promise<string | null> => Promise.resolve(null)
    },
    acp: {
      init: (acpPath: string): Promise<void> => invoke('acp:init', [acpPath]).then(() => undefined),
      start: (acpPath: string, cwd: string, remoteId?: string): Promise<void> =>
        invoke('acp:start', [acpPath, cwd, remoteId ?? undefined]).then(() => undefined),
      ensureWorker: (acpPath: string, cwd: string, remoteId?: string): Promise<void> =>
        invoke('acp:ensure-worker', [acpPath, cwd, remoteId ?? undefined]).then(() => undefined),
      stopWorker: (remoteId?: string): Promise<void> =>
        invoke('acp:stop-worker', [remoteId ?? undefined]).then(() => undefined),
      newSession: (cwd: string, remoteId?: string, applyConfig?: AcpApplyConfig): Promise<void> =>
        invoke('acp:new-session', [cwd, remoteId ?? undefined, applyConfig ?? undefined]).then(() => undefined),
      loadSession: (sessionId: string, cwd: string, remoteId?: string, applyConfig?: AcpApplyConfig): Promise<void> =>
        invoke('acp:load-session', [sessionId, cwd, remoteId ?? undefined, applyConfig ?? undefined]).then(() => undefined),
      sendPrompt: (text: string, acpSessionId?: string, remoteId?: string, images?: Array<{ data: string; mimeType: string; uri?: string }>, userMsg?: { id: string; text: string; attachments?: Array<{ path: string; isDir: boolean }> }): Promise<void> =>
        invoke('acp:send-prompt', [text, acpSessionId, remoteId ?? undefined, images ?? [], userMsg ?? undefined]).then(() => undefined),
      cancelSession: (acpSessionId: string, remoteId?: string): Promise<void> =>
        invoke('acp:cancel-session', [acpSessionId, remoteId ?? undefined]).then(() => undefined),
      setConfigOption: (key: string, value: string, sessionId?: string, remoteId?: string): Promise<void> =>
        invoke('acp:set-config-option', [key, value, sessionId, remoteId ?? undefined]).then(() => undefined),
      approveTool: (requestId: string, optionId: string, remoteId?: string): Promise<void> =>
        invoke('acp:approve-tool', [requestId, optionId, remoteId ?? undefined]).then(() => undefined),
      answerQuestion: (requestId: string, answer: string, remoteId?: string): Promise<void> =>
        invoke('acp:answer-question', [requestId, answer, remoteId ?? undefined]).then(() => undefined),
      stop: (): Promise<void> => invoke('acp:stop', []).then(() => undefined),
      isRunning: (remoteId?: string): Promise<boolean> =>
        invoke('acp:is-running', [remoteId ?? undefined]) as Promise<boolean>,
      // v1.0.13: sessions running a turn right now (boot/reconnect sync)
      busySessions: (): Promise<unknown> => invoke('acp:busy-sessions', []),
      refreshSessions: (): Promise<void> => invoke('acp:refresh-sessions', []).then(() => undefined),
      getContextSummary: (messages: Array<{ role: string; content: string }>, remoteId?: string): Promise<string> =>
        invoke('acp:get-context-summary', [messages, remoteId ?? undefined]) as Promise<string>,
      onEvent: (cb: (event: unknown) => void): (() => void) => {
        return subscribeWebEvent('acp:event', cb as (...args: unknown[]) => void)
      }
    },

    stats: {
      vibeSessions: (): Promise<unknown[]> => invoke('stats:vibe-sessions', []) as Promise<unknown[]>,
      vibeSessionLastModel: (sessionId: string): Promise<string | null> =>
        invoke('stats:vibe-session-last-model', [sessionId]) as Promise<string | null>,
      vibeSessionLastConfig: (sessionId: string): Promise<unknown> =>
        invoke('stats:vibe-session-last-config', [sessionId]),
      searchSessionContent: (query: string): Promise<unknown[]> =>
        invoke('stats:search-session-content', [query]) as Promise<unknown[]>,
      vibeSessionMessages: (sessionId: string): Promise<unknown[]> =>
        invoke('stats:vibe-session-messages', [sessionId]) as Promise<unknown[]>,
      sessionPreview: (sessionId: string): Promise<unknown> =>
        invoke('stats:session-preview', [sessionId]),
      remoteSessionPreview: (remoteId: string, sessionId: string): Promise<unknown> =>
        invoke('stats:remote-session-preview', [remoteId, sessionId]),
      vibeSessionMetadata: (sessionId: string): Promise<unknown> =>
        invoke('stats:vibe-session-metadata', [sessionId]),
      vibeSubagentSessions: (parentSessionId: string): Promise<unknown[]> =>
        invoke('stats:vibe-subagent-sessions', [parentSessionId]) as Promise<unknown[]>,
    },
    setup: {
      check: (): Promise<unknown> => invoke('setup:check', []),
      install: (): Promise<unknown> => invoke('setup:install', []),
      onProgress: (cb: (event: { message: string; type: string }) => void): (() => void) => {
        return subscribeWebEvent('setup:progress', cb as (...args: unknown[]) => void)
      }
    },
    fs: {
      listDir: (dir: string, remoteId?: string): Promise<unknown> => invoke('fs:list-dir', [dir, remoteId ?? undefined]),
      homeDir: (remoteId?: string): Promise<unknown> => invoke('fs:home-dir', [remoteId ?? undefined]),
      searchFiles: (rootDir: string, query: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:search-files', [rootDir, query, remoteId ?? undefined]),
      readFile: (filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:read-file', [filePath, remoteId ?? undefined]),
      stat: (filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:stat', [filePath, remoteId ?? undefined]),
      readText: (filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:read-text', [filePath, remoteId ?? undefined]),
      readBinary: (filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:read-binary', [filePath, remoteId ?? undefined]),
      writeText: (filePath: string, content: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:write-text', [filePath, content, remoteId ?? undefined]),
      writeBinary: (filePath: string, base64: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:write-binary', [filePath, base64, remoteId ?? undefined]),
      // v1.0.8: the browser has no host filesystem path for a dropped
      // File - the drop handler falls back to uploading the bytes.
      pathForFile: (_file: File): string | null => null,
      mkdir: (filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:mkdir', [filePath, remoteId ?? undefined]),
      // v1.0.9: file-list context menu actions (save-as is desktop-only;
      // the web client downloads via /api/download or a client-side blob)
      rename: (filePath: string, newName: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:rename', [filePath, newName, remoteId ?? undefined]),
      delete: (filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('fs:delete', [filePath, remoteId ?? undefined]),
      saveAs: (): Promise<unknown> => Promise.resolve({ error: 'save dialog unavailable in the browser', saved: false })
    },
    git: {
      status: (cwd: string, remoteId?: string): Promise<unknown> => invoke('git:status', [cwd, remoteId ?? undefined]),
      log: (cwd: string, max: number, remoteId?: string): Promise<unknown> =>
        invoke('git:log', [cwd, max, remoteId ?? undefined]),
      diffFile: (cwd: string, filePath: string, remoteId?: string): Promise<unknown> =>
        invoke('git:diff-file', [cwd, filePath, remoteId ?? undefined]),
      diffAll: (cwd: string, remoteId?: string): Promise<unknown> =>
        invoke('git:diff-all', [cwd, remoteId ?? undefined]),
      stage: (cwd: string, paths: string[], remoteId?: string): Promise<unknown> =>
        invoke('git:stage', [cwd, paths, remoteId ?? undefined]),
      unstage: (cwd: string, paths: string[], remoteId?: string): Promise<unknown> =>
        invoke('git:unstage', [cwd, paths, remoteId ?? undefined]),
      commit: (cwd: string, message: string, remoteId?: string): Promise<unknown> =>
        invoke('git:commit', [cwd, message, remoteId ?? undefined]),
      push: (cwd: string, remoteId?: string): Promise<unknown> =>
        invoke('git:push', [cwd, remoteId ?? undefined])
    },

    shell: {
      openPath: (path: string): Promise<string> => invoke('shell:open-path', [path]) as Promise<string>
    },
    vibe: {
      getConfig: (scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:get-config', scopeArgs(scope, cwd, remoteId)),
      saveConfig: (config: unknown, scope?: string, cwd?: string, remoteId?: string): Promise<void> =>
        invoke('vibe:save-config', [config, ...scopeArgs(scope, cwd, remoteId)]).then(() => undefined),
      getRawConfig: (scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:get-raw-config', scopeArgs(scope, cwd, remoteId)),
      saveRawConfig: (content: string, scope?: string, cwd?: string, remoteId?: string): Promise<unknown> =>
        invoke('vibe:save-raw-config', [content, ...scopeArgs(scope, cwd, remoteId)]),
      getAgents: (scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:get-agents', scopeArgs(scope, cwd, remoteId)),
      exposeProjectAgent: (name: string, cwd: string, remoteId?: string): Promise<void> =>
        invoke('vibe:expose-project-agent', [name, cwd, remoteId ?? '']).then(() => undefined),
      saveAgent: (name: string, config: unknown, scope?: string, cwd?: string, remoteId?: string): Promise<void> =>
        invoke('vibe:save-agent', [name, config, ...scopeArgs(scope, cwd, remoteId)]).then(() => undefined),
      deleteAgent: (name: string, scope?: string, cwd?: string, remoteId?: string): Promise<void> =>
        invoke('vibe:delete-agent', [name, ...scopeArgs(scope, cwd, remoteId)]).then(() => undefined),
      getPrompts: (scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:get-prompts', scopeArgs(scope, cwd, remoteId)),
      savePrompt: (name: string, content: string, scope?: string, cwd?: string, remoteId?: string): Promise<void> =>
        invoke('vibe:save-prompt', [name, content, ...scopeArgs(scope, cwd, remoteId)]).then(() => undefined),
      deletePrompt: (name: string, scope?: string, cwd?: string, remoteId?: string): Promise<void> =>
        invoke('vibe:delete-prompt', [name, ...scopeArgs(scope, cwd, remoteId)]).then(() => undefined),
      getSkills: (scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:get-skills', scopeArgs(scope, cwd, remoteId)),
      getDir: (scope?: string, cwd?: string, remoteId?: string): Promise<string> =>
        invoke('vibe:get-dir', scopeArgs(scope, cwd, remoteId)) as Promise<string>,
      listConfigFiles: (scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:list-config-files', scopeArgs(scope, cwd, remoteId)),
      createSkill: (name: string, scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:create-skill', [name, ...scopeArgs(scope, cwd, remoteId)]),
      deleteSkill: (name: string, scope?: string, cwd?: string, remoteId?: string): Promise<unknown> => invoke('vibe:delete-skill', [name, ...scopeArgs(scope, cwd, remoteId)]),
      writeSkillFile: (name: string, relPath: string, content: string, scope?: string, cwd?: string, remoteId?: string): Promise<unknown> =>
        invoke('vibe:write-skill-file', [name, relPath, content, ...scopeArgs(scope, cwd, remoteId)]),
      deleteSkillFile: (name: string, relPath: string, scope?: string, cwd?: string, remoteId?: string): Promise<unknown> =>
        invoke('vibe:delete-skill-file', [name, relPath, ...scopeArgs(scope, cwd, remoteId)]),
      importSkill: (src: string, isZip: boolean, scope?: string, cwd?: string, remoteId?: string): Promise<unknown> =>
        invoke('vibe:import-skill', [src, isZip, ...scopeArgs(scope, cwd, remoteId)])
    },
    serve: {
      status: (): Promise<unknown> => invoke('serve:status', []),
      start: (): Promise<unknown> => invoke('serve:start', []),
      stop: (): Promise<unknown> => invoke('serve:stop', []),
      saveConfig: (patch: unknown): Promise<unknown> => invoke('serve:save-config', [patch]),
      setCredentials: (username: string, password: string): Promise<unknown> =>
        invoke('serve:set-credentials', [username, password]),
      regenCert: (): Promise<unknown> => invoke('serve:regen-cert', [])
    },
    remote: {
      test: (draft: unknown): Promise<unknown> => invoke('remote:test', [draft]),
      installAcp: (id: string): Promise<unknown> => invoke('remote:install-acp', [id]),
      sessions: (id: string): Promise<unknown> => invoke('remote:sessions', [id]),
      statusNow: (ids?: string[]): Promise<unknown> => invoke('remote:status-now', [ids ?? null]),
      save: (conn: unknown): Promise<unknown> => invoke('remote:save', [conn]),
      delete: (id: string): Promise<unknown> => invoke('remote:delete', [id])
    },
    queue: {
      add: (item: { id: string; acpSessionId: string; remoteId: string | null; cwd: string; text: string; displayText?: string; queuedAt?: number; userMsg?: { id: string; text: string } }): Promise<unknown> =>
        invoke('queue:add', [item]),
      list: (): Promise<unknown> => invoke('queue:list', []),
      remove: (id: string): Promise<unknown> => invoke('queue:remove', [id]),
      clear: (): Promise<unknown> => invoke('queue:clear', []),
      onChanged: (cb: (items: unknown) => void): (() => void) => subscribeWebEvent('queue:changed', cb as (...args: unknown[]) => void)
    },
    updates: {
      checkApp: (): Promise<unknown> => invoke('updates:check-app', []),
      checkCli: (): Promise<unknown> => invoke('updates:check-cli', []),
      checkCliRemote: (remoteId: string): Promise<unknown> => invoke('updates:check-cli-remote', [remoteId]),
      updateCli: (remoteId?: string): Promise<unknown> => invoke('updates:update-cli', [remoteId ?? null]),
      updateApp: (): Promise<unknown> => invoke('updates:update-app', []),
      onProgress: (cb: (p: unknown) => void): (() => void) =>
        subscribeWebEvent('updates:progress', cb as (...args: unknown[]) => void)
    }
  } as unknown as AppAPI
}

// v0.13.14: download a file from the serve host through the authenticated
// /api/download endpoint (web client "Download" action in the preview dock).
export async function downloadFile(filePath: string): Promise<void> {
  const res = await fetch('/api/download?path=' + encodeURIComponent(filePath), { credentials: 'include' })
  if (!res.ok) throw new Error('Download failed: HTTP ' + res.status)
  const blob = await res.blob()
  const name = filePath.split('/').filter(Boolean).pop() || 'file'
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = name
  document.body.appendChild(a)
  a.click()
  a.remove()
  URL.revokeObjectURL(url)
}
