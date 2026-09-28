import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useState, useEffect, useMemo, useCallback, memo } from 'react'
import {
  getAllCompressedImages,
  getAllUpscaledImages,
  getAllBgRemovedImages,
  getAllCompressedVideos,
  getAllConvertedImages,
  getAllConvertedVideos,
  revealInExplorer,
  openFile,
  checkDbHealth,
  getFilters,
  updateFilters,
} from '@/lib/tauri'
import { convertFileSrc } from '@tauri-apps/api/core'
import { error as logError } from '@/lib/logger'
import { formatBytes } from '@/lib/utils'
import { useSetValueCallback } from '@/schema/tinybase-schema'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from '@/components/ui/context-menu'
import { Button } from '@/components/ui/button'
import {
  FolderSearch,
  Archive,
  Maximize2,
  FileImage,
  Scissors,
  Minimize2,
  Video,
  Repeat2,
} from 'lucide-react'
import { SearchBar } from '@/components/search-bar'
import { SortDropdown } from '@/components/sort-dropdown'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { RotateCcw } from 'lucide-react'

export const Route = createFileRoute('/_app/output')({
  beforeLoad: async () => {
    const filters = await getFilters('output')
    return { filters }
  },
  loader: async ({ context }) => {
    const { filters } = context
    const [
      compressedImages,
      upscaledImages,
      bgRemovedImages,
      compressedVideos,
      convertedImages,
      convertedVideos,
    ] = await Promise.all([
      getAllCompressedImages(),
      getAllUpscaledImages(),
      getAllBgRemovedImages(),
      getAllCompressedVideos(),
      getAllConvertedImages(),
      getAllConvertedVideos(),
    ])
    return {
      compressedImages,
      upscaledImages,
      bgRemovedImages,
      compressedVideos,
      convertedImages,
      convertedVideos,
      filters,
    }
  },
  gcTime: 0,
  staleTime: 0,
  component: OutputPage,
})

type OutputType =
  | 'all'
  | 'compressed'
  | 'upscaled'
  | 'bg_removed'
  | 'video_compressed'
  | 'converted_images'
  | 'converted_videos'

type UpscaledVersion = {
  scale: number
  filepath: string
  size: number | null
  model: string | null
}

type OutputImage = {
  id: number
  filename: string
  filepath: string
  resultType:
    | 'compressed'
    | 'upscaled'
    | 'bg_removed'
    | 'video_compressed'
    | 'converted_images'
    | 'converted_videos'
  displayFilepath: string
  displaySize: number | null
  upscaled_scale?: number
  isVideo?: boolean
  thumbnailPath?: string | null
}

function parseUpscaledVersions(
  versions: string | UpscaledVersion[] | undefined
): UpscaledVersion[] {
  if (!versions) return []
  try {
    return typeof versions === 'string' ? JSON.parse(versions) : versions
  } catch {
    return []
  }
}

// Memoized grid item to prevent unnecessary re-renders
const OutputGridItem = memo(function OutputGridItem({
  image,
  onOpenInApp,
}: {
  image: OutputImage
  onOpenInApp: (image: OutputImage) => void
}) {
  const [imageError, setImageError] = useState(false)

  const isCompressed = image.resultType === 'compressed'
  const isUpscaled = image.resultType === 'upscaled'
  const isBgRemoved = image.resultType === 'bg_removed'
  const isVideoCompressed = image.resultType === 'video_compressed'
  const isConvertedImages = image.resultType === 'converted_images'
  const isConvertedVideos = image.resultType === 'converted_videos'
  const isConverted = isConvertedImages || isConvertedVideos

  const handleOpen = useCallback(async () => {
    try {
      await openFile(image.displayFilepath)
    } catch (err) {
      logError(`Failed to open file: ${err}`)
    }
  }, [image.displayFilepath])

  const handleReveal = useCallback(async () => {
    try {
      await revealInExplorer(image.displayFilepath)
    } catch (err) {
      logError(`Failed to reveal file: ${err}`)
    }
  }, [image.displayFilepath])

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          role="button"
          tabIndex={0}
          onDoubleClick={() => onOpenInApp(image)}
          className="group bg-card hover:border-foreground/50 relative flex flex-col overflow-hidden border transition-all"
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
            {!imageError ? (
              (isVideoCompressed || isConvertedVideos) && image.thumbnailPath ? (
                <img
                  src={convertFileSrc(image.thumbnailPath)}
                  alt={image.filename}
                  className="size-full object-cover"
                  onError={() => setImageError(true)}
                  loading="lazy"
                />
              ) : (
                <img
                  src={convertFileSrc(image.displayFilepath)}
                  alt={image.filename}
                  className="size-full object-cover"
                  onError={() => setImageError(true)}
                  loading="lazy"
                />
              )
            ) : (
              <div className="flex size-full items-center justify-center">
                <FolderSearch className="text-muted-foreground h-8 w-8" />
              </div>
            )}

            <div className="absolute top-2 left-2 flex gap-1">
              {isVideoCompressed && (
                <span
                  className="flex h-5 w-5 items-center justify-center rounded-full bg-green-500 text-white"
                  title="Video Compressed"
                >
                  <Minimize2 className="h-2.5 w-2.5" />
                </span>
              )}
              {isCompressed && (
                <span
                  className="flex h-5 w-5 items-center justify-center rounded-full bg-green-500 text-white"
                  title="Compressed"
                >
                  <Archive className="h-2.5 w-2.5" />
                </span>
              )}
              {isUpscaled && (
                <span
                  className="flex h-5 w-5 items-center justify-center rounded-full bg-blue-500 text-white"
                  title={`Upscaled ${image.upscaled_scale}x`}
                >
                  <Maximize2 className="h-2.5 w-2.5" />
                </span>
              )}
              {isBgRemoved && (
                <span
                  className="flex h-5 w-5 items-center justify-center rounded-full bg-purple-500 text-white"
                  title="Background Removed"
                >
                  <Scissors className="h-2.5 w-2.5" />
                </span>
              )}
              {isConverted && (
                <span
                  className="flex h-5 w-5 items-center justify-center rounded-full bg-orange-500 text-white"
                  title="Converted"
                >
                  <Repeat2 className="h-2.5 w-2.5" />
                </span>
              )}
            </div>

            <div className="absolute bottom-0 left-0 w-full bg-linear-to-t from-black/70 to-transparent px-2 py-1.5">
              <p className="truncate text-xs font-semibold text-white">{image.filename}</p>
              <div className="flex items-center justify-between text-[10px] text-white/80">
                <span
                  className={
                    isUpscaled
                      ? 'text-blue-300'
                      : isBgRemoved
                        ? 'text-purple-300'
                        : isConverted
                          ? 'text-orange-300'
                          : 'text-green-300'
                  }
                >
                  {image.displaySize ? formatBytes(image.displaySize) : 'Unknown'}
                </span>
                {isUpscaled && image.upscaled_scale && (
                  <span className="text-blue-300">{image.upscaled_scale}x</span>
                )}
              </div>
            </div>
          </div>
        </div>
      </ContextMenuTrigger>

      <ContextMenuContent>
        <ContextMenuItem onClick={() => onOpenInApp(image)}>
          <Maximize2 className="mr-2 h-4 w-4" />
          Open in app
        </ContextMenuItem>
        <ContextMenuItem onClick={handleOpen}>
          <FileImage className="mr-2 h-4 w-4" />
          Open{' '}
          {isVideoCompressed
            ? 'Video'
            : isCompressed
              ? 'Compressed'
              : isBgRemoved
                ? 'Background Removed'
                : isConverted
                  ? 'Converted'
                  : 'Upscaled'}
        </ContextMenuItem>
        <ContextMenuItem onClick={handleReveal}>
          <FolderSearch className="mr-2 h-4 w-4" />
          Reveal in Explorer
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
})

function OutputPage() {
  const loaderData = Route.useLoaderData()
  const navigate = useNavigate()

  // Generated files open in the parent item's detail viewer at that variant.
  const handleOpenInApp = useCallback(
    (image: OutputImage) => {
      if (image.isVideo) {
        navigate({
          to: '/videos/$videoId',
          params: { videoId: String(image.id) },
          search: { view: image.displayFilepath },
        })
      } else {
        navigate({
          to: '/$imageId',
          params: { imageId: String(image.id) },
          search: { view: image.displayFilepath },
        })
      }
    },
    [navigate]
  )

  // Initialize filter state from loader data (fetched via beforeLoad)
  const [outputType, setOutputType] = useState<OutputType>(
    loaderData.filters.output_type as OutputType
  )
  const [searchQuery, setSearchQuery] = useState(loaderData.filters.search_query)
  const [sortField, setSortField] = useState<'name' | 'size' | 'date'>(
    loaderData.filters.sort_field
  )
  const [sortOrder, setSortOrder] = useState<'asc' | 'desc'>(loaderData.filters.sort_order)

  const setDbNeedsSync = useSetValueCallback('dbNeedsSync', () => true)

  // Sync output type changes to SQLite
  useEffect(() => {
    const syncToDb = async () => {
      try {
        await updateFilters({ page: 'output', output_type: outputType })
      } catch (err) {
        logError(`Failed to sync output type: ${err}`)
      }
    }
    syncToDb()
  }, [outputType])

  // Sync search query changes to SQLite
  useEffect(() => {
    const syncToDb = async () => {
      try {
        await updateFilters({ page: 'output', search_query: searchQuery })
      } catch (err) {
        logError(`Failed to sync search query: ${err}`)
      }
    }
    syncToDb()
  }, [searchQuery])

  // Sync sort field changes to SQLite
  useEffect(() => {
    const syncToDb = async () => {
      try {
        await updateFilters({ page: 'output', sort_field: sortField })
      } catch (err) {
        logError(`Failed to sync sort field: ${err}`)
      }
    }
    syncToDb()
  }, [sortField])

  // Sync sort order changes to SQLite
  useEffect(() => {
    const syncToDb = async () => {
      try {
        await updateFilters({ page: 'output', sort_order: sortOrder })
      } catch (err) {
        logError(`Failed to sync sort order: ${err}`)
      }
    }
    syncToDb()
  }, [sortOrder])

  // Check if any filters are active
  const hasActiveFilters =
    searchQuery !== '' || sortField !== 'date' || sortOrder !== 'desc' || outputType !== 'all'

  // Reset all filters to defaults
  const handleResetFilters = useCallback(async () => {
    setOutputType('all')
    setSearchQuery('')
    setSortField('date')
    setSortOrder('desc')
    try {
      await updateFilters({
        page: 'output',
        output_type: 'all',
        search_query: '',
        sort_field: 'date',
        sort_order: 'desc',
      })
    } catch (err) {
      logError(`Failed to reset filters: ${err}`)
    }
  }, [])

  // Check DB health on mount to detect orphaned records
  useEffect(() => {
    checkDbHealth()
      .then((orphanCount) => {
        if (orphanCount > 0) {
          setDbNeedsSync()
        }
      })
      .catch((err) => logError(`Failed to check DB health: ${err}`))
  }, [setDbNeedsSync])

  // Memoized image processing with filtering and sorting
  const images = useMemo(() => {
    const compressedList: OutputImage[] = loaderData.compressedImages
      .filter((img) => img.compressed_filepath)
      .map((img) => ({
        id: img.id,
        filename: img.filename,
        filepath: img.compressed_filepath!,
        resultType: 'compressed' as const,
        displayFilepath: img.compressed_filepath!,
        displaySize: img.compressed_size ?? null,
      }))

    const upscaledList: OutputImage[] = []

    // Process upscaled images - each row has one upscaled version in the JSON
    for (const img of loaderData.upscaledImages) {
      const versions = parseUpscaledVersions(img.upscaled_versions)
      for (const version of versions) {
        upscaledList.push({
          id: img.id,
          filename: img.filename,
          filepath: version.filepath,
          resultType: 'upscaled' as const,
          displayFilepath: version.filepath,
          displaySize: version.size,
          upscaled_scale: version.scale,
        })
      }
    }

    const bgRemovedList: OutputImage[] = loaderData.bgRemovedImages
      .filter((img) => img.bg_removed_filepath)
      .map((img) => ({
        id: img.id,
        filename: img.filename,
        filepath: img.bg_removed_filepath!,
        resultType: 'bg_removed' as const,
        displayFilepath: img.bg_removed_filepath!,
        displaySize: img.bg_removed_size ?? null,
      }))

    const compressedVideosList: OutputImage[] = loaderData.compressedVideos
      .filter((video) => video.compressed_filepath)
      .map((video) => ({
        id: video.id,
        filename: video.filename,
        filepath: video.compressed_filepath!,
        resultType: 'video_compressed' as const,
        displayFilepath: video.compressed_filepath!,
        displaySize: video.compressed_size ?? null,
        isVideo: true,
        thumbnailPath: video.thumbnail_path,
      }))

    const convertedImagesList: OutputImage[] = loaderData.convertedImages.flatMap((img) =>
      img.converted_images.map((ci) => ({
        id: img.id,
        filename: `${img.filename} (${ci.format.toUpperCase()})`,
        filepath: ci.filepath,
        resultType: 'converted_images' as const,
        displayFilepath: ci.filepath,
        displaySize: ci.size ?? null,
      }))
    )

    const convertedVideosList: OutputImage[] = loaderData.convertedVideos.flatMap((video) =>
      video.converted_videos.map((cv) => ({
        id: video.id,
        filename: `${video.filename} (${cv.format.toUpperCase()})`,
        filepath: cv.filepath,
        resultType: 'converted_videos' as const,
        displayFilepath: cv.filepath,
        displaySize: cv.size ?? null,
        isVideo: true,
        thumbnailPath: video.thumbnail_path,
      }))
    )

    let allImages: OutputImage[]
    if (outputType === 'all') {
      allImages = [
        ...compressedList,
        ...upscaledList,
        ...bgRemovedList,
        ...compressedVideosList,
        ...convertedImagesList,
        ...convertedVideosList,
      ]
    } else if (outputType === 'compressed') {
      allImages = compressedList
    } else if (outputType === 'upscaled') {
      allImages = upscaledList
    } else if (outputType === 'video_compressed') {
      allImages = compressedVideosList
    } else if (outputType === 'converted_images') {
      allImages = convertedImagesList
    } else if (outputType === 'converted_videos') {
      allImages = convertedVideosList
    } else {
      allImages = bgRemovedList
    }

    // Apply search filter
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase()
      allImages = allImages.filter((img) => img.filename.toLowerCase().includes(query))
    }

    allImages.sort((a, b) => {
      let comparison
      if (sortField === 'name') {
        comparison = a.filename.localeCompare(b.filename)
      } else if (sortField === 'size') {
        const sizeA = a.displaySize ?? 0
        const sizeB = b.displaySize ?? 0
        comparison = sizeA - sizeB
      } else {
        // date - use id as proxy
        comparison = a.id - b.id
      }
      return sortOrder === 'asc' ? comparison : -comparison
    })

    return allImages
  }, [loaderData, outputType, searchQuery, sortField, sortOrder])

  const handleOutputTypeChange = useCallback((type: OutputType) => {
    setOutputType(type)
  }, [])

  return (
    <>
      <div className="bg-background flex flex-wrap items-center gap-2 border-b p-3">
        {/* Type filter buttons row */}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'all' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('all')}
              size="icon"
            >
              <Archive className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            All (
            {loaderData.compressedImages.length +
              loaderData.upscaledImages.length +
              loaderData.bgRemovedImages.length +
              loaderData.compressedVideos.length +
              loaderData.convertedImages.length +
              loaderData.convertedVideos.length}
            )
          </TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'compressed' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('compressed')}
              size="icon"
            >
              <Minimize2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Compressed ({loaderData.compressedImages.length})</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'upscaled' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('upscaled')}
              size="icon"
            >
              <Maximize2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Upscaled ({loaderData.upscaledImages.length})</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'bg_removed' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('bg_removed')}
              size="icon"
            >
              <Scissors className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Background Removed ({loaderData.bgRemovedImages.length})</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'video_compressed' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('video_compressed')}
              size="icon"
            >
              <Video className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Video Compressed ({loaderData.compressedVideos.length})</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'converted_images' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('converted_images')}
              size="icon"
            >
              <Repeat2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Converted Images ({loaderData.convertedImages.length})</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={outputType === 'converted_videos' ? 'default' : 'secondary'}
              onClick={() => handleOutputTypeChange('converted_videos')}
              size="icon"
            >
              <Repeat2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Converted Videos ({loaderData.convertedVideos.length})</TooltipContent>
        </Tooltip>
        {/* Search and sort row */}
        <div className="ml-auto flex items-center gap-2">
          <SearchBar value={searchQuery} onChange={setSearchQuery} placeholder="Search files..." />
          <SortDropdown
            sortField={sortField}
            sortOrder={sortOrder}
            onSortFieldChange={setSortField}
            onSortOrderChange={setSortOrder}
          />
          {hasActiveFilters && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon"
                  variant="outline"
                  className="h-8 w-8"
                  onClick={handleResetFilters}
                >
                  <RotateCcw className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                <p>Reset filters</p>
              </TooltipContent>
            </Tooltip>
          )}
        </div>
      </div>

      <section className="customScrollStyle relative h-full max-h-[calc(100vh-133px)] overflow-auto">
        {images.length === 0 ? (
          <div className="flex h-full flex-1 flex-col items-center justify-center gap-4">
            <div className="text-center">
              <Archive className="text-muted-foreground mx-auto h-10 w-10 opacity-20" />
              <p className="mt-2 text-sm font-medium">No processed images yet</p>
              <p className="text-muted-foreground text-xs">
                Compress, upscale, or remove backgrounds from images in the main gallery to see them
                here.
              </p>
            </div>
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-4 p-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
            {images.map((image) => (
              <OutputGridItem
                key={`${image.id}-${image.resultType}-${image.displayFilepath}`}
                image={image}
                onOpenInApp={handleOpenInApp}
              />
            ))}
          </div>
        )}
      </section>
    </>
  )
}
