import { useEffect, useMemo, useRef, useState } from 'react'
import { X, Trash2, Maximize2, Minimize2, FolderSearch, History } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useSidebar } from '@/components/ui/sidebar'
import { cn } from '@/lib/utils'
import { useSetValueCallback } from '@/schema/tinybase-schema'
import { readAppLog, revealInExplorer } from '@/lib/tauri'
import { error as logError } from '@/lib/logger'

const LEVEL_CLASS: Record<string, string> = {
  TRACE: 'text-muted-foreground',
  DEBUG: 'text-muted-foreground',
  INFO: 'text-blue-400',
  WARN: 'text-amber-400',
  ERROR: 'text-red-400',
}

function parseLevel(line: string): string {
  const match = line.match(/\[(TRACE|DEBUG|INFO|WARN|ERROR)\]/)
  return match ? match[1] : 'INFO'
}

/** Shows the tail of the app log file — the same text as the terminal. */
export function LogPanel() {
  const { state } = useSidebar()
  const closeLogPanel = useSetValueCallback('logsOpen', () => false)
  const markRead = useSetValueCallback('logsUnread', () => false)

  const [lines, setLines] = useState<string[]>([])
  const [logPath, setLogPath] = useState('')
  const [expanded, setExpanded] = useState(false)
  const [allSessions, setAllSessions] = useState(false)
  const [clearMarker, setClearMarker] = useState<string | null>(null)
  const [autoScroll, setAutoScroll] = useState(true)
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    markRead()
  }, [markRead])

  useEffect(() => {
    let active = true
    const tick = async () => {
      try {
        const data = await readAppLog(1000, allSessions)
        if (!active) return
        setLogPath(data.path)
        setLines(data.lines)
      } catch {
        // Logging must never throw.
      }
    }
    void tick()
    const id = window.setInterval(tick, 1500)
    return () => {
      active = false
      window.clearInterval(id)
    }
  }, [allSessions])

  const visible = useMemo(() => {
    if (!clearMarker) return lines
    const index = lines.lastIndexOf(clearMarker)
    return index === -1 ? lines : lines.slice(index + 1)
  }, [lines, clearMarker])

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [visible, autoScroll])

  const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget
    setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 40)
  }

  const handleClear = () => setClearMarker(lines.length > 0 ? lines[lines.length - 1] : null)

  const handleReveal = () => {
    if (!logPath) return
    revealInExplorer(logPath).catch((err) => logError(`Failed to reveal log: ${err}`))
  }

  const left = state === 'expanded' ? 'var(--sidebar-width)' : 'var(--sidebar-width-icon)'

  return (
    <div
      className="bg-background fixed right-0 bottom-6.75 z-50 flex flex-col border-t shadow-lg transition-[left] duration-200 ease-linear"
      style={expanded ? { left, top: '2.5rem' } : { left, height: '240px' }}
    >
      <div className="flex shrink-0 items-center justify-between border-b px-3 py-1.5">
        <div className="flex min-w-0 items-center gap-2">
          <span className="text-muted-foreground text-xs font-semibold tracking-widest uppercase">
            Logs
          </span>
          {logPath && <span className="text-muted-foreground truncate text-[10px]">{logPath}</span>}
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant={allSessions ? 'secondary' : 'ghost'}
            size="sm"
            className="h-6 gap-1 px-1.5 text-[10px]"
            title={
              allSessions
                ? 'Showing all sessions (incl. earlier runs)'
                : 'Show all sessions (for debugging a crash)'
            }
            onClick={() => {
              setAllSessions((v) => !v)
              setClearMarker(null)
            }}
          >
            <History className="size-3.5" />
            {allSessions ? 'All' : 'Session'}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-6"
            title="Reveal log file"
            onClick={handleReveal}
          >
            <FolderSearch className="size-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-6"
            title="Clear view"
            onClick={handleClear}
          >
            <Trash2 className="size-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-6"
            title={expanded ? 'Restore' : 'Maximize'}
            onClick={() => setExpanded((v) => !v)}
          >
            {expanded ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-6"
            title="Close"
            onClick={closeLogPanel}
          >
            <X className="size-3.5" />
          </Button>
        </div>
      </div>

      <div
        ref={scrollRef}
        className="customScrollStyle min-h-0 flex-1 overflow-y-auto px-2 py-1 font-mono text-xs"
        onScroll={handleScroll}
      >
        {visible.length === 0 ? (
          <p className="text-muted-foreground py-6 text-center">No logs yet.</p>
        ) : (
          <div className="space-y-0.5">
            {visible.map((line, index) => (
              <div
                key={`${index}-${line.slice(0, 24)}`}
                className={cn(LEVEL_CLASS[parseLevel(line)])}
              >
                <span className="break-all">{line}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
