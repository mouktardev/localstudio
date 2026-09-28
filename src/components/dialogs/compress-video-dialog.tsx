import { useState, useMemo } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Input } from '@/components/ui/input'
import { Slider } from '@/components/ui/slider'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { formatBytes } from '@/lib/utils'
import type {
  Video,
  CompressionPreset,
  CompressionRequest,
  CompressionCodec,
  EncoderCapabilities,
} from '@/lib/tauri'

interface CompressVideoDialogProps {
  videos: Video[]
  videoIds: number[]
  open: boolean
  onOpenChange: (open: boolean) => void
  onConfirm: (ids: number[], request: CompressionRequest) => void
  presets: CompressionPreset[]
  capabilities: EncoderCapabilities
}

type Mode = 'quality' | 'target_size'
type TargetKind = 'absolute' | 'percent'

const MB = 1024 * 1024

const RESOLUTION_OPTIONS: { value: string; label: string; dimension: number }[] = [
  { value: 'original', label: 'Keep original resolution', dimension: 0 },
  { value: '1080', label: 'Downscale to 1080p', dimension: 1920 },
  { value: '720', label: 'Downscale to 720p', dimension: 1280 },
  { value: '480', label: 'Downscale to 480p', dimension: 854 },
]

export function CompressVideoDialog({
  videos,
  videoIds,
  open,
  onOpenChange,
  onConfirm,
  presets,
  capabilities,
}: CompressVideoDialogProps) {
  const [mode, setMode] = useState<Mode>('quality')
  const [selectedPreset, setSelectedPreset] = useState(presets[2]?.preset || 'medium')
  const [codec, setCodec] = useState<CompressionCodec>('auto')
  const [targetKind, setTargetKind] = useState<TargetKind>('absolute')
  const [targetMb, setTargetMb] = useState('20')
  const [targetPercent, setTargetPercent] = useState(50)
  const [dropAudio, setDropAudio] = useState(false)
  const [maxDimension, setMaxDimension] = useState('original')

  const selectedVideos = useMemo(
    () => videos.filter((v) => videoIds.includes(v.id)),
    [videos, videoIds]
  )

  const currentPreset = presets.find((p) => p.preset === selectedPreset) || presets[2]
  const totalSource = selectedVideos.reduce((sum, v) => sum + (v.size ?? 0), 0)

  const parsedMb = Number.parseFloat(targetMb)
  const targetBytes = Number.isFinite(parsedMb) ? Math.round(parsedMb * MB) : 0
  const targetValid =
    mode === 'quality' ||
    (targetKind === 'absolute' && targetBytes > 0) ||
    (targetKind === 'percent' && targetPercent >= 1 && targetPercent <= 99)

  const resolution = RESOLUTION_OPTIONS.find((o) => o.value === maxDimension)

  const handleConfirm = () => {
    if (!targetValid) return
    const shared: Partial<CompressionRequest> = {
      drop_audio: dropAudio,
      max_dimension: resolution && resolution.dimension > 0 ? resolution.dimension : undefined,
    }
    const request: CompressionRequest =
      mode === 'quality'
        ? {
            mode: 'quality',
            quality: currentPreset?.crf || 20,
            preset: selectedPreset,
            codec,
            ...shared,
          }
        : targetKind === 'absolute'
          ? { mode: 'target_size', preset: selectedPreset, target_bytes: targetBytes, ...shared }
          : {
              mode: 'target_size',
              preset: selectedPreset,
              target_percent: targetPercent,
              ...shared,
            }
    onConfirm(videoIds, request)
    onOpenChange(false)
  }

  const targetFor = (size: number | null): string => {
    if (mode === 'quality') return 'preset'
    if (targetKind === 'percent') {
      if (size == null) return '?'
      return formatBytes(Math.round(size * (targetPercent / 100)))
    }
    return `${formatBytes(targetBytes)}`
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Compress {videoIds.length} video(s)</DialogTitle>
          <DialogDescription>
            A compressed copy will be created and saved to your output folder.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-6 py-4">
          <div className="flex gap-2">
            <Button
              type="button"
              variant={mode === 'quality' ? 'default' : 'outline'}
              size="sm"
              className="flex-1"
              onClick={() => setMode('quality')}
            >
              Quality preset
            </Button>
            <Button
              type="button"
              variant={mode === 'target_size' ? 'default' : 'outline'}
              size="sm"
              className="flex-1"
              onClick={() => setMode('target_size')}
            >
              Target size
            </Button>
          </div>

          {mode === 'quality' && (
            <div className="space-y-4">
              <div className="flex justify-between">
                <Label className="text-sm font-medium">Preset</Label>
                <span className="text-muted-foreground text-sm">
                  CRF: {currentPreset?.crf || 20}
                </span>
              </div>
              <Select value={selectedPreset} onValueChange={setSelectedPreset}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {presets.map((preset) => (
                    <SelectItem key={preset.preset} value={preset.preset}>
                      {preset.name} (CRF: {preset.crf})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-muted-foreground text-xs">
                Lower CRF = better quality but larger file. Higher preset = slower encoding but
                better compression.
              </p>

              <div className="space-y-2">
                <Label className="text-sm font-medium">Codec</Label>
                <Select value={codec} onValueChange={(v) => setCodec(v as CompressionCodec)}>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Auto (H.264 — most compatible)</SelectItem>
                    <SelectItem value="h264">H.264 / AVC</SelectItem>
                    {capabilities.hevc && <SelectItem value="hevc">H.265 / HEVC</SelectItem>}
                    {capabilities.av1 && <SelectItem value="av1">AV1 (smallest)</SelectItem>}
                  </SelectContent>
                </Select>
                {!capabilities.hevc && !capabilities.av1 && (
                  <p className="text-muted-foreground text-xs">
                    This FFmpeg build only offers H.264.
                  </p>
                )}
              </div>
            </div>
          )}

          {mode === 'target_size' && (
            <div className="space-y-4">
              <div className="flex gap-2">
                <Button
                  type="button"
                  variant={targetKind === 'absolute' ? 'secondary' : 'ghost'}
                  size="sm"
                  className="flex-1"
                  onClick={() => setTargetKind('absolute')}
                >
                  Size (MB)
                </Button>
                <Button
                  type="button"
                  variant={targetKind === 'percent' ? 'secondary' : 'ghost'}
                  size="sm"
                  className="flex-1"
                  onClick={() => setTargetKind('percent')}
                >
                  % of original
                </Button>
              </div>

              {targetKind === 'absolute' ? (
                <div className="space-y-2">
                  <Label htmlFor="target-mb" className="text-sm font-medium">
                    Target size per video (MB)
                  </Label>
                  <Input
                    id="target-mb"
                    type="number"
                    min={1}
                    step={1}
                    value={targetMb}
                    onChange={(e) => setTargetMb(e.target.value)}
                  />
                  {targetBytes <= 0 && (
                    <p className="text-destructive text-xs">Enter a size greater than 0 MB.</p>
                  )}
                </div>
              ) : (
                <div className="space-y-2">
                  <div className="flex justify-between">
                    <Label className="text-sm font-medium">Keep this share of the original</Label>
                    <span className="text-muted-foreground text-sm">{targetPercent}%</span>
                  </div>
                  <Slider
                    value={[targetPercent]}
                    min={5}
                    max={95}
                    step={5}
                    onValueChange={(value) => setTargetPercent(value[0])}
                  />
                </div>
              )}

              <p className="text-muted-foreground text-xs">
                Two-pass encoding targets the size below. If the result would not be smaller, the
                original is kept.
              </p>
            </div>
          )}

          <div className="space-y-3">
            <div className="space-y-2">
              <Label className="text-sm font-medium">Resolution</Label>
              <Select value={maxDimension} onValueChange={setMaxDimension}>
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {RESOLUTION_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-muted-foreground text-xs">
                Videos are never upscaled; smaller inputs are left untouched.
              </p>
            </div>

            <div className="flex items-center gap-2">
              <Checkbox
                id="drop-audio"
                checked={dropAudio}
                onCheckedChange={(checked) => setDropAudio(Boolean(checked))}
              />
              <Label htmlFor="drop-audio" className="text-sm">
                Remove audio track
              </Label>
            </div>
          </div>

          {videoIds.length > 0 && (
            <div className="bg-muted max-h-40 overflow-y-auto rounded border p-2">
              {selectedVideos.map((video) => (
                <div
                  key={video.id}
                  className="flex items-center justify-between border-b py-1 last:border-b-0"
                >
                  <span className="max-w-37.5 truncate text-sm">{video.filename}</span>
                  <span className="text-muted-foreground flex items-center gap-2 text-xs">
                    <span>{video.size ? formatBytes(video.size) : '?'}</span>
                    {mode === 'target_size' && <span>→ {targetFor(video.size)}</span>}
                  </span>
                </div>
              ))}
            </div>
          )}

          {mode === 'target_size' && selectedVideos.length > 1 && totalSource > 0 && (
            <p className="text-muted-foreground text-xs">
              {selectedVideos.length} videos, {formatBytes(totalSource)} total.
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={handleConfirm} disabled={!targetValid}>
            Start Compression
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
