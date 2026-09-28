import {
  error as tauriError,
  warn as tauriWarn,
  info as tauriInfo,
  debug as tauriDebug,
  trace as tauriTrace,
} from '@tauri-apps/plugin-log'
import { readAppLog } from '@/lib/tauri'
import type { AppStore } from '@/schema/tinybase-schema'

function fmt(message: unknown, ...args: unknown[]): string {
  const parts = [typeof message === 'string' ? message : JSON.stringify(message)]
  for (const arg of args) {
    parts.push(typeof arg === 'string' ? arg : JSON.stringify(arg))
  }
  return parts.join(' ')
}

/**
 * Forward console.* to tauri-plugin-log so frontend logs land in the terminal
 * and the app log file. `console.log` maps to Info (not Debug) so it is actually
 * captured by the default Info filter.
 */
export function setupLogger() {
  const originalLog = console.log
  const originalDebug = console.debug
  const originalInfo = console.info
  const originalWarn = console.warn
  const originalError = console.error

  console.error = (message: unknown, ...args: unknown[]) => {
    tauriError(fmt(message, ...args))
    originalError(message, ...args)
  }
  console.warn = (message: unknown, ...args: unknown[]) => {
    tauriWarn(fmt(message, ...args))
    originalWarn(message, ...args)
  }
  console.info = (message: unknown, ...args: unknown[]) => {
    tauriInfo(fmt(message, ...args))
    originalInfo(message, ...args)
  }
  console.log = (message: unknown, ...args: unknown[]) => {
    tauriInfo(fmt(message, ...args))
    originalLog(message, ...args)
  }
  console.debug = (message: unknown, ...args: unknown[]) => {
    tauriDebug(fmt(message, ...args))
    originalDebug(message, ...args)
  }
}

let lastSeenLine: string | null = null

/**
 * Polls the tail of the app log so the sidebar can show an unread dot when new
 * lines arrive while the log panel is closed. Returns a stop function.
 */
export function startLogWatcher(store: AppStore): () => void {
  const tick = async () => {
    try {
      const { lines } = await readAppLog(1, false)
      const latest = lines.length > 0 ? lines[lines.length - 1] : null
      if (latest && latest !== lastSeenLine) {
        lastSeenLine = latest
        if (!store.getValue('logsOpen')) {
          store.setValue('logsUnread', true)
        }
      }
    } catch {
      // Logging must never throw.
    }
  }

  void tick()
  const id = window.setInterval(tick, 3000)
  return () => window.clearInterval(id)
}

export {
  tauriError as error,
  tauriWarn as warn,
  tauriInfo as info,
  tauriDebug as debug,
  tauriTrace as trace,
}
