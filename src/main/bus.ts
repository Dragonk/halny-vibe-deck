import { EventEmitter } from "events"

export interface BusEvent {
  ch: string
  args: unknown[]
}

/**
 * Broadcast bus: main process -> all connected web (serve) clients.
 * Anything pushed here is forwarded over every authenticated websocket.
 */
export const bus = new EventEmitter()
bus.setMaxListeners(500)

export function broadcastToWeb(ch: string, args: unknown[]): void {
  bus.emit("bus-event", { ch, args } satisfies BusEvent)
}
