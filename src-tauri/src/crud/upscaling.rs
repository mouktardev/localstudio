use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use anyhow::{Context, Result};
use serde::Serialize;
use image::GenericImageView;
use crate::DbState;
use crate::crud::models;
use crate::crud::image_pipeline::{
    self, build_batch, emit_progress, ImageBatchResult, ImageFileResult, ImageJobLimiter,
};
use futures::{stream, StreamExt};

#[derive(Clone, Serialize)]
pub struct UpscaleSettings {
    pub model: String,
    pub cache_dir: String,
}

fn check_model_downloaded(app: &AppHandle, model_name: &str) -> Result<(PathBuf, i64), String> {
    models::check_downloaded(app, model_name)
}

#[tauri::command]
pub async fn get_upscale_settings(
    app: AppHandle,
    state: State<'_, DbState>,
) -> Result<UpscaleSettings, String> {
    let pool = state.0.clone();
    
    let model: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'upscale_model'")
        .fetch_optional(&pool)
        .await
        .unwrap_or(Some("realesrgan-x4".to_string()));

    let cache_dir = models::get_cache_dir(&app)
        .to_string_lossy()
        .to_string();

    Ok(UpscaleSettings {
        model: model.unwrap_or_else(|| "realesrgan-x4".to_string()),
        cache_dir,
    })
}

#[tauri::command]
pub async fn set_upscale_settings(
    state: State<'_, DbState>,
    model: String,
) -> Result<(), String> {
    let pool = state.0.clone();
    
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES ('upscale_model', ?)")
        .bind(&model)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn get_model_status(
    app: AppHandle,
    model: String,
) -> Result<models::ModelStatus, String> {
    Ok(models::get_status(&app, model))
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    model: String,
) -> Result<String, String> {
    let app_clone = app.clone();
    let model_path = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
        models::download_model_blocking(&app_clone, &model, "model-download-progress")
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))??;

    Ok(model_path.to_string_lossy().to_string())
}

fn run_upscaling_inference(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    model_path: &std::path::Path,
) -> Result<()> {
    let img = image::open(input_path).context("Failed to open image")?;
    let (width, height) = img.dimensions();

    let scale = if model_path.to_string_lossy().contains("x2") {
        2
    } else {
        4
    };
    let new_width = width * scale;
    let new_height = height * scale;

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).context("Failed to create output directory")?;
        }
    }

    let resized = img.resize_exact(new_width, new_height, image::imageops::FilterType::Lanczos3);

    // Get extension from INPUT file (not .tmp output)
    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "jpg".to_string());

    let mut out_file = fs::File::create(output_path).context("Failed to create output file")?;

    if ext == "jpg" || ext == "jpeg" {
        let mut encoder = jpeg_encoder::Encoder::new(&mut out_file, 95);
        encoder.set_optimized_huffman_tables(true);
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        let img_rgb = resized.to_rgb8();
        encoder
            .encode(
                img_rgb.as_raw(),
                img_rgb.width() as u16,
                img_rgb.height() as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .context("JPEG Encode failed")?;
    } else if ext == "png" {
        let rgba = resized.to_rgba8();
        let mut encoder = png::Encoder::new(&mut out_file, rgba.width(), rgba.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().context("Failed to write PNG header")?;
        writer
            .write_image_data(rgba.as_raw())
            .context("Failed to write PNG data")?;
    } else {
        resized.save(output_path).context("Failed to save image")?;
    }

    out_file.sync_all().context("Failed to sync file to disk")?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upscale_one(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    id: i64,
    scale: u32,
    model: &str,
    model_path: &std::path::Path,
    output_dir: &Option<String>,
    limiter: ImageJobLimiter,
) -> ImageFileResult {
    let record: Option<(String, Option<i64>)> =
        sqlx::query_as("SELECT filepath, size FROM images WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

    let Some((filepath, source_size)) = record else {
        return ImageFileResult::failed(id, "Image not found".to_string());
    };

    let orig_path = PathBuf::from(&filepath);
    if !orig_path.exists() {
        return ImageFileResult::failed(id, "Source file is missing".to_string());
    }

    let file_stem = orig_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = orig_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let scale_str = if scale == 2 { "x2" } else { "x4" };
    let new_filename = format!("{}_upscaled_{}.{}", file_stem, scale_str, ext);

    let final_filepath = match output_dir {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
        _ => orig_path.with_file_name(&new_filename),
    };
    let temp_filepath = final_filepath.with_file_name(format!("{}.tmp", new_filename));


    emit_progress(app, "upscale-progress", id, 5, "encoding", "running", "Upscaling...");

    let (_permit, queue_position) = limiter.acquire().await;
    if queue_position > 0 {
        emit_progress(
            app,
            "upscale-progress",
            id,
            0,
            "queued",
            "running",
            &format!("Queued (position {})", queue_position + 1),
        );
    }

    let token = image_pipeline::token_for(app, id);
    if image_pipeline::is_cancelled(&token) {
        let _ = fs::remove_file(&temp_filepath);
        emit_progress(app, "upscale-progress", id, 0, "cancelled", "cancelled", "Cancelled");
        return ImageFileResult::cancelled(id, source_size);
    }

    let orig_for_blocking = orig_path.clone();
    let temp_for_blocking = temp_filepath.clone();
    let model_for_blocking = model_path.to_path_buf();
    let upscale = tauri::async_runtime::spawn_blocking(move || {
        run_upscaling_inference(&orig_for_blocking, &temp_for_blocking, &model_for_blocking)
    })
    .await;

    match upscale {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Upscaling failed: {}", e);
            emit_progress(app, "upscale-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Task failed: {}", e);
            emit_progress(app, "upscale-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
    }

    if final_filepath.exists() {
        let _ = fs::remove_file(&final_filepath);
    }
    if let Err(e) = fs::rename(&temp_filepath, &final_filepath) {
        let _ = fs::remove_file(&temp_filepath);
        let message = format!("Failed to finalize output: {}", e);
        emit_progress(app, "upscale-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    let size = fs::metadata(&final_filepath).map(|m| m.len() as i64).ok();
    let fp = dunce::canonicalize(&final_filepath)
        .unwrap_or_else(|_| final_filepath.clone())
        .to_string_lossy()
        .to_string();

    let insert = sqlx::query(
        "INSERT OR REPLACE INTO upscaled_images \
         (original_id, filepath, scale_factor, model_used, size, status, completed_at) \
         VALUES (?, ?, ?, ?, ?, 'done', datetime('now'))",
    )
    .bind(id)
    .bind(&fp)
    .bind(scale as i64)
    .bind(model)
    .bind(size)
    .execute(pool)
    .await;

    if let Err(e) = insert {
        let message = format!("Failed to save result: {}", e);
        emit_progress(app, "upscale-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    emit_progress(app, "upscale-progress", id, 100, "done", "done", "Done");
    let _ = app.emit("images-updated", ());

    ImageFileResult {
        id,
        status: "done".to_string(),
        message: None,
        output_path: Some(fp),
        size,
        source_size,
    }
}

async fn upscale_images_inner(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    ids: Vec<i64>,
    scale: u32,
    model: String,
) -> Result<ImageBatchResult, String> {
    let (model_path, _model_size) = check_model_downloaded(app, &model)?;

    let output_dir_setting: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let limiter = app.state::<ImageJobLimiter>().inner().clone();
    let ids_cleanup = ids.clone();
    image_pipeline::register_tokens(app, &ids);

    let results: Vec<ImageFileResult> = stream::iter(ids)
        .map(|id| {
            let app = app.clone();
            let pool = pool.clone();
            let output_dir_setting = output_dir_setting.clone();
            let limiter = limiter.clone();
            let model = model.clone();
            let model_path = model_path.clone();
            async move {
                upscale_one(
                    &app,
                    &pool,
                    id,
                    scale,
                    &model,
                    &model_path,
                    &output_dir_setting,
                    limiter,
                )
                .await
            }
        })
        .buffer_unordered(num_cpus::get().max(2))
        .collect()
        .await;

    image_pipeline::clear_tokens(app, &ids_cleanup);
    Ok(build_batch(results))
}

#[tauri::command]
pub async fn upscale_images_by_ids(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    scale: u32,
    model: String,
) -> Result<usize, String> {
    let result = upscale_images_inner(&app, &state.0, ids, scale, model).await?;
    Ok(result.processed)
}

#[tauri::command]
pub async fn upscale_images_by_ids_v2(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    scale: u32,
    model: String,
) -> Result<ImageBatchResult, String> {
    upscale_images_inner(&app, &state.0, ids, scale, model).await
}

#[tauri::command]
pub async fn get_all_upscaled_images(
    state: State<'_, DbState>,
) -> Result<Vec<crate::crud::images::Image>, String> {
    let pool = state.0.clone();
    
    let images = sqlx::query_as::<_, (i64, String, String, i64, String, i64, Option<i64>, Option<i64>, Option<i64>)>(
        r#"
        SELECT 
            i.id, i.filename, u.filepath, u.scale_factor, u.model_used, u.size,
            i.width, i.height, i.size
        FROM images i
        INNER JOIN upscaled_images u ON i.id = u.original_id
        ORDER BY i.id DESC, u.scale_factor ASC
        "#
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let result: Vec<crate::crud::images::Image> = images.into_iter().map(|row| {
        // Create a JSON array with a single upscaled version for each row
        let upscaled_json = format!(
            r#"[{{"scale":{},"filepath":"{}","size":{},"model":"{}"}}]"#,
            row.3,
            row.2.replace('\\', "\\\\").replace('"', "\\\""),
            row.5,
            row.4.replace('\\', "\\\\").replace('"', "\\\"")
        );
        
        crate::crud::images::Image {
            id: row.0,
            filename: row.1,
            filepath: row.2.clone(),
            mimetype: None,
            size: Some(row.5),
            width: row.6,
            height: row.7,
            compressed_filepath: None,
            compressed_size: None,
            upscaled_versions: upscaled_json,
            bg_removed_filepath: None,
            bg_removed_size: None,
            converted_images: vec![],
        }
    }).collect();

    Ok(result)
}
