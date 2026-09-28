import { useState, useCallback, memo, useEffect } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  type Video,
  revealInExplorer,
  openFile,
  cancelVideoBgRemoval,
  cancelVideoCompression,
  cancelVideoConversion,
} from '@/lib/tauri'
import { error as logError } from '@/lib/logger'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { Progress } from '@/components/ui/progress'
import {
  Play,
  Trash2,
  ExternalLink,
  FolderSearch,
  Minimize2,
  Info,
  Upload,
  Repeat2,
} from 'lucide-react'
import { formatBytes } from '@/lib/utils'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useRow } from '@/schema/tinybase-schema'
import { Button } from '@/components/ui/button'

interface VideoGridProps {
  videos: Video[]
  selectedIds: number[]
  onSelectionChange: (ids: number[]) => void
  onDelete: (id: number) => void
  onCompressClick?: (ids: number[]) => void
  onConvertClick?: (ids: number[]) => void
  onImport?: () => void
  onDrop?: (files: string[]) => void
  onVideoOpen?: (id: number) => void
}

const VideoGridItem = memo(function VideoGridItem({
  video,
  isSelected,
  onSelect,
  onKeyDown,
  onDelete,
  onCompressClick,
  onConvertClick,
  onVideoOpen,
}: {
  video: Video
  isSelected: boolean
  onSelect: (id: number, event: React.MouseEvent | React.KeyboardEvent) => void
  onKeyDown: (id: number, event: React.KeyboardEvent) => void
  onDelete: (id: number) => void
  onCompressClick?: (ids: number[]) => void
  onConvertClick?: (ids: number[]) => void
  onVideoOpen?: (id: number) => void
}) {
  const bgRemovalState = useRow('video_bg_removals', video.id.toString())
  const compressionState = useRow('video_compressions', video.id.toString())
  const conversionState = useRow('video_conversions', video.id.toString())

  const thumbnailSrc = video.thumbnail_path ? convertFileSrc(video.thumbnail_path) : ''

  const handleClick = useCallback(
    (e: React.MouseEvent) => onSelect(video.id, e),
    [video.id, onSelect]
  )

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => onKeyDown(video.id, e),
    [video.id, onKeyDown]
  )

  const handleDoubleClick = useCallback(() => {
    onVideoOpen?.(video.id)
  }, [video.id, onVideoOpen])

  const isProcessing =
    bgRemovalState?.progress != null && bgRemovalState.progress > 0 && bgRemovalState.progress < 100

  const isCompressing =
    compressionState?.progress != null &&
    compressionState.progress > 0 &&
    compressionState.progress < 100

  const isConverting =
    conversionState?.progress != null &&
    conversionState.progress > 0 &&
    conversionState.progress < 100

  const etaSeconds = isCompressing
    ? null
    : bgRemovalState?.eta_seconds
      ? bgRemovalState.eta_seconds > 0
        ? bgRemovalState.eta_seconds
        : null
      : null

  const handleCancel = useCallback(async () => {
    try {
      if (isCompressing) {
        await cancelVideoCompression([video.id])
      } else if (isConverting) {
        await cancelVideoConversion([video.id])
      } else {
        await cancelVideoBgRemoval([video.id])
      }
    } catch (e) {
      logError(`Failed to cancel: ${e}`)
    }
  }, [video.id, isCompressing, isConverting])

  const formatEta = (seconds: number) => {
    if (seconds < 60) return `${Math.round(seconds)}s`
    const mins = Math.floor(seconds / 60)
    const secs = Math.round(seconds % 60)
    return `${mins}m ${secs}s`
  }

  const formatDuration = (seconds: number) => {
    const hrs = Math.floor(seconds / 3600)
    const mins = Math.floor((seconds % 3600) / 60)
    const secs = Math.floor(seconds % 60)
    if (hrs > 0) {
      return `${hrs}:${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`
    }
    return `${mins}:${secs.toString().padStart(2, '0')}`
  }

  const getFormatBadge = (filepath: string) => {
    const ext = filepath.split('.').pop()?.toUpperCase() || ''
    return ext
  }

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="button"
          tabIndex={0}
          className={`group bg-card hover:border-foreground/50 relative flex flex-col overflow-hidden border transition-all ${
            isSelected ? 'ring-2 ring-blue-600' : ''
          }`}
          onClick={handleClick}
          onKeyDown={handleKeyDown}
          onDoubleClick={handleDoubleClick}
          aria-label={video.filename}
        >
          <div
            className="relative aspect-4/3 w-full overflow-hidden"
            style={{
              backgroundImage: `linear-gradient(45deg, #e5e5e5 25%, transparent 25%), linear-gradient(-45deg, #e5e5e5 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #e5e5e5 75%), linear-gradient(-45deg, transparent 75%, #e5e5e5 75%)`,
              backgroundSize: '20px 20px',
              backgroundPosition: '0 0, 0 10px, 10px -10px, -10px 0px',
              backgroundColor: '#fff',
            }}
          >
            {thumbnailSrc ? (
              <img src={thumbnailSrc} className="size-full object-cover" alt="" />
            ) : (
              <div className="flex size-full items-center justify-center">
                <Play className="text-muted-foreground h-8 w-8" />
              </div>
            )}

            <div className="absolute top-2 right-2 left-2 flex items-start justify-between">
              <div className="flex max-w-[calc(100%-24px)] flex-wrap gap-1">
                {video.compressed_filepath && (
                  <span
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-green-500 text-white"
                    title="Compressed"
                  >
                    <Minimize2 className="h-2.5 w-2.5" />
                  </span>
                )}
                {video.converted_videos.length > 0 && (
                  <span
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-orange-500 text-white"
                    title={`Converted (${video.converted_videos.length})`}
                  >
                    <Repeat2 className="h-2.5 w-2.5" />
                  </span>
                )}
              </div>

              {(video.compressed_filepath ||
                video.bg_removed_filepath ||
                video.converted_videos.length > 0) && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70">
                      <Info className="h-3 w-3" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right" className="max-w-50 p-3">
                    <div className="space-y-2 text-xs">
                      {video.compressed_size && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-green-600">Compressed:</span>
                          <span className="font-medium">{formatBytes(video.compressed_size)}</span>
                        </div>
                      )}
                      {video.bg_removed_size && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-purple-600">BG Removed:</span>
                          <span className="font-medium">{formatBytes(video.bg_removed_size)}</span>
                        </div>
                      )}
                      {video.converted_videos.map((cv) => (
                        <div key={cv.format} className="flex items-center justify-between gap-2">
                          <span className="text-orange-600">
                            Converted ({cv.format.toUpperCase()}):
                          </span>
                          <span className="font-medium">
                            {cv.size != null ? formatBytes(cv.size) : 'Unknown'}
                          </span>
                        </div>
                      ))}
                    </div>
                  </TooltipContent>
                </Tooltip>
              )}
            </div>

            {isProcessing && bgRemovalState && (
              <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-1 bg-black/60 p-2">
                <Progress value={bgRemovalState.progress} className="w-3/4" />
                <span className="text-xs text-white">{bgRemovalState.message}</span>
                {etaSeconds != null && (
                  <span className="text-[10px] text-white/70">ETA: {formatEta(etaSeconds)}</span>
                )}
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    handleCancel()
                  }}
                  className="mt-1 rounded bg-red-600 px-2 py-0.5 text-[10px] text-white hover:bg-red-700"
                >
                  Cancel
                </button>
              </div>
            )}

            {isCompressing && compressionState && (
              <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-1 bg-black/60 p-2">
                <Progress value={compressionState.progress} className="w-3/4" />
                <span className="text-xs text-white">
                  {compressionState.message || 'Compressing...'}
                </span>
                {(compressionState.eta_seconds as number) > 0 && (
                  <span className="text-[10px] text-white/70">
                    ~{Math.round(compressionState.eta_seconds as number)}s left
                  </span>
                )}
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    handleCancel()
                  }}
                  className="mt-1 rounded bg-red-600 px-2 py-0.5 text-[10px] text-white hover:bg-red-700"
                >
                  Cancel
                </button>
              </div>
            )}

            {isConverting && conversionState && (
              <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-1 bg-black/60 p-2">
                <Progress value={conversionState.progress} className="w-3/4" />
                <span className="text-xs text-white">
                  {conversionState.message || 'Converting...'}
                </span>
                {(conversionState.eta_seconds as number) > 0 && (
                  <span className="text-[10px] text-white/70">
                    ~{Math.round(conversionState.eta_seconds as number)}s left
                  </span>
                )}
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    handleCancel()
                  }}
                  className="mt-1 rounded bg-red-600 px-2 py-0.5 text-[10px] text-white hover:bg-red-700"
                >
                  Cancel
                </button>
              </div>
            )}

            <div className="absolute bottom-0 left-0 w-full bg-linear-to-t from-black/70 to-transparent px-2 py-1.5">
              <p className="text-[10px] text-white/80">
                {video.size ? formatBytes(video.size) : 'Unknown'}
              </p>
              <p className="truncate text-xs font-semibold text-white">{video.filename}</p>
              <div className="flex items-center gap-2 text-[10px] text-white/80">
                <span className="rounded bg-white/20 px-1 font-medium">
                  {getFormatBadge(video.filepath)}
                </span>
                {video.width && video.height && (
                  <span className="font-mono">
                    {video.width}x{video.height}
                  </span>
                )}
                {video.duration && <span>{formatDuration(video.duration)}</span>}
                {video.fps && <span>{Math.round(video.fps)}f</span>}
              </div>
            </div>
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem
          onClick={() => openFile(video.filepath).catch((e) => logError(`Failed: ${e}`))}
        >
          <ExternalLink className="mr-2 h-4 w-4" /> Open
        </ContextMenuItem>
        <ContextMenuItem
          onClick={() => revealInExplorer(video.filepath).catch((e) => logError(`Failed: ${e}`))}
        >
          <FolderSearch className="mr-2 h-4 w-4" /> Reveal in Explorer
        </ContextMenuItem>
        {video.bg_removed_filepath && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem
              onClick={() =>
                openFile(video.bg_removed_filepath!).catch((e) => logError(`Failed: ${e}`))
              }
            >
              <ExternalLink className="mr-2 h-4 w-4" /> Open Background Removed
            </ContextMenuItem>
            <ContextMenuItem
              onClick={() =>
                revealInExplorer(video.bg_removed_filepath!).catch((e) => logError(`Failed: ${e}`))
              }
            >
              <FolderSearch className="mr-2 h-4 w-4" /> Reveal Background Removed
            </ContextMenuItem>
          </>
        )}
        {video.compressed_filepath && (
          <>
            <ContextMenuSeparator />
            <ContextMenuItem
              onClick={() =>
                openFile(video.compressed_filepath!).catch((e) => logError(`Failed: ${e}`))
              }
            >
              <ExternalLink className="mr-2 h-4 w-4" /> Open Compressed
            </ContextMenuItem>
            <ContextMenuItem
              onClick={() =>
                revealInExplorer(video.compressed_filepath!).catch((e) => logError(`Failed: ${e}`))
              }
            >
              <FolderSearch className="mr-2 h-4 w-4" /> Reveal Compressed
            </ContextMenuItem>
          </>
        )}
        {video.converted_videos.length > 0 && (
          <>
            <ContextMenuSeparator />
            <ContextMenuSub>
              <ContextMenuSubTrigger>
                <Repeat2 className="mr-2 h-4 w-4" /> Open Converted
              </ContextMenuSubTrigger>
              <ContextMenuSubContent>
                {video.converted_videos.map((cv) => (
                  <ContextMenuItem
                    key={cv.format}
                    onClick={() => openFile(cv.filepath).catch((e) => logError(`Failed: ${e}`))}
                  >
                    <ExternalLink className="mr-2 h-4 w-4" /> {cv.format.toUpperCase()} (
                    {cv.size != null ? formatBytes(cv.size) : 'Unknown'})
                  </ContextMenuItem>
                ))}
              </ContextMenuSubContent>
            </ContextMenuSub>
            <ContextMenuSub>
              <ContextMenuSubTrigger>
                <FolderSearch className="mr-2 h-4 w-4" /> Reveal Converted
              </ContextMenuSubTrigger>
              <ContextMenuSubContent>
                {video.converted_videos.map((cv) => (
                  <ContextMenuItem
                    key={cv.format}
                    onClick={() =>
                      revealInExplorer(cv.filepath).catch((e) => logError(`Failed: ${e}`))
                    }
                  >
                    <FolderSearch className="mr-2 h-4 w-4" /> {cv.format.toUpperCase()}
                  </ContextMenuItem>
                ))}
              </ContextMenuSubContent>
            </ContextMenuSub>
          </>
        )}
        <ContextMenuSeparator />
        {onCompressClick && (
          <ContextMenuItem onClick={() => onCompressClick?.([video.id])}>
            <Minimize2 className="mr-2 h-4 w-4" /> Compress
          </ContextMenuItem>
        )}
        {onConvertClick && (
          <ContextMenuItem onClick={() => onConvertClick([video.id])}>
            <Repeat2 className="mr-2 h-4 w-4" /> Convert Format
          </ContextMenuItem>
        )}
        <ContextMenuSeparator />
        <ContextMenuItem className="text-destructive" onClick={() => onDelete(video.id)}>
          <Trash2 className="mr-2 h-4 w-4" /> Remove
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
})

export function VideoGrid({
  videos,
  selectedIds,
  onSelectionChange,
  onDelete,
  onCompressClick,
  onConvertClick,
  onImport,
  onDrop,
  onVideoOpen,
}: VideoGridProps) {
  const [isDragging, setIsDragging] = useState(false)

  useEffect(() => {
    if (!onDrop) return

    const unlistenDragEnter = listen<{ paths: string[] }>('tauri://drag-enter', () => {
      setIsDragging(true)
    })

    const unlistenDragLeave = listen<{ paths: string[] }>('tauri://drag-leave', () => {
      setIsDragging(false)
    })

    const unlistenDrop = listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
      setIsDragging(false)
      const files = event.payload.paths
      if (files && files.length > 0) {
        onDrop(files)
      }
    })

    return () => {
      unlistenDragEnter.then((fn) => fn())
      unlistenDragLeave.then((fn) => fn())
      unlistenDrop.then((fn) => fn())
    }
  }, [onDrop])

  const handleSelect = useCallback(
    (id: number, event: React.MouseEvent | React.KeyboardEvent) => {
      if (event.shiftKey && selectedIds.length > 0) {
        const lastSelected = selectedIds[selectedIds.length - 1]
        const allIds = videos.map((v) => v.id)
        const lastIdx = allIds.indexOf(lastSelected)
        const currIdx = allIds.indexOf(id)
        const [start, end] = [lastIdx, currIdx].sort((a, b) => a - b)
        const range = allIds.slice(start, end + 1)
        onSelectionChange([...new Set([...selectedIds, ...range])])
      } else if (event.ctrlKey || event.metaKey) {
        if (selectedIds.includes(id)) {
          onSelectionChange(selectedIds.filter((i) => i !== id))
        } else {
          onSelectionChange([...selectedIds, id])
        }
      } else {
        if (selectedIds.includes(id)) {
          onSelectionChange(selectedIds.filter((i) => i !== id))
        } else {
          onSelectionChange([...selectedIds, id])
        }
      }
    },
    [videos, selectedIds, onSelectionChange]
  )

  const handleKeyDown = useCallback(
    (id: number, event: React.KeyboardEvent) => {
      if (event.key === ' ' || event.key === 'Enter') {
        event.preventDefault()
        handleSelect(id, event)
      }
    },
    [handleSelect]
  )

  if (videos.length === 0 && !isDragging) {
    return (
      <div className="flex h-full flex-1 flex-col items-center justify-center gap-4">
        <div className="flex flex-col items-center gap-2 text-center">
          <Upload className="text-muted-foreground h-10 w-10" />
          <p className="text-sm font-medium">No videos found</p>
          <p className="text-muted-foreground text-xs">
            Import videos or drag and drop to get started
          </p>
        </div>
        {onImport && (
          <Button onClick={onImport} variant="outline" size="sm">
            Import Videos
          </Button>
        )}
      </div>
    )
  }

  if (isDragging) {
    return (
      <div className="pointer-events-none flex h-full flex-1 flex-col items-center justify-center">
        <div className="border-primary bg-background rounded-lg border-2 border-dashed p-8 text-center">
          <Upload className="text-primary mx-auto h-10 w-10" />
          <p className="mt-3 text-sm font-medium">Drop videos here</p>
        </div>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-2 gap-4 p-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
      {videos.map((video) => (
        <VideoGridItem
          key={video.id}
          video={video}
          isSelected={selectedIds.includes(video.id)}
          onSelect={handleSelect}
          onKeyDown={handleKeyDown}
          onDelete={onDelete}
          onCompressClick={onCompressClick}
          onConvertClick={onConvertClick}
          onVideoOpen={onVideoOpen}
        />
      ))}
    </div>
  )
}
