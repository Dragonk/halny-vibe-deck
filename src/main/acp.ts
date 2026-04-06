import { spawn, ChildProcess } from 'child_process'
import { BrowserWindow } from 'electron'
import * as readline from 'readline'
import { randomUUID } from 'crypto'

export interface AcpEvent {
  type:
    | 'InitializationComplete'
    | 'SessionsList'
    | 'SessionInitialized'
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
    | 'Error'
    | 'DebugLog'
    | 'Disconnected'
  data: Record<string, unknown>
}

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (err: Error) => void
}

type AcpMode = { id: string; name: string }
type AcpModel = { modelId: string }

export class AcpManager {
  private process: ChildProcess | null = null
  private pending = new Map<string, PendingRequest>()
  private currentSessionId: string | null = null
  private win: BrowserWindow | null = null
  private _pendingSummaryChunks: string[] | null = null
  private _pendingSummaryId: string | null = null
  private _resumeNotSupported = false
  // ACP sessions currently loading from disk — suppress replay notifications for these
  private _suppressedSessions = new Set<string>()

  setWindow(win: BrowserWindow): void {
    this.win = win
  }

  private emit(event: AcpEvent): void {
    if (this.win && !this.win.isDestroyed()) {
      this.win.webContents.send('acp:event', event)
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
      console.log('[ACP] >>> method:', method, 'id:', id, 'pending.size:', this.pending.size + 1)
      this.pending.set(id, { resolve, reject })

      const timeout = setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id)
          reject(new Error(`ACP request timeout: ${method}`))
        }
      }, 120_000)

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
    console.log('[ACP] <<<', line.substring(0, 200))
    let msg: Record<string, unknown>
    try {
      msg = JSON.parse(line)
    } catch {
      this.emit({ type: 'DebugLog', data: { message: `parse error: ${line}` } })
      return
    }

    const id = msg.id as string | undefined
    const method = msg.method as string | undefined

    // Response to our request
    if (id && !method && (msg.result !== undefined || msg.error !== undefined)) {
      console.log('[ACP] Response for id:', id, 'pending.size:', this.pending.size)
      const pending = this.pending.get(id)
      if (pending) {
        this.pending.delete(id)
        if (msg.error) {
          const err = msg.error as { code: number; message: string }
          pending.reject(new Error(`[${err.code}] ${err.message}`))
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

    // Agent request to client (has id + method) — auto-approve all
    if (id !== undefined && method) {
      console.log('[ACP] Agent request:', method, 'id:', id)
      this.handleAgentRequest(id, method, msg.params as Record<string, unknown>)
      return
    }

    // Notification (no id, has method)
    if (id === undefined && method === 'session/update') {
      this.handleSessionUpdate(msg.params as Record<string, unknown>)
    }
  }

  private handleSessionUpdate(params: Record<string, unknown>): void {
    const sessionId = params.sessionId as string
    // Suppress replay notifications while session/load is in progress for this session
    if (this._suppressedSessions.has(sessionId)) return
    const update = params.update as Record<string, unknown>

    const type = update.type as string | undefined
    const kind = update.kind as string | undefined
    const sessionUpdate = update.sessionUpdate as string | undefined

    // Text content updates - check sessionUpdate for thinking vs message
    const content = update.content as Record<string, unknown> | undefined
    if (content?.text) {
      const text = content.text as string
      
      if (sessionUpdate === 'agent_thought_chunk') {
        // This is thinking/reasoning content
        this.emit({
          type: 'ThinkingChunk',
          data: { sessionId, blockId: 'thinking', text }
        })
      } else if (sessionUpdate === 'agent_message_chunk') {
        // This is the actual assistant message
        if (this._pendingSummaryChunks) {
          this._pendingSummaryChunks.push(text)
        } else {
          this.emit({ type: 'MessageChunk', data: { sessionId, text } })
        }
      }
      // Other sessionUpdate values (like tool_call) are handled in their own branches
    }
    
    // Handle thinking end marker
    if (sessionUpdate === 'agent_thought_end') {
      this.emit({
        type: 'ThinkingComplete',
        data: { sessionId, blockId: 'thinking' }
      })
    }

    // Handle available commands update (comes directly in update, not in type field)
    const availableCommands = update.availableCommands as Array<{
      name: string
      description: string
      input?: { hint?: string }
    }> | undefined
    if (availableCommands) {
      this.emit({
        type: 'AvailableCommandsUpdate',
        data: { sessionId, commands: availableCommands }
      })
    }

    // Helper: parse rawInput (may be JSON string or object)
    const parseRawInput = (raw: unknown): Record<string, unknown> | undefined => {
      if (!raw) return undefined
      if (typeof raw === 'string') {
        try { return JSON.parse(raw) } catch { return undefined }
      }
      if (typeof raw === 'object') return raw as Record<string, unknown>
      return undefined
    }

    // Helper: normalize ACP status strings to our internal values
    const normalizeStatus = (s: string): 'running' | 'done' | 'error' => {
      if (s === 'completed' || s === 'done' || s === 'success') return 'done'
      if (s === 'failed' || s === 'error') return 'error'
      return 'running'
    }



    // Tool call start (session_update="tool_call")
    if (sessionUpdate === 'tool_call' || type === 'tool_call') {
      const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined
      const toolName = (update.title ?? update.name) as string | undefined
      const rawInput = update.rawInput ?? update.raw_input

      if (toolCallId && toolName) {
        this.toolCallNames.set(toolCallId, toolName)
        console.log('[ACP] Emitting ToolCall start:', toolName, toolCallId)
        this.emit({
          type: 'ToolCall',
          data: {
            sessionId,
            toolCallId,
            title: toolName,
            kind: (kind ?? update.kind ?? 'execute') as string,
            status: 'running',
            input: parseRawInput(rawInput),
          }
        })
      }
    }

    // Tool call completion (session_update="tool_call_update")
    if (sessionUpdate === 'tool_call_update') {
      const toolCallId = (update.toolCallId ?? update.tool_call_id) as string | undefined
      const rawStatus = (update.status ?? '') as string
      const rawOutput = update.output ?? update.result ?? update.content
      const output = rawOutput === undefined ? undefined
        : typeof rawOutput === 'string' ? rawOutput
        : JSON.stringify(rawOutput, null, 2)
      if (toolCallId) {
        console.log('[ACP] Emitting ToolCallUpdate:', toolCallId, rawStatus)
        this.emit({
          type: 'ToolCallUpdate',
          data: {
            sessionId,
            toolCallId,
            status: normalizeStatus(rawStatus),
            output
          }
        })
      }
      return // don't fall through to switch
    }

    // Switch for type-tagged updates (older/alternative protocol format)
    switch (type) {
      case 'agent_message_chunk': {
        const chunk = update.chunk as Record<string, unknown>
        if (typeof chunk?.text === 'string') {
          if (this._pendingSummaryChunks) {
            this._pendingSummaryChunks.push(chunk.text as string)
          } else {
            this.emit({ type: 'MessageChunk', data: { sessionId, text: chunk.text } })
          }
        }
        // Thinking/reasoning blocks
        if (chunk?.type === 'thinking' && typeof chunk?.thinking === 'string') {
          this.emit({
            type: 'ThinkingChunk',
            data: { sessionId, blockId: chunk.id ?? 'thinking', text: chunk.thinking }
          })
        }
        if (chunk?.type === 'thinking_end') {
          this.emit({
            type: 'ThinkingComplete',
            data: { sessionId, blockId: chunk.id ?? 'thinking' }
          })
        }
        break
      }
      case 'tool_call': {
        // Only emit if not already handled by sessionUpdate branch above
        if (sessionUpdate !== 'tool_call') {
          const tc = update.toolCall as Record<string, unknown>
          const input = tc?.input as Record<string, unknown> | undefined
          const toolName = (tc?.name as string) ?? ''
          const fileChanges = this.extractFileChanges(toolName, input)
          this.emit({
            type: 'ToolCall',
            data: {
              sessionId,
              toolCallId: tc?.id ?? '',
              title: toolName,
              kind: (tc?.type as string) ?? '',
              status: 'running',
              input,
              fileChanges
            }
          })
        }
        break
      }
      case 'tool_call_update': {
        // Only emit if not already handled by sessionUpdate branch above
        if (sessionUpdate !== 'tool_call_update') {
          const tc = update.toolCall as Record<string, unknown>
          const rawSt = (tc?.status as string) ?? ''
          const rawOut = tc?.output ?? tc?.result ?? tc?.content
          const output = rawOut === undefined ? undefined
            : typeof rawOut === 'string' ? rawOut
            : JSON.stringify(rawOut, null, 2)
          const st: 'running' | 'done' | 'error' =
            rawSt === 'completed' || rawSt === 'done' ? 'done'
            : rawSt === 'failed' || rawSt === 'error' ? 'error'
            : 'running'
          this.emit({
            type: 'ToolCallUpdate',
            data: {
              sessionId,
              toolCallId: tc?.id ?? '',
              status: st,
              output
            }
          })
        }
        break
      }
      case 'plan': {
        this.emit({ type: 'Plan', data: { sessionId, entries: update.entries ?? [] } })
        break
      }
      default:
        this.emit({ type: 'DebugLog', data: { message: `unknown update: ${type}` } })
    }
  }

  private extractFileChanges(
    toolName: string,
    input?: Record<string, unknown>
  ): Array<{ path: string; action: string }> | undefined {
    if (!input) return undefined
    const path = (input.path ?? input.file_path ?? input.filename) as string | undefined
    if (!path) return undefined

    const lowerName = toolName.toLowerCase()
    let action: string
    if (lowerName.includes('creat') || lowerName.includes('write') || lowerName.includes('new')) {
      action = 'added'
    } else if (lowerName.includes('delet') || lowerName.includes('remov')) {
      action = 'deleted'
    } else {
      action = 'modified'
    }
    return [{ path, action }]
  }

  private pendingApprovals = new Map<string, boolean>()
  private toolCallNames = new Map<string, string>()

  private handleAgentRequest(id: string, method: string, params: unknown): void {
    // ask_user_question: show question UI and wait for user answer
    const isQuestion = method === 'ask_user_question' || method.includes('ask_user') || method === 'user/question'
    if (isQuestion) {
      const p = params as Record<string, unknown>
      this.emit({
        type: 'AskUserQuestion',
        data: { requestId: id, question: p?.question ?? p?.prompt ?? '', choices: p?.choices ?? p?.options ?? [] }
      })
      this.pendingApprovals.set(id, false)
      return
    }

    // Methods that require explicit user approval
    const needsApproval = method.includes('approv') || method.includes('permission') || method === 'tool/call'
    if (needsApproval) {
      const p = params as Record<string, unknown>
      const toolCallId = (p?.toolCall as Record<string, unknown> | undefined)?.toolCallId as string | undefined
      const toolName = toolCallId ? (this.toolCallNames.get(toolCallId) ?? undefined) : undefined
      this.emit({
        type: 'ToolApprovalRequest',
        data: { requestId: id, method, params: { ...(p ?? {}), resolvedToolName: toolName } }
      })
      // Store as pending — will be resolved by approveToolRequest()
      this.pendingApprovals.set(id, false)
    } else {
      // Auto-approve protocol-level requests (fs/*, terminal/*, initialize, etc.)
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

  async start(acpPath: string): Promise<void> {
    if (this.process) await this.stop()
    this._resumeNotSupported = false

    const proc = spawn(acpPath, [], {
      stdio: ['pipe', 'pipe', 'pipe'],
      windowsHide: true
    })
    this.process = proc

    const rl = readline.createInterface({ input: proc.stdout! })
    rl.on('line', (line) => this.handleLine(line))

    proc.stderr!.on('data', (d: Buffer) => {
      this.emit({ type: 'DebugLog', data: { message: `stderr: ${d.toString().trim()}` } })
    })

    // Guard: only clear `this.process` if the exiting process is still the current one.
    // Without this, stop() → new spawn() → old exit event would null out the new process.
    proc.on('exit', (code) => {
      if (this.process === proc) {
        this.process = null
        this.currentSessionId = null
        this.emit({ type: 'Disconnected', data: { code: code ?? -1 } })
      }
    })

    // Initialize handshake (no clientCapabilities — vibe-acp returns -32603 if unexpected fields present)
    const result = (await this.sendRequest('initialize', {
      protocolVersion: 1,
      clientInfo: { name: 'Mistral Vibe Nexus', version: '0.2.0' }
    })) as Record<string, unknown>

    this.emit({
      type: 'InitializationComplete',
      data: { protocolVersion: result.protocolVersion ?? 1 }
    })

    // Load session history from ACP
    await this.listSessions()
  }

  async listSessions(): Promise<void> {
    try {
      const result = (await this.sendRequest('session/list', {})) as Record<string, unknown>
      const raw = (result.sessions ?? result.items ?? []) as Array<Record<string, unknown>>
      const sessions = raw.map((s) => ({
        sessionId: s.sessionId ?? s.id ?? '',
        name: (s.name ?? s.title ?? '') as string,
        cwd: (s.cwd ?? s.workingDirectory ?? '') as string,
        startTime: (s.startTime ?? s.createdAt ?? '') as string
      })).filter((s) => s.sessionId)
      this.emit({ type: 'SessionsList', data: { sessions } })
    } catch {
      // session/list not supported — emit empty list silently
      this.emit({ type: 'SessionsList', data: { sessions: [] } })
    }
  }

  async newSession(cwd: string, model: string): Promise<void> {
    const result = (await this.sendRequest('session/new', {
      cwd,
      mcpServers: []
    })) as Record<string, unknown>

    this.currentSessionId = result.sessionId as string

    const modes: Array<{ id: string; name: string }> = ((
      (result.modes as Record<string, unknown>)?.availableModes as AcpMode[]
    ) ?? []).map((m) => ({ id: m.id, name: m.name }))

    const models: string[] = ((
      (result.models as Record<string, unknown>)?.availableModels as AcpModel[]
    ) ?? []).map((m) => m.modelId)

    // Try to get session title/name
    let sessionTitle: string | undefined
    try {
      const nameResult = (await this.sendRequest('session/name', {
        sessionId: this.currentSessionId
      })) as Record<string, unknown>
      sessionTitle = nameResult.name as string | undefined
    } catch {
      // Session name not available
    }

    this.emit({
      type: 'SessionInitialized',
      data: { sessionId: this.currentSessionId, modes, models, sessionTitle }
    })

    // Set default model
    if (model && models.length > 0) {
      const target = models.includes(model) ? model : models[0]
      await this.setConfigOption('model', target).catch(() => {})
    }
  }

  async loadSession(sessionId: string, cwd: string, model: string): Promise<void> {
    console.log('[ACP] loadSession:', sessionId, 'cwd:', cwd)
    // Suppress replay notifications during load so renderer doesn't see duplicate old messages
    this._suppressedSessions.add(sessionId)
    try {
      const result = (await this.sendRequest('session/load', {
        sessionId,
        cwd,
        mcpServers: []
      })) as Record<string, unknown>

      this.currentSessionId = sessionId

      const modes: Array<{ id: string; name: string }> = ((
        (result.modes as Record<string, unknown>)?.availableModes as AcpMode[]
      ) ?? []).map((m) => ({ id: m.id, name: m.name }))

      const models: string[] = ((
        (result.models as Record<string, unknown>)?.availableModels as AcpModel[]
      ) ?? []).map((m) => m.modelId)

      this.emit({
        type: 'SessionInitialized',
        data: { sessionId, modes, models, sessionTitle: undefined }
      })

      if (model && models.length > 0) {
        const target = models.includes(model) ? model : models[0]
        await this.setConfigOption('model', target).catch(() => {})
      }
    } finally {
      this._suppressedSessions.delete(sessionId)
    }
  }

  async resumeSession(sessionId: string, cwd: string, model: string): Promise<void> {
    if (this._resumeNotSupported) {
      console.log('[ACP] resumeSession skipped - not supported, creating new session')
      await this.newSession(cwd, model)
      return
    }

    console.log('[ACP] resumeSession called with sessionId:', sessionId, 'cwd:', cwd, 'model:', model)
    try {
      const result = (await this.sendRequest('session/resume', {
        sessionId,
        cwd,
        mcpServers: []
      })) as Record<string, unknown>

      console.log('[ACP] session/resume result:', JSON.stringify(result).substring(0, 200))
      this.currentSessionId = sessionId

      const modes: Array<{ id: string; name: string }> = ((
        (result.modes as Record<string, unknown>)?.availableModes as AcpMode[]
      ) ?? []).map((m) => ({ id: m.id, name: m.name }))

      const models: string[] = ((
        (result.models as Record<string, unknown>)?.availableModels as AcpModel[]
      ) ?? []).map((m) => m.modelId)

      this.emit({
        type: 'SessionInitialized',
        data: { sessionId: this.currentSessionId, modes, models, sessionTitle: undefined }
      })

      if (model && models.length > 0) {
        const target = models.includes(model) ? model : models[0]
        await this.setConfigOption('model', target).catch(() => {})
      }
    } catch (err) {
      const errMsg = err instanceof Error ? err.message : String(err)
      if (errMsg.includes('Method not implemented') || errMsg.includes('Method not found')) {
        console.log('[ACP] session/resume not supported, marking and falling back to new session')
        this._resumeNotSupported = true
        await this.newSession(cwd, model)
      } else {
        console.log('[ACP] session/resume error:', err)
        console.log('[ACP] falling back to new session')
        await this.newSession(cwd, model)
      }
    }
  }

  async sendPrompt(text: string, acpSessionId?: string): Promise<void> {
    const sid = acpSessionId ?? this.currentSessionId
    console.log('[ACP] sendPrompt called, sid:', sid, 'currentSessionId:', this.currentSessionId)
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
    console.log('[ACP] cancelSession:', acpSessionId)
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
      const chunks: string[] = []
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
      }, 120_000)

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

      this._pendingSummaryChunks = chunks
      this._pendingSummaryId = summaryRequestId

      const formattedMessages = messages
        .map((m) => `${m.role.toUpperCase()}: ${m.content}`)
        .join('\n')

      const summaryPrompt = `Please briefly summarize the following conversation for context. Focus on key topics, decisions, and current state. Be concise (2-3 sentences max).\n\n${formattedMessages}`

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
      // Remove all listeners so the exit event doesn't fire after we null the ref
      this.process.removeAllListeners()
      this.process.stdout?.removeAllListeners()
      this.process.stderr?.removeAllListeners()
      this.process.kill()
      this.process = null
      this.currentSessionId = null
      this._resumeNotSupported = false
      this._suppressedSessions.clear()
    }
    this.pending.forEach((p) => p.reject(new Error('ACP stopped')))
    this.pending.clear()
  }

  isRunning(): boolean {
    return this.process !== null
  }
}

export const acpManager = new AcpManager()
