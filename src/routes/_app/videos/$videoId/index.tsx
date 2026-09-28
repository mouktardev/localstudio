import { createFileRoute, Link, useRouter } from '@tanstack/react-router'
import { Button } from '@/components/ui/button'
import { ArrowLeft, ExternalLink, FolderSearch, Trash2 } from 'lucide-react'
import {
  getVideoById,
  getCompressedVariants,
  deleteOutput,
  openFile,
  revealInExplorer,
} from '@/lib/tauri'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { convertFileSrc } from '@tauri-apps/api/core'
import { error as logError } from '@/lib/logger'
import { addNotification } from '@/lib/notifications'
import { useCallback, useMemo, useState } from 'react'
import { cn, formatBytes } from '@/lib/utils'

interface VersionItem {
  id: string
  label: string
  filepath: string
  size: number | null
  model?: string
}

export const Route = createFileRoute('/_app/videos/$videoId/')({
  validateSearch: (search: Record<string, string | undefined>) => ({
    view: search.view || 'original',
  }),
  loader: async ({ params }) => {
    const id = parseInt(params.videoId)
    const [video, compressedVariants] = await Promise.all([
      getVideoById(id),
      getCompressedVariants('video', id),
    ])
    return { video, compressedVariants }
  },
  component: VideoPage,
  errorComponent: () => (
    <div className="flex h-full items-center justify-center">
      <div className="space-y-4 text-center">
        <h2 className="text-destructive text-xl font-semibold">Video not found</h2>
        <p className="text-muted-foreground text-sm">The requested video could not be loaded.</p>
        <Button asChild variant="outline">
          <Link to="/videos" viewTransition={{ types: ['slide-right'] }}>
            <ArrowLeft className="mr-2 size-4" />
            Back to Videos
          </Link>
        </Button>
      </div>
    </div>
  ),
})

function VideoPage() {
  const { video, compressedVariants } = Route.useLoaderData()
  const navigate = Route.useNavigate()
  const router = useRouter()
  const { view } = Route.useSearch()

  const versions: VersionItem[] = useMemo(() => {
    const list: VersionItem[] = [
      { id: 'original', label: 'Original', filepath: video.filepath, size: video.size },
    ]
    const seenCompressed = new Set<string>()
    compressedVariants.forEach((v) => {
      if (!v.filepath || seenCompressed.has(v.filepath)) return
      seenCompressed.add(v.filepath)
      list.push({
        id: `compressed-${v.id}`,
        label: v.label || 'Compressed',
        filepath: v.filepath,
        size: v.size ?? null,
      })
    })
    // Legacy fallback for rows that predate variant metadata.
    if (video.compressed_filepath && !seenCompressed.has(video.compressed_filepath)) {
      list.push({
        id: 'compressed',
        label: 'Compressed',
        filepath: video.compressed_filepath,
        size: video.compressed_size ?? null,
      })
    }
    if (video.bg_removed_filepath) {
      list.push({
        id: 'bg_removed',
        label: 'BG Removed',
        filepath: video.bg_removed_filepath,
        size: video.bg_removed_size ?? null,
        model: video.bg_removed_model || undefined,
      })
    }
    video.converted_videos.forEach((c, i) => {
      list.push({
        id: `converted-${i}`,
        label: c.format.toUpperCase(),
        filepath: c.filepath,
        size: c.size ?? null,
      })
    })
    return list
  }, [video, compressedVariants])

  // `view` may be a tab id or a filepath (used by the Output page links).
  const currentVersion =
    versions.find((v) => v.id === view) || versions.find((v) => v.filepath === view) || versions[0]

  const compressedVariantId = currentVersion.id.startsWith('compressed-')
    ? Number.parseInt(currentVersion.id.slice('compressed-'.length), 10)
    : null

  const [confirmDeleteVariant, setConfirmDeleteVariant] = useState(false)

  const handleDeleteVariant = useCallback(async () => {
    if (compressedVariantId == null) return
    try {
      const report = await deleteOutput('compressed_video', compressedVariantId)
      await addNotification({
        message: `Deleted compressed variant (${report.files_deleted} file(s), ${formatBytes(report.bytes_freed)} freed).`,
        status: 'success',
      })
      setConfirmDeleteVariant(false)
      await router.invalidate()
      navigate({ search: { view: 'original' }, replace: true })
    } catch (err) {
      logError(`Failed to delete variant: ${err}`)
      await addNotification({
        message: `Failed to delete variant: ${err}`,
        status: 'error',
      })
    }
  }, [compressedVariantId, navigate, router])

  const handleBack = useCallback(() => {
    navigate({ to: '/videos', viewTransition: { types: ['slide-right'] } })
  }, [navigate])

  const handleTabChange = useCallback(
    (id: string) => {
      navigate({ search: { view: id }, replace: true })
    },
    [navigate]
  )

  const handleOpen = useCallback(async (fp: string) => {
    try {
      await openFile(fp)
    } catch (err) {
      logError(`Failed to open file: ${err}`)
    }
  }, [])

  const handleReveal = useCallback(async (fp: string) => {
    try {
      await revealInExplorer(fp)
    } catch (err) {
      logError(`Failed to reveal file: ${err}`)
    }
  }, [])

  const sizeDiff = useMemo(() => {
    if (currentVersion.size == null || video.size == null) return null
    return ((currentVersion.size - video.size) / video.size) * 100
  }, [currentVersion.size, video.size])

  return (
    <div className="flex flex-1 flex-col [view-transition-name:main-content]">
      <header className="flex h-12 shrink-0 items-center gap-2 border-b px-2">
        <Button
          variant="ghost"
          size="icon"
          onClick={handleBack}
          title="Back to videos"
          className="shrink-0"
        >
          <ArrowLeft className="size-5" />
        </Button>
        <h1 className="max-w-[180px] shrink-0 truncate text-sm font-semibold">{video.filename}</h1>
        {versions.length > 1 && <div className="bg-border w-px shrink-0 self-stretch" />}
        {versions.length > 1 && (
          <div className="flex min-w-0 shrink items-center gap-0 self-stretch overflow-x-auto">
            {versions.map((v, i) => (
              <div key={v.id} className="flex items-center gap-0 self-stretch">
                {i > 0 && <div className="bg-muted-foreground/20 mx-1.5 w-px self-stretch" />}
                <button
                  type="button"
                  onClick={() => handleTabChange(v.id)}
                  className={cn(
                    'shrink-0 text-[0.7rem] font-medium transition-colors',
                    view === v.id
                      ? 'text-foreground'
                      : 'text-muted-foreground hover:text-foreground'
                  )}
                >
                  {v.label}
                </button>
              </div>
            ))}
          </div>
        )}
      </header>

      <main className="flex min-h-0 flex-1 items-center justify-center bg-[#1a1a1a] p-4">
        <video
          key={currentVersion.id}
          src={convertFileSrc(currentVersion.filepath)}
          controls
          className="max-h-full max-w-full object-contain"
        />
      </main>

      <div className="text-muted-foreground flex h-10 shrink-0 items-center gap-3 border-t px-4 text-xs">
        <span className="text-foreground font-medium">{currentVersion.label}</span>
        {currentVersion.size != null && <span>{formatBytes(currentVersion.size)}</span>}
        {currentVersion.model && (
          <span className="text-muted-foreground/60">— {currentVersion.model}</span>
        )}
        {sizeDiff != null && currentVersion.id !== 'original' && (
          <span className={sizeDiff <= 0 ? 'text-emerald-500' : 'text-amber-500'}>
            ({sizeDiff >= 0 ? '+' : ''}
            {sizeDiff.toFixed(1)}%)
          </span>
        )}
        {video.width && video.height && (
          <span className="ml-auto whitespace-nowrap tabular-nums">
            {video.width}×{video.height}
          </span>
        )}
        <div className="flex items-center gap-1">
          {compressedVariantId != null && (
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive h-7 gap-1.5 text-xs"
              onClick={() => setConfirmDeleteVariant(true)}
            >
              <Trash2 className="size-3.5" />
              Delete variant
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs"
            onClick={() => handleOpen(currentVersion.filepath)}
          >
            <ExternalLink className="size-3.5" />
            Open
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 text-xs"
            onClick={() => handleReveal(currentVersion.filepath)}
          >
            <FolderSearch className="size-3.5" />
            Reveal
          </Button>
        </div>
      </div>

      <AlertDialog open={confirmDeleteVariant} onOpenChange={setConfirmDeleteVariant}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this compressed variant?</AlertDialogTitle>
            <AlertDialogDescription>
              The generated file is removed from disk. Your original video is kept.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleDeleteVariant}>Delete</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
