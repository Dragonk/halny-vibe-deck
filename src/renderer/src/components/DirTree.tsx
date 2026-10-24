import React, { useCallback, useEffect, useState } from 'react'

// Recursive, collapsible directory tree (Settings skill editor + raw
// config browser). Directories list lazily through the fs IPC bridge.

export interface DirTreeSelection {
  full: string
  rel: string
  name: string
  isDir: boolean
}

interface Props {
  rootDir: string
  selectedFull?: string | null
  onSelect: (sel: DirTreeSelection) => void
  onDeleteFile?: (sel: DirTreeSelection) => void
  deleteTitle?: string
  refreshKey?: string | number
}

export default function DirTree({
  rootDir,
  selectedFull,
  onSelect,
  onDeleteFile,
  deleteTitle,
  refreshKey,
}: Props): React.JSX.Element {
  const [entriesByDir, setEntriesByDir] = useState<Record<string, Array<{ name: string; isDir: boolean }>>>({})
  const [expanded, setExpanded] = useState<Set<string>>(new Set())

  const loadDir = useCallback(async (dir: string): Promise<void> => {
    try {
      const res = await window.api.fs.listDir(dir)
      const sorted = [...(res ?? [])].sort((a, b) => {
        if (a.isDir !== b.isDir) return a.isDir ? -1 : 1
        return a.name.localeCompare(b.name)
      })
      setEntriesByDir((prev) => ({ ...prev, [dir]: sorted }))
    } catch {
      setEntriesByDir((prev) => ({ ...prev, [dir]: [] }))
    }
  }, [])

  useEffect(() => {
    setEntriesByDir({})
    setExpanded(new Set())
    if (rootDir) void loadDir(rootDir)
  }, [rootDir, refreshKey, loadDir])

  const toggle = (dir: string): void => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(dir)) next.delete(dir)
      else {
        next.add(dir)
        void loadDir(dir)
      }
      return next
    })
  }

  const renderEntries = (dir: string, depth: number): React.JSX.Element => {
    const entries = entriesByDir[dir] ?? []
    return (
      <>
        {entries.map((entry) => {
          const full = dir + '/' + entry.name
          const rel = full.slice(rootDir.length + 1)
          const isExpanded = expanded.has(full)
          const sel = { full, rel, name: entry.name, isDir: entry.isDir }
          return (
            <div key={full}>
              <div className="dirtree-row">
                <button
                  className={'filetree-item' + (selectedFull === full ? ' selected' : '')}
                  style={{ paddingLeft: 6 + depth * 12 }}
                  onClick={() => {
                    if (entry.isDir) toggle(full)
                    else onSelect(sel)
                  }}
                  title={full}
                >
                  {entry.isDir ? (
                    <span className={'filetree-chevron' + (isExpanded ? ' open' : '')}>▸</span>
                  ) : (
                    <span className="filetree-file">📄</span>
                  )}
                  <span>{entry.name}</span>
                </button>
                {!entry.isDir && onDeleteFile && (
                  <button
                    className="skill-file-del"
                    title={deleteTitle ?? '✕'}
                    onClick={() => onDeleteFile(sel)}
                  >
                    ✕
                  </button>
                )}
              </div>
              {entry.isDir && isExpanded && renderEntries(full, depth + 1)}
            </div>
          )
        })}
      </>
    )
  }

  if (!rootDir) return <div />
  return <div className="dirtree">{renderEntries(rootDir, 0)}</div>
}
