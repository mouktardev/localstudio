use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::DbState;
use futures::{stream, StreamExt};
use image::{DynamicImage, ImageBuffer};
use ffmpeg_sidecar::ffprobe;
use ffmpeg_sidecar::paths;

use super::background_removal::{apply_bg_removal, create_onnx_session, check_model_downloaded};

const SUPPORTED_VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "avi", "mkv", "webm", "flv", "wmv", "m4v", "3gp"];

pub(crate) static FFMPEG_DIR: OnceLock<PathBuf> = OnceLock::new();

pub struct CancelTokens(pub std::sync::Mutex<HashMap<i64, Arc<AtomicBool>>>);

/// Process-wide ceiling on concurrent video encodes. Every command that spawns
/// ffmpeg for a video job acquires a permit here, so a batch of 20 videos plus a
/// conversion batch still never exceeds the configured number of encoders.
#[derive(Clone)]
pub struct VideoJobLimiter {
    semaphore: Arc<std::sync::Mutex<Arc<tokio::sync::Semaphore>>>,
    waiting: Arc<AtomicUsize>,
    limit: Arc<AtomicUsize>,
}

impl VideoJobLimiter {
    pub fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            semaphore: Arc::new(std::sync::Mutex::new(Arc::new(tokio::sync::Semaphore::new(limit)))),
            waiting: Arc::new(AtomicUsize::new(0)),
            limit: Arc::new(AtomicUsize::new(limit)),
        }
    }

    pub fn limit(&self) -> usize {
        self.limit.load(Ordering::Relaxed)
    }

    /// Change the ceiling. In-flight jobs keep their permits; the new size
    /// applies to jobs that start afterwards.
    pub fn set_limit(&self, limit: usize) {
        let limit = limit.max(1);
        self.limit.store(limit, Ordering::Relaxed);
        let mut guard = self.semaphore.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Arc::new(tokio::sync::Semaphore::new(limit));
    }

    /// Acquires a permit, returning the number of jobs that were already
    /// waiting ahead of this one (0 means it started immediately).
    pub async fn acquire(&self) -> (tokio::sync::OwnedSemaphorePermit, usize) {
        let semaphore = {
            let guard = self.semaphore.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let position = self.waiting.fetch_add(1, Ordering::Relaxed);
        let permit = semaphore
            .acquire_owned()
            .await
            .expect("video job limiter closed");
        self.waiting.fetch_sub(1, Ordering::Relaxed);
        (permit, position)
    }
}

pub fn default_video_job_limit() -> usize {
    (num_cpus::get() / 2).max(1)
}

#[derive(Clone, Serialize)]
pub struct VideoBgRemovalProgress {
    pub id: i64,
    pub progress: u8,
    pub message: String,
    pub eta_seconds: Option<f64>,
}

#[derive(Clone, Serialize)]
pub struct VideoImportResult {
    pub imported: i64,
    pub duplicates: i64,
    pub failed: i64,
}

#[derive(Clone, Serialize)]
pub struct VideoBgRemovalResult {
    pub processed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

#[derive(Clone, Serialize)]
pub struct Video {
    pub id: i64,
    pub filename: String,
    pub filepath: String,
    pub mimetype: Option<String>,
    pub size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
    pub fps: Option<f64>,
    pub thumbnail_path: Option<String>,
    pub bg_removed_filepath: Option<String>,
    pub bg_removed_size: Option<i64>,
    pub bg_removed_model: Option<String>,
    pub compressed_filepath: Option<String>,
    pub compressed_size: Option<i64>,
    #[serde(default)]
    pub converted_videos: Vec<ConvertedVideo>,
}

#[derive(Clone, Serialize)]
pub struct ConvertedVideo {
    pub filepath: String,
    pub size: Option<i64>,
    pub format: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FfmpegStatus {
    pub available: bool,
    pub path: String,
    pub size: Option<i64>,
    pub source: String,
}

fn find_ffmpeg() -> (PathBuf, String) {
    if let Some(dir) = FFMPEG_DIR.get() {
        let our = dir.join("ffmpeg.exe");
        if our.exists() {
            return (our, "sidecar".to_string());
        }
    }

    let sidecar = paths::ffmpeg_path();

    if sidecar.exists() {
        return (sidecar, "sidecar".to_string());
    }

    let mut cmd = Command::new("where");
    cmd.arg("ffmpeg")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = stdout.lines().next() {
                let found = PathBuf::from(first_line.trim());
                if found.exists() {
                    return (found, "system".to_string());
                }
            }
        }
    }

    (PathBuf::from("ffmpeg"), "none".to_string())
}

fn find_ffprobe() -> (PathBuf, String) {
    if let Some(dir) = FFMPEG_DIR.get() {
        let our = dir.join("ffprobe.exe");
        if our.exists() {
            return (our, "sidecar".to_string());
        }
    }

    let sidecar = ffprobe::ffprobe_path();

    if sidecar.exists() {
        return (sidecar, "sidecar".to_string());
    }

    let mut cmd = Command::new("where");
    cmd.arg("ffprobe")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = stdout.lines().next() {
                let found = PathBuf::from(first_line.trim());
                if found.exists() {
                    return (found, "system".to_string());
                }
            }
        }
    }

    (PathBuf::from("ffprobe"), "none".to_string())
}

fn get_ffmpeg_path() -> PathBuf {
    let (path, _) = find_ffmpeg();
    path
}

fn get_ffprobe_path() -> PathBuf {
    let (path, _) = find_ffprobe();
    path
}

#[derive(Debug, Serialize, Deserialize)]
struct ProbeStream {
    width: Option<i64>,
    height: Option<i64>,
    codec_type: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}


#[tauri::command]
pub async fn check_ffmpeg_status() -> Result<FfmpegStatus, String> {
    let (ffmpeg_path, ffmpeg_source) = find_ffmpeg();
    let (ffprobe_path, ffprobe_source) = find_ffprobe();

    let ffmpeg_available = ffmpeg_path.exists() && ffmpeg_source != "none";
    let ffprobe_available = ffprobe_path.exists() && ffprobe_source != "none";

    let status = if ffmpeg_available {
        let size = fs::metadata(&ffmpeg_path).map(|m| m.len() as i64).ok();
        let path_str = ffmpeg_path.to_string_lossy().to_string();
        let source = if ffmpeg_available && ffprobe_available {
            if ffmpeg_source == "system" || ffprobe_source == "system" {
                "system".to_string()
            } else {
                ffmpeg_source
            }
        } else if ffmpeg_available {
            ffmpeg_source
        } else {
            ffprobe_source
        };
        FfmpegStatus {
            available: true,
            path: path_str,
            size,
            source,
        }
    } else {
        FfmpegStatus {
            available: false,
            path: String::new(),
            size: None,
            source: "none".to_string(),
        }
    };

    Ok(status)
}

#[tauri::command]
pub async fn download_ffmpeg(app: AppHandle) -> Result<String, String> {
    let ffmpeg_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {}", e))?
        .join("ffmpeg");
    std::fs::create_dir_all(&ffmpeg_dir)
        .map_err(|e| format!("Failed to create ffmpeg dir: {}", e))?;
    let _ = FFMPEG_DIR.set(ffmpeg_dir.clone());

    let _ = app.emit("ffmpeg-download-progress", VideoBgRemovalProgress {
        id: 0,
        progress: 0,
        message: "Downloading FFmpeg...".to_string(),
        eta_seconds: None,
    });

    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let url = ffmpeg_sidecar::download::ffmpeg_download_url()
            .map_err(|e| format!("Failed to get FFmpeg download URL: {}", e))?;

        let temp_path = ffmpeg_dir.join("ffmpeg-release-essentials.zip");

        let client = reqwest::blocking::Client::new();
        let mut response = client
            .get(url)
            .send()
            .map_err(|e| format!("HTTP request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {} error downloading FFmpeg", response.status()));
        }

        let total = response.content_length().unwrap_or(0);
        let mut file = fs::File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        let mut downloaded: u64 = 0;
        let mut last_pct: u8 = 0;
        let mut buffer = [0u8; 65536];

        loop {
            let n = response
                .read(&mut buffer)
                .map_err(|e| format!("Read error: {}", e))?;
            if n == 0 {
                break;
            }
            file.write_all(&buffer[..n])
                .map_err(|e| format!("Write error: {}", e))?;
            downloaded += n as u64;

            let pct = if total > 0 {
                ((downloaded as f64 / total as f64) * 100.0).round() as u8
            } else {
                0
            };
            if pct >= last_pct + 5 || downloaded >= total {
                last_pct = pct;
                let _ = app_for_blocking.emit("ffmpeg-download-progress", VideoBgRemovalProgress {
                    id: 0,
                    progress: pct.min(99),
                    message: format!("Downloading... {}%", pct),
                    eta_seconds: None,
                });
            }
        }

        file.sync_all()
            .map_err(|e| format!("Failed to sync file: {}", e))?;
        drop(file);

        let _ = app_for_blocking.emit("ffmpeg-download-progress", VideoBgRemovalProgress {
            id: 0,
            progress: 99,
            message: "Extracting FFmpeg...".to_string(),
            eta_seconds: None,
        });

        ffmpeg_sidecar::download::unpack_ffmpeg(&temp_path, &ffmpeg_dir)
            .map_err(|e| format!("Failed to unpack FFmpeg: {}", e))?;
        let _ = std::fs::remove_file(&temp_path);
        Ok(())
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))??;

    let _ = app.emit("ffmpeg-download-progress", VideoBgRemovalProgress {
        id: 0,
        progress: 100,
        message: "Download complete".to_string(),
        eta_seconds: None,
    });

    let (path, _) = find_ffmpeg();
    Ok(path.to_string_lossy().to_string())
}

fn probe_video(path: &PathBuf) -> Result<(Option<i64>, Option<i64>, Option<f64>, Option<f64>)> {
    let ffprobe = get_ffprobe_path();
    if !ffprobe.exists() {
        anyhow::bail!("ffprobe not found at {:?}", ffprobe);
    }

    let mut cmd = Command::new(&ffprobe);
    cmd.args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_streams",
            "-show_format",
            &path.to_string_lossy(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output()
        .context("Failed to run ffprobe")?;

    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)
        .context("Failed to parse ffprobe output")?;

    let video_stream = probe.streams.iter().find(|s| s.codec_type.as_deref() == Some("video"));

    if let Some(stream) = video_stream {
        let fps = stream.r_frame_rate.as_ref().and_then(|r| {
            let parts: Vec<&str> = r.split('/').collect();
            if parts.len() == 2 {
                let num: f64 = parts[0].parse().ok()?;
                let den: f64 = parts[1].parse().ok()?;
                if den > 0.0 { Some(num / den) } else { None }
            } else {
                r.parse().ok()
            }
        });

        let stream_duration = stream.duration.as_ref().and_then(|d| d.parse::<f64>().ok());
        let format_duration = probe.format.as_ref()
            .and_then(|f| f.duration.as_ref())
            .and_then(|d| d.parse::<f64>().ok());
        let duration = stream_duration.or(format_duration);

        Ok((stream.width, stream.height, fps, duration))
    } else {
        Ok((None, None, None, None))
    }
}

fn guess_mimetype(path: &PathBuf) -> String {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "mp4" => "video/mp4".to_string(),
        "mov" => "video/quicktime".to_string(),
        "avi" => "video/x-msvideo".to_string(),
        "mkv" => "video/x-matroska".to_string(),
        "webm" => "video/webm".to_string(),
        "flv" => "video/x-flv".to_string(),
        "wmv" => "video/x-ms-wmv".to_string(),
        "m4v" => "video/mp4".to_string(),
        "3gp" => "video/3gpp".to_string(),
        _ => "video/octet-stream".to_string(),
    }
}

#[tauri::command]
pub async fn import_videos(
    app: AppHandle,
    state: State<'_, DbState>,
    paths: Vec<String>,
) -> Result<VideoImportResult, String> {
    let pool = state.0.clone();

    let ffmpeg_available = get_ffmpeg_path().exists();
    let ffprobe_available = get_ffprobe_path().exists();

    if !ffmpeg_available || !ffprobe_available {
        return Err("FFmpeg is required to import videos. Please download it first in Settings.".to_string());
    }

    let thumbnails_dir = get_thumbnails_dir(&app);

    let mut imported_count: i64 = 0;
    let mut duplicates: i64 = 0;
    let mut failed: i64 = 0;

    for path_str in paths {
        let path = PathBuf::from(&path_str);
        if !path.exists() {
            failed += 1;
            continue;
        }

        // Validate file extension first
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        if let Some(extension) = ext.as_deref() {
            if !SUPPORTED_VIDEO_EXTENSIONS.contains(&extension) {
                failed += 1;
                continue;
            }
        } else {
            // No extension - skip
            failed += 1;
            continue;
        }

        let filename = match path.file_name() {
            Some(name) => name.to_string_lossy().to_string(),
            None => { failed += 1; continue; }
        };

        let existing: Option<(i64,)> = sqlx::query_as("SELECT id FROM videos WHERE filepath = ?")
            .bind(&path_str)
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;

        if existing.is_some() {
            duplicates += 1;
            continue;
        }

        let size = fs::metadata(&path).map(|m| m.len() as i64).ok();
        let mimetype = guess_mimetype(&path);

        let (width, height, fps, duration) = probe_video(&path).unwrap_or((None, None, None, None));

        // Content duplicate: same bytes under a different name.
        let sha256 = crate::crud::hashing::hash_file(&path)
            .ok()
            .filter(|hash| !hash.is_empty());
        if let Some(hash) = sha256.as_ref() {
            let existing: Option<(i64,)> =
                sqlx::query_as("SELECT id FROM videos WHERE sha256 = ?")
                    .bind(hash)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e| e.to_string())?;
            if existing.is_some() {
                duplicates += 1;
                continue;
            }
        }

        let result = sqlx::query(
            "INSERT INTO videos (filename, filepath, mimetype, size, width, height, duration, fps, sha256, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))"
        )
        .bind(&filename)
        .bind(&path_str)
        .bind(&mimetype)
        .bind(size)
        .bind(width)
        .bind(height)
        .bind(duration)
        .bind(fps)
        .bind(&sha256)
        .execute(&pool)
        .await;

        match result {
            Ok(row) => {
                let video_id = row.last_insert_rowid();
                let key = crate::crud::thumbnails::key_for(sha256.as_deref(), &path, video_id);
                let thumb = crate::crud::thumbnails::existing(&thumbnails_dir, &key).or_else(|| {
                    let base = crate::crud::thumbnails::target_base(&thumbnails_dir, &key);
                    extract_video_thumbnail(&path, &base, thumbnail_seek_seconds(duration)).ok()
                });
                if let Some(thumb) = thumb {
                    if let Some(thumb_str) = thumb.to_str() {
                        let _ = sqlx::query("UPDATE videos SET thumbnail_path = ? WHERE id = ?")
                            .bind(thumb_str)
                            .bind(video_id)
                            .execute(&pool)
                            .await;
                    }
                }
                imported_count += 1;
            }
            Err(e) => {
                log::error!("Failed to insert video {}: {}", path_str, e);
                failed += 1;
            }
        }
    }

    Ok(VideoImportResult { imported: imported_count, duplicates, failed })
}

pub(crate) fn get_thumbnails_dir(app: &AppHandle) -> PathBuf {
    crate::crud::thumbnails::cache_dir(app)
}

/// A small thumbnail taken near the start of the video. Seeking to ~10% of the
/// duration avoids the black/fade-in frames many recordings open with.
fn thumbnail_seek_seconds(duration: Option<f64>) -> f64 {
    match duration {
        Some(d) if d.is_finite() && d > 0.0 => d * 0.1,
        _ => 1.0,
    }
}

/// Extract a JPEG poster into the cache. `target_base` is the key path without
/// extension; the `.jpg` file is written next to it and its path returned.
fn extract_video_thumbnail(
    video_path: &PathBuf,
    target_base: &std::path::Path,
    seek_seconds: f64,
) -> Result<PathBuf> {
    let ffmpeg_path = get_ffmpeg_path();
    if !ffmpeg_path.exists() {
        anyhow::bail!("FFmpeg not found");
    }

    let output_path = target_base.with_extension("jpg");
    let seek = format!("{:.3}", seek_seconds.max(0.0));
    let mut cmd = Command::new(&ffmpeg_path);
    cmd.args([
            "-ss", &seek,
            "-i", &video_path.to_string_lossy(),
            "-frames:v", "1",
            "-vf", "scale=512:-2",
            "-q:v", "4",
            "-y",
            &output_path.to_string_lossy(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output().context("Failed to run ffmpeg for thumbnail extraction")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("FFmpeg thumbnail extraction failed: {}", stderr);
    }

    Ok(output_path)
}

#[derive(Clone, Serialize)]
pub struct VideoCompressionProgress {
    pub id: i64,
    pub progress: u8,
    pub message: String,
    pub stage: String,
    pub status: String,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub out_time: Option<f64>,
    pub eta_seconds: Option<f64>,
}

#[derive(Clone, Serialize)]
pub struct VideoConversionProgress {
    pub id: i64,
    pub progress: u8,
    pub message: String,
    pub stage: String,
    pub status: String,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub out_time: Option<f64>,
    pub eta_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionMode {
    Quality,
    TargetSize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompressionRequest {
    pub mode: CompressionMode,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub target_bytes: Option<i64>,
    #[serde(default)]
    pub target_percent: Option<f64>,
    #[serde(default)]
    pub drop_audio: Option<bool>,
    #[serde(default)]
    pub max_dimension: Option<i64>,
}

#[derive(Clone, Serialize)]
pub struct CompressionFileResult {
    pub id: i64,
    pub status: String,
    pub message: Option<String>,
    pub planned_bitrate_kbps: Option<i64>,
    pub achieved_size: Option<i64>,
    pub source_size: Option<i64>,
    pub output_path: Option<String>,
    pub kept_original: bool,
}

#[derive(Clone, Serialize)]
pub struct CompressionBatchResult {
    pub processed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub errors: Vec<CompressionFileError>,
    pub results: Vec<CompressionFileResult>,
}

#[derive(Clone, Serialize)]
pub struct CompressionFileError {
    pub id: i64,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct CompressionPreset {
    pub name: String,
    pub crf: u8,
    pub preset: String,
    pub codec: String,
    pub container: String,
    pub audio_codec: String,
    pub audio_bitrate_kbps: u32,
    pub pix_fmt: String,
    pub description: String,
    pub available: bool,
}

#[derive(Clone, Serialize, Default)]
pub struct EncoderCapabilities {
    pub h264: bool,
    pub hevc: bool,
    pub av1: bool,
    /// Hardware H.264 encoders that were detected *and* verified with a real
    /// (tiny) encode. Detection alone is not proof, so anything listed here has
    /// actually produced a frame on this machine.
    pub hardware: Vec<String>,
}

static ENCODER_CAPS: OnceLock<EncoderCapabilities> = OnceLock::new();

const MIN_VIDEO_BITRATE_KBPS: f64 = 150.0;
const DEFAULT_AUDIO_BITRATE_KBPS: f64 = 128.0;
const TARGET_SAFETY_FACTOR: f64 = 0.95;
const PROGRESS_THROTTLE_MS: u128 = 250;

#[derive(Debug, PartialEq, Eq)]
struct TargetPlan {
    video_bitrate_kbps: i64,
    audio_bitrate_kbps: i64,
    target_bytes: i64,
}

struct TargetMeta {
    kind: &'static str,
    value: f64,
    planned_bitrate_kbps: i64,
}

type VerifiedOutput = (Option<i64>, Option<i64>, Option<f64>);
type VideoRow = (String, Option<i64>, Option<f64>, Option<i64>, Option<i64>);

#[derive(Clone)]
struct QualityPreset {
    id: &'static str,
    name: &'static str,
    crf: u8,
    speed: &'static str,
}

const QUALITY_PRESETS: &[QualityPreset] = &[
    QualityPreset { id: "ultrafast", name: "Ultra Fast", crf: 23, speed: "ultrafast" },
    QualityPreset { id: "fast", name: "Fast", crf: 21, speed: "fast" },
    QualityPreset { id: "medium", name: "Medium", crf: 20, speed: "medium" },
    QualityPreset { id: "slow", name: "Slow", crf: 19, speed: "slow" },
    QualityPreset { id: "veryslow", name: "Very Slow", crf: 18, speed: "veryslow" },
];

#[derive(Clone)]
struct EncodeSpec {
    codec: &'static str,
    crf: Option<u8>,
    container: &'static str,
    pix_fmt: Option<&'static str>,
    video_args: Vec<String>,
    audio_codec: &'static str,
    audio_bitrate_kbps: u32,
    faststart: bool,
    tag: Option<&'static str>,
    hardware: Option<String>,
}

#[derive(Clone)]
enum AudioMode {
    Drop,
    Encode { codec: String, bitrate: u32 },
}

struct ProgressSample {
    fps: Option<f64>,
    speed: Option<f64>,
    out_time: Option<f64>,
    eta_seconds: Option<f64>,
}

type ProgressCallback<'a> = &'a dyn Fn(u8, &str, &ProgressSample);

fn quality_preset(id: &str) -> QualityPreset {
    QUALITY_PRESETS
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .unwrap_or_else(|| QUALITY_PRESETS[2].clone())
}

/// A hardware encoder is only usable if a real encode succeeds.
fn verify_hardware_encoder(ffmpeg: &std::path::Path, encoder: &str) -> bool {
    let mut cmd = ffmpeg_command(ffmpeg);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x144:d=0.2",
        "-c:v",
        encoder,
        "-frames:v",
        "1",
        "-f",
        "null",
        "-",
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null());

    matches!(cmd.status(), Ok(status) if status.success())
}

fn detect_encoder_capabilities() -> EncoderCapabilities {
    let fallback = EncoderCapabilities {
        h264: true,
        hevc: false,
        av1: false,
        hardware: Vec::new(),
    };

    let ffmpeg = get_ffmpeg_path();
    if !ffmpeg.exists() {
        // Assume only the widely supported baseline when we cannot ask FFmpeg.
        return fallback;
    }

    let mut cmd = ffmpeg_command(&ffmpeg);
    cmd.args(["-hide_banner", "-encoders"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let output = match cmd.output() {
        Ok(output) => output,
        Err(_) => return fallback,
    };
    let text = String::from_utf8_lossy(&output.stdout);

    let mut hardware = Vec::new();
    for candidate in [
        "h264_nvenc",
        "h264_qsv",
        "h264_videotoolbox",
        "h264_amf",
    ] {
        if text.contains(candidate) && verify_hardware_encoder(&ffmpeg, candidate) {
            hardware.push(candidate.to_string());
        }
    }

    EncoderCapabilities {
        h264: text.contains("libx264"),
        hevc: text.contains("libx265"),
        av1: text.contains("libsvtav1") || text.contains("libaom-av1"),
        hardware,
    }
}

fn encoder_capabilities() -> EncoderCapabilities {
    ENCODER_CAPS.get_or_init(detect_encoder_capabilities).clone()
}

#[tauri::command]
pub fn get_encoder_capabilities() -> EncoderCapabilities {
    encoder_capabilities()
}

#[tauri::command]
pub fn get_compression_presets() -> Vec<CompressionPreset> {
    let caps = encoder_capabilities();
    QUALITY_PRESETS
        .iter()
        .map(|p| CompressionPreset {
            name: p.name.to_string(),
            crf: p.crf,
            preset: p.id.to_string(),
            codec: "h264".to_string(),
            container: "mp4".to_string(),
            audio_codec: "aac".to_string(),
            audio_bitrate_kbps: DEFAULT_AUDIO_BITRATE_KBPS as u32,
            pix_fmt: "yuv420p".to_string(),
            description: format!("H.264 CRF {} · {} preset · AAC 128k · MP4", p.crf, p.speed),
            available: caps.h264,
        })
        .collect()
}

fn software_spec(
    codec: &'static str,
    encoder: &str,
    crf: u8,
    speed: &str,
    tag: Option<&'static str>,
) -> EncodeSpec {
    EncodeSpec {
        codec,
        crf: Some(crf),
        container: "mp4",
        pix_fmt: Some("yuv420p"),
        video_args: vec![
            "-c:v".to_string(),
            encoder.to_string(),
            "-crf".to_string(),
            crf.to_string(),
            "-preset".to_string(),
            speed.to_string(),
        ],
        audio_codec: "aac",
        audio_bitrate_kbps: DEFAULT_AUDIO_BITRATE_KBPS as u32,
        faststart: true,
        tag,
        hardware: None,
    }
}

fn hardware_spec(encoder: &str, crf: u8) -> EncodeSpec {
    let (video_args, pix_fmt): (Vec<String>, &'static str) = match encoder {
        "h264_nvenc" => (
            vec![
                "-c:v".to_string(),
                "h264_nvenc".to_string(),
                "-preset".to_string(),
                "p5".to_string(),
                "-rc".to_string(),
                "vbr".to_string(),
                "-cq".to_string(),
                crf.to_string(),
                "-b:v".to_string(),
                "0".to_string(),
            ],
            "yuv420p",
        ),
        "h264_qsv" => (
            vec![
                "-c:v".to_string(),
                "h264_qsv".to_string(),
                "-global_quality".to_string(),
                crf.to_string(),
            ],
            "nv12",
        ),
        "h264_amf" => (
            vec![
                "-c:v".to_string(),
                "h264_amf".to_string(),
                "-rc".to_string(),
                "cqp".to_string(),
                "-qp_i".to_string(),
                crf.to_string(),
                "-qp_p".to_string(),
                crf.to_string(),
            ],
            "yuv420p",
        ),
        "h264_videotoolbox" => {
            let quality = (100 - crf as i32 * 2).clamp(1, 100);
            (
                vec![
                    "-c:v".to_string(),
                    "h264_videotoolbox".to_string(),
                    "-q:v".to_string(),
                    quality.to_string(),
                ],
                "yuv420p",
            )
        }
        _ => return software_spec("h264", "libx264", crf, "medium", None),
    };

    EncodeSpec {
        codec: "h264",
        crf: Some(crf),
        container: "mp4",
        pix_fmt: Some(pix_fmt),
        video_args,
        audio_codec: "aac",
        audio_bitrate_kbps: DEFAULT_AUDIO_BITRATE_KBPS as u32,
        faststart: true,
        tag: None,
        hardware: Some(encoder.to_string()),
    }
}

/// `off` forces software, `auto`/unset picks the best verified encoder, and a
/// specific name is used as-is (resolve_spec still checks it was verified).
fn resolve_hardware_choice(setting: Option<&str>, verified: &[String]) -> Option<String> {
    match setting {
        Some("off") => None,
        Some("") | None | Some("auto") => verified.first().cloned(),
        Some(name) => Some(name.to_string()),
    }
}

fn resolve_spec(
    preset_id: &str,
    codec: Option<&str>,
    hardware: Option<&str>,
    crf_override: Option<u8>,
) -> Result<EncodeSpec, String> {
    let preset = quality_preset(preset_id);
    let caps = encoder_capabilities();
    let codec = codec.unwrap_or("h264");
    let crf = crf_override.unwrap_or(preset.crf);

    // Hardware is H.264 only, and only when verified available.
    if let Some(encoder) = hardware {
        if codec == "h264" && caps.hardware.iter().any(|h| h == encoder) {
            return Ok(hardware_spec(encoder, crf));
        }
    }

    match codec {
        "h264" | "auto" => Ok(software_spec("h264", "libx264", crf, preset.speed, None)),
        "hevc" => {
            if !caps.hevc {
                return Err("HEVC encoding is not available in this FFmpeg build".to_string());
            }
            Ok(software_spec(
                "hevc",
                "libx265",
                (crf + 5).min(51),
                preset.speed,
                Some("hvc1"),
            ))
        }
        "av1" => {
            if !caps.av1 {
                return Err("AV1 encoding is not available in this FFmpeg build".to_string());
            }
            let svt_preset = match preset.speed {
                "ultrafast" => "13",
                "fast" => "10",
                "medium" => "8",
                "slow" => "6",
                "veryslow" => "4",
                _ => "8",
            };
            Ok(software_spec(
                "av1",
                "libsvtav1",
                (crf + 12).min(50),
                svt_preset,
                None,
            ))
        }
        other => Err(format!("Unsupported codec: {}", other)),
    }
}

fn scale_filter(
    source_width: Option<i64>,
    source_height: Option<i64>,
    max_dimension: Option<i64>,
) -> Option<String> {
    let max = max_dimension.filter(|m| *m > 0)?;
    let (width, height) = (source_width?, source_height?);
    if width <= 0 || height <= 0 {
        return None;
    }
    if width.max(height) <= max {
        return None;
    }
    // Never upscale; keep aspect ratio; force even dimensions for the encoders.
    Some(format!(
        "scale='min({max},iw)':'min({max},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2",
        max = max
    ))
}

fn probe_has_audio(path: &PathBuf) -> bool {
    let ffprobe = get_ffprobe_path();
    if !ffprobe.exists() {
        return false;
    }

    let mut cmd = Command::new(&ffprobe);
    cmd.args(["-v", "quiet", "-print_format", "json", "-show_streams"])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    match cmd.output() {
        Ok(output) => match serde_json::from_slice::<ProbeOutput>(&output.stdout) {
            Ok(probe) => probe
                .streams
                .iter()
                .any(|s| s.codec_type.as_deref() == Some("audio")),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn parse_ffmpeg_time(value: &str) -> Option<f64> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    match parts.as_slice() {
        [h, m, s] => {
            let hours: f64 = h.parse().ok()?;
            let minutes: f64 = m.parse().ok()?;
            let seconds: f64 = s.parse().ok()?;
            Some(hours * 3600.0 + minutes * 60.0 + seconds)
        }
        _ => None,
    }
}

fn plan_target_size(
    target_bytes: i64,
    duration: f64,
    audio_kbps: f64,
) -> Result<TargetPlan, String> {
    if target_bytes <= 0 {
        return Err("Target size must be greater than zero".to_string());
    }
    if !duration.is_finite() || duration <= 0.0 {
        return Err("Cannot compute a target size: the video duration is unknown".to_string());
    }

    let total_kbps = (target_bytes as f64 * 8.0 * TARGET_SAFETY_FACTOR) / duration / 1000.0;
    let video_kbps = total_kbps - audio_kbps;

    if video_kbps < MIN_VIDEO_BITRATE_KBPS {
        let min_total_kbps = MIN_VIDEO_BITRATE_KBPS + audio_kbps;
        let min_bytes = (min_total_kbps * 1000.0 / 8.0 / TARGET_SAFETY_FACTOR) * duration;
        return Err(format!(
            "Target too small for a {} video: the minimum usable bitrate is {} kbps, so allow at least ~{}.",
            format_duration_human(duration),
            min_total_kbps as i64,
            format_size_human(min_bytes),
        ));
    }

    Ok(TargetPlan {
        video_bitrate_kbps: video_kbps.round() as i64,
        audio_bitrate_kbps: audio_kbps as i64,
        target_bytes,
    })
}

fn format_duration_human(seconds: f64) -> String {
    let total = seconds.round() as i64;
    if total >= 3600 {
        format!("{}h {}m", total / 3600, (total % 3600) / 60)
    } else if total >= 120 {
        format!("{}-minute", total / 60)
    } else {
        format!("{}-second", total)
    }
}

fn format_size_human(bytes: f64) -> String {
    format!("{:.1} MB", bytes / (1024.0 * 1024.0))
}

fn ffmpeg_command(program: &std::path::Path) -> Command {
    let mut cmd = Command::new(program);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

const CANCELLED_ERROR: &str = "Cancelled";
const WATCHDOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Kill the child and any processes it spawned.
fn terminate_child(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status();
    }

    let _ = child.kill();
    let _ = child.wait();
}

fn cancel_token_for(app: &AppHandle, id: i64) -> Arc<AtomicBool> {
    let state = app.state::<CancelTokens>();
    let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
    map.get(&id)
        .cloned()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
}

static RUNNING_FFMPEG_PIDS: OnceLock<std::sync::Mutex<std::collections::HashSet<u32>>> =
    OnceLock::new();

fn running_pids() -> &'static std::sync::Mutex<std::collections::HashSet<u32>> {
    RUNNING_FFMPEG_PIDS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Removes a PID from the registry when the running command ends for any reason.
struct PidGuard(u32);

impl Drop for PidGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = running_pids().lock() {
            set.remove(&self.0);
        }
    }
}

/// Kill every ffmpeg process we started. Called on app shutdown so nothing is
/// left running (and writing) after the window closes.
pub fn kill_all_ffmpeg() {
    let pids: Vec<u32> = {
        let set = running_pids().lock().unwrap_or_else(|e| e.into_inner());
        set.iter().copied().collect()
    };

    for pid in pids {
        #[cfg(windows)]
        {
            let mut cmd = Command::new("taskkill");
            cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
            let _ = cmd.status();
        }
        #[cfg(not(windows))]
        {
            let _ = pid;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_ffmpeg_with_progress(
    ffmpeg: &std::path::Path,
    args: &[String],
    duration: Option<f64>,
    stage: &str,
    progress_start: u8,
    progress_end: u8,
    on_progress: Option<ProgressCallback>,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<(), String> {
    let mut cmd = ffmpeg_command(ffmpeg);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start ffmpeg: {}", e))?;

    // Register the PID so app shutdown can reap it, and drop the guard on exit.
    let _pid_guard = {
        let pid = child.id();
        running_pids()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(pid);
        PidGuard(pid)
    };

    // Drain stderr on its own thread so a full pipe can never deadlock us.
    let stderr_handle = {
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            let mut buffer = String::new();
            if let Some(mut stream) = stderr {
                let _ = stream.read_to_string(&mut buffer);
            }
            buffer
        })
    };

    // Forward progress lines over a channel so the main loop can watch for
    // cancellation and stalls without blocking on a read.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Failed to capture ffmpeg progress output".to_string())?;
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(|line| line.ok()) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let start = Instant::now();
    let mut last_emit = start;
    let mut last_activity = start;
    let mut out_time: Option<f64> = None;
    let mut fps: Option<f64> = None;
    let mut speed: Option<f64> = None;

    let is_cancelled =
        |token: &Option<Arc<AtomicBool>>| token.as_ref().map(|t| t.load(Ordering::Relaxed)).unwrap_or(false);

    loop {
        if is_cancelled(&cancel) {
            terminate_child(&mut child);
            let _ = stderr_handle.join();
            return Err(CANCELLED_ERROR.to_string());
        }

        match rx.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(line) => {
                last_activity = Instant::now();
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                match key {
                    "out_time" => out_time = parse_ffmpeg_time(value),
                    "fps" => fps = value.trim().parse().ok(),
                    "speed" => speed = value.trim().trim_end_matches('x').trim().parse().ok(),
                    "progress" => {
                        let now = Instant::now();
                        let is_end = value.trim() == "end";
                        let due = now.duration_since(last_emit).as_millis() >= PROGRESS_THROTTLE_MS;
                        if let (Some(callback), Some(duration)) = (on_progress, duration) {
                            if (due || is_end) && duration > 0.0 {
                                last_emit = now;
                                let fraction = (out_time.unwrap_or(0.0) / duration).clamp(0.0, 1.0);
                                let overall = (progress_start as f64
                                    + fraction * (progress_end - progress_start) as f64)
                                    .round() as u8;
                                let elapsed = now.duration_since(start).as_secs_f64();
                                let eta = if fraction > 0.01 {
                                    Some((elapsed / fraction) - elapsed)
                                } else {
                                    None
                                };
                                callback(
                                    overall,
                                    stage,
                                    &ProgressSample {
                                        fps,
                                        speed,
                                        out_time,
                                        eta_seconds: eta,
                                    },
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last_activity.elapsed() >= WATCHDOG_TIMEOUT {
                    terminate_child(&mut child);
                    let _ = stderr_handle.join();
                    return Err("Encoding stalled and was stopped (no progress)".to_string());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for ffmpeg: {}", e))?;
    let stderr = stderr_handle.join().unwrap_or_default();

    if !status.success() {
        if is_cancelled(&cancel) {
            return Err(CANCELLED_ERROR.to_string());
        }
        let tail: Vec<&str> = stderr.lines().rev().take(8).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!("FFmpeg failed: {}", tail.join(" | ")));
    }

    Ok(())
}

fn audio_args(audio: &AudioMode) -> Vec<String> {
    match audio {
        AudioMode::Drop => vec!["-an".to_string()],
        AudioMode::Encode { codec, bitrate } => vec![
            "-c:a".to_string(),
            codec.clone(),
            "-b:a".to_string(),
            format!("{}k", bitrate),
        ],
    }
}

fn build_video_encode_args(
    spec: &EncodeSpec,
    input: &str,
    output: &str,
    filter: Option<&str>,
    audio: &AudioMode,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
    ];

    if let Some(filter) = filter {
        args.push("-vf".to_string());
        args.push(filter.to_string());
    }

    args.extend(spec.video_args.clone());
    if let Some(pix_fmt) = spec.pix_fmt {
        args.push("-pix_fmt".to_string());
        args.push(pix_fmt.to_string());
    }

    if let Some(tag) = spec.tag {
        args.push("-tag:v".to_string());
        args.push(tag.to_string());
    }

    args.extend(audio_args(audio));

    if spec.faststart {
        args.push("-movflags".to_string());
        args.push("+faststart".to_string());
    }
    args.push("-map_metadata".to_string());
    args.push("0".to_string());
    args.push("-f".to_string());
    args.push(spec.container.to_string());
    args.push(output.to_string());

    args
}

#[cfg(test)]
fn run_quality_encode(
    ffmpeg: &std::path::Path,
    input: &std::path::Path,
    output: &std::path::Path,
    spec: &EncodeSpec,
    filter: Option<&str>,
    audio: &AudioMode,
) -> Result<(), String> {
    let args = build_video_encode_args(
        spec,
        &input.to_string_lossy(),
        &output.to_string_lossy(),
        filter,
        audio,
    );
    run_ffmpeg_with_progress(ffmpeg, &args, None, "encoding", 0, 100, None, None)
}

fn build_pass1_args(
    input: &str,
    video_kbps: i64,
    preset: &str,
    passlog: &std::path::Path,
    filter: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
    ];
    if let Some(filter) = filter {
        args.push("-vf".to_string());
        args.push(filter.to_string());
    }
    args.extend([
        "-c:v".to_string(),
        "libx264".to_string(),
        "-b:v".to_string(),
        format!("{}k", video_kbps),
        "-preset".to_string(),
        preset.to_string(),
        "-pass".to_string(),
        "1".to_string(),
        "-passlogfile".to_string(),
        passlog.to_string_lossy().to_string(),
        "-an".to_string(),
        "-f".to_string(),
        "null".to_string(),
        "-".to_string(),
    ]);
    args
}

fn build_pass2_args(
    input: &str,
    output: &str,
    video_kbps: i64,
    preset: &str,
    passlog: &std::path::Path,
    filter: Option<&str>,
    audio: &AudioMode,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
    ];
    if let Some(filter) = filter {
        args.push("-vf".to_string());
        args.push(filter.to_string());
    }
    args.extend([
        "-c:v".to_string(),
        "libx264".to_string(),
        "-b:v".to_string(),
        format!("{}k", video_kbps),
        "-preset".to_string(),
        preset.to_string(),
        "-pass".to_string(),
        "2".to_string(),
        "-passlogfile".to_string(),
        passlog.to_string_lossy().to_string(),
    ]);
    args.extend(audio_args(audio));
    args.extend([
        "-movflags".to_string(),
        "+faststart".to_string(),
        "-map_metadata".to_string(),
        "0".to_string(),
        "-f".to_string(),
        "mp4".to_string(),
        "-y".to_string(),
        output.to_string(),
    ]);
    args
}

fn target_suffix(request: &CompressionRequest, target_bytes: i64) -> String {
    if let Some(percent) = request.target_percent {
        format!("{}pct", percent.round() as i64)
    } else {
        format!(
            "{}MB",
            (target_bytes as f64 / (1024.0 * 1024.0)).round() as i64
        )
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_compressed_video(
    pool: &sqlx::SqlitePool,
    id: i64,
    filepath: &str,
    size: i64,
    codec: Option<&str>,
    target_kind: Option<&str>,
    target_value: Option<f64>,
    crf: Option<u8>,
    target_bitrate_kbps: Option<i64>,
    achieved_bitrate_kbps: Option<i64>,
    preset: &str,
    duration: Option<f64>,
    width: Option<i64>,
    height: Option<i64>,
    source_size: Option<i64>,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO compressed_videos \
         (original_id, filepath, size, codec, target_kind, target_value, crf, \
          target_bitrate_kbps, achieved_bitrate_kbps, preset, duration, width, height, \
          source_size, status, is_primary, completed_at, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'done', 1, datetime('now'), datetime('now'))",
    )
    .bind(id)
    .bind(filepath)
    .bind(size)
    .bind(codec)
    .bind(target_kind)
    .bind(target_value)
    .bind(crf.map(|c| c as i64))
    .bind(target_bitrate_kbps)
    .bind(achieved_bitrate_kbps)
    .bind(preset)
    .bind(duration)
    .bind(width)
    .bind(height)
    .bind(source_size)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_compression_progress(
    app: &AppHandle,
    id: i64,
    progress: u8,
    stage: &str,
    status: &str,
    sample: Option<&ProgressSample>,
    message: &str,
) {
    let _ = app.emit(
        "video-compression-progress",
        VideoCompressionProgress {
            id,
            progress,
            message: message.to_string(),
            stage: stage.to_string(),
            status: status.to_string(),
            fps: sample.and_then(|s| s.fps),
            speed: sample.and_then(|s| s.speed),
            out_time: sample.and_then(|s| s.out_time),
            eta_seconds: sample.and_then(|s| s.eta_seconds),
        },
    );
}

fn stage_message(stage: &str, progress: u8) -> String {
    match stage {
        "probing" => "Probing...".to_string(),
        "pass1" => format!("First pass... {}%", progress),
        "pass2" => format!("Second pass... {}%", progress),
        "verifying" => "Verifying...".to_string(),
        _ => format!("Encoding... {}%", progress),
    }
}

async fn compress_one(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    id: i64,
    request: &CompressionRequest,
    output_dir_setting: &Option<String>,
    cancel: Arc<AtomicBool>,
    limiter: VideoJobLimiter,
) -> CompressionFileResult {
    let failed = |message: String| CompressionFileResult {
        id,
        status: "failed".to_string(),
        message: Some(message),
        planned_bitrate_kbps: None,
        achieved_size: None,
        source_size: None,
        output_path: None,
        kept_original: false,
    };

    let record: Option<VideoRow> =
        sqlx::query_as("SELECT filepath, size, duration, width, height FROM videos WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    let Some((filepath, db_size, db_duration, db_width, db_height)) = record else {
        return failed("Video not found".to_string());
    };

    let orig_path = PathBuf::from(&filepath);
    if !orig_path.exists() {
        return failed("Source file is missing".to_string());
    }

    let source_size = db_size.or_else(|| fs::metadata(&orig_path).ok().map(|m| m.len() as i64));

    emit_compression_progress(app, id, 2, "probing", "running", None, "Probing...");

    let probe = probe_video(&orig_path).ok();
    let duration = match db_duration {
        Some(d) if d > 0.0 => Some(d),
        _ => probe.as_ref().and_then(|p| p.3),
    };
    let source_width = db_width.or_else(|| probe.as_ref().and_then(|p| p.0));
    let source_height = db_height.or_else(|| probe.as_ref().and_then(|p| p.1));
    let has_audio = probe_has_audio(&orig_path);
    let drop_audio = request.drop_audio.unwrap_or(false);
    let filter = scale_filter(source_width, source_height, request.max_dimension);

    let file_stem = orig_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let preset = request
        .preset
        .clone()
        .unwrap_or_else(|| "medium".to_string());

    // Hardware acceleration: "off" forces software, a specific encoder name is
    // used if verified, and the default ("auto") picks the best verified encoder.
    let hardware_setting: Option<String> =
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = 'hardware_acceleration'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let hardware = resolve_hardware_choice(
        hardware_setting.as_deref(),
        &encoder_capabilities().hardware,
    );

    // `request.quality` preserves the legacy explicit-CRF argument.
    let spec = match resolve_spec(
        &preset,
        request.codec.as_deref(),
        hardware.as_deref(),
        request.quality,
    ) {
        Ok(spec) => spec,
        Err(e) => return failed(e),
    };

    let audio = if drop_audio || !has_audio {
        AudioMode::Drop
    } else {
        AudioMode::Encode {
            codec: spec.audio_codec.to_string(),
            bitrate: spec.audio_bitrate_kbps,
        }
    };
    let audio_kbps = match &audio {
        AudioMode::Encode { bitrate, .. } => *bitrate as f64,
        AudioMode::Drop => 0.0,
    };

    enum Job {
        Quality,
        Target { plan: TargetPlan, preset: String, audio: AudioMode, filter: Option<String> },
    }

    let (job, suffix, target_meta) = match request.mode {
        CompressionMode::Quality => (Job::Quality, String::new(), None),
        CompressionMode::TargetSize => {
            let Some(source_size) = source_size else {
                return failed("Source size is unknown; cannot compute a target".to_string());
            };

            let target_bytes = match (request.target_bytes, request.target_percent) {
                (Some(bytes), _) if bytes > 0 => bytes,
                (_, Some(percent)) if percent > 0.0 => {
                    ((source_size as f64) * percent / 100.0).round() as i64
                }
                _ => return failed("A target size or percentage is required".to_string()),
            };

            // Asking for a file at least as large as the source is a no-op.
            if target_bytes >= source_size {
                emit_compression_progress(
                    app,
                    id,
                    100,
                    "done",
                    "done",
                    None,
                    "Already optimal",
                );
                return CompressionFileResult {
                    id,
                    status: "kept".to_string(),
                    message: Some("Already optimal — kept the original".to_string()),
                    planned_bitrate_kbps: None,
                    achieved_size: Some(source_size),
                    source_size: Some(source_size),
                    output_path: None,
                    kept_original: true,
                };
            }

            let Some(duration) = duration else {
                return failed(
                    "Cannot compute a target size: the video duration is unknown".to_string(),
                );
            };

            let plan = match plan_target_size(target_bytes, duration, audio_kbps) {
                Ok(plan) => plan,
                Err(e) => return failed(e),
            };

            let kind = if request.target_percent.is_some() {
                "percent"
            } else {
                "absolute"
            };
            let value = request.target_percent.unwrap_or(target_bytes as f64);
            let meta = TargetMeta {
                kind,
                value,
                planned_bitrate_kbps: plan.video_bitrate_kbps,
            };
            let suffix = target_suffix(request, target_bytes);
            (
                Job::Target {
                    plan,
                    preset: preset.clone(),
                    audio: audio.clone(),
                    filter: filter.clone(),
                },
                suffix,
                Some(meta),
            )
        }
    };

    let new_filename = if suffix.is_empty() {
        format!("{}_compressed.mp4", file_stem)
    } else {
        format!("{}_compressed_{}.mp4", file_stem, suffix)
    };

    let final_filepath = match output_dir_setting {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
        _ => orig_path.with_file_name(&new_filename),
    };
    let temp_filepath = final_filepath.with_file_name(format!("{}.tmp", new_filename));

    let ffmpeg_path = get_ffmpeg_path();
    if !ffmpeg_path.exists() {
        return failed("FFmpeg not found".to_string());
    }

    let is_target_mode = target_meta.is_some();

    // Global ceiling: never run more than `max_concurrent_video_jobs` encoders.
    let (_permit, queue_position) = limiter.acquire().await;
    if queue_position > 0 {
        emit_compression_progress(
            app,
            id,
            0,
            "queued",
            "running",
            None,
            &format!("Queued (position {})", queue_position + 1),
        );
    }

    emit_compression_progress(app, id, 5, "encoding", "running", None, "Compressing...");

    let app_for_blocking = app.clone();
    let cancel_for_blocking = cancel.clone();
    let input_str = orig_path.to_string_lossy().to_string();
    let output_str = temp_filepath.to_string_lossy().to_string();
    let quality_filter = filter.clone();

    let quality_audio = audio_for_quality(&spec, drop_audio, has_audio);
    let quality_args = build_video_encode_args(
        &spec,
        &input_str,
        &output_str,
        quality_filter.as_deref(),
        &quality_audio,
    );
    // If a hardware encode fails for any reason, retry once with the software
    // encoder so the user never gets a broken job.
    let fallback_args = if spec.hardware.is_some() {
        resolve_spec(&preset, request.codec.as_deref(), None, request.quality)
            .ok()
            .map(|software| {
                let audio = audio_for_quality(&software, drop_audio, has_audio);
                build_video_encode_args(
                    &software,
                    &input_str,
                    &output_str,
                    quality_filter.as_deref(),
                    &audio,
                )
            })
    } else {
        None
    };

    let blocking = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let progress_callback = |progress: u8, stage: &str, sample: &ProgressSample| {
            emit_compression_progress(
                &app_for_blocking,
                id,
                progress,
                stage,
                "running",
                Some(sample),
                &stage_message(stage, progress),
            );
        };

        match job {
            Job::Quality => {
                let primary = run_ffmpeg_with_progress(
                    &ffmpeg_path,
                    &quality_args,
                    duration,
                    "encoding",
                    0,
                    100,
                    Some(&progress_callback),
                    Some(cancel_for_blocking.clone()),
                );
                match primary {
                    Err(e)
                        if e != CANCELLED_ERROR
                            && !cancel_for_blocking.load(Ordering::Relaxed)
                            && fallback_args.is_some() =>
                    {
                        log::warn!("Hardware encode failed ({}); falling back to software", e);
                        run_ffmpeg_with_progress(
                            &ffmpeg_path,
                            fallback_args.as_ref().expect("checked"),
                            duration,
                            "encoding",
                            0,
                            100,
                            Some(&progress_callback),
                            Some(cancel_for_blocking.clone()),
                        )
                    }
                    other => other,
                }
            }
            Job::Target { plan, preset, audio, filter } => {
                let pass_dir = tempfile::Builder::new()
                    .prefix("localstudio_ffmpeg2pass")
                    .tempdir()
                    .map_err(|e| format!("Failed to create pass log directory: {}", e))?;
                let passlog = pass_dir.path().join("ffmpeg2pass");

                let pass1 = build_pass1_args(
                    &input_str,
                    plan.video_bitrate_kbps,
                    &preset,
                    &passlog,
                    filter.as_deref(),
                );
                run_ffmpeg_with_progress(
                    &ffmpeg_path,
                    &pass1,
                    duration,
                    "pass1",
                    0,
                    45,
                    Some(&progress_callback),
                    Some(cancel_for_blocking.clone()),
                )?;

                let pass2 = build_pass2_args(
                    &input_str,
                    &output_str,
                    plan.video_bitrate_kbps,
                    &preset,
                    &passlog,
                    filter.as_deref(),
                    &audio,
                );
                // `pass_dir` is dropped at the end of this block, removing the
                // ffmpeg2pass logs.
                run_ffmpeg_with_progress(
                    &ffmpeg_path,
                    &pass2,
                    duration,
                    "pass2",
                    45,
                    100,
                    Some(&progress_callback),
                    Some(cancel_for_blocking.clone()),
                )
            }
        }
    })
    .await;

    let blocking = match blocking {
        Ok(inner) => inner,
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
            let message = format!("Encoding task failed: {}", e);
            return failed(message);
        }
    };

    if let Err(e) = blocking {
        let _ = fs::remove_file(&temp_filepath);
        if e == CANCELLED_ERROR || cancel.load(Ordering::Relaxed) {
            emit_compression_progress(app, id, 0, "cancelled", "cancelled", None, "Cancelled");
            return CompressionFileResult {
                id,
                status: "cancelled".to_string(),
                message: Some("Cancelled".to_string()),
                planned_bitrate_kbps: None,
                achieved_size: None,
                source_size,
                output_path: None,
                kept_original: false,
            };
        }
        emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
        return failed(e);
    }

    let achieved_size = match fs::metadata(&temp_filepath) {
        Ok(m) => m.len() as i64,
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
            let message = format!("Failed to read output size: {}", e);
            return failed(message);
        }
    };

    // Target-size jobs are verified before being trusted. The legacy quality
    // path is left exactly as it was (ffmpeg exit status only) so it does not
    // gain a new ffprobe dependency.
    let (out_width, out_height, out_duration) = if is_target_mode {
        emit_compression_progress(app, id, 97, "verifying", "running", None, "Verifying...");

        let temp_for_verify = temp_filepath.clone();
        let verified = tauri::async_runtime::spawn_blocking(
            move || -> Result<VerifiedOutput, String> {
                let (width, height, _fps, duration) = probe_video(&temp_for_verify)
                    .map_err(|e| format!("Verification failed: {}", e))?;
                if width.is_none() || height.is_none() {
                    return Err("Output has no video stream".to_string());
                }
                match duration {
                    Some(d) if d > 0.0 => {}
                    _ => return Err("Output has no duration".to_string()),
                }
                Ok((width, height, duration))
            },
        )
        .await;

        match verified {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                let _ = fs::remove_file(&temp_filepath);
                emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
                return failed(e);
            }
            Err(e) => {
                let _ = fs::remove_file(&temp_filepath);
                emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
                let message = format!("Verification task failed: {}", e);
                return failed(message);
            }
        }
    } else {
        (None, None, None)
    };

    // Never write a file that is not smaller than the source.
    if is_target_mode {
        if let Some(source_size) = source_size {
            if achieved_size >= source_size {
                let _ = fs::remove_file(&temp_filepath);
                emit_compression_progress(
                    app,
                    id,
                    100,
                    "done",
                    "done",
                    None,
                    "Already optimal",
                );
                return CompressionFileResult {
                    id,
                    status: "kept".to_string(),
                    message: Some("Already optimal — kept the original".to_string()),
                    planned_bitrate_kbps: None,
                    achieved_size: Some(achieved_size),
                    source_size: Some(source_size),
                    output_path: None,
                    kept_original: true,
                };
            }
        }
    }

    if final_filepath.exists() {
        let _ = fs::remove_file(&final_filepath);
    }
    if let Err(e) = fs::rename(&temp_filepath, &final_filepath) {
        let _ = fs::remove_file(&temp_filepath);
        emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
        let message = format!("Failed to finalize output: {}", e);
        return failed(message);
    }

    let fp = dunce::canonicalize(&final_filepath)
        .unwrap_or_else(|_| final_filepath.clone())
        .to_string_lossy()
        .to_string();

    let achieved_bitrate_kbps = out_duration
        .filter(|d| *d > 0.0)
        .map(|d| ((achieved_size as f64 * 8.0 / d) / 1000.0).round() as i64);

    let (target_kind, target_value, crf, planned_bitrate) = match &target_meta {
        Some(meta) => (
            Some(meta.kind),
            Some(meta.value),
            None,
            Some(meta.planned_bitrate_kbps),
        ),
        None => (Some("quality"), None, spec.crf, None),
    };

    let insert = insert_compressed_video(
        pool,
        id,
        &fp,
        achieved_size,
        Some(spec.codec),
        target_kind,
        target_value,
        crf,
        planned_bitrate,
        achieved_bitrate_kbps,
        &preset,
        out_duration.or(duration),
        out_width.or(source_width),
        out_height.or(source_height),
        source_size,
    )
    .await;

    if let Err(e) = insert {
        emit_compression_progress(app, id, 0, "error", "failed", None, "Failed");
        let message = format!("Failed to save result: {}", e);
        return failed(message);
    }

    emit_compression_progress(app, id, 100, "done", "done", None, "Done");
    let _ = app.emit("videos-updated", ());

    CompressionFileResult {
        id,
        status: "done".to_string(),
        message: None,
        planned_bitrate_kbps: planned_bitrate,
        achieved_size: Some(achieved_size),
        source_size,
        output_path: Some(fp),
        kept_original: false,
    }
}

fn audio_for_quality(spec: &EncodeSpec, drop_audio: bool, has_audio: bool) -> AudioMode {
    if drop_audio || !has_audio {
        AudioMode::Drop
    } else {
        AudioMode::Encode {
            codec: spec.audio_codec.to_string(),
            bitrate: spec.audio_bitrate_kbps,
        }
    }
}

async fn run_compression(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    ids: Vec<i64>,
    request: CompressionRequest,
) -> Result<CompressionBatchResult, String> {
    let output_dir_setting: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let concurrency_limit = num_cpus::get().max(2);
    let request = Arc::new(request);
    let ids_for_cleanup = ids.clone();
    let limiter = app.state::<VideoJobLimiter>().inner().clone();

    {
        let state = app.state::<CancelTokens>();
        let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        for &id in &ids {
            map.insert(id, Arc::new(AtomicBool::new(false)));
        }
    }

    let results: Vec<CompressionFileResult> = stream::iter(ids)
        .map(|id| {
            let app = app.clone();
            let pool = pool.clone();
            let output_dir_setting = output_dir_setting.clone();
            let request = request.clone();
            let cancel = cancel_token_for(&app, id);
            let limiter = limiter.clone();

            async move {
                compress_one(&app, &pool, id, &request, &output_dir_setting, cancel, limiter)
                    .await
            }
        })
        .buffer_unordered(concurrency_limit)
        .collect()
        .await;

    {
        let state = app.state::<CancelTokens>();
        let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        for id in &ids_for_cleanup {
            map.remove(id);
        }
    }

    let processed = results
        .iter()
        .filter(|r| r.status == "done" || r.status == "kept")
        .count();
    let failed = results.iter().filter(|r| r.status == "failed").count();
    let cancelled = results.iter().filter(|r| r.status == "cancelled").count();
    let errors = results
        .iter()
        .filter(|r| r.status == "failed")
        .filter_map(|r| {
            r.message.as_ref().map(|message| CompressionFileError {
                id: r.id,
                message: message.clone(),
            })
        })
        .collect();

    Ok(CompressionBatchResult {
        processed,
        failed,
        cancelled,
        errors,
        results,
    })
}

#[tauri::command]
pub async fn compress_videos_by_ids(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    quality: u8,
    preset: String,
) -> Result<usize, String> {
    let request = CompressionRequest {
        mode: CompressionMode::Quality,
        quality: Some(quality),
        preset: Some(preset),
        codec: None,
        target_bytes: None,
        target_percent: None,
        drop_audio: None,
        max_dimension: None,
    };
    let result = run_compression(&app, &state.0, ids, request).await?;
    Ok(result.processed)
}

#[tauri::command]
pub async fn compress_videos_by_ids_v2(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    request: CompressionRequest,
) -> Result<CompressionBatchResult, String> {
    run_compression(&app, &state.0, ids, request).await
}

fn build_video_ffmpeg_args(
    format: &str,
    input: &str,
    output: &str,
    has_audio: bool,
    filter: Option<&str>,
) -> Vec<String> {
    let container = match format {
        "webm" => "webm",
        "mov" => "mov",
        "gif" => "gif",
        _ => "mp4",
    };

    let mut args: Vec<String> = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
    ];

    if format != "gif" {
        if let Some(filter) = filter {
            args.push("-vf".to_string());
            args.push(filter.to_string());
        }
    }

    match format {
        "webm" => {
            args.extend(
                ["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-row-mt", "1"]
                    .map(String::from),
            );
            if has_audio {
                args.extend(["-c:a", "libopus", "-b:a", "128k"].map(String::from));
            } else {
                args.push("-an".to_string());
            }
        }
        "gif" => {
            args.extend(
                ["-an", "-vf", "fps=10,scale=320:-1:flags=lanczos"].map(String::from),
            );
        }
        _ => {
            args.extend(["-c:v", "libx264", "-pix_fmt", "yuv420p"].map(String::from));
            if has_audio {
                args.extend(["-c:a", "aac", "-b:a", "128k"].map(String::from));
            } else {
                args.push("-an".to_string());
            }
            args.extend(["-movflags", "+faststart"].map(String::from));
        }
    }

    if format != "gif" {
        args.extend(["-map_metadata", "0"].map(String::from));
    }

    args.extend(["-f", container, "-y", output].map(String::from));
    args
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConversionOptions {
    #[serde(default)]
    pub drop_audio: Option<bool>,
    #[serde(default)]
    pub max_dimension: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
fn emit_conversion_progress(
    app: &AppHandle,
    id: i64,
    progress: u8,
    stage: &str,
    status: &str,
    sample: Option<&ProgressSample>,
    message: &str,
) {
    let _ = app.emit(
        "video-conversion-progress",
        VideoConversionProgress {
            id,
            progress,
            message: message.to_string(),
            stage: stage.to_string(),
            status: status.to_string(),
            fps: sample.and_then(|s| s.fps),
            speed: sample.and_then(|s| s.speed),
            out_time: sample.and_then(|s| s.out_time),
            eta_seconds: sample.and_then(|s| s.eta_seconds),
        },
    );
}

#[allow(clippy::too_many_arguments)]
async fn convert_one(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    id: i64,
    format: String,
    opts: ConversionOptions,
    output_dir_setting: &Option<String>,
    cancel: Arc<AtomicBool>,
    limiter: VideoJobLimiter,
) -> CompressionFileResult {
    let failed = |message: String| CompressionFileResult {
        id,
        status: "failed".to_string(),
        message: Some(message),
        planned_bitrate_kbps: None,
        achieved_size: None,
        source_size: None,
        output_path: None,
        kept_original: false,
    };

    let record: Option<VideoRow> =
        sqlx::query_as("SELECT filepath, size, duration, width, height FROM videos WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    let Some((filepath, db_size, db_duration, db_width, db_height)) = record else {
        return failed("Video not found".to_string());
    };

    let orig_path = PathBuf::from(&filepath);
    if !orig_path.exists() {
        return failed("Source file is missing".to_string());
    }

    let source_size = db_size.or_else(|| fs::metadata(&orig_path).ok().map(|m| m.len() as i64));

    let ffmpeg_path = get_ffmpeg_path();
    if !ffmpeg_path.exists() {
        return failed("FFmpeg not found".to_string());
    }

    let probe = probe_video(&orig_path).ok();
    let duration = match db_duration {
        Some(d) if d > 0.0 => Some(d),
        _ => probe.as_ref().and_then(|p| p.3),
    };
    let source_width = db_width.or_else(|| probe.as_ref().and_then(|p| p.0));
    let source_height = db_height.or_else(|| probe.as_ref().and_then(|p| p.1));
    let has_audio = !opts.drop_audio.unwrap_or(false) && probe_has_audio(&orig_path);
    let filter = scale_filter(source_width, source_height, opts.max_dimension);

    let file_stem = orig_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let new_filename = format!("{}_{}.{}", file_stem, format, format);

    let final_filepath = match output_dir_setting {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
        _ => orig_path.with_file_name(&new_filename),
    };
    let temp_filepath = final_filepath.with_file_name(format!("{}.tmp", new_filename));

    let (_permit, queue_position) = limiter.acquire().await;
    if queue_position > 0 {
        emit_conversion_progress(
            app,
            id,
            0,
            "queued",
            "running",
            None,
            &format!("Queued (position {})", queue_position + 1),
        );
    }

    emit_conversion_progress(app, id, 5, "encoding", "running", None, "Converting...");

    let app_for_blocking = app.clone();
    let cancel_for_blocking = cancel.clone();
    let input_str = orig_path.to_string_lossy().to_string();
    let output_str = temp_filepath.to_string_lossy().to_string();
    let filter_clone = filter.clone();
    let format_for_blocking = format.clone();

    let convert_result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let progress_callback = |progress: u8, stage: &str, sample: &ProgressSample| {
            emit_conversion_progress(
                &app_for_blocking,
                id,
                progress,
                stage,
                "running",
                Some(sample),
                &format!("Converting... {}%", progress),
            );
        };

        let args = build_video_ffmpeg_args(
            &format_for_blocking,
            &input_str,
            &output_str,
            has_audio,
            filter_clone.as_deref(),
        );
        run_ffmpeg_with_progress(
            &ffmpeg_path,
            &args,
            duration,
            "encoding",
            0,
            100,
            Some(&progress_callback),
            Some(cancel_for_blocking.clone()),
        )
    })
    .await;

    match convert_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_filepath);
            if e == CANCELLED_ERROR || cancel.load(Ordering::Relaxed) {
                emit_conversion_progress(app, id, 0, "cancelled", "cancelled", None, "Cancelled");
                return CompressionFileResult {
                    id,
                    status: "cancelled".to_string(),
                    message: Some("Cancelled".to_string()),
                    planned_bitrate_kbps: None,
                    achieved_size: None,
                    source_size,
                    output_path: None,
                    kept_original: false,
                };
            }
            emit_conversion_progress(app, id, 0, "error", "failed", None, "Failed");
            return failed(e);
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            emit_conversion_progress(app, id, 0, "error", "failed", None, "Failed");
            let message = format!("Conversion task failed: {}", e);
            return failed(message);
        }
    }

    if final_filepath.exists() {
        let _ = fs::remove_file(&final_filepath);
    }
    if let Err(e) = fs::rename(&temp_filepath, &final_filepath) {
        let _ = fs::remove_file(&temp_filepath);
        emit_conversion_progress(app, id, 0, "error", "failed", None, "Failed");
        let message = format!("Failed to finalize output: {}", e);
        return failed(message);
    }

    let size = fs::metadata(&final_filepath).map(|m| m.len() as i64).ok();
    let fp = dunce::canonicalize(&final_filepath)
        .unwrap_or_else(|_| final_filepath.clone())
        .to_string_lossy()
        .to_string();

    let insert = sqlx::query(
        "INSERT OR REPLACE INTO converted_videos \
         (original_id, filepath, format, size, codec, status, source_size, completed_at, created_at) \
         VALUES (?, ?, ?, ?, ?, 'done', ?, datetime('now'), datetime('now'))",
    )
    .bind(id)
    .bind(&fp)
    .bind(&format)
    .bind(size)
    .bind(format.as_str())
    .bind(source_size)
    .execute(pool)
    .await;

    if let Err(e) = insert {
        emit_conversion_progress(app, id, 0, "error", "failed", None, "Failed");
        let message = format!("Failed to save result: {}", e);
        return failed(message);
    }

    emit_conversion_progress(app, id, 100, "done", "done", None, "Done");
    let _ = app.emit("videos-updated", ());

    CompressionFileResult {
        id,
        status: "done".to_string(),
        message: None,
        planned_bitrate_kbps: None,
        achieved_size: size,
        source_size,
        output_path: Some(fp),
        kept_original: false,
    }
}

async fn run_conversion(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    ids: Vec<i64>,
    format: String,
    opts: ConversionOptions,
) -> Result<CompressionBatchResult, String> {
    let output_dir_setting: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let concurrency_limit = num_cpus::get().max(2);
    let format = Arc::new(format.to_lowercase());
    let opts = Arc::new(opts);
    let ids_for_cleanup = ids.clone();
    let limiter = app.state::<VideoJobLimiter>().inner().clone();

    {
        let state = app.state::<CancelTokens>();
        let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        for &id in &ids {
            map.insert(id, Arc::new(AtomicBool::new(false)));
        }
    }

    let results: Vec<CompressionFileResult> = stream::iter(ids)
        .map(|id| {
            let app = app.clone();
            let pool = pool.clone();
            let output_dir_setting = output_dir_setting.clone();
            let format = format.clone();
            let opts = opts.clone();
            let cancel = cancel_token_for(&app, id);
            let limiter = limiter.clone();

            async move {
                convert_one(
                    &app,
                    &pool,
                    id,
                    (*format).clone(),
                    (*opts).clone(),
                    &output_dir_setting,
                    cancel,
                    limiter,
                )
                .await
            }
        })
        .buffer_unordered(concurrency_limit)
        .collect()
        .await;

    {
        let state = app.state::<CancelTokens>();
        let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
        for id in &ids_for_cleanup {
            map.remove(id);
        }
    }

    let processed = results.iter().filter(|r| r.status == "done").count();
    let failed = results.iter().filter(|r| r.status == "failed").count();
    let cancelled = results.iter().filter(|r| r.status == "cancelled").count();
    let errors = results
        .iter()
        .filter(|r| r.status == "failed")
        .filter_map(|r| {
            r.message.as_ref().map(|message| CompressionFileError {
                id: r.id,
                message: message.clone(),
            })
        })
        .collect();

    Ok(CompressionBatchResult {
        processed,
        failed,
        cancelled,
        errors,
        results,
    })
}

#[tauri::command]
pub async fn convert_videos_by_ids(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    format: String,
) -> Result<usize, String> {
    let result = run_conversion(&app, &state.0, ids, format, ConversionOptions::default()).await?;
    Ok(result.processed)
}

#[tauri::command]
pub async fn convert_videos_by_ids_v2(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    format: String,
    opts: Option<ConversionOptions>,
) -> Result<CompressionBatchResult, String> {
    run_conversion(&app, &state.0, ids, format, opts.unwrap_or_default()).await
}

fn set_cancel_tokens(cancel_tokens: &CancelTokens, ids: &[i64]) {
    let map = cancel_tokens.0.lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if let Some(token) = map.get(id) {
            token.store(true, Ordering::Relaxed);
        }
    }
}

#[tauri::command]
pub async fn cancel_video_compression(
    cancel_tokens: State<'_, CancelTokens>,
    ids: Vec<i64>,
) -> Result<(), String> {
    set_cancel_tokens(&cancel_tokens, &ids);
    Ok(())
}

#[tauri::command]
pub async fn cancel_video_conversion(
    cancel_tokens: State<'_, CancelTokens>,
    ids: Vec<i64>,
) -> Result<(), String> {
    set_cancel_tokens(&cancel_tokens, &ids);
    Ok(())
}

/// Remove leftover `.tmp` / pass-log files from a previous run that did not
/// finish cleanly (crash, power loss, killed process).
pub async fn cleanup_stale_temp_files(pool: &sqlx::SqlitePool) -> usize {
    let output_dir: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    let mut removed = 0usize;

    if let Some(dir) = output_dir {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let is_temp = name.ends_with(".tmp")
                        || name.contains(".tmp.")
                        || (name.starts_with("ffmpeg2pass") && name.ends_with(".log"));
                    if is_temp && fs::remove_file(&path).is_ok() {
                        removed += 1;
                    }
                }
            }
        }
    }

    if removed > 0 {
        log::info!("Removed {} stale temp/output file(s)", removed);
    }
    removed
}

#[derive(Debug, Deserialize)]
pub struct VideoQueryParams {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub sort_field: Option<String>,
    #[serde(default)]
    pub sort_order: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[tauri::command]
pub async fn get_all_videos(
    state: State<'_, DbState>,
    params: Option<VideoQueryParams>,
) -> Result<Vec<Video>, String> {
    let pool = state.0.clone();

    let order_clause = if let Some(ref p) = params {
        let sort_field = match p.sort_field.as_deref() {
            Some("name") => "v.filename",
            Some("size") => "v.size",
            _ => "v.id",
        };
        format!(
            "ORDER BY {} {}",
            sort_field,
            crate::crud::query::sort_direction(p.sort_order.as_deref())
        )
    } else {
        "ORDER BY v.id DESC".to_string()
    };

    let search = params
        .as_ref()
        .and_then(|p| p.search.clone())
        .unwrap_or_default();
    let (search_clause, search_bind) = if search.trim().is_empty() {
        (String::new(), None)
    } else {
        (
            "WHERE LOWER(v.filename) LIKE ? ESCAPE '\\'".to_string(),
            Some(crate::crud::query::like_contains(&search)),
        )
    };

    let (limit_clause, limit_bind) = params
        .as_ref()
        .map(|p| crate::crud::query::limit_offset(p.limit, p.offset))
        .unwrap_or((String::new(), None));

    let query = format!(
        "SELECT id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path FROM videos v {} {}{}",
        search_clause, order_clause, limit_clause
    );

    let mut q = sqlx::query_as::<
        _,
        (
            i64,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<f64>,
            Option<f64>,
            Option<String>,
        ),
    >(&query);
    if let Some(bind) = search_bind {
        q = q.bind(bind);
    }
    if let Some((limit, offset)) = limit_bind {
        q = q.bind(limit).bind(offset);
    }
    let rows = q.fetch_all(&pool).await.map_err(|e| e.to_string())?;

    let bg_rows = sqlx::query_as::<_, (i64, String, Option<i64>, String)>(
        "SELECT original_id, filepath, size, model_used FROM bg_removed_videos",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut bg_map: std::collections::HashMap<i64, (String, Option<i64>, String)> =
        std::collections::HashMap::new();
    for (orig_id, fp, sz, model) in bg_rows {
        bg_map.insert(orig_id, (fp, sz, model));
    }

    let comp_rows = sqlx::query_as::<_, (i64, String, Option<i64>)>(
        "SELECT original_id, filepath, size FROM compressed_videos",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut comp_map: std::collections::HashMap<i64, (String, Option<i64>)> =
        std::collections::HashMap::new();
    for (orig_id, fp, sz) in comp_rows {
        comp_map.insert(orig_id, (fp, sz));
    }

    let conv_rows = sqlx::query_as::<_, (i64, String, Option<i64>, String)>(
        "SELECT original_id, filepath, size, format FROM converted_videos",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut conv_map: std::collections::HashMap<i64, Vec<ConvertedVideo>> =
        std::collections::HashMap::new();
    for (orig_id, fp, sz, fmt) in conv_rows {
        conv_map.entry(orig_id).or_default().push(ConvertedVideo {
            filepath: fp,
            size: sz,
            format: fmt,
        });
    }

    let videos: Vec<Video> = rows
        .into_iter()
        .map(
            |(id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path)| {
                let (bg_removed_filepath, bg_removed_size, bg_removed_model) = match bg_map.get(&id) {
                    Some((fp, sz, model)) => (Some(fp.clone()), *sz, Some(model.clone())),
                    None => (None, None, None),
                };
                let (compressed_filepath, compressed_size) = match comp_map.get(&id) {
                    Some((fp, sz)) => (Some(fp.clone()), *sz),
                    None => (None, None),
                };
                let converted_videos = conv_map.remove(&id).unwrap_or_default();
                Video {
                    id,
                    filename,
                    filepath,
                    mimetype,
                    size,
                    width,
                    height,
                    duration,
                    fps,
                    thumbnail_path,
                    bg_removed_filepath,
                    bg_removed_size,
                    bg_removed_model,
                    compressed_filepath,
                    compressed_size,
                    converted_videos,
                }
            },
        )
        .collect();

    Ok(videos)
}

#[tauri::command]
pub async fn get_video_by_id(
    id: i64,
    state: State<'_, DbState>,
) -> Result<Video, String> {
    let pool = &state.0;

    let row = sqlx::query_as::<_, (
        i64, String, String, Option<String>, Option<i64>,
        Option<i64>, Option<i64>, Option<f64>, Option<f64>, Option<String>,
    )>(
        "SELECT id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path
         FROM videos WHERE id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "Video not found".to_string())?;

    let (id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path) = row;

    let bg_row = sqlx::query_as::<_, (String, Option<i64>, String)>(
        "SELECT filepath, size, model_used FROM bg_removed_videos WHERE original_id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let (bg_removed_filepath, bg_removed_size, bg_removed_model) = match bg_row {
        Some((fp, sz, model)) => (Some(fp), sz, Some(model)),
        None => (None, None, None),
    };

    let comp_row = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT filepath, size FROM compressed_videos WHERE original_id = ?"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let (compressed_filepath, compressed_size) = match comp_row {
        Some((fp, sz)) => (Some(fp), sz),
        None => (None, None),
    };

    let conv_rows = sqlx::query_as::<_, (String, Option<i64>, String)>(
        "SELECT filepath, size, format FROM converted_videos WHERE original_id = ? ORDER BY id DESC"
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let converted_videos: Vec<ConvertedVideo> = conv_rows
        .into_iter()
        .map(|(fp, sz, fmt)| ConvertedVideo { filepath: fp, size: sz, format: fmt })
        .collect();

    Ok(Video {
        id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path,
        bg_removed_filepath, bg_removed_size, bg_removed_model,
        compressed_filepath, compressed_size,
        converted_videos,
    })
}

#[tauri::command]
pub async fn delete_videos_by_ids(
    state: State<'_, DbState>,
    ids: Vec<i64>,
) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }

    // Single delete path; library files are kept (delete_files = false).
    crate::crud::lifecycle::delete_items_impl(
        &state.0,
        crate::crud::lifecycle::ItemKind::Video,
        &ids,
        false,
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn generate_video_thumbnails(
    app: AppHandle,
    state: State<'_, DbState>,
) -> Result<usize, String> {
    let pool = state.0.clone();
    let thumbnails_dir = get_thumbnails_dir(&app);

    // Self-healing: regenerate when the stored thumbnail is missing on disk.
    let rows: Vec<(i64, String, Option<f64>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, filepath, duration, sha256, thumbnail_path FROM videos")
            .fetch_all(&pool)
            .await
            .map_err(|e| e.to_string())?;

    let mut generated = 0;
    for (id, filepath, duration, sha256, thumbnail_path) in rows {
        if let Some(existing) = thumbnail_path.as_deref() {
            if std::path::Path::new(existing).is_file() {
                continue;
            }
        }

        let video_path = PathBuf::from(&filepath);
        if !video_path.is_file() {
            continue;
        }

        let key = crate::crud::thumbnails::key_for(sha256.as_deref(), &video_path, id);
        let thumb = crate::crud::thumbnails::existing(&thumbnails_dir, &key).or_else(|| {
            let base = crate::crud::thumbnails::target_base(&thumbnails_dir, &key);
            extract_video_thumbnail(&video_path, &base, thumbnail_seek_seconds(duration)).ok()
        });

        if let Some(thumb) = thumb {
            if let Some(thumb_str) = thumb.to_str() {
                if sqlx::query("UPDATE videos SET thumbnail_path = ? WHERE id = ?")
                    .bind(thumb_str)
                    .bind(id)
                    .execute(&pool)
                    .await
                    .is_ok()
                {
                    generated += 1;
                }
            }
        }
    }

    Ok(generated)
}

#[tauri::command]
pub async fn get_all_bg_removed_videos(state: State<'_, DbState>) -> Result<Vec<Video>, String> {
    let pool = state.0.clone();

    let rows = sqlx::query_as::<_, (i64, String, String, String, Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>)>(
        r#"
        SELECT v.id, v.filename, b.filepath, b.model_used, b.size,
               v.width, v.height, v.duration, v.fps
        FROM videos v
        INNER JOIN bg_removed_videos b ON v.id = b.original_id
        ORDER BY v.id DESC
        "#
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let result: Vec<Video> = rows.into_iter().map(|row| {
        Video {
            id: row.0,
            filename: row.1,
            filepath: row.2.clone(),
            mimetype: Some("video/webm".to_string()),
            size: row.4,
            width: row.5,
            height: row.6,
            duration: row.7,
            fps: row.8,
            thumbnail_path: None,
            bg_removed_filepath: Some(row.2),
            bg_removed_size: row.4,
            bg_removed_model: Some(row.3),
            compressed_filepath: None,
            compressed_size: None,
            converted_videos: vec![],
        }
    }).collect();

    Ok(result)
}

#[tauri::command]
pub async fn get_all_compressed_videos(state: State<'_, DbState>) -> Result<Vec<Video>, String> {
    let pool = state.0.clone();

    let rows = sqlx::query_as::<_, (i64, String, String, Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>, Option<String>)>(
        r#"
        SELECT v.id, v.filename, c.filepath, c.size,
               v.width, v.height, v.duration, v.fps, v.thumbnail_path
        FROM videos v
        INNER JOIN compressed_videos c ON v.id = c.original_id
        ORDER BY v.id DESC
        "#
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let result: Vec<Video> = rows.into_iter().map(|row| {
        Video {
            id: row.0,
            filename: row.1,
            filepath: row.2.clone(),
            mimetype: Some("video/mp4".to_string()),
            size: row.3,
            width: row.4,
            height: row.5,
            duration: row.6,
            fps: row.7,
            thumbnail_path: row.8,
            bg_removed_filepath: None,
            bg_removed_size: None,
            bg_removed_model: None,
            compressed_filepath: Some(row.2),
            compressed_size: row.3,
            converted_videos: vec![],
        }
    }).collect();

    Ok(result)
}

#[tauri::command]
pub async fn get_all_converted_videos(state: State<'_, DbState>) -> Result<Vec<Video>, String> {
    let pool = state.0.clone();

    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<f64>, Option<f64>, Option<String>)>(
        r#"
        SELECT v.id, v.filename, v.filepath, v.mimetype, v.size,
               v.width, v.height, v.duration, v.fps, v.thumbnail_path
        FROM videos v
        INNER JOIN converted_videos cvi ON v.id = cvi.original_id
        GROUP BY v.id
        ORDER BY MAX(cvi.id) DESC
        "#
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let conv_rows = sqlx::query_as::<_, (i64, String, Option<i64>, String)>(
        "SELECT original_id, filepath, size, format FROM converted_videos ORDER BY id DESC"
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut conv_map: std::collections::HashMap<i64, Vec<ConvertedVideo>> = std::collections::HashMap::new();
    for (orig_id, fp, sz, fmt) in conv_rows {
        conv_map.entry(orig_id).or_default().push(ConvertedVideo { filepath: fp, size: sz, format: fmt });
    }

    let result: Vec<Video> = rows.into_iter().map(|(id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path)| {
        let converted_videos = conv_map.remove(&id).unwrap_or_default();
        Video {
            id, filename, filepath, mimetype, size, width, height, duration, fps, thumbnail_path,
            bg_removed_filepath: None,
            bg_removed_size: None,
            bg_removed_model: None,
            compressed_filepath: None,
            compressed_size: None,
            converted_videos,
        }
    }).collect();

    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn process_video_frames(
    video_path: &PathBuf,
    output_path: &PathBuf,
    model_path: &PathBuf,
    width: u32,
    height: u32,
    fps: f64,
    total_frames: usize,
    video_id: i64,
    app: &AppHandle,
    cancel_token: Arc<AtomicBool>,
) -> Result<()> {
    let ffmpeg_path = get_ffmpeg_path();
    if !ffmpeg_path.exists() {
        anyhow::bail!("FFmpeg not found. Please download FFmpeg first.");
    }

    let session = create_onnx_session(model_path)?;

    let mut decoder = Command::new(&ffmpeg_path)
        .args([
            "-i", &video_path.to_string_lossy(),
            "-f", "rawvideo",
            "-pix_fmt", "rgb24",
            "-v", "quiet",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn FFmpeg decoder")?;

    let mut decoder_stdout = decoder.stdout.take()
        .context("Failed to get decoder stdout")?;

    let fps_str = format!("{:.2}", fps);
    let size_str = format!("{}x{}", width, height);

    let mut encoder = Command::new(&ffmpeg_path)
        .args([
            "-y",
            "-f", "rawvideo",
            "-pix_fmt", "rgba",
            "-s", &size_str,
            "-r", &fps_str,
            "-i", "pipe:0",
            "-c:v", "libvpx-vp9",
            "-pix_fmt", "yuva420p",
            "-crf", "30",
            "-b:v", "0",
            "-an",
            "-v", "quiet",
            &output_path.to_string_lossy(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn FFmpeg encoder")?;

    let mut encoder_stdin = encoder.stdin.take()
        .context("Failed to get encoder stdin")?;

    let row_bytes = width as usize * 3;
    let frame_size = row_bytes * height as usize;
    let mut frame_buffer = vec![0u8; frame_size];
    let mut frames_processed = 0usize;
    let start_time = Instant::now();

    loop {
        if cancel_token.load(Ordering::Relaxed) {
            drop(decoder_stdout);
            let _ = decoder.wait();
            drop(encoder_stdin);
            let _ = encoder.wait();
            anyhow::bail!("Processing cancelled");
        }

        match decoder_stdout.read_exact(&mut frame_buffer) {
            Ok(()) => {
                let mut rgba = vec![255u8; width as usize * height as usize * 4];
                for y in 0..height as usize {
                    for x in 0..width as usize {
                        let src_idx = y * row_bytes + x * 3;
                        let dst_idx = (y * width as usize + x) * 4;
                        rgba[dst_idx] = frame_buffer[src_idx];
                        rgba[dst_idx + 1] = frame_buffer[src_idx + 1];
                        rgba[dst_idx + 2] = frame_buffer[src_idx + 2];
                        // alpha already 255
                    }
                }

                let original_image = match image::RgbaImage::from_raw(width, height, rgba) {
                    Some(img) => DynamicImage::ImageRgba8(img),
                    None => DynamicImage::ImageRgba8(ImageBuffer::new(width, height)),
                };

                match apply_bg_removal(&original_image, &session) {
                    Ok(result_image) => {
                        let raw_rgba = result_image.as_raw();
                        if encoder_stdin.write_all(raw_rgba).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::error!("Frame {} bg removal failed for video {}: {}", frames_processed, video_id, e);
                        let blank_frame = vec![0u8; width as usize * height as usize * 4];
                        let _ = encoder_stdin.write_all(&blank_frame);
                    }
                }

                frames_processed += 1;

                if frames_processed % 5 == 0 || frames_processed == total_frames {
                    let progress = 10 + ((frames_processed as f32 / total_frames as f32) * 85.0) as u8;
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let frames_remaining = total_frames - frames_processed;
                    let eta = if frames_processed > 0 {
                        Some((elapsed / frames_processed as f64) * frames_remaining as f64)
                    } else {
                        None
                    };
                    let _ = app.emit("video-bg-removal-progress", VideoBgRemovalProgress {
                        id: video_id,
                        progress: progress.min(95),
                        message: format!("Processing frame {}/{}", frames_processed, total_frames),
                        eta_seconds: eta,
                    });
                }
            }
            Err(_) => break,
        }
    }

    drop(decoder_stdout);
    let _ = decoder.wait();

    drop(encoder_stdin);
    let _ = encoder.wait();

    Ok(())
}

#[tauri::command]
pub async fn remove_video_bg(
    app: AppHandle,
    state: State<'_, DbState>,
    cancel_tokens: State<'_, CancelTokens>,
    ids: Vec<i64>,
) -> Result<VideoBgRemovalResult, String> {
    let pool = state.0.clone();

    let (model_path, _model_size) = check_model_downloaded(&app)?;

    let ffmpeg_path = get_ffmpeg_path();
    if !ffmpeg_path.exists() {
        return Err("FFmpeg not found. Please download FFmpeg first.".to_string());
    }

    let output_dir_setting: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
        .fetch_optional(&pool)
        .await
        .unwrap_or(None);

    let model_path = Arc::new(model_path);

    let ids_for_cleanup = ids.clone();
    {
        let mut cancel_map = cancel_tokens.0.lock().unwrap_or_else(|e| e.into_inner());
        for &id in &ids {
            cancel_map.insert(id, Arc::new(AtomicBool::new(false)));
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum FrameResult { Success, Failed, Cancelled }

    let limiter = app.state::<VideoJobLimiter>().inner().clone();

    let results: Vec<FrameResult> = stream::iter(ids)
        .map(|id| {
            let app_clone = app.clone();
            let pool_clone = pool.clone();
            let output_dir_clone = output_dir_setting.clone();
            let model_path = model_path.clone();
            let cancel_tokens_state = app.state::<CancelTokens>();
            let cancel_tokens_inner = cancel_tokens_state.inner();
            let limiter = limiter.clone();

            async move {
                let cancel_token = {
                    let map = cancel_tokens_inner.0.lock().unwrap_or_else(|e| e.into_inner());
                    map.get(&id).cloned().unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
                };
                // Share the global encoder ceiling with compression/conversion.
                let _permit = limiter.acquire().await.0;
                let video_record: Option<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
                    "SELECT filepath, width, height FROM videos WHERE id = ?"
                )
                .bind(id)
                .fetch_optional(&pool_clone)
                .await
                .ok()
                .flatten();

                if let Some((filepath, db_width, db_height)) = video_record {
                    let orig_path = PathBuf::from(&filepath);
                    if !orig_path.exists() {
                        return FrameResult::Failed;
                    }

                    let _ = app_clone.emit("video-bg-removal-progress", VideoBgRemovalProgress {
                        id,
                        progress: 5,
                        message: "Preparing...".to_string(),
                        eta_seconds: None,
                    });

                    let probe_data = probe_video(&orig_path).unwrap_or((None, None, None, None));
                    let video_width = db_width.or(probe_data.0).unwrap_or(1920);
                    let video_height = db_height.or(probe_data.1).unwrap_or(1080);
                    let video_fps = probe_data.2.unwrap_or(30.0);
                    let video_duration = probe_data.3.unwrap_or(10.0);

                    let total_frames = ((video_duration * video_fps).ceil() as usize).max(1);

                    let file_stem = orig_path.file_stem().unwrap_or_default().to_string_lossy();
                    let new_filename = format!("{}_no_bg.webm", file_stem);

                    let final_filepath = match &output_dir_clone {
                        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
                        _ => orig_path.with_file_name(&new_filename),
                    };

                    let temp_filename = format!("{}.tmp.webm", file_stem);
                    let temp_filepath = match &output_dir_clone {
                        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&temp_filename),
                        _ => orig_path.with_file_name(&temp_filename),
                    };

                    let _ = app_clone.emit("video-bg-removal-progress", VideoBgRemovalProgress {
                        id,
                        progress: 10,
                        message: "Processing frames...".to_string(),
                        eta_seconds: None,
                    });

                    if let Some(parent) = temp_filepath.parent() {
                        if !parent.exists() {
                            let _ = fs::create_dir_all(parent);
                        }
                    }

                    let temp_path_for_cleanup = temp_filepath.clone();
                    let app_for_emit = app_clone.clone();

                    let result = tauri::async_runtime::spawn_blocking(move || -> Result<()> {
                        process_video_frames(
                            &orig_path,
                            &temp_filepath,
                            &model_path,
                            video_width as u32,
                            video_height as u32,
                            video_fps,
                            total_frames,
                            id,
                            &app_clone,
                            cancel_token,
                        )
                    }).await;

                    match result {
                        Ok(Ok(())) => {
                            if final_filepath.exists() {
                                let _ = fs::remove_file(&final_filepath);
                            }

                            if let Err(e) = fs::rename(&temp_path_for_cleanup, &final_filepath) {
                                log::error!("Failed to rename temp video: {}", e);
                                let _ = fs::remove_file(&temp_path_for_cleanup);
                                return FrameResult::Failed;
                            }

                            let size = fs::metadata(&final_filepath).map(|m| m.len() as i64).ok();
                            let fp = dunce::canonicalize(&final_filepath).unwrap_or(final_filepath).to_string_lossy().to_string();

                            let insert_result = sqlx::query(
                                "INSERT OR REPLACE INTO bg_removed_videos (original_id, filepath, size, model_used) VALUES (?, ?, ?, ?)"
                            )
                            .bind(id)
                            .bind(fp)
                            .bind(size)
                            .bind("bria-rmbg-1.4")
                            .execute(&pool_clone)
                            .await;

                            if insert_result.is_ok() {
                                let _ = app_for_emit.emit("video-bg-removal-progress", VideoBgRemovalProgress {
                                    id,
                                    progress: 100,
                                    message: "Done".to_string(),
                                    eta_seconds: None,
                                });
                                let _ = app_for_emit.emit("videos-updated", ());
                                return FrameResult::Success;
                            }
                        }
                        Ok(Err(e)) => {
                            if e.to_string().contains("cancelled") {
                                log::info!("Video background removal cancelled for ID {}", id);
                                let _ = fs::remove_file(&temp_path_for_cleanup);
                                let _ = app_for_emit.emit("video-bg-removal-progress", VideoBgRemovalProgress {
                                    id,
                                    progress: 0,
                                    message: "Cancelled".to_string(),
                                    eta_seconds: None,
                                });
                                return FrameResult::Cancelled;
                            } else {
                                log::error!("Video background removal error for ID {}: {}", id, e);
                            }
                        }
                        Err(e) => {
                            log::error!("Tokio spawn error for ID {}: {}", id, e);
                        }
                    }

                    let _ = fs::remove_file(&temp_path_for_cleanup);
                    let _ = app_for_emit.emit("video-bg-removal-progress", VideoBgRemovalProgress {
                        id,
                        progress: 0,
                        message: "Failed".to_string(),
                        eta_seconds: None,
                    });
                }
                FrameResult::Failed
            }
        })
        .buffer_unordered(1)
        .collect()
        .await;

    let processed_count = results.iter().filter(|&&r| r == FrameResult::Success).count();
    let cancelled_count = results.iter().filter(|&&r| r == FrameResult::Cancelled).count();
    let failed_count = results.iter().filter(|&&r| r == FrameResult::Failed).count();

    {
        let mut cancel_map = cancel_tokens.0.lock().unwrap_or_else(|e| e.into_inner());
        for id in &ids_for_cleanup {
            cancel_map.remove(id);
        }
    }

    Ok(VideoBgRemovalResult { processed: processed_count, failed: failed_count, cancelled: cancelled_count })
}

#[tauri::command]
pub async fn cancel_video_bg_removal(
    cancel_tokens: State<'_, CancelTokens>,
    ids: Vec<i64>,
) -> Result<(), String> {
    let cancel_map = cancel_tokens.0.lock().unwrap_or_else(|e| e.into_inner());
    for id in &ids {
        if let Some(token) = cancel_map.get(id) {
            token.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[cfg(test)]
mod compression_tests {
    use super::*;

    fn h264_spec() -> EncodeSpec {
        resolve_spec("medium", Some("h264"), None, None).expect("spec")
    }

    #[test]
    fn plan_matches_expected_bitrate() {
        let target = 20 * 1024 * 1024;
        let plan = plan_target_size(target, 75.0, DEFAULT_AUDIO_BITRATE_KBPS).expect("plan");
        let expected = ((target as f64 * 8.0 * TARGET_SAFETY_FACTOR) / 75.0 / 1000.0
            - DEFAULT_AUDIO_BITRATE_KBPS)
            .round() as i64;
        assert_eq!(plan.video_bitrate_kbps, expected);
        assert_eq!(plan.audio_bitrate_kbps, 128);
        assert!(plan.video_bitrate_kbps > MIN_VIDEO_BITRATE_KBPS as i64);
    }

    #[test]
    fn plan_rejects_tiny_target_for_long_video() {
        let err =
            plan_target_size(1024 * 1024, 45.0 * 60.0, DEFAULT_AUDIO_BITRATE_KBPS).unwrap_err();
        assert!(err.contains("Target too small"), "{err}");
        assert!(err.contains("allow at least"), "{err}");
    }

    #[test]
    fn plan_rejects_unknown_duration() {
        assert!(plan_target_size(20 * 1024 * 1024, 0.0, DEFAULT_AUDIO_BITRATE_KBPS).is_err());
    }

    #[test]
    fn percent_target_derives_expected_bytes() {
        let source = 100 * 1024 * 1024i64;
        let bytes = ((source as f64) * 50.0 / 100.0).round() as i64;
        assert_eq!(bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn scale_filter_skips_within_bounds_and_downscales_even() {
        assert!(scale_filter(Some(1280), Some(720), Some(1920)).is_none());
        assert!(scale_filter(Some(1920), Some(1080), None).is_none());
        let filter = scale_filter(Some(3840), Some(2160), Some(1920)).expect("filter");
        assert!(filter.contains("force_divisible_by=2"), "{filter}");
        assert!(filter.contains("min(1920,iw)"), "{filter}");
    }

    #[test]
    fn spec_resolution_reflects_codec_capabilities() {
        let h264 = resolve_spec("medium", Some("h264"), None, None).expect("h264");
        assert!(h264.video_args.iter().any(|a| a == "libx264"), "{:?}", h264.video_args);
        assert_eq!(h264.container, "mp4");
        assert!(h264.faststart);
        assert!(h264.hardware.is_none());

        assert!(resolve_spec("medium", Some("vp9000"), None, None).is_err());

        // Explicit CRF override wins over the preset default.
        let overridden = resolve_spec("medium", Some("h264"), None, Some(30)).expect("override");
        assert_eq!(overridden.crf, Some(30));

        // Hardware is only used when it appears in the verified capability list.
        let caps = encoder_capabilities();
        if let Some(first) = caps.hardware.first() {
            let hw = resolve_spec("medium", Some("h264"), Some(first), None).expect("hw");
            assert_eq!(hw.hardware.as_deref(), Some(first.as_str()));
        }
        assert!(resolve_spec("medium", Some("h264"), Some("h264_nvenc"), None)
            .map(|s| s.hardware.is_some())
            .unwrap_or(false)
            == caps.hardware.iter().any(|h| h == "h264_nvenc"));

        // HEVC/AV1 are accepted only when the local FFmpeg reports them.
        assert_eq!(resolve_spec("medium", Some("hevc"), None, None).is_ok(), caps.hevc);
        assert_eq!(resolve_spec("medium", Some("av1"), None, None).is_ok(), caps.av1);
    }

    #[test]
    fn parse_ffmpeg_time_reads_timestamps() {
        assert_eq!(parse_ffmpeg_time("00:01:02.500000"), Some(62.5));
        assert_eq!(parse_ffmpeg_time("garbage"), None);
    }

    #[test]
    fn conversion_args_are_modern_and_audio_aware() {
        let pair = |args: &[String], key: &str, value: &str| {
            args.windows(2)
                .any(|w| w[0] == key && w[1] == value)
        };

        let webm = build_video_ffmpeg_args("webm", "in.mp4", "out.webm", true, None);
        assert!(pair(&webm, "-c:v", "libvpx-vp9"), "{webm:?}");
        assert!(pair(&webm, "-c:a", "libopus"), "{webm:?}");

        let no_audio = build_video_ffmpeg_args("mp4", "in.mov", "out.mp4", false, None);
        assert!(no_audio.iter().any(|a| a == "-an"), "{no_audio:?}");
        assert!(pair(&no_audio, "-movflags", "+faststart"), "{no_audio:?}");

        let gif = build_video_ffmpeg_args("gif", "in.mp4", "out.gif", true, None);
        assert!(gif.iter().any(|a| a == "-an"), "{gif:?}");
        assert!(pair(&gif, "-f", "gif"), "{gif:?}");

        let scaled = build_video_ffmpeg_args(
            "mp4",
            "in.mp4",
            "out.mp4",
            true,
            Some("scale='min(1920,iw)':-2"),
        );
        assert!(
            pair(&scaled, "-vf", "scale='min(1920,iw)':-2"),
            "{scaled:?}"
        );
    }

    #[test]
    fn hardware_choice_defaults_to_auto() {
        let verified = vec!["h264_nvenc".to_string(), "h264_qsv".to_string()];
        assert_eq!(
            resolve_hardware_choice(None, &verified).as_deref(),
            Some("h264_nvenc")
        );
        assert_eq!(
            resolve_hardware_choice(Some("auto"), &verified).as_deref(),
            Some("h264_nvenc")
        );
        assert_eq!(resolve_hardware_choice(Some("off"), &verified), None);
        assert_eq!(
            resolve_hardware_choice(Some("h264_qsv"), &verified).as_deref(),
            Some("h264_qsv")
        );
        assert_eq!(resolve_hardware_choice(None, &[]), None);
    }

    #[test]
    fn thumbnail_seek_uses_ten_percent_and_handles_unknown() {
        assert!((thumbnail_seek_seconds(Some(75.0)) - 7.5).abs() < 1e-9);
        assert_eq!(thumbnail_seek_seconds(None), 1.0);
        assert_eq!(thumbnail_seek_seconds(Some(0.0)), 1.0);
        assert_eq!(thumbnail_seek_seconds(Some(f64::NAN)), 1.0);
    }

    #[test]
    fn limiter_reports_queue_and_resizes() {
        tauri::async_runtime::block_on(async {
            let limiter = VideoJobLimiter::new(2);
            assert_eq!(limiter.limit(), 2);

            let (_p1, position1) = limiter.acquire().await;
            let (_p2, position2) = limiter.acquire().await;
            assert_eq!(position1, 0);
            assert_eq!(position2, 0);

            // Resizing takes effect for subsequent acquisitions.
            limiter.set_limit(1);
            assert_eq!(limiter.limit(), 1);
            let (_p3, _position3) = limiter.acquire().await;
        });
    }

    #[test]
    fn cleanup_removes_only_stale_temp_files() {
        tauri::async_runtime::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(dir.path().join("db.sqlite"))
                .create_if_missing(true);
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .expect("pool");
            sqlx::query(
                "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL)",
            )
            .execute(&pool)
            .await
            .expect("settings table");

            let out = dir.path().join("out");
            std::fs::create_dir_all(&out).expect("out dir");
            for name in [
                "clip_compressed.mp4.tmp",
                "clip_no_bg.tmp.webm",
                "ffmpeg2pass-0.log",
                "keep.mp4",
                "movie_compressed_20MB.mp4",
            ] {
                std::fs::write(out.join(name), b"x").expect("write file");
            }

            sqlx::query("INSERT INTO settings (key, value) VALUES ('output', ?)")
                .bind(out.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed output setting");

            let removed = cleanup_stale_temp_files(&pool).await;
            assert_eq!(removed, 3, "should remove the three temp files");

            assert!(!out.join("clip_compressed.mp4.tmp").exists());
            assert!(!out.join("clip_no_bg.tmp.webm").exists());
            assert!(!out.join("ffmpeg2pass-0.log").exists());
            assert!(out.join("keep.mp4").exists(), "real output kept");
            assert!(
                out.join("movie_compressed_20MB.mp4").exists(),
                "real output kept"
            );
        });
    }

    /// Cancelling mid-encode must stop ffmpeg and report CANCELLED.
    /// `LOCALSTUDIO_TEST_FFMPEG=<ffmpeg.exe> cargo test --lib cancel_stops_encoding -- --ignored --nocapture`
    #[test]
    #[ignore = "set LOCALSTUDIO_TEST_FFMPEG to ffmpeg.exe and run with --ignored"]
    fn cancel_stops_encoding() {
        let ffmpeg = match std::env::var_os("LOCALSTUDIO_TEST_FFMPEG") {
            Some(path) => PathBuf::from(path),
            None => return,
        };
        let ffmpeg = ffmpeg.as_path();

        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(800));
            cancel_for_thread.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        // `-re` makes ffmpeg read the infinite test source in real time, so it
        // would run forever without the cancel.
        let args: Vec<String> = [
            "-y",
            "-progress",
            "pipe:1",
            "-nostats",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=25",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-f",
            "null",
            "-",
        ]
        .map(String::from)
        .to_vec();

        let start = std::time::Instant::now();
        let result =
            run_ffmpeg_with_progress(ffmpeg, &args, None, "encoding", 0, 100, None, Some(cancel));
        assert_eq!(result.unwrap_err(), CANCELLED_ERROR);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "cancel took too long: {:?}",
            start.elapsed()
        );
    }

    /// Real encoder smoke test. Run with:
    /// `LOCALSTUDIO_TEST_FFMPEG=<ffmpeg.exe> cargo test --lib two_pass_lands_within_target -- --ignored --nocapture`
    #[test]
    #[ignore = "set LOCALSTUDIO_TEST_FFMPEG to ffmpeg.exe and run with --ignored"]
    fn two_pass_lands_within_target() {
        let ffmpeg = match std::env::var_os("LOCALSTUDIO_TEST_FFMPEG") {
            Some(path) => PathBuf::from(path),
            None => return,
        };
        let ffmpeg = ffmpeg.as_path();

        // Helpers that resolve FFmpeg themselves (e.g. thumbnails) need the dir.
        if let Some(parent) = ffmpeg.parent() {
            let _ = FFMPEG_DIR.set(parent.to_path_buf());
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.mp4");
        let output = dir.path().join("out.mp4");

        // Grain makes the source incompressible so ABR must use its full budget.
        let mut gen = ffmpeg_command(ffmpeg);
        gen.args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30:duration=75",
        ])
        .args(["-vf", "noise=alls=25:allf=t"])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-b:v",
            "10M",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&source);
        let gen_out = gen.output().expect("run ffmpeg generator");
        if !gen_out.status.success() {
            panic!(
                "generate source failed: {}",
                String::from_utf8_lossy(&gen_out.stderr)
            );
        }

        let source_size = std::fs::metadata(&source).expect("source size").len() as i64;
        let target_bytes = 20 * 1024 * 1024i64;
        assert!(
            source_size > target_bytes,
            "source should be larger than target"
        );

        let plan = plan_target_size(target_bytes, 75.0, DEFAULT_AUDIO_BITRATE_KBPS).expect("plan");
        let pass_dir = tempfile::tempdir().expect("pass dir");
        let passlog = pass_dir.path().join("ffmpeg2pass");
        let input = source.to_string_lossy().to_string();
        let output_str = output.to_string_lossy().to_string();

        let samples: std::cell::RefCell<Vec<(String, u8)>> = std::cell::RefCell::new(Vec::new());
        let on_progress = |progress: u8, stage: &str, _sample: &ProgressSample| {
            samples.borrow_mut().push((stage.to_string(), progress));
        };

        let pass1 = build_pass1_args(&input, plan.video_bitrate_kbps, "veryfast", &passlog, None);
        run_ffmpeg_with_progress(
            ffmpeg,
            &pass1,
            Some(75.0),
            "pass1",
            0,
            45,
            Some(&on_progress),
            None,
        )
        .expect("pass1");

        let audio = AudioMode::Drop;
        let pass2 = build_pass2_args(
            &input,
            &output_str,
            plan.video_bitrate_kbps,
            "veryfast",
            &passlog,
            None,
            &audio,
        );
        run_ffmpeg_with_progress(
            ffmpeg,
            &pass2,
            Some(75.0),
            "pass2",
            45,
            100,
            Some(&on_progress),
            None,
        )
        .expect("pass2");

        // Progress must never run backwards across both passes.
        let samples = samples.into_inner();
        assert!(!samples.is_empty(), "expected progress events");
        let mut last = 0u8;
        for (stage, value) in &samples {
            assert!(
                *value >= last,
                "progress went backwards: {last} -> {value} ({stage})"
            );
            last = *value;
        }
        assert!(samples.iter().any(|(stage, _)| stage == "pass1"));
        assert!(samples.iter().any(|(stage, _)| stage == "pass2"));

        let achieved = std::fs::metadata(&output).expect("output size").len() as i64;
        let lower = (target_bytes as f64 * 0.85) as i64;
        println!(
            "source={source_size} target={target_bytes} achieved={achieved} planned_kbps={}",
            plan.video_bitrate_kbps
        );
        assert!(
            achieved >= lower && achieved <= target_bytes,
            "achieved {achieved} not within [{lower}, {target_bytes}]"
        );

        // Legacy single-pass quality path with a resolved spec.
        let quality_out = dir.path().join("quality.mp4");
        let spec = h264_spec();
        run_quality_encode(ffmpeg, &source, &quality_out, &spec, None, &AudioMode::Drop)
            .expect("quality encode");
        let quality_size = std::fs::metadata(&quality_out)
            .expect("quality size")
            .len() as i64;
        println!("quality_output={quality_size}");
        assert!(quality_size > 0);

        // A8: thumbnail is taken at 10% of the duration and is a real JPEG.
        let thumb_base = dir.path().join("thumb");
        let thumb = extract_video_thumbnail(&source, &thumb_base, thumbnail_seek_seconds(Some(75.0)))
            .expect("thumbnail");
        let thumb_size = std::fs::metadata(&thumb).expect("thumb size").len();
        println!("thumbnail_bytes={thumb_size}");
        assert!(thumb_size > 0);
        assert_eq!(thumbnail_seek_seconds(Some(75.0)), 7.5);
    }
}
