import { useCallback, useEffect, useState } from 'react'
import {
  X,
  RefreshCw,
  FolderSearch,
  ChevronLeft,
  ChevronRight,
  Maximize2,
  Minimize2,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useSidebar } from '@/components/ui/sidebar'
import { cn } from '@/lib/utils'
import { useSetValueCallback } from '@/schema/tinybase-schema'
import {
  dbOverview,
  dbTableRows,
  revealInExplorer,
  type DbOverview,
  type DbTableRows,
} from '@/lib/tauri'
import { error as logError } from '@/lib/logger'

const PAGE_SIZE = 50

function cellText(value: string | number | null): string {
  if (value == null) return '∅'
  return String(value)
}

export function DatabaseViewerPanel() {
  const { state } = useSidebar()
  const close = useSetValueCallback('dbViewerOpen', () => false)

  const [overview, setOverview] = useState<DbOverview | null>(null)
  const [table, setTable] = useState('')
  const [rows, setRows] = useState<DbTableRows | null>(null)
  const [offset, setOffset] = useState(0)
  const [loading, setLoading] = useState(false)
  const [expanded, setExpanded] = useState(false)

  const loadOverview = useCallback(async () => {
    try {
      const data = await dbOverview()
      setOverview(data)
      setTable((current) => current || data.tables[0]?.name || '')
    } catch (err) {
      logError(`Failed to load database overview: ${err}`)
    }
  }, [])

  useEffect(() => {
    loadOverview()
  }, [loadOverview])

  const loadRows = useCallback(async (name: string, off: number) => {
    setLoading(true)
    try {
      setRows(await dbTableRows(name, PAGE_SIZE, off))
    } catch (err) {
      logError(`Failed to load rows from ${name}: ${err}`)
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (table) loadRows(table, offset)
  }, [table, offset, loadRows])

  const handleSelectTable = (name: string) => {
    setTable(name)
    setOffset(0)
  }

  const handleReveal = () => {
    if (!overview?.path) return
    revealInExplorer(overview.path).catch((err) => logError(`Failed to reveal DB: ${err}`))
  }

  const left = state === 'expanded' ? 'var(--sidebar-width)' : 'var(--sidebar-width-icon)'
  const total = rows?.total ?? 0
  const page = Math.floor(offset / PAGE_SIZE) + 1
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE))

  return (
    <div
      className="bg-background fixed right-0 bottom-6.75 z-50 flex flex-col border-t shadow-lg transition-[left] duration-200 ease-linear"
      style={expanded ? { left, top: '2.5rem' } : { left, height: '240px' }}
    >
      <div className="flex shrink-0 items-center justify-between border-b px-3 py-1.5">
        <div className="flex min-w-0 items-center gap-2">
          <span className="text-muted-foreground text-xs font-semibold tracking-widest uppercase">
            Database
          </span>
          {overview && (
            <span className="text-muted-foreground truncate text-[10px]">
              schema v{overview.schema_version} · {overview.path}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="size-6"
            title="Reveal database file"
            onClick={handleReveal}
          >
            <FolderSearch className="size-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-6"
            title="Refresh"
            onClick={() => {
              loadOverview()
              if (table) loadRows(table, offset)
            }}
          >
            <RefreshCw className={cn('size-3.5', loading && 'animate-spin')} />
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
          <Button variant="ghost" size="icon" className="size-6" title="Close" onClick={close}>
            <X className="size-3.5" />
          </Button>
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="customScrollStyle w-52 shrink-0 overflow-y-auto border-r">
          {overview?.tables.map((t) => (
            <button
              key={t.name}
              type="button"
              onClick={() => handleSelectTable(t.name)}
              className={cn(
                'flex w-full items-center justify-between px-3 py-1 text-left text-xs transition-colors',
                t.name === table
                  ? 'bg-muted text-foreground'
                  : 'text-muted-foreground hover:bg-muted/50'
              )}
            >
              <span className="truncate font-mono">{t.name}</span>
              <span className="text-muted-foreground tabular-nums">{t.rows}</span>
            </button>
          ))}
        </div>

        <div className="customScrollStyle min-w-0 flex-1 overflow-auto">
          {rows && rows.rows.length > 0 ? (
            <table className="w-full border-collapse text-xs">
              <thead className="bg-muted/60 sticky top-0">
                <tr>
                  {rows.columns.map((column) => (
                    <th
                      key={column}
                      className="border-b px-2 py-1 text-left font-mono font-medium whitespace-nowrap"
                    >
                      {column}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {rows.rows.map((row, rowIndex) => (
                  <tr key={rowIndex} className="hover:bg-muted/40">
                    {row.map((value, cellIndex) => (
                      <td
                        key={cellIndex}
                        className="max-w-[22rem] truncate border-b px-2 py-1 font-mono"
                        title={cellText(value)}
                      >
                        {cellText(value)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="text-muted-foreground py-6 text-center text-xs">
              {loading ? 'Loading…' : table ? 'No rows.' : 'Select a table.'}
            </p>
          )}
        </div>
      </div>

      <div className="text-muted-foreground flex shrink-0 items-center justify-between border-t px-3 py-1 text-[10px]">
        <span>
          {table
            ? `${table} · ${total} row${total === 1 ? '' : 's'}${
                pages > 1 ? ` · page ${page}/${pages}` : ''
              }`
            : 'Read-only view'}
        </span>
        {pages > 1 && (
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              className="size-5"
              disabled={offset === 0}
              onClick={() => setOffset((o) => Math.max(0, o - PAGE_SIZE))}
            >
              <ChevronLeft className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="size-5"
              disabled={offset + PAGE_SIZE >= total}
              onClick={() => setOffset((o) => o + PAGE_SIZE)}
            >
              <ChevronRight className="size-3.5" />
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}
