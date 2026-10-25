import React, { useEffect, useMemo, useState } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'
import type { PreviewTab } from '../types'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'

// ---------------------------------------------------------------------------
// Base64 → Uint8Array helper
// ---------------------------------------------------------------------------

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
  return bytes
}

function base64ToDataUrl(b64: string, mime: string): string {
  return `data:${mime};base64,${b64}`
}

// ---------------------------------------------------------------------------
// CodeMirror — syntax-highlighted read-only preview + file editor
// (modules are loaded lazily so the app start stays fast)
// ---------------------------------------------------------------------------

interface CmCache {
  EditorView: typeof import('@codemirror/view').EditorView
  EditorState: typeof import('@codemirror/state').EditorState
  keymap: typeof import('@codemirror/view').keymap
  basicSetup: import('@codemirror/state').Extension
  theme: import('@codemirror/state').Extension
  oneDark: import('@codemirror/state').Extension
  LanguageDescription: typeof import('@codemirror/language').LanguageDescription
  languages: import('@codemirror/language').LanguageDescription[]
}
let cmCache: CmCache | null = null

async function ensureCm(): Promise<CmCache> {
  if (!cmCache) {
    const [viewMod, stateMod, cmMod, langMod, langDataMod, oneDarkMod] = await Promise.all([
      import('@codemirror/view'),
      import('@codemirror/state'),
      import('codemirror'),
      import('@codemirror/language'),
      import('@codemirror/language-data'),
      import('@codemirror/theme-one-dark'),
    ])
    cmCache = {
      EditorView: viewMod.EditorView,
      EditorState: stateMod.EditorState,
      keymap: viewMod.keymap,
      basicSetup: cmMod.basicSetup,
      theme: viewMod.EditorView.theme({
        '&': { backgroundColor: 'transparent', color: 'var(--text)', height: '100%' },
        '.cm-content': {
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
          caretColor: 'var(--accent)',
        },
        '.cm-gutters': { backgroundColor: 'var(--panel)', color: 'var(--text-dim)', border: 'none' },
        '.cm-editor.cm-focused': { outline: 'none' },
      }),
      oneDark: oneDarkMod.oneDark,
      LanguageDescription: langMod.LanguageDescription,
      languages: langDataMod.languages as unknown as CmCache['languages'],
    }
  }
  return cmCache
}

function CodeMirrorPane({
  path,
  content,
  editable,
  registerDoc,
  onSave,
}: {
  path: string
  content: string
  editable: boolean
  registerDoc?: (getDoc: (() => string) | null) => void
  onSave?: () => void
}): React.JSX.Element {
  const hostRef = React.useRef<HTMLDivElement>(null)
  const contentRef = React.useRef(content)
  contentRef.current = content
  const onSaveRef = React.useRef(onSave)
  onSaveRef.current = onSave

  useEffect(() => {
    let alive = true
    let view: { destroy: () => void; state: { doc: { toString: () => string } } } | null = null
    ;(async () => {
      try {
        const cm = await ensureCm()
        if (!alive || !hostRef.current) return
        let langExt: unknown[] = []
        try {
          const desc = cm.LanguageDescription.matchFilename(cm.languages, path)
          if (desc) langExt = [await desc.load()]
        } catch {
          /* language support unavailable — fall back to plain text */
        }
        // oneDark only in dark theme; cm.theme comes after it so the editor
        // background stays transparent over the panel
        const dark = document.documentElement.getAttribute('data-theme') === 'dark'
        const extensions: unknown[] = [
          cm.basicSetup,
          ...(dark ? [cm.oneDark] : []),
          cm.theme,
          cm.EditorView.lineWrapping,
          ...langExt,
        ]
        if (editable) {
          if (onSaveRef.current) {
            extensions.push(
              cm.keymap.of([
                {
                  key: 'Mod-s',
                  run: () => {
                    onSaveRef.current?.()
                    return true
                  },
                },
              ]),
            )
          }
        } else {
          extensions.push(cm.EditorView.editable.of(false), cm.EditorState.readOnly.of(true))
        }
        view = new cm.EditorView({
          doc: contentRef.current,
          parent: hostRef.current,
          extensions: extensions as never,
        })
        registerDoc?.(() => view?.state.doc.toString() ?? '')
      } catch (err) {
        console.error('CodeMirror init failed:', err)
      }
    })()
    return () => {
      alive = false
      registerDoc?.(null)
      view?.destroy()
      view = null
    }
  }, [path, editable, registerDoc])

  return <div ref={hostRef} className="cm-host" />
}

// ---------------------------------------------------------------------------
// Markdown + Mermaid
// ---------------------------------------------------------------------------

function MermaidView({ code }: { code: string }): React.JSX.Element {
  const [svg, setSvg] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    ;(async () => {
      try {
        const mermaid = (await import('mermaid')).default
        mermaid.initialize({
          startOnLoad: false,
          theme: document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'default',
          securityLevel: 'strict',
        })
        const { svg } = await mermaid.render(`mmd-${Date.now()}`, code)
        if (alive) setSvg(svg)
      } catch (err) {
        if (alive) setError((err as Error).message)
      }
    })()
    return () => {
      alive = false
    }
  }, [code])

  if (error) {
    return <div className="preview-error-box">Mermaid: {error}</div>
  }
  if (!svg) {
    return <div className="preview-loading">…</div>
  }
  return (
    <div
      className="preview-mermaid"
      // Mermaid svg output with securityLevel: strict
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

function MarkdownView({ content }: { content: string }): React.JSX.Element {

  // Split out ```mermaid blocks and render them with the mermaid engine
  const segments = useMemo(() => {
    const parts: Array<{ type: 'md' | 'mermaid'; text: string }> = []
    const re = /```mermaid[ \t]*\n([\s\S]*?)```/g
    let last = 0
    let match: RegExpExecArray | null
    while ((match = re.exec(content)) !== null) {
      if (match.index > last) {
        parts.push({ type: 'md', text: content.slice(last, match.index) })
      }
      parts.push({ type: 'mermaid', text: match[1] })
      last = match.index + match[0].length
    }
    if (last < content.length) parts.push({ type: 'md', text: content.slice(last) })
    return parts
  }, [content])

  return (
    <div className="preview-markdown">
      <React.Suspense fallback={<div className="preview-loading">…</div>}>
        {segments.map((seg, i) =>
          seg.type === 'mermaid' ? (
            <MermaidView key={i} code={seg.text} />
          ) : (
            <ReactMarkdown key={i} remarkPlugins={[remarkGfm]}>
              {seg.text}
            </ReactMarkdown>
          ),
        )}
      </React.Suspense>
    </div>
  )
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

function PdfView({ path, base64 }: { path: string; base64: string }): React.JSX.Element {
  const containerRef = React.useRef<HTMLDivElement>(null)
  const [error, setError] = useState<string | null>(null)
  const [pageNum, setPageNum] = useState(1)
  const [total, setTotal] = useState(0)

  useEffect(() => {
    let alive = true
    let pdf: { destroy: () => void } | null = null
    ;(async () => {
      try {
        const pdfjs = await import('pdfjs-dist')
        // Worker as a module URL — Vite bundles and resolves this
        pdfjs.GlobalWorkerOptions.workerSrc = new URL(
          'pdfjs-dist/build/pdf.worker.min.mjs',
          import.meta.url,
        ).toString()

        const data = base64ToBytes(base64)
        const task = pdfjs.getDocument({ data })
        const doc = await task.promise
        if (!alive) {
          void task.destroy()
          return
        }
        pdf = { destroy: () => { void task.destroy() } }
        setTotal(doc.numPages)
        const canvasContainer = containerRef.current
        if (!canvasContainer) return
        canvasContainer.innerHTML = ''

        // Render up to 20 pages (large PDFs stay responsive)
        const maxPages = Math.min(doc.numPages, 20)
        for (let i = 1; i <= maxPages; i++) {
          const page = await doc.getPage(i)
          const viewport = page.getViewport({ scale: 1.4 })
          const canvas = document.createElement('canvas')
          canvas.className = 'preview-pdf-canvas'
          canvas.width = viewport.width
          canvas.height = viewport.height
          canvasContainer.appendChild(canvas)
          const ctx = canvas.getContext('2d')
          if (!ctx) continue
          await page.render({
            canvas,
            viewport,
          }).promise
        }
      } catch (err) {
        if (alive) setError((err as Error).message)
      }
    })()
    return () => {
      alive = false
      pdf?.destroy()
    }
  }, [base64])

  useEffect(() => {
    setPageNum(1)
  }, [path])

  if (error) return <div className="preview-error-box">PDF: {error}</div>
  return (
    <div className="preview-pdf">
      {total > 20 && (
        <div className="preview-pdf-hint">
          Showing 1–20 of {total} pages · open externally for the full document
        </div>
      )}
      <div ref={containerRef} className="preview-pdf-container" />
      <span className="hidden-pdf-page">{pageNum}</span>
    </div>
  )
}

// ---------------------------------------------------------------------------
// XLSX / CSV
// ---------------------------------------------------------------------------

function SheetView({ base64, ext }: { base64: string; ext: string }): React.JSX.Element {
  const [sheets, setSheets] = useState<Array<{ name: string; rows: unknown[][] }>>([])
  const [active, setActive] = useState(0)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    ;(async () => {
      try {
        const XLSX = await import('xlsx')
        const data = base64ToBytes(base64)
        const wb =
          ext === 'csv'
            ? XLSX.read(data, { type: 'array', raw: false })
            : XLSX.read(data, { type: 'array' })
        if (!alive) return
        const parsed = wb.SheetNames.map((name) => ({
          name,
          rows: XLSX.utils.sheet_to_json(wb.Sheets[name], {
            header: 1,
            defval: '',
            blankrows: false,
          }) as unknown[][],
        }))
        setSheets(parsed)
        setActive(0)
      } catch (err) {
        if (alive) setError((err as Error).message)
      }
    })()
    return () => {
      alive = false
    }
  }, [base64, ext])

  if (error) return <div className="preview-error-box">XLSX: {error}</div>
  if (sheets.length === 0) return <div className="preview-loading">…</div>

  const sheet = sheets[Math.min(active, sheets.length - 1)]
  const maxCols = Math.min(40, Math.max(...sheet.rows.map((r) => r.length), 1))

  return (
    <div className="preview-sheet">
      {sheets.length > 1 && (
        <div className="sheet-tabs">
          {sheets.map((s, i) => (
            <button
              key={s.name}
              className={`sheet-tab ${i === active ? 'active' : ''}`}
              onClick={() => setActive(i)}
            >
              {s.name}
            </button>
          ))}
        </div>
      )}
      <div className="sheet-table-wrap">
        <table className="sheet-table">
          <tbody>
            {sheet.rows.slice(0, 200).map((row, ri) => (
              <tr key={ri}>
                {Array.from({ length: maxCols }).map((_, ci) => (
                  <td key={ci} className={ri === 0 ? 'sheet-header-cell' : ''}>
                    {String(row[ci] ?? '')}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
        {sheet.rows.length > 200 && (
          <div className="preview-pdf-hint">Showing first 200 rows…</div>
        )}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------------

function DocxView({ base64 }: { base64: string }): React.JSX.Element {
  const [html, setHtml] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    ;(async () => {
      try {
        const mammoth = await import('mammoth')
        const buf = base64ToBytes(base64)
        const result = await mammoth.convertToHtml({
          arrayBuffer: buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer,
        })
        if (alive) setHtml(result.value)
      } catch (err) {
        if (alive) setError((err as Error).message)
      }
    })()
    return () => {
      alive = false
    }
  }, [base64])

  if (error) return <div className="preview-error-box">DOCX: {error}</div>
  if (!html) return <div className="preview-loading">…</div>
  return <div className="preview-docx" dangerouslySetInnerHTML={{ __html: html }} />
}

// ---------------------------------------------------------------------------
// ZIP
// ---------------------------------------------------------------------------

function ZipView({ base64 }: { base64: string }): React.JSX.Element {
  const [entries, setEntries] = useState<Array<{ name: string; size: number }>>([])
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let alive = true
    ;(async () => {
      try {
        const { unzipSync, strFromU8 } = await import('fflate')
        const data = base64ToBytes(base64)
        const files = unzipSync(data)
        if (!alive) return
        const list = Object.entries(files)
          .filter(([name]) => !name.endsWith('/'))
          .map(([name, content]) => ({ name, size: content.length }))
          .sort((a, b) => a.name.localeCompare(b.name))
        setEntries(list)
        // Touch strFromU8 so the import is retained for future text entry preview
        void strFromU8
      } catch (err) {
        if (alive) setError((err as Error).message)
      }
    })()
    return () => {
      alive = false
    }
  }, [base64])

  if (error) return <div className="preview-error-box">ZIP: {error}</div>
  if (entries.length === 0) return <div className="preview-loading">…</div>
  return (
    <div className="preview-zip">
      <div className="zip-count">
        {entries.length} entries
      </div>
      {entries.map((e) => (
        <div key={e.name} className="zip-entry" title={e.name}>
          <span className="zip-entry-name">
            {e.name.split('/').pop()}
            {e.name.includes('/') && (
              <span className="zip-entry-dir">
                {' '}
                — {e.name.split('/').slice(0, -1).join('/')}
              </span>
            )}
          </span>
          <span className="zip-entry-size">{(e.size / 1024).toFixed(1)} KB</span>
        </div>
      ))}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

function DiffView({ diff }: { diff: string }): React.JSX.Element {
  const lines = diff.split('\n')
  return (
    <div className="preview-diff">
      {lines.map((line, i) => {
        let cls = 'diff-line'
        if (line.startsWith('+++') || line.startsWith('---')) cls += ' diff-file-header'
        else if (line.startsWith('@@')) cls += ' diff-hunk'
        else if (line.startsWith('+')) cls += ' diff-add'
        else if (line.startsWith('-')) cls += ' diff-del'
        return (
          <div key={i} className={cls}>
            <span className="diff-no">{i + 1}</span>
            <span className="diff-text">{line || ' '}</span>
          </div>
        )
      })}
    </div>
  )
}

// ---------------------------------------------------------------------------
// FilePreview router
// ---------------------------------------------------------------------------

// Directory-listing helpers (paths may use / or \ separators)
function joinPath(dir: string, name: string): string {
  const sep = dir.includes('\\') && !dir.includes('/') ? '\\' : '/'
  return dir.replace(/[\\/]+$/, '') + sep + name
}

function parentOf(p: string): string | null {
  const norm = p.replace(/[\\/]+$/, '')
  const idx = Math.max(norm.lastIndexOf('/'), norm.lastIndexOf('\\'))
  if (idx <= 0) return null
  const parent = norm.slice(0, idx)
  if (/^[A-Za-z]:$/.test(parent)) return parent + '\\'
  return parent
}

const IMAGE_EXTS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg', 'ico', 'tiff'])
// Files up to this size are viewed (and can be edited) with CodeMirror;
// larger text files fall back to the plain <pre> preview.
const CM_CAP = 2 * 1024 * 1024

export default function FilePreview({ tab }: { tab: PreviewTab }): React.JSX.Element {
  const { config, currentSessionWorkingDirectory, openFilePreview } = useStore()
  const tr = useTranslation(config.language)
  const [text, setText] = useState<string | null>(null)
  const [textError, setTextError] = useState<string | null>(null)
  const [truncated, setTruncated] = useState(false)
  const [editMode, setEditMode] = useState(false)
  const [saving, setSaving] = useState(false)
  const [savedAt, setSavedAt] = useState(0)
  const [binary, setBinary] = useState<{ base64: string; mime: string } | null>(null)
  const [stat, setStat] = useState<{ exists: boolean; isDir: boolean; size: number } | null>(null)
  const [dirEntries, setDirEntries] = useState<Array<{ name: string; isDir: boolean }>>([])
  const docRef = React.useRef<(() => string) | null>(null)
  const registerDoc = React.useCallback((fn: (() => string) | null) => {
    docRef.current = fn
  }, [])

  const ext = useMemo(() => {
    const e = tab.path.split('.').pop()?.toLowerCase() ?? ''
    // Handle dotfiles like .gitignore (no real extension)
    return e.length > 0 && !tab.path.endsWith('.') ? e : 'txt'
  }, [tab.path])

  const isImage = IMAGE_EXTS.has(ext)
  const isPdf = ext === 'pdf'
  const isSheet = ext === 'xlsx' || ext === 'xls' || ext === 'csv'
  const isDocx = ext === 'docx'
  const isZip = ext === 'zip'
  const isMd = ext === 'md' || ext === 'markdown' || ext === 'mdx'
  const isMermaid = ext === 'mmd' || ext === 'mermaid'

  useEffect(() => {
    let alive = true
    setText(null)
    setTextError(null)
    setTruncated(false)
    setEditMode(false)
    setSavedAt(0)
    setBinary(null)
    setStat(null)
    setDirEntries([])

    ;(async () => {
      const rid = tab.remoteId ?? undefined
      const s = await window.api.fs.stat(tab.path, rid)
      if (!alive) return
      setStat(s)
      if (!s.exists) return
      if (s.isDir) {
        const entries = await window.api.fs.listDir(tab.path, rid)
        if (alive) setDirEntries(entries)
        return
      }
      if (isImage || isPdf || isSheet || isDocx || isZip) {
        const b = await window.api.fs.readBinary(tab.path, rid)
        if (alive) {
          if (b.error) setTextError(b.error)
          else setBinary({ base64: b.base64, mime: b.mime })
        }
      } else {
        const t = await window.api.fs.readText(tab.path, rid)
        if (alive) {
          if (t.error) setTextError(t.error)
          else {
            setText(t.content)
            setTruncated(t.truncated)
          }
        }
      }
    })()

    return () => {
      alive = false
    }
  }, [tab.path, tab.remoteId, isImage, isPdf, isSheet, isDocx, isZip])

  const canEdit =
    tab.kind === 'file' &&
    text !== null &&
    !truncated &&
    !text.includes('\u0000') &&
    (stat?.size ?? 0) <= CM_CAP

  const handleSave = async (): Promise<void> => {
    const newText = docRef.current?.() ?? text
    if (newText === null) return
    setSaving(true)
    const res = await window.api.fs.writeText(tab.path, newText, tab.remoteId ?? undefined)
    setSaving(false)
    if (res.error) {
      setTextError(res.error)
      return
    }
    setText(newText)
    const now = Date.now()
    setSavedAt(now)
    window.setTimeout(() => setSavedAt((v) => (v === now ? 0 : v)), 2000)
  }

  const formatSize = (n: number): string => {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
    return `${(n / (1024 * 1024)).toFixed(1)} MB`
  }

  return (
    <div className="file-preview">
      <div className="file-preview-header">
        <div className="file-preview-title">
          <span className="file-preview-name">{tab.name}</span>
          {stat?.exists && !stat.isDir && (
            <span className="file-preview-size">{formatSize(stat.size)}</span>
          )}
        </div>
        <span className="file-preview-path" title={tab.path}>
          {tab.path}
        </span>
        <div className="file-preview-actions">
          {editMode ? (
            <>
              <button
                className="file-preview-btn primary"
                onClick={handleSave}
                disabled={saving}
                title={tr.saveLabel}
              >
                {saving ? '…' : savedAt ? '✓' : '💾'}
              </button>
              <button
                className="file-preview-btn"
                onClick={() => setEditMode(false)}
                title={tr.cancelLabel}
              >
                ✕
              </button>
            </>
          ) : (
            canEdit && (
              <button
                className="file-preview-btn"
                onClick={() => setEditMode(true)}
                title={tr.editLabel}
              >
                ✏️
              </button>
            )
          )}
          {!(tab.remoteId ?? null) && (
            <button
              className="file-preview-btn"
              title={tr.previewOpenExternally}
              onClick={() => window.api.shell.openPath(tab.path).catch(console.error)}
            >
              ↗
            </button>
          )}
        </div>
      </div>

      <div className="file-preview-body">
        {!stat && <div className="preview-loading">{tr.previewLoading}</div>}
        {stat && !stat.exists && <div className="dock-empty">{tr.previewError}</div>}
        {stat?.isDir && (
          <div className="preview-dir-listing">
            {parentOf(tab.path) && (
              <div
                className="preview-dir-entry preview-dir-click"
                onClick={() => openFilePreview(parentOf(tab.path) as string, undefined, tab.remoteId ?? null)}
              >
                <span>📁</span>
                <span>..</span>
              </div>
            )}
            {dirEntries.map((e) => (
              <div
                key={e.name}
                className="preview-dir-entry preview-dir-click"
                onClick={() => openFilePreview(joinPath(tab.path, e.name), undefined, tab.remoteId ?? null)}
              >
                <span>{e.isDir ? '📁' : '📄'}</span>
                <span>{e.name}</span>
              </div>
            ))}
            {dirEntries.length === 0 && <div className="dock-empty">{tr.fileTreeEmpty}</div>}
          </div>
        )}
        {textError && <div className="preview-error-box">{textError}</div>}

        {tab.kind === 'diff' && tab.diff !== undefined && <DiffView diff={tab.diff} />}

        {tab.kind === 'file' && binary && isImage && (
          <img
            className="preview-image"
            src={base64ToDataUrl(binary.base64, binary.mime)}
            alt={tab.name}
          />
        )}
        {tab.kind === 'file' && binary && isPdf && <PdfView path={tab.path} base64={binary.base64} />}
        {tab.kind === 'file' && binary && isSheet && <SheetView base64={binary.base64} ext={ext} />}
        {tab.kind === 'file' && binary && isDocx && <DocxView base64={binary.base64} />}
        {tab.kind === 'file' && binary && isZip && <ZipView base64={binary.base64} />}

        {/* Edit mode: CodeMirror editor for text / md / code files */}
        {tab.kind === 'file' && text !== null && editMode && (
          <CodeMirrorPane
            key={tab.path + '|edit'}
            path={tab.path}
            content={text}
            editable
            registerDoc={registerDoc}
            onSave={handleSave}
          />
        )}

        {/* View mode: syntax-highlighted CodeMirror for code files */}
        {tab.kind === 'file' &&
          text !== null &&
          !editMode &&
          !isMd &&
          !isMermaid &&
          !text.includes('\u0000') &&
          (stat?.size ?? 0) <= CM_CAP && (
            <CodeMirrorPane key={tab.path + '|view'} path={tab.path} content={text} editable={false} />
          )}
        {tab.kind === 'file' &&
          text !== null &&
          !editMode &&
          !isMd &&
          !isMermaid &&
          !text.includes('\u0000') &&
          (stat?.size ?? 0) > CM_CAP && (
            <pre className="preview-text">
              {text.length > 200_000 ? text.slice(0, 200_000) + '\n\n[...truncated...]' : text}
            </pre>
          )}
        {tab.kind === 'file' && text !== null && !editMode && isMd && <MarkdownView content={text} />}
        {tab.kind === 'file' && text !== null && !editMode && isMermaid && <MermaidView code={text} />}
        {tab.kind === 'file' &&
          text !== null &&
          !editMode &&
          !isMd &&
          !isMermaid &&
          text.includes('\u0000') && (
          <div className="dock-empty">{tr.binaryPreviewHint}</div>
        )}
      </div>
    </div>
  )
}
