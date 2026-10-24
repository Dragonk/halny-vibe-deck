import { simpleGit, SimpleGit } from 'simple-git'
import { join } from 'path'

export interface GitFileStatus {
  path: string
  index: string
  working: string
  staged: boolean
}

export interface GitStatus {
  repo: boolean
  branch: string | null
  ahead: number
  behind: number
  files: GitFileStatus[]
  error?: string
}

export interface GitCommit {
  hash: string
  shortHash: string
  message: string
  author: string
  date: string
}

function git(cwd: string): SimpleGit {
  return simpleGit({ baseDir: cwd })
}

export async function gitStatus(cwd: string): Promise<GitStatus> {
  const result: GitStatus = {
    repo: false,
    branch: null,
    ahead: 0,
    behind: 0,
    files: [],
  }
  if (!cwd) return result
  try {
    const g = git(cwd)
    const isRepo = await g.checkIsRepo()
    if (!isRepo) return result
    result.repo = true

    const status = await g.status()
    result.branch = status.current ?? null
    result.ahead = status.ahead
    result.behind = status.behind

    const files = (status.files ?? []).map((f) => {
      const index = (f.index ?? ' ').trim()
      const working = (f.working_dir ?? ' ').trim()
      return {
        path: f.path,
        index: index === '' ? '?' : index,
        working: working === '' ? '?' : working,
        staged: index !== '' && index !== '?' && index !== ' ',
      }
    })
    result.files = files
  } catch (err) {
    result.error = err instanceof Error ? err.message : String(err)
  }
  return result
}

export async function gitLog(cwd: string, max: number): Promise<GitCommit[]> {
  if (!cwd) return []
  try {
    const g = git(cwd)
    const isRepo = await g.checkIsRepo()
    if (!isRepo) return []

    const log = await g.log({ maxCount: max })
    return log.all.map((c) => ({
      hash: c.hash,
      shortHash: c.hash.slice(0, 7),
      message: c.message.split('\n')[0],
      author: (c.author_name ?? '') + (c.author_email ? ' <' + c.author_email + '>' : ''),
      date: new Date(c.date).toISOString(),
    }))
  } catch {
    return []
  }
}

export async function gitDiffFile(cwd: string, filePath: string): Promise<string | null> {
  if (!cwd) return null
  try {
    const g = git(cwd)
    return await g.diff(['--', filePath])
  } catch {
    return null
  }
}

export async function gitDiffAll(cwd: string): Promise<string | null> {
  if (!cwd) return null
  try {
    const g = git(cwd)
    return await g.diff()
  } catch {
    return null
  }
}


export async function gitStage(cwd: string, paths: string[]): Promise<string | null> {
  if (!cwd || paths.length === 0) return null
  try {
    const g = git(cwd)
    if (!(await g.checkIsRepo())) return 'Not a git repository'
    await g.add(paths)
    return null
  } catch (err) {
    return err instanceof Error ? err.message : String(err)
  }
}

export async function gitUnstage(cwd: string, paths: string[]): Promise<string | null> {
  if (!cwd || paths.length === 0) return null
  try {
    const g = git(cwd)
    if (!(await g.checkIsRepo())) return 'Not a git repository'
    await g.reset(['--', ...paths])
    return null
  } catch (err) {
    return err instanceof Error ? err.message : String(err)
  }
}

export async function gitCommit(cwd: string, message: string): Promise<string | null> {
  if (!cwd || !message.trim()) return 'Empty commit message'
  try {
    const g = git(cwd)
    if (!(await g.checkIsRepo())) return 'Not a git repository'
    await g.commit(message)
    return null
  } catch (err) {
    return err instanceof Error ? err.message : String(err)
  }
}

export async function gitPush(cwd: string): Promise<string | null> {
  if (!cwd) return null
  try {
    const g = git(cwd)
    if (!(await g.checkIsRepo())) return 'Not a git repository'
    await g.push()
    return null
  } catch (err) {
    return err instanceof Error ? err.message : String(err)
  }
}