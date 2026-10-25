import React, { useEffect, useMemo } from 'react'
import { useStore, fileTreeKey } from '../store'
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
  /** v0.7.0: remote connection id (null = local filesystem) */
  remoteId?: string | null
}

// Normalize a directory path so the load key and the render key always
// match (Windows paths arrive with backslashes; keys must be stable).
function normalizeDirPath(dir: string): string {
  return dir.split(BS).join('/')
}

// The expansion state and directory entries live in the app store, so they
// survive switching dock tabs (e.g. opening a file preview and coming back).
export default function FileTree({ cwd, remoteId }: Props): React.JSX.Element {
  const { openFilePreview, config, fileTreeExpanded, fileTreeEntries, loadFileTreeDir, toggleFileTreeDir } = useStore()
  const tr = useTranslation(config.language)
  const rid = remoteId ?? null

  useEffect(() => {
    if (cwd) loadFileTreeDir(cwd, rid)
  }, [cwd, rid, loadFileTreeDir])

  const rootName = useMemo(
    () => (cwd ? cwd.split(/[/\\\\]/).filter(Boolean).pop() ?? cwd : ''),
    [cwd],
  )

  if (!cwd) {
    return <div className="dock-empty">{tr.openFolderFirst}</div>
  }

  const renderEntries = (dir: string, depth: number): React.JSX.Element => {
    const entries = fileTreeEntries[fileTreeKey(dir, rid)] ?? []
    return (
      <>
        {entries
          .filter((e) => !(e.isDir && SKIP_DIRS.has(e.name)))
          .map((entry) => {
            const fullPath = `${dir}/${entry.name}`.split(BS).join('/')
            const isExpanded = fileTreeExpanded.has(fileTreeKey(fullPath, rid))
            return (
              <div key={fullPath}>
                <button
                  className={`filetree-item ${depth > 0 ? '' : ''}`}
                  style={{ paddingLeft: 6 + depth * 12 }}
                  onClick={() => {
                    if (entry.isDir) toggleFileTreeDir(fullPath, rid)
                    else openFilePreview(fullPath, entry.name, rid)
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
