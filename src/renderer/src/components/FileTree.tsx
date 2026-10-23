import React, { useEffect, useState, useCallback, useMemo } from 'react'
import { useStore } from '../store'
import { useTranslation } from '../i18n'

const BS = String.fromCharCode(92)

const SKIP_DIRS = new Set([
  'node_modules',
  '.git',
  '.venv',
  'venv',
  '__pycache__',
  'dist',
  'build',
  'out',
  'target',
  '.next',
  '.cache',
  '.turbo',
  '.idea',
  '.vscode',
])

interface DirEntry {
  name: string
  isDir: boolean
}

interface Props {
  cwd: string
}

// Normalize a directory path so the load key and the render key always
// match (Windows paths arrive with backslashes; keys must be stable).
function normalizeDirPath(dir: string): string {
  return dir.split(BS).join('/')
}

export default function FileTree({ cwd }: Props): React.JSX.Element {
  const { openFilePreview, config } = useStore()
  const tr = useTranslation(config.language)
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [entriesByDir, setEntriesByDir] = useState<Record<string, DirEntry[]>>({})

  const loadDir = useCallback((rawDir: string) => {
    const dir = normalizeDirPath(rawDir)
    window.api.fs
      .listDir(dir)
      .then((entries) => {
        const sorted = [...(entries as DirEntry[])].sort((a, b) => {
          if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
          return a.name.localeCompare(b.name)
        })
        setEntriesByDir((prev) => ({ ...prev, [dir]: sorted }))
      })
      .catch(() => {
        setEntriesByDir((prev) => ({ ...prev, [dir]: [] }))
      })
  }, [])

  useEffect(() => {
    if (cwd) loadDir(cwd)
  }, [cwd, loadDir])

  const toggleExpand = (dir: string): void => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(dir)) next.delete(dir)
      else {
        next.add(dir)
        loadDir(dir)
      }
      return next
    })
  }

  const rootName = useMemo(
    () => (cwd ? cwd.split(/[/\\]/).filter(Boolean).pop() ?? cwd : ''),
    [cwd],
  )

  if (!cwd) {
    return <div className="dock-empty">{tr.openFolderFirst}</div>
  }

  const renderEntries = (dir: string, depth: number): React.JSX.Element => {
    const entries = entriesByDir[dir] ?? []
    return (
      <>
        {entries
          .filter((e) => !(e.isDir && SKIP_DIRS.has(e.name)))
          .map((entry) => {
            const fullPath = `${dir}/${entry.name}`.split(BS).join('/')
            const isExpanded = expanded.has(fullPath)
            return (
              <div key={fullPath}>
                <button
                  className={`filetree-item ${depth > 0 ? '' : ''}`}
                  style={{ paddingLeft: 6 + depth * 12 }}
                  onClick={() => {
                    if (entry.isDir) toggleExpand(fullPath)
                    else openFilePreview(fullPath, entry.name)
                  }}
                  title={fullPath}
                >
                  {entry.isDir ? (
                    <span className={`filetree-chevron ${isExpanded ? 'open' : ''}`}>▸</span>
                  ) : (
                    <span className="filetree-chevron invisible">▸</span>
                  )}
                  <span className={entry.isDir ? 'filetree-dir' : 'filetree-file'}>
                    {entry.name}
                  </span>
                </button>
                {entry.isDir && isExpanded && renderEntries(fullPath, depth + 1)}
              </div>
            )
          })}
        {entries.length === 0 && (
          <div className="filetree-empty" style={{ paddingLeft: 6 + depth * 12 }}>
            {tr.fileTreeEmpty}
          </div>
        )}
      </>
    )
  }

  return (
    <div className="filetree">
      <div className="filetree-root" title={cwd}>
        <span className="filetree-root-name">📁 {rootName}</span>
      </div>
      <div className="filetree-body">{renderEntries(normalizeDirPath(cwd), 0)}</div>
    </div>
  )
}
