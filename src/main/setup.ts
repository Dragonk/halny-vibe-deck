import { spawn } from 'child_process'
import path from 'path'
import os from 'os'
import fs from 'fs'

export interface SetupStatus {
  vibAcpFound: boolean
  vibAcpPath: string | null
  uvFound: boolean
  uvPath: string | null
}

export type ProgressCallback = (message: string, type: 'info' | 'error' | 'success') => void

// Build an env with extra bin paths prepended so we find uv/vibe-acp even if not in system PATH
function augmentedEnv(): NodeJS.ProcessEnv {
  const env = { ...process.env }
  const extra: string[] = []
  if (process.platform === 'win32') {
    const appData = env['APPDATA'] || path.join(os.homedir(), 'AppData', 'Roaming')
    extra.push(
      path.join(appData, 'uv', 'bin'),
      path.join(appData, 'uv', 'tools', 'mistral-vibe', 'Scripts'),
      path.join(os.homedir(), '.local', 'bin')
    )
  } else {
    extra.push(
      path.join(os.homedir(), '.local', 'share', 'uv', 'tools', 'mistral-vibe', 'bin'),
      path.join(os.homedir(), '.local', 'bin'),
      '/usr/local/bin'
    )
  }
  const sep = process.platform === 'win32' ? ';' : ':'
  env['PATH'] = [...extra, env['PATH'] || ''].join(sep)
  return env
}

// v1.0.29: async replacement for the old execSync helper. execSync ran in
// the MAIN process and blocked the whole event loop; with a shell that
// died but left grandchildren holding the stdio pipes it hung FOREVER
// (the loop stopped pumping while the port stayed open - the silent 521
// behind the reverse proxy). This version never blocks and kills the
// entire process group on timeout, so no descendant can keep it alive.
export function tryExecAsync(cmd: string, timeoutMs = 5000): Promise<string | null> {
  return new Promise((resolve) => {
    let out = ''
    let settled = false
    const done = (v: string | null): void => {
      if (!settled) {
        settled = true
        clearTimeout(timer)
        resolve(v)
      }
    }
    let proc: ReturnType<typeof spawn>
    try {
      // detached on POSIX = own process group, killable as a group below
      proc = spawn(cmd, [], {
        shell: true,
        env: augmentedEnv(),
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: process.platform !== 'win32',
      })
    } catch {
      resolve(null)
      return
    }
    const timer = setTimeout(() => {
      try {
        if (process.platform === 'win32') proc.kill()
        else process.kill(-proc.pid!, 'SIGKILL')
      } catch {
        try {
          proc.kill('SIGKILL')
        } catch {
          /* already gone */
        }
      }
      done(null)
    }, timeoutMs)
    proc.stdout?.on('data', (d: Buffer) => {
      out += d.toString()
    })
    proc.stderr?.on('data', (d: Buffer) => {
      out += d.toString()
    })
    proc.on('error', () => done(null))
    proc.on('close', (code) => done(code === 0 ? out.trim() : null))
  })
}

// Candidate filesystem paths for a binary (in order of priority)
function candidatePaths(name: string): string[] {
  const ext = process.platform === 'win32' ? '.exe' : ''
  if (process.platform === 'win32') {
    const appData = process.env['APPDATA'] || path.join(os.homedir(), 'AppData', 'Roaming')
    return [
      // Primary location after `uv tool install mistral-vibe`
      path.join(appData, 'uv', 'tools', 'mistral-vibe', 'Scripts', `${name}${ext}`),
      // uv bin directory (may have shim scripts)
      path.join(appData, 'uv', 'bin', `${name}${ext}`),
      path.join(os.homedir(), '.local', 'bin', `${name}${ext}`)
    ]
  } else {
    return [
      path.join(os.homedir(), '.local', 'share', 'uv', 'tools', 'mistral-vibe', 'bin', name),
      path.join(os.homedir(), '.local', 'bin', name),
      `/usr/local/bin/${name}`
    ]
  }
}

export async function findBinaryAsync(name: string): Promise<string | null> {
  // 1. Check PATH (with augmented env)
  const whereResult = await tryExecAsync(
    process.platform === 'win32' ? `where "${name}"` : `which "${name}"`
  )
  if (whereResult) return whereResult.split('\n')[0].trim()

  // 2. Check known filesystem locations
  for (const p of candidatePaths(name)) {
    if (fs.existsSync(p)) return p
  }
  return null
}

export async function checkSetupAsync(): Promise<SetupStatus> {
  const vibAcpPath = await findBinaryAsync('vibe-acp')
  const uvPath = await findBinaryAsync('uv')
  return {
    vibAcpFound: vibAcpPath !== null,
    vibAcpPath,
    uvFound: uvPath !== null,
    uvPath
  }
}

function runCommand(
  cmd: string,
  args: string[],
  onProgress: ProgressCallback
): Promise<void> {
  return new Promise((resolve, reject) => {
    const proc = spawn(cmd, args, {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: augmentedEnv(),
      shell: false
    })

    const handleData = (data: Buffer): void => {
      data
        .toString()
        .split('\n')
        .map((l) => l.trim())
        .filter(Boolean)
        .forEach((l) => onProgress(l, 'info'))
    }

    proc.stdout.on('data', handleData)
    proc.stderr.on('data', handleData)
    proc.on('close', (code) => {
      if (code === 0) resolve()
      else reject(new Error(`Process exited with code ${code}`))
    })
    proc.on('error', reject)
  })
}

async function installUv(onProgress: ProgressCallback): Promise<string> {
  onProgress('Instalowanie uv package manager…', 'info')

  if (process.platform === 'win32') {
    await runCommand('powershell', [
      '-ExecutionPolicy', 'ByPass',
      '-NonInteractive',
      '-Command',
      'irm https://astral.sh/uv/install.ps1 | iex'
    ], onProgress)
  } else {
    await runCommand('sh', ['-c', 'curl -LsSf https://astral.sh/uv/install.sh | sh'], onProgress)
  }

  const uvPath = await findBinaryAsync('uv')
  if (!uvPath) {
    throw new Error(
      'uv nie znaleziono po instalacji. Uruchom aplikację ponownie lub zainstaluj uv ręcznie.'
    )
  }
  onProgress(`uv zainstalowano: ${uvPath}`, 'success')
  return uvPath
}

export async function installDependencies(
  onProgress: ProgressCallback
): Promise<{ success: boolean; vibAcpPath: string | null }> {
  try {
    const status = await checkSetupAsync()

    // Step 1: uv
    let uvPath = status.uvPath
    if (!uvPath) {
      uvPath = await installUv(onProgress)
    } else {
      onProgress(`uv: ${uvPath}`, 'success')
    }

    // Step 2: mistral-vibe
    onProgress('Instalowanie mistral-vibe…', 'info')
    await runCommand(uvPath, ['tool', 'install', 'mistral-vibe'], onProgress)
    onProgress('mistral-vibe zainstalowano.', 'success')

    // Step 3: locate vibe-acp
    const vibAcpPath = await findBinaryAsync('vibe-acp')
    if (vibAcpPath) {
      onProgress(`vibe-acp gotowy: ${vibAcpPath}`, 'success')
      return { success: true, vibAcpPath }
    }

    onProgress(
      'vibe-acp nie znaleziono po instalacji. Uruchom aplikację ponownie.',
      'error'
    )
    return { success: false, vibAcpPath: null }
  } catch (err) {
    onProgress(`Błąd instalacji: ${(err as Error).message}`, 'error')
    return { success: false, vibAcpPath: null }
  }
}
