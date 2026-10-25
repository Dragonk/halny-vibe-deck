import { ipcMain } from "electron"

export type LocalHandler = (event: unknown, ...args: unknown[]) => unknown

/**
 * Registry of all ipcMain handlers, so the serve websocket can dispatch
 * the same channels. installIpcCapture() must run BEFORE any handler
 * registration (monkey-patches ipcMain.handle).
 */
export const localHandlers = new Map<string, LocalHandler>()

export function installIpcCapture(): void {
  const origHandle = ipcMain.handle.bind(ipcMain) as (
    channel: string,
    fn: (event: Electron.IpcMainInvokeEvent, ...args: never[]) => unknown) => void
  const patched = ipcMain as unknown as { handle: unknown }
  patched.handle = (channel: string, fn: LocalHandler): void => {
    if (typeof channel === "string" && typeof fn === "function") {
      localHandlers.set(channel, fn)
    }
    return origHandle(channel, fn as never)
  }
}
