import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Label } from '@/components/ui/label'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Trash2, Upload, Loader2, Minimize2, Repeat2 } from 'lucide-react'
import { formatBytes } from '@/lib/utils'
import type { Video } from '@/lib/tauri'
import { SearchBar } from '@/components/search-bar'
import { SortDropdown } from '@/components/sort-dropdown'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { DeleteAction } from '@/lib/tauri'

interface VideoToolsProps {
  videos: Video[]
  selectedIds: number[]
  onSelectionChange: (ids: number[]) => void
  onDeleteClick: (action: DeleteAction) => void
  onImportClick: () => void
  onCompressClick?: () => void
  onConvertClick?: () => void
  isImporting?: boolean
  searchQuery?: string
  onSearchChange?: (value: string) => void
  sortField?: 'name' | 'size' | 'date'
  sortOrder?: 'asc' | 'desc'
  onSortFieldChange?: (field: 'name' | 'size' | 'date') => void
  onSortOrderChange?: (order: 'asc' | 'desc') => void
  hasActiveFilters?: boolean
  onResetFilters?: () => void
}

export function VideoTools({
  videos,
  selectedIds,
  onSelectionChange,
  onDeleteClick,
  onImportClick,
  onCompressClick,
  onConvertClick,
  isImporting = false,
  searchQuery = '',
  onSearchChange,
  sortField = 'date',
  sortOrder = 'desc',
  onSortFieldChange,
  onSortOrderChange,
  hasActiveFilters = false,
  onResetFilters,
}: VideoToolsProps) {
  const [openDeleteDialog, setOpenDeleteDialog] = useState(false)
  const [deleteAction, setDeleteAction] = useState<DeleteAction>('library')

  useEffect(() => {
    if (selectedIds.length === 0) {
      setOpenDeleteDialog(false)
      setDeleteAction('library')
    }
  }, [selectedIds.length])

  const handleSelectAll = () => {
    if (selectedIds.length === videos.length) {
      onSelectionChange([])
    } else {
      onSelectionChange(videos.map((v) => v.id))
    }
  }

  const handleDeleteConfirm = () => {
    onDeleteClick(deleteAction)
    setOpenDeleteDialog(false)
  }

  const selectedVideos = videos.filter((v) => selectedIds.includes(v.id))

  return (
    <div className="bg-background flex flex-wrap items-center gap-2 border-b p-3">
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            onClick={onImportClick}
            disabled={isImporting}
            variant="secondary"
            size="icon"
            className="h-8 w-8"
          >
            {isImporting ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Upload className="size-4" />
            )}
          </Button>
        </TooltipTrigger>
        <TooltipContent>Import Videos</TooltipContent>
      </Tooltip>

      {selectedIds.length > 0 && onCompressClick && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button onClick={onCompressClick} size="icon" className="h-8 w-8">
              <Minimize2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Compress ({selectedIds.length})</TooltipContent>
        </Tooltip>
      )}

      {selectedIds.length > 0 && onConvertClick && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button onClick={onConvertClick} size="icon" className="h-8 w-8">
              <Repeat2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Convert Format ({selectedIds.length})</TooltipContent>
        </Tooltip>
      )}

      {selectedIds.length > 0 && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="destructive"
              onClick={() => setOpenDeleteDialog(true)}
              size="icon"
              className="h-8 w-8"
            >
              <Trash2 className="size-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Remove</TooltipContent>
        </Tooltip>
      )}

      {/* Mounted unconditionally so it cannot unmount mid-interaction when the
          selection empties out from under it. */}
      <Dialog open={openDeleteDialog} onOpenChange={setOpenDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove selected videos?</DialogTitle>
            <DialogDescription>
              You are about to remove {selectedIds.length} video(s) from the app and database. The
              original file on your device will not be affected. This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          {selectedVideos.length > 0 && (
            <div className="bg-muted max-h-40 overflow-y-auto rounded border p-2">
              {selectedVideos.map((v) => (
                <div
                  key={v.id}
                  className="flex items-center justify-between border-b py-1 last:border-b-0"
                >
                  <span className="max-w-50 truncate">{v.filename}</span>
                  <span className="text-muted-foreground text-xs">
                    {v.size ? formatBytes(v.size) : 'Unknown'}
                  </span>
                </div>
              ))}
            </div>
          )}
          <div className="space-y-2">
            <Label className="text-sm font-medium">Action</Label>
            <Select value={deleteAction} onValueChange={(v) => setDeleteAction(v as DeleteAction)}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="library">Remove from library (keep files)</SelectItem>
                <SelectItem value="outputs">Delete generated outputs (keep originals)</SelectItem>
                <SelectItem value="all">Delete originals + outputs from disk</SelectItem>
              </SelectContent>
            </Select>
            {deleteAction === 'all' && (
              <p className="text-destructive text-xs">
                This permanently deletes files from disk. This cannot be undone.
              </p>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setOpenDeleteDialog(false)}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={handleDeleteConfirm}>
              Remove
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <div className="ml-auto flex items-center gap-2">
        <Checkbox
          id="select-all-videos"
          checked={
            videos.length === 0
              ? false
              : selectedIds.length === videos.length
                ? true
                : selectedIds.length > 0
                  ? 'indeterminate'
                  : false
          }
          onCheckedChange={handleSelectAll}
          disabled={videos.length === 0}
        />
        <Label
          htmlFor="select-all-videos"
          className={`text-sm ${videos.length === 0 ? 'cursor-not-allowed opacity-50' : 'cursor-pointer'}`}
        >
          {selectedIds.length > 0 ? `${selectedIds.length} selected` : 'Select all'}
        </Label>
      </div>

      {onSearchChange && onSortFieldChange && onSortOrderChange && (
        <div className="flex items-center gap-2">
          <SearchBar value={searchQuery} onChange={onSearchChange} placeholder="Search videos..." />
          <SortDropdown
            sortField={sortField}
            sortOrder={sortOrder}
            onSortFieldChange={onSortFieldChange}
            onSortOrderChange={onSortOrderChange}
          />
          {hasActiveFilters && onResetFilters && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="icon" variant="outline" className="h-8 w-8" onClick={onResetFilters}>
                  <span className="text-xs">⟲</span>
                </Button>
              </TooltipTrigger>
              <TooltipContent>Reset filters</TooltipContent>
            </Tooltip>
          )}
        </div>
      )}
    </div>
  )
}
