import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState, useCallback, useEffect } from 'react'
import { useValue } from '@/schema/tinybase-schema'
import {
  getAllVideos,
  selectVideoFiles,
  importVideos,
  deleteVideosByIds,
  deleteItems,
  deleteOutputs,
  downloadFfmpeg,
  getFilters,
  updateFilters,
  resetFilters,
  getCompressionPresets,
  getEncoderCapabilities,
  compressVideosByIdsV2,
  generateVideoThumbnails,
  convertVideosByIdsV2,
  type CompressionPreset,
  type CompressionRequest,
  type DeleteAction,
  type EncoderCapabilities,
  type VideoFormat,
} from '@/lib/tauri'
import type { Video, VideoQueryParams } from '@/lib/tauri'
import {
  getVideoSelections,
  setVideoSelections,
  removeVideoSelection,
  clearVideoSelections,
  addNotification,
} from '@/lib/notifications'
import { VideoGrid } from '@/components/video-grid'
import { VideoTools } from '@/components/video-tools'
import { CompressVideoDialog } from '@/components/dialogs/compress-video-dialog'
import { ConvertFormatDialog } from '@/components/dialogs'
import { error as logError, info as logInfo } from '@/lib/logger'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Progress } from '@/components/ui/progress'
import { Download, Loader2 } from 'lucide-react'

export const Route = createFileRoute('/_app/videos/')({
  beforeLoad: async () => {
    const filters = await getFilters('videos')
    const presets = await getCompressionPresets()
    return { filters, presets }
  },
  loader: async ({ context }) => {
    const { filters } = context
    const [videos, selectedIds] = await Promise.all([getAllVideos(), getVideoSelections()])
    return { videos, selectedIds, filters }
  },
  gcTime: 0,
  staleTime: 0,
  component: VideosPage,
})

function VideosPage() {
  const loaderData = Route.useLoaderData()
  const navigate = useNavigate()

  const handleVideoOpen = useCallback(
    (id: number) => {
      navigate({
        to: '/videos/$videoId',
        params: { videoId: id.toString() },
        search: { view: 'original' },
        viewTransition: { types: ['slide-left'] },
      })
    },
    [navigate]
  )

  const [videos, setVideos] = useState<Video[]>(loaderData.videos)
  const [selectedIds, setSelectedIds] = useState<number[]>(loaderData.selectedIds)

  const [searchQuery, setSearchQuery] = useState(loaderData.filters.search_query)
  const [sortField, setSortField] = useState<'name' | 'size' | 'date'>(
    loaderData.filters.sort_field
  )
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc'>(loaderData.filters.sort_order)

  const [isImporting, setIsImporting] = useState(false)
  const [ffmpegNeeded, setFfmpegNeeded] = useState(false)

  const isDownloadingFfmpeg = useValue('isDownloadingFfmpeg')
  const ffmpegDownloadProgress = useValue('ffmpegDownloadProgress')
  const ffmpegAvailable = useValue('ffmpegAvailable')

  useEffect(() => {
    if (ffmpegAvailable && ffmpegNeeded) {
      setFfmpegNeeded(false)
    }
  }, [ffmpegAvailable, ffmpegNeeded])

  const [compressDialogOpen, setCompressDialogOpen] = useState(false)
  const [convertDialogOpen, setConvertDialogOpen] = useState(false)
  const [compressionPresets, setCompressionPresets] = useState<CompressionPreset[]>([])
  const [encoderCapabilities, setEncoderCapabilities] = useState<EncoderCapabilities>({
    h264: true,
    hevc: false,
    av1: false,
    hardware: [],
  })

  useEffect(() => {
    getCompressionPresets()
      .then(setCompressionPresets)
      .catch((err) => {
        logError(`Failed to load compression presets: ${err}`)
      })
    getEncoderCapabilities()
      .then(setEncoderCapabilities)
      .catch((err) => {
        logError(`Failed to detect encoder capabilities: ${err}`)
      })
  }, [])

  useEffect(() => {
    generateVideoThumbnails()
      .then((count) => {
        if (count > 0) {
          logInfo(`Generated ${count} video thumbnail(s)`)
        }
      })
      .catch((err) => logError(`Failed to generate thumbnails: ${err}`))
  }, [])

  const hasActiveFilters = searchQuery !== '' || sortField !== 'date' || sortOrder !== 'desc'

  const reloadVideos = useCallback(async () => {
    try {
      const params: VideoQueryParams = {
        search: searchQuery || undefined,
        sort_field: sortField,
        sort_order: sortOrder,
      }
      const result = await getAllVideos(params)
      setVideos(result)
    } catch (err) {
      logError(`Failed to load videos: ${err}`)
    }
  }, [searchQuery, sortField, sortOrder])

  useEffect(() => {
    setVideos(loaderData.videos)
  }, [loaderData])

  // Sync filter changes to DB and reload
  useEffect(() => {
    setVideos(loaderData.videos)
  }, [loaderData])

  // Sync filters to DB and reload videos with params when filter state changes
  useEffect(() => {
    const syncAndReload = async () => {
      try {
        await updateFilters({
          page: 'videos',
          search_query: searchQuery,
          sort_field: sortField,
          sort_order: sortOrder,
        })
        const params: VideoQueryParams = {
          search: searchQuery || undefined,
          sort_field: sortField,
          sort_order: sortOrder,
        }
        const result = await getAllVideos(params)
        setVideos(result)
      } catch (err) {
        logError(`Failed to sync and reload videos: ${err}`)
      }
    }
    syncAndReload()
  }, [searchQuery, sortField, sortOrder])

  // Reload videos with current filter params
  useEffect(() => {
    const doReload = async () => {
      try {
        const params: VideoQueryParams = {
          search: searchQuery || undefined,
          sort_field: sortField,
          sort_order: sortOrder,
        }
        const result = await getAllVideos(params)
        setVideos(result)
      } catch (err) {
        logError(`Failed to reload videos: ${err}`)
      }
    }
    doReload()
  }, [searchQuery, sortField, sortOrder])

  const resetFiltersHandler = useCallback(async () => {
    try {
      const data = await resetFilters('videos')
      setSearchQuery(data.search_query)
      setSortField(data.sort_field)
      setSortOrder(data.sort_order)
      const params: VideoQueryParams = {
        search: data.search_query || undefined,
        sort_field: data.sort_field,
        sort_order: data.sort_order,
      }
      const result = await getAllVideos(params)
      setVideos(result)
    } catch (err) {
      logError(`Failed to reset filters: ${err}`)
    }
  }, [])

  const handleImport = useCallback(async () => {
    try {
      if (!ffmpegAvailable) {
        setFfmpegNeeded(true)
        return
      }

      setIsImporting(true)
      const paths = await selectVideoFiles()
      if (!paths || paths.length === 0) {
        setIsImporting(false)
        return
      }

      const result = await importVideos(paths)
      const { imported, duplicates, failed } = result

      if (imported > 0 && paths.length === 1) {
        await addNotification({
          message: `Imported ${imported} video${imported > 1 ? 's' : ''}`,
          status: 'success',
        })
      }

      if (duplicates > 0 && paths.length === 1) {
        await addNotification({
          message: `${duplicates} video${duplicates > 1 ? 's' : ''} already exists, skipped`,
          status: 'info',
        })
      }

      if (paths.length > 1) {
        const parts: string[] = []
        if (imported > 0) parts.push(`${imported} imported videos`)
        if (duplicates > 0) parts.push(`${duplicates} already exists`)
        if (failed > 0) parts.push(`${failed} failed`)

        await addNotification({
          message: parts.join(', '),
          status: failed > 0 ? 'error' : 'success',
        })
      }

      await reloadVideos()
    } catch (err) {
      logError(`Failed to import videos: ${err}`)
    } finally {
      setIsImporting(false)
    }
  }, [reloadVideos, ffmpegAvailable])

  const handleDownloadFfmpeg = useCallback(async () => {
    try {
      await downloadFfmpeg()
      setFfmpegNeeded(false)
      await addNotification({ message: 'FFmpeg downloaded successfully', status: 'success' })
    } catch (err) {
      logError(`Failed to download FFmpeg: ${err}`)
    }
  }, [])

  const handleSelectionChange = useCallback(async (ids: number[]) => {
    setSelectedIds(ids)
    try {
      await setVideoSelections(ids)
    } catch (err) {
      logError(`Failed to sync video selections: ${err}`)
    }
  }, [])

  const handleDelete = useCallback(
    async (id: number) => {
      try {
        await deleteVideosByIds([id])
        // Only touch the persisted selection when this id was actually selected.
        if (selectedIds.includes(id)) {
          await removeVideoSelection(id)
        }
        setSelectedIds((prev) => prev.filter((i) => i !== id))
        await reloadVideos()
        await addNotification({ message: 'Removed 1 video', status: 'success' })
      } catch (err) {
        logError(`Failed to delete video: ${err}`)
      }
    },
    [reloadVideos, selectedIds]
  )

  const handleDeleteSelected = useCallback(
    async (action: DeleteAction) => {
      if (selectedIds.length === 0) return
      const count = selectedIds.length
      try {
        const report =
          action === 'outputs'
            ? await deleteOutputs('video', selectedIds)
            : await deleteItems('video', selectedIds, action === 'all')

        setSelectedIds([])
        await clearVideoSelections()
        await reloadVideos()

        const parts = [
          action === 'outputs'
            ? `Deleted generated outputs for ${count} video${count > 1 ? 's' : ''}`
            : `Removed ${count} video${count > 1 ? 's' : ''}`,
        ]
        if (report.files_deleted > 0) parts.push(`${report.files_deleted} file(s) deleted`)
        if (report.skipped.length > 0) parts.push(`${report.skipped.length} file(s) kept`)
        await addNotification({ message: parts.join(' · '), status: 'success' })
      } catch (err) {
        logError(`Failed to delete videos: ${err}`)
      }
    },
    [selectedIds, reloadVideos]
  )

  const handleCompress = useCallback(
    async (ids: number[], request: CompressionRequest) => {
      try {
        const result = await compressVideosByIdsV2(ids, request)
        const kept = result.results.filter((r) => r.kept_original).length

        if (result.failed > 0) {
          const firstError = result.results.find((r) => r.status === 'failed')?.message
          await addNotification({
            message: `Compressed ${result.processed}, failed ${result.failed}${firstError ? ` — ${firstError}` : ''}`,
            status: 'error',
          })
        } else if (result.processed > 0) {
          await addNotification({
            message: `Compressed ${result.processed} video${result.processed > 1 ? 's' : ''}${kept > 0 ? ` (${kept} already optimal)` : ''}`,
            status: 'success',
          })
        } else if (result.cancelled > 0) {
          await addNotification({
            message: `Cancelled ${result.cancelled} compression${result.cancelled > 1 ? 's' : ''}`,
            status: 'info',
          })
        } else {
          await addNotification({ message: 'Nothing to compress', status: 'info' })
        }

        if (result.processed > 0) {
          await reloadVideos()
        }
      } catch (err) {
        logError(`Failed to compress videos: ${err}`)
      }
    },
    [reloadVideos]
  )

  const handleConvertSelected = useCallback(
    async (ids: number[], format: string) => {
      try {
        const result = await convertVideosByIdsV2(ids, format as VideoFormat)

        if (result.failed > 0) {
          const firstError = result.errors[0]?.message
          await addNotification({
            message: `Converted ${result.processed}, failed ${result.failed}${firstError ? ` — ${firstError}` : ''}`,
            status: 'error',
          })
        } else if (result.processed > 0) {
          await addNotification({
            message: `Converted ${result.processed} video${result.processed === 1 ? '' : 's'} to ${format.toUpperCase()}`,
            status: 'success',
          })
        } else if (result.cancelled > 0) {
          await addNotification({
            message: `Cancelled ${result.cancelled} conversion${result.cancelled > 1 ? 's' : ''}`,
            status: 'info',
          })
        } else {
          await addNotification({ message: 'Nothing to convert', status: 'info' })
        }

        await reloadVideos()
      } catch (err) {
        logError(`Failed to convert videos: ${err}`)
      }
    },
    [reloadVideos]
  )

  const openCompressDialog = useCallback(() => {
    if (selectedIds.length > 0) {
      setCompressDialogOpen(true)
    }
  }, [selectedIds])

  const openConvertDialog = useCallback(() => {
    if (selectedIds.length > 0) {
      setConvertDialogOpen(true)
    }
  }, [selectedIds])

  const handleDrop = useCallback(
    async (files: string[]) => {
      if (files.length === 0) return

      if (!ffmpegAvailable) {
        setFfmpegNeeded(true)
        return
      }

      setIsImporting(true)
      try {
        const result = await importVideos(files)
        const { imported, duplicates, failed } = result

        if (imported > 0 && files.length === 1) {
          await addNotification({
            message: `Imported ${imported} video${imported > 1 ? 's' : ''}`,
            status: 'success',
          })
        }

        if (duplicates > 0 && files.length === 1) {
          await addNotification({
            message: `${duplicates} video${duplicates > 1 ? 's' : ''} already exists, skipped`,
            status: 'info',
          })
        }

        if (files.length > 1) {
          const parts: string[] = []
          if (imported > 0) parts.push(`${imported} imported`)
          if (duplicates > 0) parts.push(`${duplicates} already exists`)
          if (failed > 0) parts.push(`${failed} failed`)

          await addNotification({
            message: parts.join(', '),
            status: failed > 0 ? 'error' : 'success',
          })
        }

        await reloadVideos()
      } catch (err) {
        logError(`Failed to import dropped videos: ${err}`)
      } finally {
        setIsImporting(false)
      }
    },
    [reloadVideos, ffmpegAvailable]
  )

  return (
    <>
      {isImporting && (
        <div className="bg-background/80 absolute inset-0 z-50 flex items-center justify-center">
          <div className="flex flex-col items-center gap-3">
            <Loader2 className="text-primary h-8 w-8 animate-spin" />
            <p className="text-muted-foreground text-sm">Importing videos...</p>
          </div>
        </div>
      )}

      <VideoTools
        videos={videos}
        selectedIds={selectedIds}
        onSelectionChange={handleSelectionChange}
        onDeleteClick={handleDeleteSelected}
        onImportClick={handleImport}
        onCompressClick={openCompressDialog}
        onConvertClick={openConvertDialog}
        isImporting={isImporting}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        sortField={sortField}
        sortOrder={sortOrder}
        onSortFieldChange={setSortField}
        onSortOrderChange={setSortOrder}
        hasActiveFilters={hasActiveFilters}
        onResetFilters={resetFiltersHandler}
      />

      <div className="flex-1 overflow-auto">
        <VideoGrid
          videos={videos}
          selectedIds={selectedIds}
          onSelectionChange={handleSelectionChange}
          onDelete={handleDelete}
          onCompressClick={(ids) => {
            handleSelectionChange(ids)
            setCompressDialogOpen(true)
          }}
          onConvertClick={(ids) => {
            handleSelectionChange(ids)
            setConvertDialogOpen(true)
          }}
          onImport={handleImport}
          onDrop={handleDrop}
          onVideoOpen={handleVideoOpen}
        />
      </div>

      <Dialog open={ffmpegNeeded} onOpenChange={setFfmpegNeeded}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>FFmpeg Required</DialogTitle>
            <DialogDescription>
              FFmpeg is required to import and process videos. It needs to be downloaded before you
              can continue.
            </DialogDescription>
          </DialogHeader>
          {!ffmpegAvailable && (
            <div className="flex items-start gap-2 rounded-md border p-3">
              <Download className="text-muted-foreground mt-0.5 h-5 w-5" />
              <div className="flex flex-col gap-1">
                <span className="text-sm font-medium">FFmpeg (~80 MB)</span>
                <span className="text-muted-foreground text-xs">
                  Required for video metadata extraction and processing
                </span>
                {isDownloadingFfmpeg && (
                  <div className="flex items-center gap-2 pt-1">
                    <Progress value={ffmpegDownloadProgress} className="w-24" />
                    <span className="text-xs">{ffmpegDownloadProgress}%</span>
                  </div>
                )}
              </div>
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setFfmpegNeeded(false)}>
              Cancel
            </Button>
            {!isDownloadingFfmpeg && (
              <Button onClick={handleDownloadFfmpeg}>
                <Download className="mr-2 h-4 w-4" />
                Download FFmpeg
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <CompressVideoDialog
        videos={videos}
        videoIds={selectedIds}
        open={compressDialogOpen}
        onOpenChange={setCompressDialogOpen}
        onConfirm={handleCompress}
        presets={compressionPresets}
        capabilities={encoderCapabilities}
      />
      <ConvertFormatDialog
        items={videos.map((v) => ({ id: v.id, filename: v.filename, size: v.size }))}
        ids={selectedIds}
        open={convertDialogOpen}
        onOpenChange={setConvertDialogOpen}
        onConfirm={handleConvertSelected}
        isVideo={true}
      />
    </>
  )
}
