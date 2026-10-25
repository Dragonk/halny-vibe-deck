import React, { useState, useRef, useCallback, useEffect } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import { resolveLanguage, translateModeName, translateModeDescription, translateThinkingLevel, translateThinkingDescription } from '../i18n/terms'
import type { TermsLang } from '../i18n/terms'
import type { AppConfig, ConfigOption, ConfigOptionChoice } from '../types'
import UsageBar from './UsageBar'
import MessageQueue from './MessageQueue'

interface Props {
  config: AppConfig
}

interface SlashItem {
  id: string
  label: string
  kind: 'skill' | 'prompt' | 'builtin' | 'command'
  content?: string
  description?: string
}

const SendIcon = (): React.JSX.Element => (
  <svg
    xmlns="http://www.w3.org/2000/svg"
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="currentColor"
    style={{ transform: 'rotate(-90deg)' }}
  >
    <path d="M12 18v4h4v-4h-4ZM16 14v4h4v-4h-4ZM20 10v4h4v-4h-4ZM16 6v4h4V6h-4ZM12 2v4h4V2h-4ZM12 10v4h4v-4h-4ZM8 10v4h4v-4H8ZM4 10v4h4v-4H4ZM0 10v4h4v-4H0Z" />
  </svg>
)

const StopIcon = (): React.JSX.Element => (
  <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
    <rect x="4" y="4" width="16" height="16" rx="2" />
  </svg>
)

const FolderIcon = (): React.JSX.Element => (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
  >
    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
  </svg>
)

const ChevronIcon = (): React.JSX.Element => (
  <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
    <path d="M1 3l4 4 4-4" />
  </svg>
)

/** One config picker chip (model / mode / thinking) driven by ACP configOptions */
function ConfigChip({
  option,
  currentValue,
  label,
  disabled,
  lang,
  onSelect,
}: {
  option: ConfigOption | undefined
  currentValue: string
  label: string
  disabled?: boolean
  lang?: TermsLang
  onSelect: (value: string) => void
}): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent): void => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open])

  const displayName = (() => {
    const found = option?.options.find((o) => o.value === currentValue)
    if (!found) return currentValue ?? '—'
    if (option?.id === 'mode') return translateModeName(found.value, lang ?? 'en')
    if (option?.id === 'thinking') return translateThinkingLevel(found.value, lang ?? 'en')
    return found.name
  })()

  // Translated descriptions for mode/thinking; other options keep the
  // description the CLI provides
  const describe = (value: string, fallback: string | undefined): string => {
    if (option?.id === 'mode') return translateModeDescription(value, lang ?? 'en') || fallback || ''
    if (option?.id === 'thinking') return translateThinkingDescription(value, lang ?? 'en') || fallback || ''
    return fallback || ''
  }

  return (
    <div className="ic-chip-wrap" ref={ref}>
      <button
        className={`ic-chip ${disabled ? 'ic-chip-disabled' : ''}`}
        onClick={() => !disabled && setOpen(!open)}
        title={describe(currentValue, option?.description) || label}
        disabled={disabled}
      >
        <span className="ic-chip-label">{label}</span>
        <span className="ic-chip-value">{displayName}</span>
        <ChevronIcon />
      </button>
      {open && option && (
        <div className="ic-chip-dropdown">
          {option.options.map((o) => (
            <button
              key={o.value}
              className={`ic-chip-item ${o.value === currentValue ? 'active' : ''}`}
              onClick={() => {
                onSelect(o.value)
                setOpen(false)
              }}
              title={describe(o.value, o.description) || o.name}
            >
              <span className="ic-chip-item-check">{o.value === currentValue ? '✓' : ''}</span>
              <span className="ic-chip-item-body">
                <span className="ic-chip-item-name">
                  {option.id === 'mode'
                    ? translateModeName(o.value, lang ?? 'en')
                    : option.id === 'thinking'
                      ? translateThinkingLevel(o.value, lang ?? 'en')
                      : o.name}
                </span>
                {describe(o.value, o.description) && (
                  <span className="ic-chip-item-desc">{describe(o.value, o.description)}</span>
                )}
              </span>
            </button>
          ))}
          {option.options.length === 0 && (
            <span className="ic-chip-empty">—</span>
          )}
        </div>
      )}
    </div>
  )
}

export default function InputBar({ config }: Props): React.JSX.Element {
  const {
    acpStatus,
    currentModel,
    currentMode,
    currentThinking,
    configOptions,
    availableCommands,
    workingDirectory,
    setWorkingDirectory,
    setCurrentMode,
    setCurrentModel,
    setCurrentThinking,
    addUserMessage,
    setAcpStatus,
    setAcpError,
    currentSession,
    currentSessionWorkingDirectory,
    sessions,
    currentSessionId,
    updateSession,
    unlinkAcpSession,
    pendingNewSession,
    setPendingNewSession,
    sessionStarting,
    setSessionStarting,
    addRecentFolder,
    streamingSessions,
    busySessions,
    stoppingSessions,
    addToMessageQueue,
  } = useStore()

  const tr = useTranslation(config.language)
  const lang = resolveLanguage(config.language)

  const [inputText, setInputText] = useState('')
  const [isStarting, setIsStarting] = useState(false)

  // @ file picker
  const [atQuery, setAtQuery] = useState('')
  const [atFiles, setAtFiles] = useState<Array<{ path: string; isDir: boolean }>>([])
  const [showAtPicker, setShowAtPicker] = useState(false)
  const [atPickerIdx, setAtPickerIdx] = useState(0)

  // / slash picker (commands + skills + prompts)
  const [slashQuery, setSlashQuery] = useState('')
  const [showSlashPicker, setShowSlashPicker] = useState(false)
  const [slashPickerIdx, setSlashPickerIdx] = useState(0)
  const [slashSkills, setSlashSkills] = useState<{ name: string; content: string }[]>([])
  const [slashPrompts, setSlashPrompts] = useState<{ name: string; content: string }[]>([])

  // Pasted images
  const [pastedImages, setPastedImages] = useState<{ name: string; url: string }[]>([])

  // Folder picker dropdown
  const [showFolderPicker, setShowFolderPicker] = useState(false)

  const editorRef = useRef<HTMLDivElement>(null)
  const atPickerRef = useRef<HTMLDivElement>(null)
  const folderPickerRef = useRef<HTMLDivElement>(null)
  const attachMenuRef = useRef<HTMLDivElement>(null)
  const [showAttachMenu, setShowAttachMenu] = useState(false)

  // Close pickers on outside click
  useEffect(() => {
    const handler = (e: MouseEvent): void => {
      if (atPickerRef.current && !atPickerRef.current.contains(e.target as Node)) {
        setShowAtPicker(false)
        setShowSlashPicker(false)
      }
      if (folderPickerRef.current && !folderPickerRef.current.contains(e.target as Node)) {
        setShowFolderPicker(false)
      }
      if (attachMenuRef.current && !attachMenuRef.current.contains(e.target as Node)) {
        setShowAttachMenu(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [])

  // Auto-resize textarea
  useEffect(() => {
    if (editorRef.current) {
      editorRef.current.style.height = 'auto'
      editorRef.current.style.height = `${Math.min(editorRef.current.scrollHeight, 160)}px`
    }
  }, [inputText])

  // Load files when @ picker is active
  useEffect(() => {
    if (!showAtPicker) return
    const dir = currentSessionWorkingDirectory() || config.workingDirectory || ''
    if (!dir) {
      setAtFiles([])
      return
    }
    // v0.7.1: remote sessions must search their own host, not the local disk
    const atRid = useStore.getState().currentSession()?.remoteId ?? undefined
    window.api.fs.searchFiles(dir, atQuery, atRid).then((entries) => {
      setAtFiles(entries)
      setAtPickerIdx(0)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showAtPicker, atQuery, currentSessionId, config.workingDirectory])

  // v0.10.2: project agents (<cwd>/.vibe/agents/*.toml) are not reported by
  // the CLI in configOptions, but they are selectable once exposed to the
  // global agents dir - load them so the mode dropdown can offer them.
  const [projectAgents, setProjectAgents] = useState<Array<{ name: string; displayName: string; description?: string }>>([])
  const projectCwd = currentSessionWorkingDirectory() || config.workingDirectory || ''
  const projectRemoteId = currentSession()?.remoteId ?? null
  useEffect(() => {
    let cancelled = false
    if (!projectCwd) {
      setProjectAgents([])
      return
    }
    window.api.vibe
      .getAgents('project', projectCwd, projectRemoteId ?? undefined)
      .then((list) => {
        if (cancelled) return
        const agents = (list as Array<Record<string, unknown>>)
          .filter((a) => a && a.agent_type === 'agent')
          .map((a) => ({
            name: String(a.name ?? ''),
            displayName: String(a.display_name ?? a.name ?? ''),
            description: typeof a.description === 'string' ? a.description : undefined,
          }))
          .filter((a) => a.name.length > 0)
        setProjectAgents(agents)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [projectCwd, projectRemoteId])

  // Load skills and prompts when slash picker first opens
  useEffect(() => {
    if (!showSlashPicker) return
    Promise.all([
      window.api.vibe.getSkills().catch(() => []),
      window.api.vibe.getPrompts().catch(() => []),
    ]).then(([skills, prompts]) => {
      setSlashSkills(skills as { name: string; content: string }[])
      setSlashPrompts(prompts as { name: string; content: string }[])
    })
  }, [showSlashPicker])

  const allSlashItems: SlashItem[] = [
    ...availableCommands.map((c) => ({
      id: `cmd:${c.name}`,
      label: c.name,
      kind: 'command' as const,
      content: `/${c.name}${c.hint ? ' ' + c.hint : ''}`,
      description: c.description,
    })),
    ...slashSkills.map((s) => ({
      id: `skill:${s.name}`,
      label: s.name,
      kind: 'skill' as const,
      content: s.content,
    })),
    ...slashPrompts.map((p) => ({
      id: `prompt:${p.name}`,
      label: p.name,
      kind: 'prompt' as const,
      content: p.content,
    })),
  ]

  const filteredSlashItems = allSlashItems.filter((item) =>
    item.label.toLowerCase().includes(slashQuery.toLowerCase()),
  )

  // Insert @path marker at cursor
  const insertAtFile = useCallback(async (entry: { path: string; isDir: boolean }) => {
    const editor = editorRef.current
    if (!editor) return

    const selection = window.getSelection()
    if (!selection || selection.rangeCount === 0) return

    const range = selection.getRangeAt(0)
    const allTextBefore = getTextBeforeCursor(editor, range)
    const atIdx = allTextBefore.lastIndexOf('@')
    if (atIdx === -1) return

    setShowAtPicker(false)

    const pathSuffix = entry.isDir && !entry.path.endsWith('/') ? '/' : ''
    const insertText = `@${entry.path}${pathSuffix}`

    let charCount = 0
    let atNode: Text | null = null
    let atNodeOffset = 0

    const walker = document.createTreeWalker(editor, NodeFilter.SHOW_TEXT, null)
    let node: Node | null = walker.nextNode()
    while (node) {
      const nodeText = node.textContent ?? ''
      const nextCount = charCount + nodeText.length
      if (charCount <= atIdx && atIdx < nextCount) {
        atNode = node as Text
        atNodeOffset = atIdx - charCount
        break
      }
      charCount = nextCount
      node = walker.nextNode()
    }

    if (!atNode) return

    const deleteRange = document.createRange()
    const cursorTextNode =
      range.startContainer.nodeType === Node.TEXT_NODE ? (range.startContainer as Text) : null

    if (atNode === cursorTextNode) {
      deleteRange.setStart(atNode, atNodeOffset)
      deleteRange.setEnd(cursorTextNode!, range.startOffset)
    } else {
      deleteRange.setStart(atNode, atNodeOffset)
      deleteRange.setEnd(range.startContainer, range.startOffset)
    }

    deleteRange.deleteContents()

    const textNode = document.createTextNode(insertText)
    range.insertNode(textNode)

    range.setStartAfter(textNode)
    range.collapse(true)
    selection.removeAllRanges()
    selection.addRange(range)

    setInputText(editor.textContent ?? '')
  }, [])

  const insertSlashItem = useCallback((item: SlashItem) => {
    const editor = editorRef.current
    if (!editor) return

    const selection = window.getSelection()
    if (!selection || selection.rangeCount === 0) return

    const range = selection.getRangeAt(0)
    const beforeCursor = getTextBeforeCursor(editor, range)
    const slashIdx = beforeCursor.lastIndexOf('/')
    if (slashIdx === -1) return

    const content = item.content ?? ''
    const textContent = editor.textContent ?? ''
    const afterCursor = textContent.slice(beforeCursor.length)

    editor.textContent =
      textContent.slice(0, slashIdx) + content + (content.endsWith('\n') ? '' : '\n') + afterCursor

    const newPos = slashIdx + content.length + 1
    const newRange = document.createRange()
    const textNode = editor.firstChild
    if (textNode && textNode.textContent && newPos <= textNode.textContent.length) {
      newRange.setStart(textNode, newPos)
      newRange.collapse(true)
      selection.removeAllRanges()
      selection.addRange(newRange)
    }

    setShowSlashPicker(false)
    setInputText(editor.textContent ?? '')
  }, [])

  // Helper to get text before cursor in contenteditable
  const getTextBeforeCursor = (editor: Node, range: Range): string => {
    const textNodes: string[] = []
    const walker = document.createTreeWalker(editor, NodeFilter.SHOW_TEXT, null)
    let node: Node | null = walker.nextNode()
    while (node) {
      if (node === range.startContainer) {
        textNodes.push((node.textContent ?? '').slice(0, range.startOffset))
        break
      }
      textNodes.push(node.textContent ?? '')
      node = walker.nextNode()
    }
    return textNodes.join('')
  }

  const handleEditorInput = (): void => {
    const editor = editorRef.current
    if (!editor) return

    setInputText(editor.textContent ?? '')

    const selection = window.getSelection()
    if (!selection || selection.rangeCount === 0) return

    const range = selection.getRangeAt(0)
    const beforeCursor = getTextBeforeCursor(editor, range)

    const atMatch = beforeCursor.match(/@([^\s@]*)$/)
    if (atMatch) {
      setAtQuery(atMatch[1])
      setShowAtPicker(true)
      setShowSlashPicker(false)
    } else {
      setShowAtPicker(false)
    }

    const slashMatch = beforeCursor.match(/(?:^|[\s\n])\/([^\s/]*)$/)
    if (slashMatch) {
      setSlashQuery(slashMatch[1])
      setShowSlashPicker(true)
      setShowAtPicker(false)
    } else {
      setShowSlashPicker(false)
    }
  }

  const handleSend = useCallback(async () => {
    const editor = editorRef.current
    let textToSend = inputText.trim()

    if (editor) {
      textToSend = editor.textContent ?? ''
    }

    if (!textToSend && pastedImages.length === 0) return

    // Extract @path markers and fetch file/folder contents for ACP
    const atPathRegex = /@([^\s]+)/g
    const cwd = currentSessionWorkingDirectory() || config.workingDirectory || ''
    const pathMatches = [...textToSend.matchAll(atPathRegex)]
    let expandedText = textToSend
    const attachments: { path: string; isDir: boolean; content: string }[] = []
    // v0.7.1: @paths resolve on the session's own worker (remote-aware)
    const attachRid = useStore.getState().currentSession()?.remoteId ?? undefined

    if (pathMatches.length > 0) {
      const fetchPromises = pathMatches.map(async (match) => {
        const rawPath = match[1]
        const isDir = rawPath.endsWith('/')
        const path = isDir ? rawPath.slice(0, -1) : rawPath
        const fullPath =
          path.startsWith('/') || path.match(/^[A-Za-z]:/)
            ? path
            : `${cwd}/${path}`.replace(/\\/g, '/')
        try {
          const result = await window.api.fs.readFile(fullPath, attachRid)
          return { original: match[0], result, isDir, path: rawPath }
        } catch {
          return { original: match[0], result: { type: 'error' as const, content: '' }, isDir, path: rawPath }
        }
      })

      const results = await Promise.all(fetchPromises)

      for (const { original, result, isDir, path: rawPath } of results) {
        const name = rawPath.replace(/\/$/, '').split(/[/\\]/).pop() ?? rawPath
        const location = rawPath
        if (result.type === 'file') {
          attachments.push({ path: rawPath, isDir: false, content: result.content })
          expandedText = expandedText.replace(
            original,
            `<vibe_attach_file><name>${name}</name><location>${location}</location><content>${result.content}</content></vibe_attach_file>`,
          )
        } else if (result.type === 'directory') {
          attachments.push({ path: rawPath, isDir: true, content: result.content })
          expandedText = expandedText.replace(
            original,
            `<vibe_attach_directory><name>${name}</name><location>${location}</location><content>${result.content}</content></vibe_attach_directory>`,
          )
        }
      }
    }

    const currentSess = currentSession()
    // v0.9.0: a previewed (not live) session with an empty preview must
    // CONTINUE its own ACP session after a real resume, not fork into a
    // fresh one - only live empty drafts (primed) re-bind to a new session.
    const isDraft =
      !currentSess ||
      !currentSess.acpSessionId ||
      (currentSess.messages.length === 0 &&
        useStore.getState().liveSessions.includes(currentSess.id))

    if (pendingNewSession || sessionStarting || !currentSess?.acpSessionId) {
      // ---- First message on a draft session — create the ACP session ----
      const rid = currentSess?.remoteId ?? null
      setPendingNewSession(false)
      setSessionStarting(true)
      setIsStarting(true)
      try {
        if (acpStatus === 'connected') {
          // Re-bind an empty draft to a fresh session with the chosen cwd
          if (currentSess?.acpSessionId && currentSess.id) {
            unlinkAcpSession(currentSess.id)
          }
          // v0.10.2: hand the recorded config to the main process - it is
          // applied inside session/new, deterministically BEFORE the first
          // prompt (a mid-turn set_config_option hangs the CLI turn).
          const applyCfg = {
            model: currentSess?.lastModel,
            mode: currentSess?.lastMode,
            thinking: currentSess?.lastThinking
          }
          const hasApplyCfg = !!(applyCfg.model || applyCfg.mode || applyCfg.thinking)
          // v0.7.0: remote drafts need their own worker alive first
          if (rid) await window.api.acp.ensureWorker(config.acpPath || 'vibe-acp', cwd, rid)
          await window.api.acp.newSession(cwd, rid ?? undefined, hasApplyCfg ? applyCfg : undefined)
        } else if (rid) {
          setAcpStatus('connecting')
          await window.api.acp.ensureWorker(config.acpPath || 'vibe-acp', cwd, rid)
        } else {
          if (!config.acpPath) {
            setAcpError('ACP path not configured. Go to Settings.')
            setIsStarting(false)
            setSessionStarting(false)
            setPendingNewSession(true)
            return
          }
          setAcpStatus('connecting')
          await window.api.acp.start(config.acpPath, cwd)
        }
      } catch (err) {
        setAcpStatus('error')
        setAcpError((err as Error).message)
        setIsStarting(false)
        setSessionStarting(false)
        setPendingNewSession(true)
        return
      }
      setIsStarting(false)
      setSessionStarting(false)
      // SessionInitialized has fired by now → the local session is linked
      const linkedSess = useStore.getState().currentSession()
      if (linkedSess?.id && !linkedSess.workingDirectory) {
        updateSession(linkedSess.id, { workingDirectory: cwd || undefined })
      }
      setInputText('')
      if (editorRef.current) editorRef.current.textContent = ''
      setPastedImages([])
      addUserMessage(textToSend, attachments)
      if (linkedSess?.id) useStore.getState().markSessionBusy(linkedSess.id)
      try {
        await window.api.acp.sendPrompt(expandedText, linkedSess?.acpSessionId ?? undefined, rid ?? undefined)
      } catch (err) {
        setAcpError((err as Error).message)
      }
      return
    }

    // ---- Existing session — continue in the same ACP session ----
    const sessionWd = currentSessionWorkingDirectory() || config.workingDirectory || ''
    const acpSessId = currentSess.acpSessionId
    const rid = currentSess.remoteId ?? null

    if (acpStatus === 'disconnected' || acpStatus === 'error') {
      // v0.7.0: remote sessions revive their own worker (no prime);
      // local ones restart the local agent as before.
      if (rid) {
        setIsStarting(true)
        setAcpStatus('connecting')
        try {
          await window.api.acp.ensureWorker(config.acpPath || 'vibe-acp', sessionWd, rid)
        } catch (err) {
          setAcpError((err as Error).message)
          setIsStarting(false)
          return
        }
        setIsStarting(false)
      } else {
        if (!config.acpPath) {
          setAcpError('ACP path not configured. Go to Settings.')
          return
        }
        setIsStarting(true)
        setAcpStatus('connecting')
        try {
          // start() creates the session with the right cwd; the draft re-binding
          // logic in App keeps the current local session attached to it.
          await window.api.acp.start(config.acpPath, sessionWd)
        } catch (err) {
          setAcpStatus('error')
          setAcpError((err as Error).message)
          setIsStarting(false)
          return
        }
        setIsStarting(false)
      }
    }

    setInputText('')
    if (editorRef.current) editorRef.current.textContent = ''
    setPastedImages([])

    // If the agent is busy, queue the message instead of sending
    const isCurrentSessionBusy = currentSessionId != null && busySessions.includes(currentSessionId)
    if (isCurrentSessionBusy) {
      addToMessageQueue(expandedText)
      return
    }

    addUserMessage(textToSend, attachments)
    useStore.getState().markSessionBusy(currentSess.id)
    const promptText = currentSess.contextSummary
      ? `${currentSess.contextSummary}\n\nUser: ${expandedText}`
      : expandedText
    try {
      await window.api.acp.sendPrompt(promptText, acpSessId, rid ?? undefined)
    } catch (err) {
      const errMsg = (err as Error).message
      // Session not in ACP memory (e.g. app restarted) — load it from disk first
      const sessionExpired =
        errMsg.includes('No active session') ||
        errMsg.includes('Session not found') ||
        (rid !== null && errMsg.includes('Remote ACP worker not started'))
      if (sessionExpired) {
        try {
          setSessionStarting(true)
          if (rid) {
            // v0.7.0: revive the remote worker first (idempotent, no prime)
            await window.api.acp.ensureWorker(config.acpPath || 'vibe-acp', sessionWd, rid)
          }
          // v0.10.2: recorded config is applied inside session/load
          // (main process) - before the prompt resend, never mid-turn.
          const retryCfg = {
            model: currentSess.lastModel,
            mode: currentSess.lastMode,
            thinking: currentSess.lastThinking
          }
          const hasRetryCfg = !!(retryCfg.mode || retryCfg.model || retryCfg.thinking)
          try {
            // Real resume: session/load replays the full history
            await window.api.acp.loadSession(acpSessId, sessionWd, rid ?? undefined, hasRetryCfg ? retryCfg : undefined)
            useStore.getState().markSessionLive(currentSess.id)
            useStore.getState().markSessionBusy(currentSess.id)
            await window.api.acp.sendPrompt(promptText, acpSessId, rid ?? undefined)
          } catch (loadErr) {
            // session/load failed (log missing) → fall back to a new session
            console.log('[InputBar] loadSession failed:', loadErr, '— creating new session')
            await window.api.acp.newSession(sessionWd, rid ?? undefined, hasRetryCfg ? retryCfg : undefined)
            await window.api.acp.sendPrompt(promptText, undefined, rid ?? undefined)
          }
          setSessionStarting(false)
        } catch (err2) {
          setSessionStarting(false)
          setAcpError((err2 as Error).message)
        }
      } else {
        setAcpError(errMsg)
      }
    }
  }, [
    inputText,
    pastedImages,
    acpStatus,
    config,
    currentSessionWorkingDirectory,
    currentSession,
    addUserMessage,
    setAcpStatus,
    setAcpError,
    pendingNewSession,
    setPendingNewSession,
    sessionStarting,
    setSessionStarting,
    updateSession,
    unlinkAcpSession,
    currentSessionId,
    busySessions,
    addToMessageQueue,
  ])

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>): void => {
    if (showAtPicker && atFiles.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setAtPickerIdx((i) => Math.min(i + 1, atFiles.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setAtPickerIdx((i) => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        insertAtFile(atFiles[atPickerIdx])
        return
      }
      if (e.key === 'Escape') {
        setShowAtPicker(false)
        return
      }
    }
    if (showSlashPicker && filteredSlashItems.length > 0) {
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSlashPickerIdx((i) => Math.min(i + 1, filteredSlashItems.length - 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSlashPickerIdx((i) => Math.max(i - 1, 0))
        return
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault()
        insertSlashItem(filteredSlashItems[slashPickerIdx])
        return
      }
      if (e.key === 'Escape') {
        setShowSlashPicker(false)
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSend()
    }
  }

  // ---- Attach menu (file / directory / image / branch diff) ----
  const insertTextAtCursor = (text: string): void => {
    const editor = editorRef.current
    if (!editor) return
    const textNode = document.createTextNode(text)
    const selection = window.getSelection()
    if (selection && selection.rangeCount > 0 && editor.contains(selection.anchorNode)) {
      const range = selection.getRangeAt(0)
      range.deleteContents()
      range.insertNode(textNode)
      range.setStartAfter(textNode)
      range.collapse(true)
      selection.removeAllRanges()
      selection.addRange(range)
    } else {
      editor.appendChild(textNode)
    }
    setInputText(editor.textContent ?? '')
  }

  const handleAttachFile = async (): Promise<void> => {
    const filePath = await window.api.dialog.openFile()
    if (filePath) insertTextAtCursor('@' + filePath + ' ')
    setShowAttachMenu(false)
  }

  const handleAttachDirectory = async (): Promise<void> => {
    const folder = await window.api.dialog.openFolder()
    if (folder) insertTextAtCursor('@' + folder + '/ ')
    setShowAttachMenu(false)
  }

  const handleAttachImage = async (): Promise<void> => {
    const filePath = await window.api.dialog.openFile()
    setShowAttachMenu(false)
    if (!filePath) return
    const name = filePath.split(/[/\\]/).pop() ?? 'image.png'
    try {
      const res = await window.api.fs.readBinary(filePath)
      if (res.error || !res.base64) return
      const url = 'data:' + (res.mime || 'image/png') + ';base64,' + res.base64
      setPastedImages((prev) => [...prev, { name, url }])
      insertTextAtCursor('@' + name + ' ')
    } catch (err) {
      console.error('Failed to attach image:', err)
    }
  }

  const handleAttachBranchDiff = async (): Promise<void> => {
    setShowAttachMenu(false)
    const cwd = currentSessionWorkingDirectory() || config.workingDirectory || ''
    if (!cwd) return
    // v0.7.1: remote sessions diff their own repository over ssh
    const rid = useStore.getState().currentSession()?.remoteId ?? undefined
    try {
      const diff = await window.api.git.diffAll(cwd, rid)
      if (!diff || diff.trim().length === 0) {
        insertTextAtCursor(tr.branchDiffEmpty)
        return
      }
      const capped = diff.length > 100000 ? diff.slice(0, 100000) + '\n… (truncated)' : diff
      insertTextAtCursor('```diff\n' + capped + '\n```\n')
    } catch (err) {
      console.error('Failed to attach branch diff:', err)
    }
  }

  const handlePaste = (e: React.ClipboardEvent<HTMLDivElement>): void => {
    const items = Array.from(e.clipboardData.items)
    const imageItem = items.find((item) => item.type.startsWith('image/'))
    if (!imageItem) return
    e.preventDefault()
    const file = imageItem.getAsFile()
    if (!file) return
    const url = URL.createObjectURL(file)
    const name = `image_${Date.now()}.png`
    setPastedImages((prev) => [...prev, { name, url }])
    const editor = editorRef.current
    if (editor) {
      const textNode = document.createTextNode('@' + name + ' ')
      const selection = window.getSelection()
      if (selection && selection.rangeCount > 0) {
        const range = selection.getRangeAt(0)
        range.deleteContents()
        range.insertNode(textNode)
        range.setStartAfter(textNode)
        range.collapse(true)
        selection.removeAllRanges()
        selection.addRange(range)
      } else {
        editor.appendChild(textNode)
      }
      setInputText(editor.textContent ?? '')
    }
  }

  // ---- Folder picker ----
  const canChangeFolder = (): boolean => {
    const session = currentSession()
    // v0.7.1: remote sessions work on the remote host - the native dialog
    // picks local folders, which would produce a bogus remote cwd.
    if (session?.remoteId) return false
    return !session || session.messages.length === 0
  }

  const applyFolder = (folder: string): void => {
    addRecentFolder(folder)
    const session = sessions.find((s) => s.id === currentSessionId)
    if (session) {
      updateSession(session.id, { workingDirectory: folder })
      // Re-bind an empty draft to a fresh ACP session with the new cwd
      if (session.acpSessionId && session.messages.length === 0) {
        if (acpStatus === 'connected') {
          window.api.acp.newSession(folder, session.remoteId ?? undefined).catch((err: Error) => {
            console.error('Failed to re-bind session folder:', err)
          })
        } else {
          unlinkAcpSession(session.id)
        }
      }
    } else {
      setWorkingDirectory(folder)
    }
  }

  const handleFolderPick = async (): Promise<void> => {
    const folder = await window.api.dialog.openFolder()
    if (folder) {
      applyFolder(folder)
      setShowFolderPicker(false)
    }
  }

  const handleFolderSelect = (folder: string): void => {
    applyFolder(folder)
    setShowFolderPicker(false)
  }

  // ---- Config options (model / mode / thinking) ----
  const handleConfigSelect = async (key: string, value: string): Promise<void> => {
    // v0.10.2: selecting a project agent requires exposing its file to the
    // CLI's global agents dir first - the CLI cannot see project agents.
    if (key === 'mode' && projectAgents.some((a) => a.name === value)) {
      const cliHasIt = configOptions.find((o) => o.id === 'mode')?.options.some((o) => o.value === value)
      if (!cliHasIt) {
        const cwd = currentSessionWorkingDirectory() || config.workingDirectory || ''
        const rid = currentSession()?.remoteId ?? undefined
        await window.api.vibe.exposeProjectAgent(value, cwd, rid).catch(console.error)
      }
    }
    if (key === 'model') setCurrentModel(value)
    if (key === 'mode') setCurrentMode(value)
    if (key === 'thinking') setCurrentThinking(value)
    // Record the choice on the session so switching back restores it.
    const sess = currentSession()
    if (sess) {
      const patch =
        key === 'model'
          ? { lastModel: value }
          : key === 'mode'
            ? { lastMode: value }
            : { lastThinking: value }
      useStore.getState().updateSession(sess.id, patch)
    }
    const acpSessId = useStore.getState().activeAcpSessionId ?? sess?.acpSessionId
    await window.api.acp
      .setConfigOption(key, value, acpSessId ?? undefined, sess?.remoteId ?? undefined)
      .catch((err: Error) => {
        // v0.10.2: the CLI validates mode values against the agents it knew
        // when the ACP session was created. A freshly exposed project agent
        // is rejected for the current session, but the recorded lastMode is
        // applied when the next session starts (syncSessionChips), so this
        // specific failure is expected and must not alarm the user.
        if (key === 'mode' && /Unsupported config option/.test(String(err?.message ?? err))) return
        console.error(err)
      })
  }

  const isSending = acpStatus === 'connecting' || isStarting
  const isCurrentSessionBusy = currentSessionId != null && busySessions.includes(currentSessionId)
  // v0.10.5: stop request in flight - show explicit stopping feedback.
  const isStoppingSession = currentSessionId != null && stoppingSessions.includes(currentSessionId)
  const canSend =
    (inputText.trim().length > 0 || pastedImages.length > 0) && !isSending && !isCurrentSessionBusy

  const handleStop = useCallback(() => {
    const sess = currentSession()
    const acpSessId = sess?.acpSessionId
    if (!acpSessId || !sess?.id) return
    // v0.10.5: mark the session as stopping so the stop button and the busy
    // row show explicit feedback while session/cancel is in flight;
    // markSessionIdle clears the marker once the turn really ends (or when
    // the force-release timer below fires).
    useStore.getState().markSessionStopping(sess.id)
    const stoppingId = sess.id
    window.api.acp.cancelSession(acpSessId, sess?.remoteId ?? undefined).catch((err: Error) => {
      console.error(err)
      useStore.getState().clearSessionStopping(stoppingId)
    })
    // v0.8.1: a stuck turn (e.g. the remote CLI waiting for an MCP OAuth
    // login) never emits a turn end, so the session stays busy forever and
    // nothing can be sent. Give the CLI 4 s to acknowledge session/cancel,
    // then force-release the UI; for remote sessions also restart the stuck
    // worker when no other session on that host is busy, so the next
    // prompt gets a fresh transport.
    const localId = sess.id
    const rid = sess.remoteId ?? null
    window.setTimeout(() => {
      const st = useStore.getState()
      if (!st.busySessions.includes(localId)) return
      st.markSessionIdle(localId)
      if (rid) {
        const othersBusy = st.sessions.some(
          (s) => s.remoteId === rid && s.id !== localId && st.busySessions.includes(s.id),
        )
        if (!othersBusy) window.api.acp.stopWorker(rid).catch(console.error)
      }
    }, 4000)
  }, [currentSession])

  const currentWd = currentSessionWorkingDirectory()
  const projectLabel = currentWd
    ? currentWd.split(/[/\\]/).filter(Boolean).pop() ?? currentWd
    : ''
  const folderLocked = !canChangeFolder()

  const modelOption = configOptions.find((o) => o.id === 'model')
  const modeOption = configOptions.find((o) => o.id === 'mode')
  // v0.10.2: project agents join the mode dropdown; selecting one exposes
  // the agent file globally first (see handleConfigSelect).
  const extraModeChoices: ConfigOptionChoice[] = projectAgents
    .filter((a) => !modeOption?.options.some((o) => o.value === a.name))
    .map((a) => ({
      value: a.name,
      name: a.displayName,
      description: a.description
        ? tr.projectAgentBadge + ' — ' + a.description
        : tr.projectAgentBadge,
    }))
  const modeOptionMerged: ConfigOption | undefined = modeOption
    ? { ...modeOption, options: [...modeOption.options, ...extraModeChoices] }
    : undefined
  const thinkingOption = configOptions.find((o) => o.id === 'thinking')
  const pickersReady = configOptions.length > 0

  return (
    <div className="input-card-wrap">
      <div className="input-card">
        {/* v0.10.5: queued messages panel (Mistral Vibe style), attached to the card top */}
        <MessageQueue config={config} />
        {/* Pasted image previews */}
        {pastedImages.length > 0 && (
          <div className="input-attachments">
            {pastedImages.map((img, i) => (
              <div key={i} className="attachment-chip">
                <img src={img.url} alt={img.name} className="attachment-thumb" />
                <span className="attachment-name">{img.name}</span>
                <button
                  className="attachment-remove"
                  onClick={() => {
                    URL.revokeObjectURL(img.url)
                    setPastedImages((prev) => prev.filter((_, j) => j !== i))
                    setInputText((prev) =>
                      prev.replace('@' + img.name + ' ', '').replace('@' + img.name, ''),
                    )
                  }}
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Contenteditable input */}
        <div className="input-card-textarea-wrap" ref={atPickerRef}>
          <div
            ref={editorRef}
            className="input-card-editor"
            contentEditable
            onInput={handleEditorInput}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            data-placeholder={tr.typeMessage}
            suppressContentEditableWarning
          />

          {/* @ file picker dropdown */}
          {showAtPicker && atFiles.length > 0 && (
            <div className="input-picker-dropdown">
              {atFiles.slice(0, 12).map((f, i) => (
                <button
                  key={f.path}
                  className={`input-picker-item ${i === atPickerIdx ? 'active' : ''}`}
                  onMouseDown={(e) => {
                    e.preventDefault()
                    insertAtFile(f)
                  }}
                >
                  <span className="picker-item-icon">{f.isDir ? '📁' : '📄'}</span>
                  {f.path}
                </button>
              ))}
            </div>
          )}

          {/* / slash picker dropdown */}
          {showSlashPicker && filteredSlashItems.length > 0 && (
            <div className="input-picker-dropdown">
              {filteredSlashItems.map((item, i) => (
                <button
                  key={item.id}
                  className={`input-picker-item ${i === slashPickerIdx ? 'active' : ''}`}
                  onMouseDown={(e) => {
                    e.preventDefault()
                    insertSlashItem(item)
                  }}
                >
                  <span className="picker-item-icon">/</span>
                  <span className="picker-item-body">
                    <span className="picker-item-label">{item.label}</span>
                    {item.description && <span className="picker-item-desc">{item.description}</span>}
                  </span>
                  <span className={`picker-item-kind picker-item-kind-${item.kind}`}>{item.kind}</span>
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Bottom toolbar */}
        <div className="input-card-toolbar">
          {/* Left: folder chip */}
          <div className="input-card-left" ref={folderPickerRef}>
            <button
              className={`ic-folder-chip ${folderLocked ? 'ic-chip-disabled' : ''}`}
              onClick={() => canChangeFolder() && setShowFolderPicker(!showFolderPicker)}
              title={folderLocked ? tr.folderChangeDisabled : currentWd || tr.workingDirectory}
              disabled={folderLocked}
            >
              <FolderIcon />
              <span className="ic-folder-name">{projectLabel || tr.chooseFolder}</span>
              <ChevronIcon />
            </button>
            {showFolderPicker && (
              <div className="folder-picker-dropdown">
                <button className="folder-picker-item" onClick={handleFolderPick}>
                  <span>📁 {tr.addNew}</span>
                </button>
                {config.recentFolders.map((folder) => (
                  <button
                    key={folder}
                    className="folder-picker-item"
                    onClick={() => handleFolderSelect(folder)}
                    title={folder}
                  >
                    <span>📁 {folder.split(/[/\\]/).filter(Boolean).pop()}</span>
                  </button>
                ))}
              </div>
            )}
            <div className="attach-menu-wrap" ref={attachMenuRef}>
              <button
                className="ic-btn"
                onClick={() => setShowAttachMenu(!showAttachMenu)}
                title={tr.attachHint}
                aria-label={tr.attachLabel}
              >
                ＋
              </button>
              {showAttachMenu && (
                <div className="folder-picker-dropdown attach-dropdown">
                  <button className="folder-picker-item" onClick={handleAttachFile}>
                    <span>📄 {tr.attachFile}</span>
                  </button>
                  <button className="folder-picker-item" onClick={handleAttachDirectory}>
                    <span>📁 {tr.attachDirectory}</span>
                  </button>
                  <button className="folder-picker-item" onClick={handleAttachImage}>
                    <span>🖼️ {tr.attachImage}</span>
                  </button>
                  <button className="folder-picker-item" onClick={handleAttachBranchDiff}>
                    <span>🌿 {tr.attachBranchDiff}</span>
                  </button>
                </div>
              )}
            </div>
          </div>

          {/* Right: model / mode / thinking chips + send */}
          <div className="input-card-right">
            <ConfigChip
              option={modelOption}
              currentValue={currentModel}
              label={tr.modelPicker}
              disabled={!pickersReady || isCurrentSessionBusy}
              lang={lang}
              onSelect={(v) => handleConfigSelect('model', v)}
            />
            <ConfigChip
              option={modeOptionMerged}
              currentValue={currentMode}
              label={tr.modePicker}
              disabled={!pickersReady}
              lang={lang}
              onSelect={(v) => handleConfigSelect('mode', v)}
            />
            <ConfigChip
              option={thinkingOption}
              currentValue={currentThinking}
              label={tr.thinkingPicker}
              disabled={!pickersReady}
              lang={lang}
              onSelect={(v) => handleConfigSelect('thinking', v)}
            />

            {isCurrentSessionBusy ? (
              <button
                className={'ic-stop-btn' + (isStoppingSession ? ' stopping' : '')}
                onClick={handleStop}
                disabled={isStoppingSession}
                title={isStoppingSession ? tr.stoppingLabel : tr.stopLabel}
              >
                {isStoppingSession ? <span className="ic-stop-spinner" /> : <StopIcon />}
              </button>
            ) : (
              <button
                className={`ic-send-btn ${canSend ? 'enabled' : ''}`}
                onClick={handleSend}
                disabled={!canSend}
                title={tr.sendEnter}
              >
                {isSending ? <span className="spinner">⟳</span> : <SendIcon />}
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Live usage / cache stats */}
      <UsageBar />

      {/* Status bar below card */}
      <div className="input-status-bar">
        <div className={`acp-status ${acpStatus}`}>
          <span className="status-dot" />
          <span className="status-label">
            {acpStatus === 'connected'
              ? tr.connected
              : acpStatus === 'connecting'
                ? tr.connecting
                : acpStatus === 'error'
                  ? tr.error
                  : tr.disconnected}
          </span>
        </div>
        <div className="input-usage-inline">
          <UsageInline />
        </div>
        {currentWd && (
          <span className="status-cwd" title={currentWd}>
            {currentWd.split(/[/\\]/).filter(Boolean).pop()}
          </span>
        )}
      </div>
    </div>
  )
}

/** Compact inline usage summary rendered inside the status bar */
function UsageInline(): React.JSX.Element | null {
  const { usage, currentSessionId } = useStore()
  const u = currentSessionId ? usage[currentSessionId] : undefined
  if (!u || !u.contextSize) return null

  const ctxPct = Math.min(100, Math.round((u.contextUsed / u.contextSize) * 100))
  const cached = u.cachedTokens ?? 0
  const prompt = u.promptTokens ?? 0
  const hitPct = prompt > 0 ? Math.round((cached / prompt) * 100) : null

  return (
    <>
      <span className="usage-inline-item" title={`${u.contextUsed} / ${u.contextSize} tokens`}>
        {ctxPct}%
      </span>
      {hitPct !== null && (
        <span
          className="usage-inline-item usage-cache"
          title={`Cache hit: ${cached} / ${prompt} prompt tokens`}
        >
          ⚡{hitPct}%
        </span>
      )}
      {u.tokensPerSecond !== undefined && u.tokensPerSecond > 0 && (
        <span className="usage-inline-item">{Math.round(u.tokensPerSecond)} tok/s</span>
      )}
      {u.cost !== undefined && (
        <span className="usage-inline-item usage-cost">
          ${u.cost.toFixed(4)}
        </span>
      )}
    </>
  )
}
