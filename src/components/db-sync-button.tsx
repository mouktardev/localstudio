import { useCallback } from 'react'
import { useRouter } from '@tanstack/react-router'
import { DatabaseZap } from 'lucide-react'
import { useValue, useSetValueCallback } from '@/schema/tinybase-schema'
import { orphanScan, orphanCleanup } from '@/lib/tauri'
import { addNotification } from '@/lib/notifications'
import { error as logError } from '@/lib/logger'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

export function DbSyncButton() {
  const router = useRouter()
  const dbNeedsSync = useValue('dbNeedsSync')

  const clearDbNeedsSync = useSetValueCallback('dbNeedsSync', () => false)

  const handleSyncDb = useCallback(async () => {
    try {
      // Preview first, then confirm, then run the shared cleanup routine.
      const preview = await orphanScan()
      const total = preview.orphaned_rows.length + preview.orphaned_files.length
      if (total === 0) {
        await addNotification({ message: 'Database is in sync.', status: 'info' })
        clearDbNeedsSync()
        return
      }

      const confirmed = window.confirm(
        `Remove ${preview.orphaned_rows.length} orphaned record(s) and ` +
          `${preview.orphaned_files.length} orphaned file(s)?`
      )
      if (!confirmed) return

      const result = await orphanCleanup()
      await addNotification({
        message: `Cleaned ${result.deleted_rows} record(s) and ${result.deleted_files} file(s).`,
        status: 'success',
      })
      clearDbNeedsSync()
      await router.invalidate()
    } catch (err) {
      logError(`Failed to sync database: ${err}`)
    }
  }, [clearDbNeedsSync, router])

  return (
    <Tooltip>
      <div className="flex items-center gap-1">
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            className="h-auto justify-start gap-1.5 px-1 py-0.5"
            onClick={handleSyncDb}
          >
            <div className="relative">
              <DatabaseZap className="h-3.5 w-3.5" />
              <span
                className={`absolute -top-0.5 -right-0.5 h-1.5 w-1.5 rounded-full ${
                  dbNeedsSync ? 'bg-amber-500' : 'bg-green-500'
                }`}
              />
            </div>
            <span className="text-[10px]">DB</span>
          </Button>
        </TooltipTrigger>
        <TooltipContent side="top" sideOffset={4}>
          <p>{dbNeedsSync ? 'Database needs sync' : 'Database in sync'} - Click to sync</p>
        </TooltipContent>
      </div>
    </Tooltip>
  )
}
