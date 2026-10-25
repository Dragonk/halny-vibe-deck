import { spawn, ChildProcess } from 'child_process'
import { BrowserWindow } from 'electron'
import * as readline from 'readline'
import { randomUUID } from 'crypto'
import { broadcastToWeb } from './bus'

export interface AcpConfigOptionChoice {
  value: string
  name: string
  description?: string
}

export interface AcpConfigOption {
  id: string
  name: string
  description?: string
  category?: string
  type: 'select' | 'boolean'
  currentValue: string | boolean
  options: AcpConfigOptionChoice[]
}

export interface AcpHistoryToolCall {
  id: string
  title: string
  kind: string
  status: 'running' | 'done' | 'error'
  input?: Record<string, unknown>
  output?: string
  diffs?: Array<{ path: string; oldText: string | null; newText: string }>
}

export interface AcpHistoryMessage {
  role: 'user' | 'ai'
  content: string
  thinking?: Array<{ id: string; content: string }>
  toolCalls?: AcpHistoryToolCall[]
}

export interface AcpEvent {
  type:
    | 'InitializationComplete'
    | 'SessionsList'
    | 'SessionInitialized'
    | 'SessionHistory'
    | 'SessionLoadProgress'
    | 'SessionInfoUpdate'
    | 'MessageChunk'
    | 'ThinkingChunk'
    | 'ThinkingComplete'
    | 'PromptComplete'
    | 'ToolCall'
    | 'ToolCallUpdate'
    | 'ToolApprovalRequest'
    | 'AskUserQuestion'
    | 'Plan'
    | 'AvailableCommandsUpdate'
    | 'UsageUpdate'
    | 'ConfigOptionsUpdate'
    | 'Error'
    | 'DebugLog'
    | 'Disconnected'
  data: Record<string, unknown>
}

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (err: Error) => void
}

type AcpMode = { id: string; name: string; description?: string }

/** One buffered raw session/update notification (replay during session/load). */
type RawUpdate = Record<string, unknown>

const REQUEST_TIMEOUT_MS = 180_000

export class AcpManager {
  private process: ChildProcess | null = null
  private pending = new Map<string, PendingRequest>()
  private currentSessionId: string | null = null
  private win: BrowserWindow | null = null
  /** set when this instance drives a remote vibe-acp over ssh */
  private remote: { sshCmd: string } | null = null
  /** v0.7.0: id of the remote connection this instance drives (null = local) */
  private remoteTag: string | null = null
  private _pendingSummaryChunks: string[] | null = null
  private _pendingSummaryId: string | null = null
  /** session/load in flight for these ACP session ids — replay is buffered, not emitted */
  private _replaying = new Map<string, RawUpdate[]>()

  setWindow(win: BrowserWindow | null): void {
    this.win = win
  }

  /** v0.7.0: tag events with the remote connection id (multi-worker support) */
  setRemoteTag(id: string | null): void {
    this.remoteTag = id
  }

  private emit(event: AcpEvent): void {
    // v0.7.0: tag every event with the remote connection it came from so the
    // renderer can run several ACP workers (local + remotes) at once.
    const ev: AcpEvent =
      this.remoteTag !== null
        ? { type: event.type, data: { ...event.data, remoteId: this.remoteTag } }
        : event
    broadcastToWeb('acp:event', [ev])
    if (process.env.NEXUS_DEBUG_EVENTS) {
      const dbg = ev as unknown as Record<string, unknown>
      console.log('[ACP] <<<', String(dbg.type), JSON.stringify(dbg.data ?? {}).slice(0, 160))
    }
    if (this.win && !this.win.isDestroyed()) {
      this.win.webContents.send('acp:event', ev)
    }
  }

  private sendRequest(method: string, params: unknown): Promise<unknown> {
    return new Promise((resolve, reject) => {
      if (!this.process || !this.process.stdin) {
        reject(new Error('ACP process not running'))
        return
      }
      const id = randomUUID()
      const msg = JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n'
      console.log('[ACP] >>> method:', method, 'id:', id)
      this.pending.set(id, { resolve, reject })

      const timeout = setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id)
          reject(new Error('ACP request timeout: ' + method))
        }
      }, REQUEST_TIMEOUT_MS)

      this.pending.get(id)!.resolve = (v) => {
        clearTimeout(timeout)
        resolve(v)
      }
      this.pending.get(id)!.reject = (e) => {
        clearTimeout(timeout)
        reject(e)
      }

      this.process.stdin!.write(msg)
    })
  }

  private handleLine(line: string): void {
    if (!line.trim()) return
    let msg: Record<string, unknown>
    try {
      msg = JSON.parse(line)
    } catch {
      this.emit({ type: 'DebugLog', data: { message: 'parse error: ' + line.slice(0, 200) } })
      return
    }

    const id = msg.id as string | undefined
    const method = msg.method as string | undefined

    // Response to our request
    if (id && !method && (msg.result !== undefined || msg.error !== undefined)) {
      const pending = this.pending.get(id)
      if (pending) {
        this.pending.delete(id)
        if (msg.error) {
          const err = msg.error as { code: number; message: string }
          pending.reject(new Error('[' + err.code + '] ' + err.message))
        } else {
          if (id === this._pendingSummaryId && this._pendingSummaryChunks) {
            const summary = this._pendingSummaryChunks.join('')
            this._pendingSummaryChunks = null
            this._pendingSummaryId = null
            pending.resolve(summary)
          } else {
            pending.resolve(msg.result)
          }
        }
      }
      return
    }

    // Agent request to client (has id + method)
    if (id !== undefined && method) {
      this.handleAgentRequest(id, method, msg.params as Record<string, unknown>)
      return
    }

    // Notification (no id, has method)
    if (id === undefined && method === 'session/update') {
      this.handleSessionUpdate(msg.params as Record<string, unknown>)
    }
  }


  // ---------------------------------------------------------------------------
  // Config options helpers
  // ---------------------------------------------------------------------------

  private parseConfigOptions(
    raw: unknown
  ): AcpConfigOption[] {
    if (!Array.isArray(raw)) return []
    const out: AcpConfigOption[] = []
    for (const item of raw as Array<Record<string, unknown>>) {
      const id = (item.id ?? item.configId) as string | undefined
      if (!id) continue
      const type = (item.type as string) === 'boolean' ? 'boolean' : 'select'
      const optionsRaw = (item.options ?? []) as Array<Record<string, unknown>>
      out.push({
        id,
        name: (item.name as string) ?? id,
        description: item.description as string | undefined,
        category: (item.category as string) ?? undefined,
        type,
        currentValue: (item.currentValue as string | boolean) ?? (type === 'boolean' ? false : ''),
        options: optionsRaw.map((o) => ({
          value: String(o.value ?? ''),
          name: (o.name as string) ?? String(o.value ?? ''),
          description: o.description as string | undefined
        }))
      })
    }
    return out
  }

  private parseModes(result: Record<string, unknown>): {
    modes: AcpMode[]
    currentModeId: string | null
  } {
    const modesRaw = (result.modes as Record<string, unknown> | undefined) ?? {}
    const available = (modesRaw.availableModes as AcpMode[] | undefined) ?? []
    const modes = available.map((m) => ({ id: m.id, name: m.name, description: m.description }))
    const currentModeId = (modesRaw.currentModeId as string | undefined) ?? (modes.length > 0 ? modes[0].id : null)
    return { modes, currentModeId }
  }

  // ---------------------------------------------------------------------------
  // session/update handling (live + replay)
  // ---------------------------------------------------------------------------

  private contentText(content: unknown): string {
    if (typeof content === 'string') return content
    if (content && typeof content === 'object') {
      const c = content as Record<string, unknown>
      if (typeof c.text === 'string') return c.text
      if (typeof c.thinking === 'string') return c.thinking
    }
    return ''
  }

  /** Extract displayable text out of a tool call content/rawOutput payload. */
  private toolOutputText(update: Record<string, unknown>): string | undefined {
    const rawOutput = update.rawOutput ?? update.raw_output ?? update.output
    const content = update.content
    const parts: string[] = []

    if (Array.isArray(content)) {
      for (const block of content as Array<Record<string, unknown>>) {
        if (block.type === 'content' || block.type === 'tool_error') {
          const inner = this.contentText(block.content)
          if (inner) parts.push(inner)
        } else if (block.type === 'file_edit' || block.type === 'terminal') {
          const inner = this.contentText(block.content)
          if (inner) parts.push(inner)
        }
      }
    } else if (content && typeof content === 'object') {
      const inner = this.contentText(content)
      if (inner) parts.push(inner)
    }

    if (parts.length === 0) {
      if (typeof rawOutput === 'string') return rawOutput
      if (rawOutput !== undefined && rawOutput !== null) {
        try {
          return JSON.stringify(rawOutput, null, 2)
        } catch {
          return String(rawOutput)
        }
      }
    }
    return parts.length > 0 ? parts.join('\n') : undefined
  }

  /** Extract ACP diff content blocks ({type:'diff', path, oldText, newText}) from a tool update. */
  private diffBlocks(
    update: Record<string, unknown>
  ): Array<{ path: string; oldText: string | null; newText: string }> | undefined {
    const content = update.content as unknown
    if (!Array.isArray(content)) return undefined
    const diffs: Array<{ path: string; oldText: string | null; newText: string }> = []
    for (const block of content as Array<Record<string, unknown>>) {
      if (!block || typeof block !== 'object') continue
      if (block.type !== 'diff') continue
      const p = block.path
      if (typeof p !== 'string' || p.length === 0) continue
      const oldText = (block.oldText ?? block.old_text) as string | null
      const newText = (block.newText ?? block.new_text) as string | undefined
      if (newText === undefined) continue
      diffs.push({ path: p, oldText: oldText ?? null, newText })
    }
    return diffs.length > 0 ? diffs : undefined
  }
  private normalizeToolStatus(s: unknown): 'running' | 'done' | 'error' {
    if (s === 'completed' || s === 'done' || s === 'success') return 'done'
    if (s === 'failed' || s === 'error') return 'error'
    return 'running'
  }

  private parseRawInput(raw: unknown): Record<string, unknown> | undefined {
    if (!raw) return undefined
    if (typeof raw === 'string') {
      try {
        return JSON.parse(raw)
      } catch {
        return undefined
      }
    }
    if (typeof raw === 'object') return raw as Record<string, unknown>
    return undefined
  }

  private handleSessionUpdate(params: Record<string, unknown>): void {
    const sessionId = params.sessionId as string
    const update = params.update as Record<string, unknown>
    if (!update) return

    const sessionUpdate = update.sessionUpdate as string | undefined
    const type = update.type as string | undefined

    // Replay buffering: while session/load is in flight for this session,
    // hold the raw update; assembly happens when the response arrives.
    if (this._replaying.has(sessionId)) {
      const buffer = this._replaying.get(sessionId)!
      buffer.push(params)
      if (buffer.length % 25 === 0) {
        this.emit({ type: 'SessionLoadProgress', data: { sessionId, count: buffer.length } })
      }
      return
    }

    switch (sessionUpdate) {
      case 'user_message_chunk':
        // Live user echo — the renderer already displays the message it sent.
        break

      case 'agent_message_chunk': {
        const text = this.contentText(update.content)
        const messageId = (update.messageId as string | undefined) ?? 'current'
        if (text) {
          this.emit({ type: 'MessageChunk', data: { sessionId, text, messageId } })
        }
        break
      }

      case 'agent_thought_chunk': {
        const text = this.contentText(update.content)
        const blockId = (update.messageId as string | undefined) ?? 'thinking'
        if (text) {
          this.emit({ type: 'ThinkingChunk', data: { sessionId, blockId, text } })
        }
        break
      }

      case 'agent_thought_end':
        this.emit({ type: 'ThinkingComplete', data: { sessionId, blockId: 'thinking' } })
        break

      case 'tool_call': {
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined
        const toolName = (update.title ?? update.name) as string | undefined
        if (toolCallId && toolName) {
          this.toolCallNames.set(toolCallId, toolName)
          this.emit({
            type: 'ToolCall',
            data: {
              sessionId,
              toolCallId,
              title: toolName,
              kind: (update.kind ?? 'execute') as string,
              status: this.normalizeToolStatus(update.status ?? 'in_progress'),
              input: this.parseRawInput(update.rawInput ?? update.raw_input),
              diffs: this.diffBlocks(update)
            }
          })
        }
        break
      }

      case 'tool_call_update': {
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined
        if (toolCallId) {
          this.emit({
            type: 'ToolCallUpdate',
            data: {
              sessionId,
              toolCallId,
              status: this.normalizeToolStatus(update.status),
              output: this.toolOutputText(update),
              diffs: this.diffBlocks(update)
            }
          })
        }
        break
      }

      case 'plan':
      case 'plan_update':
        this.emit({ type: 'Plan', data: { sessionId, entries: (update.entries as unknown[]) ?? [] } })
        break

      case 'available_commands_update': {
        const commands = (update.availableCommands as Array<Record<string, unknown>>) ?? []
        this.emit({
          type: 'AvailableCommandsUpdate',
          data: {
            sessionId,
            commands: commands.map((c) => ({
              name: c.name as string,
              description: (c.description as string) ?? '',
              hint: ((c.input as Record<string, unknown> | undefined)?.hint as string) ?? undefined
            }))
          }
        })
        break
      }

      case 'config_option_update': {
        const options = this.parseConfigOptions(update.configOptions)
        if (options.length > 0) {
          this.emit({ type: 'ConfigOptionsUpdate', data: { sessionId, configOptions: options } })
        }
        break
      }

      case 'current_mode_update': {
        const modeId = (update.currentModeId as string) ?? undefined
        if (modeId) {
          this.emit({ type: 'ConfigOptionsUpdate', data: { sessionId, modeChanged: modeId } })
        }
        break
      }

      case 'session_info_update': {
        const title = (update.title as string) ?? undefined
        this.emit({ type: 'SessionInfoUpdate', data: { sessionId, title } })
        break
      }

      case 'usage_update': {
        const meta = (update._meta ?? update.meta) as Record<string, unknown> | undefined
        const cost = (update.cost as Record<string, unknown> | undefined) ?? undefined
        this.emit({
          type: 'UsageUpdate',
          data: {
            sessionId,
            contextUsed: (update.used as number) ?? 0,
            contextSize: (update.size as number) ?? 0,
            cost: cost ? { amount: cost.amount as number, currency: (cost.currency as string) ?? 'USD' } : undefined,
            steps: (meta?.steps as number) ?? undefined,
            promptTokens: (meta?.promptTokens as number) ?? undefined,
            completionTokens: (meta?.completionTokens as number) ?? undefined,
            cachedTokens: (meta?.cachedTokens as number) ?? undefined,
            totalTokens: (meta?.totalTokens as number) ?? undefined,
            tokensPerSecond: (meta?.tokensPerSecond as number) ?? undefined,
            lastTurnDuration: (meta?.lastTurnDuration as number) ?? undefined,
            lastTurnTotalTokens: (meta?.lastTurnTotalTokens as number) ?? undefined
          }
        })
        break
      }

      default: {
        // Legacy/alternative protocol shapes
        if (type === 'agent_message_chunk') {
          const chunk = update.chunk as Record<string, unknown> | undefined
          const text = this.contentText(chunk)
          if (text) {
            if (this._pendingSummaryChunks) this._pendingSummaryChunks.push(text)
            else this.emit({ type: 'MessageChunk', data: { sessionId, text } })
          }
        } else {
          this.emit({ type: 'DebugLog', data: { message: 'unknown update: ' + (sessionUpdate ?? type) } })
        }
      }
    }
  }


  // ---------------------------------------------------------------------------
  // Replay assembly (real session resume)
  // ---------------------------------------------------------------------------

  private assembleHistory(rawUpdates: RawUpdate[]): { messages: AcpHistoryMessage[]; title?: string } {
    const messages: AcpHistoryMessage[] = []
    const toolCallsById = new Map<string, AcpHistoryToolCall>()
    let currentAi: AcpHistoryMessage | null = null

    const ensureAi = (): AcpHistoryMessage => {
      if (!currentAi) {
        currentAi = { role: 'ai', content: '', thinking: [], toolCalls: [] }
        messages.push(currentAi)
      }
      return currentAi
    }

    for (const params of rawUpdates) {
      const update = (params.update as Record<string, unknown>) ?? {}
      const sessionUpdate = update.sessionUpdate as string | undefined
      const type = update.type as string | undefined
      const kind = sessionUpdate ?? type

      if (kind === 'user_message_chunk') {
        const text = this.contentText(update.content) || this.contentText(update.chunk)
        if (text) {
          messages.push({ role: 'user', content: text })
          currentAi = null
        }
      } else if (kind === 'agent_message_chunk') {
        const text = this.contentText(update.content) || this.contentText(update.chunk)
        if (text) {
          ensureAi().content += text
        }
      } else if (kind === 'agent_thought_chunk') {
        const text = this.contentText(update.content) || this.contentText(update.chunk)
        if (text) {
          const ai = ensureAi()
          const blocks = (ai.thinking ??= [])
          const last = blocks[blocks.length - 1]
          if (last) {
            last.content += text
          } else {
            blocks.push({ id: 'thinking', content: text })
          }
        }
      } else if (kind === 'tool_call') {
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined
        const toolName = (update.title ?? update.name) as string | undefined
        if (toolCallId && toolName) {
          const tc: AcpHistoryToolCall = {
            id: toolCallId,
            title: toolName,
            kind: (update.kind as string) ?? 'execute',
            status: this.normalizeToolStatus(update.status ?? 'in_progress'),
            input: this.parseRawInput(update.rawInput ?? update.raw_input),
            diffs: this.diffBlocks(update)
          }
          toolCallsById.set(toolCallId, tc)
          ensureAi().toolCalls!.push(tc)
        }
      } else if (kind === 'tool_call_update') {
        const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined
        if (toolCallId) {
          const tc = toolCallsById.get(toolCallId)
          if (tc) {
            const out = this.toolOutputText(update)
            if (out !== undefined) tc.output = out
            const rdiffs = this.diffBlocks(update)
            if (rdiffs) tc.diffs = rdiffs
            tc.status = this.normalizeToolStatus(update.status)
          }
        }
      }
    }

    // Session title: the last session_info_update that carries one
    const lastInfo = rawUpdates
      .map((p) => (p.update as Record<string, unknown>) ?? {})
      .filter((u) => (u.sessionUpdate ?? u.type) === 'session_info_update' && typeof u.title === 'string')
      .map((u) => u.title as string)
    const title = lastInfo.length > 0 ? lastInfo[lastInfo.length - 1] : undefined

    // Drop empty AI wrappers (only thinking/toolcalls present is fine, but no
    // content at all and no calls means nothing to show)
    return { messages, title }
  }

  // ---------------------------------------------------------------------------
  // Agent → client requests (approvals, questions)
  // ---------------------------------------------------------------------------

  private pendingApprovals = new Map<string, boolean>()
  private toolCallNames = new Map<string, string>()

  private handleAgentRequest(id: string, method: string, params: unknown): void {
    const isQuestion =
      method === 'ask_user_question' || method.includes('ask_user') || method === 'user/question'
    if (isQuestion) {
      const p = params as Record<string, unknown>
      this.emit({
        type: 'AskUserQuestion',
        data: {
          requestId: id,
          question: (p?.question as string) ?? (p?.prompt as string) ?? '',
          choices: (p?.choices ?? p?.options ?? []) as string[]
        }
      })
      this.pendingApprovals.set(id, false)
      return
    }

    const needsApproval = method.includes('approv') || method.includes('permission') || method === 'tool/call'
    if (needsApproval) {
      const p = params as Record<string, unknown>
      const toolCallId = (
        (p?.toolCall as Record<string, unknown> | undefined)?.toolCallId as string | undefined
      )
      const toolName = toolCallId ? this.toolCallNames.get(toolCallId) ?? undefined : undefined
      this.emit({
        type: 'ToolApprovalRequest',
        data: { requestId: id, method, params: { ...(p ?? {}), resolvedToolName: toolName } }
      })
      this.pendingApprovals.set(id, false)
    } else {
      // Auto-approve protocol-level requests
      const response = JSON.stringify({ jsonrpc: '2.0', id, result: { approved: true } }) + '\n'
      this.process?.stdin?.write(response)
    }
  }

  answerUserQuestion(requestId: string, answer: string): void {
    if (!this.pendingApprovals.has(requestId)) return
    this.pendingApprovals.delete(requestId)
    const response = JSON.stringify({ jsonrpc: '2.0', id: requestId, result: { answer } }) + '\n'
    this.process?.stdin?.write(response)
  }

  approveToolRequest(requestId: string, optionId: string): void {
    if (!this.pendingApprovals.has(requestId)) return
    this.pendingApprovals.delete(requestId)
    const response = JSON.stringify({
      jsonrpc: '2.0',
      id: requestId,
      result: {
        outcome: {
          outcome: 'selected',
          option_id: optionId
        }
      }
    }) + '\n'
    this.process?.stdin?.write(response)
  }


  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  async start(acpPath: string, remote?: { sshCmd: string }): Promise<void> {
    if (this.process) await this.stop()

    this.remote = remote ?? null
    let proc: ChildProcess
    if (remote) {
      const tokens = remote.sshCmd.trim().split(/\s+/).filter(Boolean)
      if (tokens.length === 0) throw new Error('Empty ssh command for remote connection')
      console.log('[ACP] remote start over ssh:', remote.sshCmd)
      // Login shell (-l) so the remote PATH includes ~/.local/bin (uv tools).
      proc = spawn(tokens[0], [...tokens.slice(1), 'bash -lc "exec vibe-acp"'], {
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true
      })
    } else {
      proc = spawn(acpPath, [], {
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true
      })
    }
    this.process = proc

    const rl = readline.createInterface({ input: proc.stdout! })
    rl.on('line', (line) => this.handleLine(line))

    proc.stderr!.on('data', (d: Buffer) => {
      this.emit({ type: 'DebugLog', data: { message: 'stderr: ' + d.toString().trim() } })
    })

    proc.on('exit', (code) => {
      if (this.process === proc) {
        this.process = null
        this.currentSessionId = null
        this.emit({ type: 'Disconnected', data: { code: code ?? -1 } })
      }
    })

    // Initialize handshake (no clientCapabilities — vibe-acp rejects unexpected fields)
    const result = (await this.sendRequest('initialize', {
      protocolVersion: 1,
      clientInfo: { name: 'Mistral Vibe Nexus', version: '0.3.0' }
    })) as Record<string, unknown>

    this.emit({
      type: 'InitializationComplete',
      data: { protocolVersion: result.protocolVersion ?? 1 }
    })

    await this.listSessions()
  }

  async listSessions(): Promise<void> {
    try {
      const result = (await this.sendRequest('session/list', {})) as Record<string, unknown>
      const raw = (result.sessions ?? result.items ?? []) as Array<Record<string, unknown>>
      const sessions = raw
        .map((s) => ({
          sessionId: (s.sessionId ?? s.id ?? '') as string,
          title: (s.title ?? s.name ?? '') as string,
          cwd: (s.cwd ?? s.workingDirectory ?? '') as string,
          updatedAt: (s.updatedAt ?? s.updated_at ?? s.startTime ?? '') as string
        }))
        .filter((s) => s.sessionId)
        // Newest first — the sidebar relies on this order
        .sort(
          (a, b) =>
            (b.updatedAt ? new Date(b.updatedAt).getTime() : 0) -
            (a.updatedAt ? new Date(a.updatedAt).getTime() : 0)
        )
      this.emit({ type: 'SessionsList', data: { sessions } })
    } catch {
      this.emit({ type: 'SessionsList', data: { sessions: [] } })
    }
  }

  async newSession(cwd: string): Promise<void> {
    const result = (await this.sendRequest('session/new', {
      cwd,
      mcpServers: []
    })) as Record<string, unknown>

    this.currentSessionId = result.sessionId as string
    const { modes, currentModeId } = this.parseModes(result)
    const configOptions = this.parseConfigOptions(result.configOptions)

    this.emit({
      type: 'SessionInitialized',
      data: {
        sessionId: this.currentSessionId,
        modes,
        currentModeId,
        configOptions
      }
    })
  }

  async loadSession(sessionId: string, cwd: string): Promise<void> {
    console.log('[ACP] loadSession:', sessionId, 'cwd:', cwd)
    // Buffer replayed updates while the request is in flight; vibe-acp replays
    // the whole history via session/update BEFORE the session/load response
    // arrives, so emitting them live would lose them.
    this._replaying.set(sessionId, [])
    try {
      const result = (await this.sendRequest('session/load', {
        sessionId,
        cwd,
        mcpServers: []
      })) as Record<string, unknown>

      this.currentSessionId = sessionId

      const buffered = this._replaying.get(sessionId) ?? []
      this._replaying.delete(sessionId)
      const { messages, title } = this.assembleHistory(buffered)

      const { modes, currentModeId } = this.parseModes(result)
      const configOptions = this.parseConfigOptions(result.configOptions)

      this.emit({
        type: 'SessionHistory',
        data: { sessionId, messages, title }
      })
      this.emit({
        type: 'SessionInitialized',
        data: { sessionId, modes, currentModeId, configOptions, sessionTitle: title }
      })
    } catch (err) {
      this._replaying.delete(sessionId)
      throw err
    }
  }

  async sendPrompt(text: string, acpSessionId?: string): Promise<void> {
    const sid = acpSessionId ?? this.currentSessionId
    if (!sid) throw new Error('No active session')
    if (sid !== this.currentSessionId) {
      this.currentSessionId = sid
    }
    const result = (await this.sendRequest('session/prompt', {
      sessionId: sid,
      prompt: [{ type: 'text', text }]
    })) as Record<string, unknown>

    this.emit({
      type: 'PromptComplete',
      data: {
        sessionId: sid,
        stopReason: (result?.stopReason as string) ?? 'end_turn'
      }
    })
  }

  cancelSession(acpSessionId: string): void {
    if (!this.process?.stdin) return
    const notification = JSON.stringify({
      jsonrpc: '2.0',
      method: 'session/cancel',
      params: { sessionId: acpSessionId }
    }) + '\n'
    this.process.stdin.write(notification)
  }

  async setConfigOption(key: string, value: string, sessionId?: string): Promise<void> {
    const sid = sessionId ?? this.currentSessionId
    if (!sid) return
    console.log('[ACP] setConfigOption:', key, '=', value, 'sessionId:', sid)
    await this.sendRequest('session/set_config_option', {
      sessionId: sid,
      configId: key,
      value
    })
  }

  async getContextSummary(messages: Array<{ role: string; content: string }>): Promise<string> {
    if (!this.process || !this.currentSessionId) throw new Error('ACP not ready')

    return new Promise((resolve, reject) => {
      const summaryRequestId = randomUUID()
      let settled = false

      const cleanup = (): void => {
        this.pending.delete(summaryRequestId)
      }

      const timeout = setTimeout(() => {
        if (!settled) {
          settled = true
          cleanup()
          reject(new Error('Context summary timeout'))
        }
      }, REQUEST_TIMEOUT_MS)

      this.pending.set(summaryRequestId, {
        resolve: (value: unknown) => {
          if (!settled) {
            settled = true
            clearTimeout(timeout)
            cleanup()
            resolve(value as string)
          }
        },
        reject: (err: Error) => {
          if (!settled) {
            settled = true
            clearTimeout(timeout)
            cleanup()
            reject(err)
          }
        }
      })

      this._pendingSummaryChunks = []
      this._pendingSummaryId = summaryRequestId

      const formattedMessages = messages
        .map((m) => m.role.toUpperCase() + ': ' + m.content)
        .join('\n')

      const summaryPrompt =
        'Please briefly summarize the following conversation for context. Focus on key topics, decisions, and current state. Be concise (2-3 sentences max).\n\n' +
        formattedMessages

      const msg = JSON.stringify({
        jsonrpc: '2.0',
        id: summaryRequestId,
        method: 'session/prompt',
        params: {
          sessionId: this.currentSessionId,
          prompt: [{ type: 'text', text: summaryPrompt }]
        }
      }) + '\n'

      this.process!.stdin!.write(msg)
    })
  }

  async stop(): Promise<void> {
    if (this.process) {
      this.process.removeAllListeners()
      this.process.stdout?.removeAllListeners()
      this.process.stderr?.removeAllListeners()
      this.process.kill()
      this.process = null
      this.currentSessionId = null
      this._replaying.clear()
    }
    this.pending.forEach((p) => p.reject(new Error('ACP stopped')))
    this.pending.clear()
  }

  isRemote(): boolean {
    return this.remote !== null
  }

  isRunning(): boolean {
    return this.process !== null
  }
}

export const acpManager = new AcpManager()

