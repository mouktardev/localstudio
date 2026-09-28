import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { revealItemInDir, openPath, openUrl } from '@tauri-apps/plugin-opener'

export interface UpscaledVersion {
  scale: number
  filepath: string
  size: number | null
  model: string | null
}

export interface ConvertedImage {
  filepath: string
  size: number | null
  format: string
}

export interface Image {
  id: number
  filename: string
  filepath: string
  mimetype: string | null
  size: number | null
  width: number | null
  height: number | null
  compressed_filepath?: string | null
  compressed_size?: number | null
  upscaled_versions?: UpscaledVersion[] // JSON parsed from backend
  bg_removed_filepath?: string | null
  bg_removed_size?: number | null
  converted_images: ConvertedImage[]
}

export interface AddImageData {
  filename: string
  filepath: string
  mimetype: string | null
  size: number | null
  width: number | null
  height: number | null
}

export interface ImageMetadata {
  width: number
  height: number
  size: number
  mimetype: string
}

export async function initDatabase(): Promise<string> {
  return invoke<string>('init_database')
}

export async function dbExists(): Promise<boolean> {
  return invoke<boolean>('db_exists')
}

export async function getDbPath(): Promise<string> {
  return invoke<string>('get_db_path_cmd')
}

export async function getSetting(key: string): Promise<string | null> {
  return invoke<string | null>('get_setting', { key })
}

export async function setSetting(key: string, value: string): Promise<void> {
  return invoke<void>('set_setting', { key, value })
}

export async function revealInExplorer(path: string): Promise<void> {
  return revealItemInDir(path)
}

export async function openFile(path: string): Promise<void> {
  return openPath(path)
}

export async function openExternalUrl(url: string): Promise<void> {
  return openUrl(url)
}

export async function selectFolder(): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title: 'Select Output Folder',
  })
  return selected as string | null
}

export async function selectFiles(): Promise<string[] | null> {
  const selected = await open({
    multiple: true,
    title: 'Select Images',
    filters: [
      {
        name: 'Images',
        extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'],
      },
    ],
  })
  if (!selected) return null
  return Array.isArray(selected) ? selected : [selected]
}

export async function getAllImages(params?: ImageQueryParams): Promise<Image[]> {
  return invoke<Image[]>('get_all_images', { params })
}

export async function getAllCompressedImages(): Promise<Image[]> {
  return invoke<Image[]>('get_all_compressed_images')
}

export async function addImage(data: AddImageData): Promise<Image> {
  return invoke<Image>('add_image', { data })
}

export async function deleteImage(id: number): Promise<void> {
  return invoke<void>('delete_image', { id })
}

export async function syncDatabase(): Promise<number> {
  return invoke<number>('sync_database')
}

export async function checkDbHealth(): Promise<number> {
  return invoke<number>('check_db_health')
}

export async function deleteImagesByIds(ids: number[]): Promise<void> {
  return invoke<void>('delete_images_by_ids', { ids })
}

export async function getImageMetadata(filepath: string): Promise<ImageMetadata> {
  return invoke<ImageMetadata>('get_image_metadata', { filepath })
}

export interface ImportResult {
  imported: number
  duplicates: number
  failed: number
}

export async function importImagesBulk(filepaths: string[]): Promise<ImportResult> {
  return invoke<ImportResult>('import_images_bulk', { filepaths })
}

export async function compressImagesByIds(ids: number[], quality: number): Promise<number> {
  return invoke<number>('compress_images_by_ids', { ids, quality })
}

export interface UpscaleSettings {
  model: string
  models_dir: string
}

export interface ModelStatus {
  name: string
  downloaded: boolean
  path: string
  size: number | null
}

export async function getUpscaleSettings(): Promise<UpscaleSettings> {
  return invoke<UpscaleSettings>('get_upscale_settings')
}

export async function setUpscaleSettings(model: string): Promise<void> {
  return invoke<void>('set_upscale_settings', { model })
}

export async function getModelStatus(model: string): Promise<ModelStatus> {
  return invoke<ModelStatus>('get_model_status', { model })
}

export async function downloadModel(model: string): Promise<string> {
  return invoke<string>('download_model', { model })
}

export async function upscaleImagesByIds(
  ids: number[],
  scale: number,
  model: string
): Promise<number> {
  return invoke<number>('upscale_images_by_ids', { ids, scale, model })
}

export async function getAllUpscaledImages(): Promise<Image[]> {
  return invoke<Image[]>('get_all_upscaled_images')
}

// Background Removal API
export interface BgRemovalModelStatus {
  name: string
  downloaded: boolean
  path: string
  size: number | null
}

export async function getBgRemovalModelStatus(): Promise<BgRemovalModelStatus> {
  return invoke<BgRemovalModelStatus>('get_bg_removal_model_status')
}

export async function downloadBgRemovalModel(): Promise<string> {
  return invoke<string>('download_bg_removal_model')
}

export async function removeBackgroundByIds(ids: number[]): Promise<number> {
  return invoke<number>('remove_background_by_ids', { ids })
}

export async function getAllBgRemovedImages(): Promise<Image[]> {
  return invoke<Image[]>('get_all_bg_removed_images')
}

// --- Image batch results (v2: parity with the video pipeline) ---

export interface ImageFileResult {
  id: number
  status: string
  message: string | null
  output_path: string | null
  size: number | null
  source_size: number | null
}

export interface ImageFileError {
  id: number
  message: string
}

export interface ImageBatchResult {
  processed: number
  failed: number
  cancelled: number
  errors: ImageFileError[]
  results: ImageFileResult[]
}

export async function compressImagesByIdsV2(
  ids: number[],
  quality: number
): Promise<ImageBatchResult> {
  return invoke<ImageBatchResult>('compress_images_by_ids_v2', { ids, quality })
}

export async function convertImagesByIdsV2(
  ids: number[],
  format: ImageFormat
): Promise<ImageBatchResult> {
  return invoke<ImageBatchResult>('convert_images_by_ids_v2', { ids, format })
}

export async function upscaleImagesByIdsV2(
  ids: number[],
  scale: number,
  model: string
): Promise<ImageBatchResult> {
  return invoke<ImageBatchResult>('upscale_images_by_ids_v2', { ids, scale, model })
}

export async function removeBackgroundByIdsV2(ids: number[]): Promise<ImageBatchResult> {
  return invoke<ImageBatchResult>('remove_background_by_ids_v2', { ids })
}

export async function cancelImageJobs(ids: number[]): Promise<void> {
  return invoke<void>('cancel_image_jobs', { ids })
}

export async function getImageJobLimit(): Promise<number> {
  return invoke<number>('get_image_job_limit')
}

export async function setImageJobLimit(limit: number): Promise<void> {
  return invoke<void>('set_image_job_limit', { limit })
}

// Video Processing API
export interface ConvertedVideo {
  filepath: string
  size: number | null
  format: string
}

export interface Video {
  id: number
  filename: string
  filepath: string
  mimetype: string | null
  size: number | null
  width: number | null
  height: number | null
  duration: number | null
  fps: number | null
  thumbnail_path?: string | null
  bg_removed_filepath?: string | null
  bg_removed_size?: number | null
  bg_removed_model?: string | null
  compressed_filepath?: string | null
  compressed_size?: number | null
  converted_videos: ConvertedVideo[]
}

export interface FfmpegStatus {
  available: boolean
  path: string
  size: number | null
  source: string
}

export interface VideoBgRemovalProgress {
  id: number
  progress: number
  message: string
  eta_seconds: number | null
}

export async function selectVideoFiles(): Promise<string[] | null> {
  const selected = await open({
    multiple: true,
    title: 'Select Videos',
    filters: [
      {
        name: 'Videos',
        extensions: ['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv', 'm4v', '3gp'],
      },
    ],
  })
  if (!selected) return null
  return Array.isArray(selected) ? selected : [selected]
}

export interface VideoImportResult {
  imported: number
  duplicates: number
  failed: number
}

export async function importVideos(paths: string[]): Promise<VideoImportResult> {
  return invoke<VideoImportResult>('import_videos', { paths })
}

export async function getVideoById(id: number): Promise<Video> {
  return invoke<Video>('get_video_by_id', { id })
}

export async function getAllVideos(params?: VideoQueryParams): Promise<Video[]> {
  return invoke<Video[]>('get_all_videos', { params: params ?? null })
}

export async function generateVideoThumbnails(): Promise<number> {
  return invoke<number>('generate_video_thumbnails')
}

export async function deleteVideosByIds(ids: number[]): Promise<void> {
  return invoke<void>('delete_videos_by_ids', { ids })
}

export interface SkippedFile {
  path: string
  reason: string
}

export interface DeleteReport {
  rows_deleted: number
  files_deleted: number
  bytes_freed: number
  skipped: SkippedFile[]
}

export type DeleteAction = 'library' | 'outputs' | 'all'

export async function deleteItems(
  kind: 'image' | 'video',
  ids: number[],
  deleteFiles: boolean
): Promise<DeleteReport> {
  return invoke<DeleteReport>('delete_items', { kind, ids, deleteFiles })
}

/** Delete generated outputs for the items, keeping their originals. */
export async function deleteOutputs(
  kind: 'image' | 'video',
  ids: number[],
  outputKind?: string
): Promise<DeleteReport> {
  return invoke<DeleteReport>('delete_outputs', { kind, ids, outputKind: outputKind ?? null })
}

/** Delete a single generated output variant (by its table kind + row id). */
export async function deleteOutput(outputKind: string, id: number): Promise<DeleteReport> {
  return invoke<DeleteReport>('delete_output', { outputKind, id })
}

export interface CompressedVariant {
  id: number
  filepath: string
  size: number | null
  label: string
}

export async function getCompressedVariants(
  kind: 'image' | 'video',
  id: number
): Promise<CompressedVariant[]> {
  return invoke<CompressedVariant[]>('get_compressed_variants', { kind, id })
}

export interface VideoBgRemovalResult {
  processed: number
  failed: number
  cancelled: number
}

export async function removeVideoBg(ids: number[]): Promise<VideoBgRemovalResult> {
  return invoke<VideoBgRemovalResult>('remove_video_bg', { ids })
}

export async function getAllBgRemovedVideos(): Promise<Video[]> {
  return invoke<Video[]>('get_all_bg_removed_videos')
}

export async function checkFfmpegStatus(): Promise<FfmpegStatus> {
  return invoke<FfmpegStatus>('check_ffmpeg_status')
}

export async function downloadFfmpeg(): Promise<string> {
  return invoke<string>('download_ffmpeg')
}

export async function cancelVideoBgRemoval(ids: number[]): Promise<void> {
  return invoke<void>('cancel_video_bg_removal', { ids })
}

export async function cancelVideoCompression(ids: number[]): Promise<void> {
  return invoke<void>('cancel_video_compression', { ids })
}

export async function cancelVideoConversion(ids: number[]): Promise<void> {
  return invoke<void>('cancel_video_conversion', { ids })
}

export async function getAllCompressedVideos(): Promise<Video[]> {
  return invoke<Video[]>('get_all_compressed_videos')
}

export interface CompressionPreset {
  name: string
  crf: number
  preset: string
}

export async function getCompressionPresets(): Promise<CompressionPreset[]> {
  return invoke<CompressionPreset[]>('get_compression_presets')
}

export async function compressVideosByIds(
  ids: number[],
  quality: number,
  preset: string
): Promise<number> {
  return invoke<number>('compress_videos_by_ids', { ids, quality, preset })
}

export type CompressionMode = 'quality' | 'target_size'
export type CompressionCodec = 'auto' | 'h264' | 'hevc' | 'av1'

export interface CompressionRequest {
  mode: CompressionMode
  quality?: number
  preset?: string
  codec?: CompressionCodec
  target_bytes?: number
  target_percent?: number
  drop_audio?: boolean
  max_dimension?: number
}

export interface CompressionFileResult {
  id: number
  status: string
  message: string | null
  planned_bitrate_kbps: number | null
  achieved_size: number | null
  source_size: number | null
  output_path: string | null
  kept_original: boolean
}

export interface CompressionFileError {
  id: number
  message: string
}

export interface CompressionBatchResult {
  processed: number
  failed: number
  cancelled: number
  errors: CompressionFileError[]
  results: CompressionFileResult[]
}

export async function compressVideosByIdsV2(
  ids: number[],
  request: CompressionRequest
): Promise<CompressionBatchResult> {
  return invoke<CompressionBatchResult>('compress_videos_by_ids_v2', { ids, request })
}

export interface EncoderCapabilities {
  h264: boolean
  hevc: boolean
  av1: boolean
  hardware: string[]
}

export async function getEncoderCapabilities(): Promise<EncoderCapabilities> {
  return invoke<EncoderCapabilities>('get_encoder_capabilities')
}

export interface OrphanRow {
  table: string
  id: number
  filepath: string
}

export interface OrphanFile {
  path: string
  bytes: number
}

export interface OrphanScan {
  orphaned_rows: OrphanRow[]
  orphaned_files: OrphanFile[]
  deleted_rows: number
  deleted_files: number
  bytes_reclaimed: number
  statements: number
  dry_run: boolean
  skipped_files: string[]
}

export interface DbTableInfo {
  name: string
  rows: number
}

export interface DbOverview {
  path: string
  schema_version: number
  tables: DbTableInfo[]
}

export interface DbTableRows {
  columns: string[]
  rows: (string | number | null)[][]
  total: number
}

export async function dbOverview(): Promise<DbOverview> {
  return invoke<DbOverview>('db_overview')
}

export async function dbTableRows(
  table: string,
  limit: number,
  offset: number
): Promise<DbTableRows> {
  return invoke<DbTableRows>('db_table_rows', { table, limit, offset })
}

export interface AppLog {
  path: string
  lines: string[]
}

/** Tail of the app log file (same text as the terminal). */
export async function readAppLog(maxLines: number, allSessions = false): Promise<AppLog> {
  return invoke<AppLog>('read_app_log', { maxLines, allSessions })
}

export async function orphanScan(): Promise<OrphanScan> {
  return invoke<OrphanScan>('orphan_scan')
}

export async function orphanCleanup(): Promise<OrphanScan> {
  return invoke<OrphanScan>('orphan_cleanup')
}

export async function getVideoJobLimit(): Promise<number> {
  return invoke<number>('get_video_job_limit')
}

export async function setVideoJobLimit(limit: number): Promise<void> {
  return invoke<void>('set_video_job_limit', { limit })
}

export interface ConversionOptions {
  drop_audio?: boolean
  max_dimension?: number
}

export async function convertVideosByIdsV2(
  ids: number[],
  format: VideoFormat,
  opts?: ConversionOptions
): Promise<CompressionBatchResult> {
  return invoke<CompressionBatchResult>('convert_videos_by_ids_v2', {
    ids,
    format,
    opts: opts ?? null,
  })
}

// Convert Format API
export type ImageFormat = 'jpg' | 'png' | 'webp'
export type VideoFormat = 'mp4' | 'webm' | 'mov' | 'gif'

export async function convertImagesByIds(ids: number[], format: ImageFormat): Promise<number> {
  return invoke<number>('convert_images_by_ids', { ids, format })
}

export async function convertVideosByIds(ids: number[], format: VideoFormat): Promise<number> {
  return invoke<number>('convert_videos_by_ids', { ids, format })
}

export async function getAllConvertedImages(): Promise<Image[]> {
  return invoke<Image[]>('get_all_converted_images')
}

export async function getAllConvertedVideos(): Promise<Video[]> {
  return invoke<Video[]>('get_all_converted_videos')
}

// Filters API
export interface FilterState {
  page: string
  search_query: string
  sort_field: 'name' | 'size' | 'date'
  sort_order: 'asc' | 'desc'
  output_type:
    | 'all'
    | 'compressed'
    | 'upscaled'
    | 'bg_removed'
    | 'video_compressed'
    | 'converted_images'
    | 'converted_videos'
}

export interface UpdateFilterRequest {
  page: string
  search_query?: string
  sort_field?: 'name' | 'size' | 'date'
  sort_order?: 'asc' | 'desc'
  output_type?:
    | 'all'
    | 'compressed'
    | 'upscaled'
    | 'bg_removed'
    | 'video_compressed'
    | 'converted_images'
    | 'converted_videos'
}

export interface ImageQueryParams {
  search?: string
  sort_field: 'name' | 'size' | 'date'
  sort_order: 'asc' | 'desc'
}

export interface VideoQueryParams {
  search?: string
  sort_field: 'name' | 'size' | 'date'
  sort_order: 'asc' | 'desc'
}

export async function getImageById(id: number): Promise<Image> {
  return invoke<Image>('get_image_by_id', { id })
}

export async function getFilters(page: string): Promise<FilterState> {
  return invoke<FilterState>('get_filters', { page })
}

export async function updateFilters(request: UpdateFilterRequest): Promise<void> {
  return invoke<void>('update_filters', { request })
}

export async function resetFilters(page: string): Promise<FilterState> {
  return invoke<FilterState>('reset_filters', { page })
}
