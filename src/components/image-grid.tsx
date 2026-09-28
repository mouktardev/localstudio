import { useState, useCallback, useEffect, useMemo, memo } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
  ContextMenuLabel,
} from '@/components/ui/context-menu'
import { Progress } from '@/components/ui/progress'
import { Button } from '@/components/ui/button'
import { formatBytes } from '@/lib/utils'
import { type Image, revealInExplorer, openFile, deleteImage, cancelImageJobs } from '@/lib/tauri'
import { error as logError } from '@/lib/logger'
import {
  ExternalLink,
  FolderSearch,
  Trash2,
  Upload,
  Archive,
  Columns,
  Maximize2,
  Scissors,
  Info,
  Minimize2,
  Repeat2,
  ImageIcon,
} from 'lucide-react'
import { useRow } from '@/schema/tinybase-schema'
import { ImageCompare } from '@/components/image-compare'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

interface ImageGridProps {
  images: Image[]
  selectedIds: number[]
  onSelectionChange: (ids: number[]) => void
  onDelete: (id: number) => void
  onImport: () => void
  onDrop: (files: string[]) => void
  onCompressClick?: (ids: number[]) => void
  onUpscaleClick?: (ids: number[]) => void
  onBgRemovalClick?: (ids: number[]) => void
  onConvertClick?: (ids: number[]) => void
  onImageOpen?: (id: number) => void
}

const ImageGridItem = memo(function ImageGridItem({
  image,
  isSelected,
  onSelect,
  onKeyDown,
  onDelete,
  onCompare,
  onCompressClick,
  onUpscaleClick,
  onBgRemovalClick,
  onConvertClick,
  onImageOpen,
}: {
  image: Image
  isSelected: boolean
  onSelect: (id: number, event: React.MouseEvent | React.KeyboardEvent) => void
  onKeyDown: (id: number, event: React.KeyboardEvent) => void
  onDelete: (id: number) => void
  onCompare: (image: Image, type: 'compressed' | 'bg_removed') => void
  onCompressClick?: (ids: number[]) => void
  onUpscaleClick?: (ids: number[]) => void
  onBgRemovalClick?: (ids: number[]) => void
  onConvertClick?: (ids: number[]) => void
  onImageOpen?: (id: number) => void
}) {
  const [imageError, setImageError] = useState(false)
  const compressingState = useRow('compressions', image.id.toString())
  const upscalingState = useRow('upscalings', image.id.toString())
  const bgRemovalState = useRow('bg_removals', image.id.toString())
  const conversionState = useRow('image_conversions', image.id.toString())

  // Parse upscaled_versions from JSON string
  const upscaledVersions = useMemo(() => {
    if (!image.upscaled_versions) return []
    try {
      const parsed =
        typeof image.upscaled_versions === 'string'
          ? JSON.parse(image.upscaled_versions)
          : image.upscaled_versions
      return Array.isArray(parsed) ? parsed.filter((v) => v && v.scale && v.filepath) : []
    } catch {
      return []
    }
  }, [image.upscaled_versions])

  const handleOpen = useCallback(async () => {
    try {
      await openFile(image.filepath)
    } catch (err) {
      logError(`Failed to open file: ${err}`)
    }
  }, [image.filepath])

  const handleReveal = useCallback(async () => {
    try {
      await revealInExplorer(image.filepath)
    } catch (err) {
      logError(`Failed to reveal file: ${err}`)
    }
  }, [image.filepath])

  const handleOpenCompressed = useCallback(async () => {
    if (!image.compressed_filepath) return
    try {
      await openFile(image.compressed_filepath)
    } catch (err) {
      logError(`Failed to open compressed file: ${err}`)
    }
  }, [image.compressed_filepath])

  const handleRevealCompressed = useCallback(async () => {
    if (!image.compressed_filepath) return
    try {
      await revealInExplorer(image.compressed_filepath)
    } catch (err) {
      logError(`Failed to reveal compressed file: ${err}`)
    }
  }, [image.compressed_filepath])

  const handleDelete = useCallback(async () => {
    try {
      await deleteImage(image.id)
      onDelete(image.id)
    } catch (err) {
      logError(`Failed to delete image: ${err}`)
    }
  }, [image.id, onDelete])

  const handleCompareCompressed = useCallback(() => {
    if (!image.compressed_filepath) return
    onCompare(image, 'compressed')
  }, [image, onCompare])

  const handleCompareBgRemoved = useCallback(() => {
    if (!image.bg_removed_filepath) return
    onCompare(image, 'bg_removed')
  }, [image, onCompare])

  const handleOpenBgRemoved = useCallback(async () => {
    if (!image.bg_removed_filepath) return
    try {
      await openFile(image.bg_removed_filepath)
    } catch (err) {
      logError(`Failed to open background removed file: ${err}`)
    }
  }, [image.bg_removed_filepath])

  const handleRevealBgRemoved = useCallback(async () => {
    if (!image.bg_removed_filepath) return
    try {
      await revealInExplorer(image.bg_removed_filepath)
    } catch (err) {
      logError(`Failed to reveal background removed file: ${err}`)
    }
  }, [image.bg_removed_filepath])

  const handleOpenConverted = useCallback(async (filepath: string) => {
    try {
      await openFile(filepath)
    } catch (err) {
      logError(`Failed to open converted file: ${err}`)
    }
  }, [])

  const handleRevealConverted = useCallback(async (filepath: string) => {
    try {
      await revealInExplorer(filepath)
    } catch (err) {
      logError(`Failed to reveal converted file: ${err}`)
    }
  }, [])

  const handleClick = useCallback(
    (e: React.MouseEvent) => onSelect(image.id, e),
    [image.id, onSelect]
  )

  const handleKeyDownLocal = useCallback(
    (e: React.KeyboardEvent) => onKeyDown(image.id, e),
    [image.id, onKeyDown]
  )

  const handleDoubleClick = useCallback(() => {
    onImageOpen?.(image.id)
  }, [image.id, onImageOpen])

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
          onKeyDown={handleKeyDownLocal}
          onDoubleClick={handleDoubleClick}
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
              <img
                src={convertFileSrc(image.filepath)}
                alt={image.filename}
                className="size-full object-cover"
                onError={() => setImageError(true)}
                loading="lazy"
              />
            ) : (
              <div className="flex size-full items-center justify-center">
                <FolderSearch className="text-muted-foreground h-8 w-8" />
              </div>
            )}

            <div className="absolute top-2 right-2 left-2 flex items-start justify-between">
              <div className="flex max-w-[calc(100%-24px)] flex-wrap gap-1">
                {/* {isSelected && (
                  <span className="text-primary-foreground flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-blue-600">
                    <Check className="h-3 w-3" />
                  </span>
                )} */}
                {image.compressed_filepath && (
                  <span
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-green-500 text-white"
                    title="Compressed"
                  >
                    <Archive className="h-2.5 w-2.5" />
                  </span>
                )}
                {upscaledVersions.map(
                  (version: { scale: number; filepath: string }, idx: number) => (
                    <span
                      key={idx}
                      className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-blue-500 text-white"
                      title={`Upscaled ${version.scale}x`}
                    >
                      <Maximize2 className="h-2.5 w-2.5" />
                    </span>
                  )
                )}
                {image.bg_removed_filepath && (
                  <span
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-purple-500 text-white"
                    title="Background Removed"
                  >
                    <Scissors className="h-2.5 w-2.5" />
                  </span>
                )}
                {image.converted_images.length > 0 && (
                  <span
                    className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-orange-500 text-white"
                    title={`Converted (${image.converted_images.length})`}
                  >
                    <Repeat2 className="h-2.5 w-2.5" />
                  </span>
                )}
              </div>

              {(image.compressed_filepath ||
                upscaledVersions.length > 0 ||
                image.bg_removed_filepath ||
                image.converted_images.length > 0) && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button className="flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70">
                      <Info className="h-3 w-3" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right" className="max-w-50 p-3">
                    <div className="space-y-2 text-xs">
                      {image.compressed_size && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-green-600">Compressed:</span>
                          <span className="font-medium">{formatBytes(image.compressed_size)}</span>
                        </div>
                      )}
                      {upscaledVersions.length > 0 && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-blue-600">Upscaled:</span>
                          <span className="font-medium">
                            {upscaledVersions
                              .map((v: { scale: number }) => `${v.scale}x`)
                              .join(', ')}
                          </span>
                        </div>
                      )}
                      {image.bg_removed_size && (
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-purple-600">BG Removed:</span>
                          <span className="font-medium">{formatBytes(image.bg_removed_size)}</span>
                        </div>
                      )}
                      {image.converted_images.map((ci) => (
                        <div key={ci.format} className="flex items-center justify-between gap-2">
                          <span className="text-orange-600">
                            Converted ({ci.format.toUpperCase()}):
                          </span>
                          <span className="font-medium">
                            {ci.size != null ? formatBytes(ci.size) : 'Unknown'}
                          </span>
                        </div>
                      ))}
                    </div>
                  </TooltipContent>
                </Tooltip>
              )}
            </div>

            <div className="absolute bottom-0 left-0 w-full bg-linear-to-t from-black/70 to-transparent px-2 py-1.5">
              <p className="truncate text-xs font-semibold text-white">{image.filename}</p>
              <p className="text-[10px] text-white/80">
                {image.size ? formatBytes(image.size) : 'Unknown'}
              </p>
            </div>

            {(Object.keys(compressingState).length > 0 ||
              Object.keys(upscalingState).length > 0 ||
              Object.keys(bgRemovalState).length > 0 ||
              Object.keys(conversionState).length > 0) && (
              <div className="bg-background/80 absolute inset-0 z-10 flex flex-col items-center justify-center p-2">
                <Progress
                  value={
                    (compressingState.progress as number) ||
                    (upscalingState.progress as number) ||
                    (bgRemovalState.progress as number) ||
                    (conversionState.progress as number)
                  }
                  className="mb-1.5 h-1.5 w-full"
                />
                <span className="text-[10px] font-medium">
                  {(compressingState.message as string) ||
                    (upscalingState.message as string) ||
                    (bgRemovalState.message as string) ||
                    (conversionState.message as string)}
                </span>
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    cancelImageJobs([image.id]).catch((err) =>
                      logError(`Failed to cancel image job: ${err}`)
                    )
                  }}
                  className="mt-1.5 rounded bg-red-600 px-2 py-0.5 text-[10px] text-white hover:bg-red-700"
                >
                  Cancel
                </button>
              </div>
            )}
          </div>
        </div>
      </ContextMenuTrigger>

      <ContextMenuContent className="w-56">
        <ContextMenuLabel>File</ContextMenuLabel>
        <ContextMenuItem onClick={handleOpen}>
          <ExternalLink className="mr-2 h-4 w-4" />
          Open Original
        </ContextMenuItem>
        <ContextMenuItem onClick={handleReveal}>
          <FolderSearch className="mr-2 h-4 w-4" />
          Reveal Original
        </ContextMenuItem>
        {onImageOpen && (
          <ContextMenuItem onClick={() => onImageOpen(image.id)}>
            <ImageIcon className="mr-2 h-4 w-4" />
            Open in Editor
          </ContextMenuItem>
        )}

        {(onCompressClick || onUpscaleClick || onBgRemovalClick || onConvertClick) && (
          <>
            <ContextMenuSeparator />
            <ContextMenuLabel>Actions</ContextMenuLabel>
            {onCompressClick && (
              <ContextMenuItem onClick={() => onCompressClick([image.id])}>
                <Minimize2 className="mr-2 h-4 w-4" />
                Compress
              </ContextMenuItem>
            )}
            {onUpscaleClick && (
              <ContextMenuItem onClick={() => onUpscaleClick([image.id])}>
                <Maximize2 className="mr-2 h-4 w-4" />
                Upscale
              </ContextMenuItem>
            )}
            {onBgRemovalClick && (
              <ContextMenuItem onClick={() => onBgRemovalClick([image.id])}>
                <Scissors className="mr-2 h-4 w-4" />
                Remove Background
              </ContextMenuItem>
            )}
            {onConvertClick && (
              <ContextMenuItem onClick={() => onConvertClick([image.id])}>
                <Repeat2 className="mr-2 h-4 w-4" />
                Convert Format
              </ContextMenuItem>
            )}
          </>
        )}

        {(image.compressed_filepath ||
          upscaledVersions.length > 0 ||
          image.bg_removed_filepath ||
          image.converted_images.length > 0) && (
          <>
            <ContextMenuSeparator />
            <ContextMenuLabel>Versions</ContextMenuLabel>
            {image.compressed_filepath && (
              <>
                <ContextMenuItem onClick={handleCompareCompressed}>
                  <Columns className="mr-2 h-4 w-4" />
                  Compare (Compressed)
                </ContextMenuItem>
                <ContextMenuItem onClick={handleOpenCompressed}>
                  <ExternalLink className="mr-2 h-4 w-4" />
                  Open Compressed
                </ContextMenuItem>
                <ContextMenuItem onClick={handleRevealCompressed}>
                  <FolderSearch className="mr-2 h-4 w-4" />
                  Reveal Compressed
                </ContextMenuItem>
              </>
            )}

            {upscaledVersions.length > 0 && (
              <>
                {upscaledVersions.map(
                  (version: { scale: number; filepath: string }, idx: number) => (
                    <ContextMenuItem key={idx} onClick={() => openFile(version.filepath)}>
                      <ExternalLink className="mr-2 h-4 w-4" />
                      Open Upscaled {version.scale}x
                    </ContextMenuItem>
                  )
                )}
                {upscaledVersions.map(
                  (version: { scale: number; filepath: string }, idx: number) => (
                    <ContextMenuItem
                      key={`reveal-${idx}`}
                      onClick={async () => {
                        try {
                          await revealInExplorer(version.filepath)
                        } catch (err) {
                          logError(`Failed to reveal upscaled file: ${err}`)
                        }
                      }}
                    >
                      <FolderSearch className="mr-2 h-4 w-4" />
                      Reveal Upscaled {version.scale}x
                    </ContextMenuItem>
                  )
                )}
              </>
            )}

            {image.bg_removed_filepath && (
              <>
                <ContextMenuItem onClick={handleCompareBgRemoved}>
                  <Columns className="mr-2 h-4 w-4" />
                  Compare (Background Removed)
                </ContextMenuItem>
                <ContextMenuItem onClick={handleOpenBgRemoved}>
                  <ExternalLink className="mr-2 h-4 w-4" />
                  Open Background Removed
                </ContextMenuItem>
                <ContextMenuItem onClick={handleRevealBgRemoved}>
                  <FolderSearch className="mr-2 h-4 w-4" />
                  Reveal Background Removed
                </ContextMenuItem>
              </>
            )}

            {image.converted_images.length > 0 && (
              <>
                <ContextMenuSub>
                  <ContextMenuSubTrigger>
                    <ExternalLink className="mr-2 h-4 w-4" /> Open Converted
                  </ContextMenuSubTrigger>
                  <ContextMenuSubContent>
                    {image.converted_images.map((ci) => (
                      <ContextMenuItem
                        key={ci.format}
                        onClick={() => handleOpenConverted(ci.filepath)}
                      >
                        {ci.format.toUpperCase()}{' '}
                        {ci.size != null ? `(${formatBytes(ci.size)})` : ''}
                      </ContextMenuItem>
                    ))}
                  </ContextMenuSubContent>
                </ContextMenuSub>
                <ContextMenuSub>
                  <ContextMenuSubTrigger>
                    <FolderSearch className="mr-2 h-4 w-4" /> Reveal Converted
                  </ContextMenuSubTrigger>
                  <ContextMenuSubContent>
                    {image.converted_images.map((ci) => (
                      <ContextMenuItem
                        key={ci.format}
                        onClick={() => handleRevealConverted(ci.filepath)}
                      >
                        {ci.format.toUpperCase()}
                      </ContextMenuItem>
                    ))}
                  </ContextMenuSubContent>
                </ContextMenuSub>
              </>
            )}
          </>
        )}

        <ContextMenuSeparator />
        <ContextMenuItem onClick={handleDelete} className="text-red-500">
          <Trash2 className="mr-2 h-4 w-4" />
          Remove
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
})

export function ImageGrid({
  images,
  selectedIds,
  onSelectionChange,
  onDelete,
  onImport,
  onDrop,
  onCompressClick,
  onUpscaleClick,
  onBgRemovalClick,
  onConvertClick,
  onImageOpen,
}: ImageGridProps) {
  const [isDragging, setIsDragging] = useState(false)
  const [compareImage, setCompareImage] = useState<Image | null>(null)
  const [compareType, setCompareType] = useState<'compressed' | 'bg_removed' | null>(null)

  useEffect(() => {
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
        const allIds = images.map((img) => img.id)
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
    [images, selectedIds, onSelectionChange]
  )

  const handleKeyDown = useCallback(
    (id: number, event: React.KeyboardEvent) => {
      if (event.key === 'Enter' || event.key === ' ') {
        handleSelect(id, event)
      }
    },
    [handleSelect]
  )

  return (
    <section className="customScrollStyle relative h-full max-h-[calc(100vh-133px)] overflow-auto">
      {images.length === 0 && !isDragging ? (
        <div className="flex h-full flex-1 flex-col items-center justify-center gap-4">
          <div className="flex flex-col items-center gap-2 text-center">
            <Upload className="text-muted-foreground h-10 w-10" />
            <p className="text-sm font-medium">No images found</p>
            <p className="text-muted-foreground text-xs">
              Import images or drag and drop to get started
            </p>
          </div>
          <Button onClick={onImport} variant="outline" size="sm">
            Import Images
          </Button>
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-4 p-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
          {images.map((image) => (
            <ImageGridItem
              key={image.id}
              image={image}
              isSelected={selectedIds.includes(image.id)}
              onSelect={handleSelect}
              onKeyDown={handleKeyDown}
              onDelete={onDelete}
              onCompare={(image, type) => {
                setCompareImage(image)
                setCompareType(type)
              }}
              onCompressClick={onCompressClick}
              onUpscaleClick={onUpscaleClick}
              onBgRemovalClick={onBgRemovalClick}
              onConvertClick={onConvertClick}
              onImageOpen={onImageOpen}
            />
          ))}
        </div>
      )}

      {isDragging && (
        <div className="pointer-events-none flex h-full flex-1 flex-col items-center justify-center">
          <div className="border-primary bg-background rounded-lg border-2 border-dashed p-8 text-center">
            <Upload className="text-primary mx-auto h-10 w-10" />
            <p className="mt-3 text-sm font-medium">Drop images here</p>
          </div>
        </div>
      )}

      <ImageCompare
        image={compareImage}
        compareType={compareType}
        open={!!compareImage}
        onOpenChange={(open) => {
          if (!open) {
            setCompareImage(null)
            setCompareType(null)
          }
        }}
      />
    </section>
  )
}
