use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use anyhow::{Context, Result};

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};
use crate::DbState;
use crate::crud::models;
use crate::crud::image_pipeline::{
    self, build_batch, emit_progress, ImageBatchResult, ImageFileResult, ImageJobLimiter,
};
use futures::{stream, StreamExt};
use ort::{Environment, SessionBuilder, Value};
use ndarray::{Array, Axis};

const MODEL_NAME: &str = "bria-rmbg-1.4";

pub fn check_model_downloaded(app: &AppHandle) -> Result<(PathBuf, i64), String> {
    models::check_downloaded(app, MODEL_NAME)
}



#[tauri::command]
pub async fn get_bg_removal_model_status(app: AppHandle) -> Result<models::ModelStatus, String> {
    Ok(models::get_status(&app, MODEL_NAME.to_string()))
}

#[tauri::command]
pub async fn download_bg_removal_model(
    app: AppHandle,
) -> Result<String, String> {
    let app_clone = app.clone();
    let model_path = tauri::async_runtime::spawn_blocking(move || -> Result<PathBuf, String> {
        models::download_model_blocking(&app_clone, MODEL_NAME, "bg-removal-model-download-progress")
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))??;

    Ok(model_path.to_string_lossy().to_string())
}

pub fn create_onnx_session(model_path: &std::path::Path) -> Result<ort::Session> {
    let environment = Arc::new(
        Environment::builder()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create ONNX Runtime environment: {}", e))?,
    );
    SessionBuilder::new(&environment)
        .and_then(|b| b.with_optimization_level(ort::GraphOptimizationLevel::Level3))
        .and_then(|b| b.with_intra_threads(0))
        .and_then(|b| b.with_model_from_file(model_path))
        .map_err(|e| anyhow::anyhow!("Failed to create ONNX session: {}", e))
}

pub fn apply_bg_removal(
    original_image: &DynamicImage,
    session: &ort::Session,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
    let (orig_width, orig_height) = original_image.dimensions();
    let resized_image = original_image.resize_exact(1024, 1024, image::imageops::FilterType::Lanczos3);

    let mut input_array = Array::zeros((1, 3, 1024, 1024));
    for y in 0..1024 {
        for x in 0..1024 {
            let pixel = resized_image.get_pixel(x, y);
            input_array[[0, 0, y as usize, x as usize]] = pixel[0] as f32 / 255.0;
            input_array[[0, 1, y as usize, x as usize]] = pixel[1] as f32 / 255.0;
            input_array[[0, 2, y as usize, x as usize]] = pixel[2] as f32 / 255.0;
        }
    }

    let input_tensor_values = ndarray::CowArray::from(input_array).into_dyn();
    let input_tensor = Value::from_array(session.allocator(), &input_tensor_values)
        .map_err(|e| anyhow::anyhow!("Failed to create input tensor: {}", e))?;

    let outputs = session
        .run(vec![input_tensor])
        .map_err(|e| anyhow::anyhow!("ONNX inference failed: {}", e))?;

    let output_tensor_value = &outputs[0];
    let extracted_tensor: ort::tensor::OrtOwnedTensor<f32, _> = output_tensor_value
        .try_extract()
        .map_err(|e| anyhow::anyhow!("Failed to extract output tensor: {}", e))?;
    let output_view = extracted_tensor.view();
    let mask = output_view.to_owned().remove_axis(Axis(0));

    let mut mask_image: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(1024, 1024);
    for y in 0..1024 {
        for x in 0..1024 {
            let mask_value = (mask[[0, y as usize, x as usize]].clamp(0.0, 1.0) * 255.0) as u8;
            mask_image.put_pixel(x, y, Rgba([mask_value, mask_value, mask_value, 255]));
        }
    }

    let mask_dynamic = image::DynamicImage::ImageRgba8(mask_image);
    let resized_mask = mask_dynamic.resize_exact(orig_width, orig_height, image::imageops::FilterType::Triangle);
    let final_mask = resized_mask.to_rgba8();

    let mut result_img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(orig_width, orig_height);
    for y in 0..orig_height {
        for x in 0..orig_width {
            let orig_pixel = original_image.get_pixel(x, y);
            let mask_pixel = final_mask.get_pixel(x, y);
            let alpha = mask_pixel[0];
            result_img.put_pixel(x, y, Rgba([orig_pixel[0], orig_pixel[1], orig_pixel[2], alpha]));
        }
    }

    Ok(result_img)
}

fn run_bg_removal_inference(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
    model_path: &std::path::Path,
) -> Result<()> {
    let session = create_onnx_session(model_path)?;
    let original_image = image::open(input_path).context("Failed to open image")?;
    let result_img = apply_bg_removal(&original_image, &session)?;

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).with_context(|| format!("Failed to create output directory: {}", parent.display()))?;
        }
    }

    result_img.save(output_path).with_context(|| format!("Failed to save output image to: {}", output_path.display()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn bg_one(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    id: i64,
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
    // Always output PNG for background removal.
    let new_filename = format!("{}_no_bg.png", file_stem);
    let final_filepath = match output_dir {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
        _ => orig_path.with_file_name(&new_filename),
    };
    // `.tmp.png` so the image crate can still detect the format.
    let temp_filename = format!("{}.tmp.png", file_stem);
    let temp_filepath = match output_dir {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&temp_filename),
        _ => orig_path.with_file_name(&temp_filename),
    };


    emit_progress(app, "bg-removal-progress", id, 5, "encoding", "running", "Processing...");

    let (_permit, queue_position) = limiter.acquire().await;
    if queue_position > 0 {
        emit_progress(
            app,
            "bg-removal-progress",
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
        emit_progress(app, "bg-removal-progress", id, 0, "cancelled", "cancelled", "Cancelled");
        return ImageFileResult::cancelled(id, source_size);
    }

    let orig_for_blocking = orig_path.clone();
    let temp_for_blocking = temp_filepath.clone();
    let model_for_blocking = model_path.to_path_buf();
    let removal = tauri::async_runtime::spawn_blocking(move || {
        run_bg_removal_inference(&orig_for_blocking, &temp_for_blocking, &model_for_blocking)
    })
    .await;

    match removal {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Background removal failed: {}", e);
            emit_progress(app, "bg-removal-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Task failed: {}", e);
            emit_progress(app, "bg-removal-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
    }

    if final_filepath.exists() {
        let _ = fs::remove_file(&final_filepath);
    }
    if let Err(e) = fs::rename(&temp_filepath, &final_filepath) {
        let _ = fs::remove_file(&temp_filepath);
        let message = format!("Failed to finalize output: {}", e);
        emit_progress(app, "bg-removal-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    let size = fs::metadata(&final_filepath).map(|m| m.len() as i64).ok();
    let fp = dunce::canonicalize(&final_filepath)
        .unwrap_or_else(|_| final_filepath.clone())
        .to_string_lossy()
        .to_string();

    let insert = sqlx::query(
        "INSERT OR REPLACE INTO bg_removed_images \
         (original_id, filepath, size, model_used, status, completed_at) \
         VALUES (?, ?, ?, ?, 'done', datetime('now'))",
    )
    .bind(id)
    .bind(&fp)
    .bind(size)
    .bind(MODEL_NAME)
    .execute(pool)
    .await;

    if let Err(e) = insert {
        let message = format!("Failed to save result: {}", e);
        emit_progress(app, "bg-removal-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    emit_progress(app, "bg-removal-progress", id, 100, "done", "done", "Done");
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

async fn remove_background_inner(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    ids: Vec<i64>,
) -> Result<ImageBatchResult, String> {
    let (model_path, _model_size) = check_model_downloaded(app)?;

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
            let model_path = model_path.clone();
            async move {
                bg_one(&app, &pool, id, &model_path, &output_dir_setting, limiter).await
            }
        })
        .buffer_unordered(num_cpus::get().max(2))
        .collect()
        .await;

    image_pipeline::clear_tokens(app, &ids_cleanup);
    Ok(build_batch(results))
}

#[tauri::command]
pub async fn remove_background_by_ids(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
) -> Result<usize, String> {
    let result = remove_background_inner(&app, &state.0, ids).await?;
    Ok(result.processed)
}

#[tauri::command]
pub async fn remove_background_by_ids_v2(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
) -> Result<ImageBatchResult, String> {
    remove_background_inner(&app, &state.0, ids).await
}

#[tauri::command]
pub async fn get_all_bg_removed_images(
    state: State<'_, DbState>,
) -> Result<Vec<crate::crud::images::Image>, String> {
    let pool = state.0.clone();
    
    let images = sqlx::query_as::<_, (i64, String, String, String, i64, Option<i64>, Option<i64>, Option<i64>)>(
        r#"
        SELECT 
            i.id, i.filename, b.filepath, b.model_used, b.size,
            i.width, i.height, i.size
        FROM images i
        INNER JOIN bg_removed_images b ON i.id = b.original_id
        ORDER BY i.id DESC
        "#
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let result: Vec<crate::crud::images::Image> = images.into_iter().map(|row| {
        crate::crud::images::Image {
            id: row.0,
            filename: row.1,
            filepath: row.2.clone(),
            mimetype: Some("image/png".to_string()),
            size: Some(row.4),
            width: row.5,
            height: row.6,
            compressed_filepath: None,
            compressed_size: None,
            upscaled_versions: "[]".to_string(),
            bg_removed_filepath: Some(row.2),
            bg_removed_size: Some(row.4),
            converted_images: vec![],
        }
    }).collect();

    Ok(result)
}
