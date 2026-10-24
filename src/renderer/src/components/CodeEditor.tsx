import React, { useEffect } from 'react'

// CodeMirror 6 editor pane — lazy-loaded modules, syntax highlighting by
// filename. Shared by the Settings skill/config editors.

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

export function CodeMirrorPane({
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
