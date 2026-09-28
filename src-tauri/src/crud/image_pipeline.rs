//! Shared plumbing for the image operations (compress / convert / upscale /
//! background-removal): batch results, staged progress, cancellation, the
//! concurrency limiter and job-history rows. Mirrors the video pipeline.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

use super::video_processing::VideoJobLimiter;
use crate::DbState;

/// Separate from `VideoJobLimiter` so both can be managed as distinct state.
#[derive(Clone)]
pub struct ImageJobLimiter(pub VideoJobLimiter);

impl ImageJobLimiter {
    pub fn new(limit: usize) -> Self {
        Self(VideoJobLimiter::new(limit))
    }
    pub fn limit(&self) -> usize {
        self.0.limit()
    }
    pub fn set_limit(&self, limit: usize) {
        self.0.set_limit(limit)
    }
    pub async fn acquire(&self) -> (tokio::sync::OwnedSemaphorePermit, usize) {
        self.0.acquire().await
    }
}

/// Cancellation tokens for image jobs, kept separate from video ids.
pub struct ImageCancelTokens(pub Mutex<HashMap<i64, Arc<AtomicBool>>>);

pub fn default_image_job_limit() -> usize {
    (num_cpus::get() / 2).max(1)
}

#[derive(Clone, Serialize)]
pub struct ImageJobProgress {
    pub id: i64,
    pub progress: u8,
    pub stage: String,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct ImageFileResult {
    pub id: i64,
    pub status: String,
    pub message: Option<String>,
    pub output_path: Option<String>,
    pub size: Option<i64>,
    pub source_size: Option<i64>,
}

#[derive(Clone, Serialize)]
pub struct ImageFileError {
    pub id: i64,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct ImageBatchResult {
    pub processed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub errors: Vec<ImageFileError>,
    pub results: Vec<ImageFileResult>,
}

impl ImageFileResult {
    pub fn failed(id: i64, message: String) -> Self {
        Self {
            id,
            status: "failed".to_string(),
            message: Some(message),
            output_path: None,
            size: None,
            source_size: None,
        }
    }

    pub fn cancelled(id: i64, source_size: Option<i64>) -> Self {
        Self {
            id,
            status: "cancelled".to_string(),
            message: Some("Cancelled".to_string()),
            output_path: None,
            size: None,
            source_size,
        }
    }
}

pub fn build_batch(results: Vec<ImageFileResult>) -> ImageBatchResult {
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
            r.message.as_ref().map(|message| ImageFileError {
                id: r.id,
                message: message.clone(),
            })
        })
        .collect();
    ImageBatchResult {
        processed,
        failed,
        cancelled,
        errors,
        results,
    }
}

pub fn emit_progress(
    app: &AppHandle,
    event: &str,
    id: i64,
    progress: u8,
    stage: &str,
    status: &str,
    message: &str,
) {
    let _ = app.emit(
        event,
        ImageJobProgress {
            id,
            progress,
            stage: stage.to_string(),
            status: status.to_string(),
            message: message.to_string(),
        },
    );
}

pub fn register_tokens(app: &AppHandle, ids: &[i64]) {
    let state = app.state::<ImageCancelTokens>();
    let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
    for &id in ids {
        map.insert(id, Arc::new(AtomicBool::new(false)));
    }
}

pub fn token_for(app: &AppHandle, id: i64) -> Arc<AtomicBool> {
    let state = app.state::<ImageCancelTokens>();
    let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
    map.get(&id)
        .cloned()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)))
}

pub fn clear_tokens(app: &AppHandle, ids: &[i64]) {
    let state = app.state::<ImageCancelTokens>();
    let mut map = state.0.lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        map.remove(id);
    }
}

pub fn set_cancel_tokens(state: &ImageCancelTokens, ids: &[i64]) {
    let map = state.0.lock().unwrap_or_else(|e| e.into_inner());
    for id in ids {
        if let Some(token) = map.get(id) {
            token.store(true, Ordering::Relaxed);
        }
    }
}

pub fn is_cancelled(token: &AtomicBool) -> bool {
    token.load(Ordering::Relaxed)
}

/// Should the freshly-encoded temp file replace the original? Only when we
/// know both sizes and the new file is strictly smaller.
pub fn output_is_smaller(temp: &Path, source_size: Option<i64>) -> bool {
    match (fs::metadata(temp).ok().map(|m| m.len() as i64), source_size) {
        (Some(out), Some(source)) => out < source,
        _ => true,
    }
}

#[tauri::command]
pub async fn cancel_image_jobs(
    state: State<'_, ImageCancelTokens>,
    ids: Vec<i64>,
) -> Result<(), String> {
    set_cancel_tokens(&state, &ids);
    Ok(())
}

#[tauri::command]
pub async fn get_image_job_limit(limiter: State<'_, ImageJobLimiter>) -> Result<usize, String> {
    Ok(limiter.limit())
}

#[tauri::command]
pub async fn set_image_job_limit(
    limit: usize,
    state: State<'_, DbState>,
    limiter: State<'_, ImageJobLimiter>,
) -> Result<(), String> {
    crate::db::set_setting(&state.0, "max_concurrent_image_jobs", &limit.to_string())
        .await
        .map_err(|e| e.to_string())?;
    limiter.set_limit(limit);
    Ok(())
}
