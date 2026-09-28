use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use crate::DbState;
use futures::{stream, StreamExt};

use super::image_pipeline::{
    self, build_batch, emit_progress, ImageBatchResult, ImageFileResult, ImageJobLimiter,
};

/// Encode the compressed copy of an image to a temp file.
fn encode_compressed_image(orig: &Path, temp: &Path, ext_lower: &str, quality: u8) -> Result<()> {
    let img_data = image::open(orig).context("Failed to open image")?;
    let mut out_file = fs::File::create(temp)?;

    if ext_lower == "jpg" || ext_lower == "jpeg" {
        let mut encoder = jpeg_encoder::Encoder::new(&mut out_file, quality);
        encoder.set_optimized_huffman_tables(true);
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        let img_rgb = img_data.to_rgb8();
        encoder
            .encode(
                img_rgb.as_raw(),
                img_rgb.width() as u16,
                img_rgb.height() as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .context("JPEG encode failed")?;
    } else if ext_lower == "png" {
        // Extreme lossy compression for PNG using imagequant (TinyPNG style).
        let rgba = img_data.to_rgba8();
        let mut liq = imagequant::new();
        liq.set_quality(0, quality).context("Failed to set PNG quality")?;
        liq.set_speed(4).context("Failed to set PNG speed")?;
        let mut img_quant = liq
            .new_image_borrowed(
                rgb::FromSlice::as_rgba(rgba.as_raw().as_slice()),
                rgba.width() as usize,
                rgba.height() as usize,
                0.0,
            )
            .context("Failed to create imagequant image")?;
        let mut res = liq.quantize(&mut img_quant).context("PNG quantize failed")?;
        let (palette, pixels) = res.remapped(&mut img_quant).context("PNG remap failed")?;

        let mut palette_bytes = Vec::with_capacity(palette.len() * 3);
        let mut trns_bytes = Vec::with_capacity(palette.len());
        for color in palette {
            palette_bytes.push(color.r);
            palette_bytes.push(color.g);
            palette_bytes.push(color.b);
            trns_bytes.push(color.a);
        }

        let mut encoder = png::Encoder::new(&mut out_file, rgba.width(), rgba.height());
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(palette_bytes);
        encoder.set_trns(trns_bytes);
        let mut writer = encoder.write_header().context("Failed to write PNG header")?;
        writer
            .write_image_data(&pixels)
            .context("Failed to write paletted PNG")?;
    } else {
        img_data.save(temp).context("Image save failed")?;
    }

    out_file.sync_all().context("Failed to sync file")?;
    Ok(())
}

fn encode_converted_image(orig: &Path, temp: &Path, format: &str) -> Result<()> {
    let img = image::open(orig).context("Failed to open image")?;
    let output_format = match format {
        "jpg" | "jpeg" => image::ImageFormat::Jpeg,
        "png" => image::ImageFormat::Png,
        "webp" => image::ImageFormat::WebP,
        _ => image::ImageFormat::Png,
    };
    if format == "jpg" || format == "jpeg" {
        img.to_rgb8()
            .save_with_format(temp, output_format)
            .context("Failed to save converted image")?;
    } else {
        img.save_with_format(temp, output_format)
            .context("Failed to save converted image")?;
    }
    Ok(())
}

fn resolve_filepaths(
    orig_path: &Path,
    suffix: &str,
    output_dir: &Option<String>,
) -> (PathBuf, PathBuf) {
    let stem = orig_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = orig_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let new_filename = format!("{}_{}.{}", stem, suffix, ext);
    let final_filepath = match output_dir {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
        _ => orig_path.with_file_name(&new_filename),
    };
    let temp_filepath = final_filepath.with_file_name(format!("{}.tmp", new_filename));
    (final_filepath, temp_filepath)
}

async fn compress_one(
    app: &AppHandle,
    pool: &SqlitePool,
    id: i64,
    quality: u8,
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

    let ext_lower = orig_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let (final_filepath, temp_filepath) = resolve_filepaths(&orig_path, "compressed", output_dir);


    emit_progress(app, "compression-progress", id, 5, "encoding", "running", "Reading...");

    let (_permit, queue_position) = limiter.acquire().await;
    if queue_position > 0 {
        emit_progress(
            app,
            "compression-progress",
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
        emit_progress(app, "compression-progress", id, 0, "cancelled", "cancelled", "Cancelled");
        return ImageFileResult::cancelled(id, source_size);
    }

    let orig_for_blocking = orig_path.clone();
    let temp_for_blocking = temp_filepath.clone();
    let ext_for_blocking = ext_lower.clone();
    let encode = tauri::async_runtime::spawn_blocking(move || {
        encode_compressed_image(&orig_for_blocking, &temp_for_blocking, &ext_for_blocking, quality)
    })
    .await;

    match encode {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Compression failed: {}", e);
            emit_progress(app, "compression-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Task failed: {}", e);
            emit_progress(app, "compression-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
    }

    if image_pipeline::is_cancelled(&token) {
        let _ = fs::remove_file(&temp_filepath);
        emit_progress(app, "compression-progress", id, 0, "cancelled", "cancelled", "Cancelled");
        return ImageFileResult::cancelled(id, source_size);
    }

    let size = fs::metadata(&temp_filepath).map(|m| m.len() as i64).ok();

    // Never write a compressed image that is not smaller than the source.
    if !image_pipeline::output_is_smaller(&temp_filepath, source_size) {
        let _ = fs::remove_file(&temp_filepath);
        emit_progress(app, "compression-progress", id, 100, "done", "done", "Already optimal");
        return ImageFileResult {
            id,
            status: "kept".to_string(),
            message: Some("Already optimal — kept the original".to_string()),
            output_path: None,
            size,
            source_size,
        };
    }

    if final_filepath.exists() {
        let _ = fs::remove_file(&final_filepath);
    }
    if let Err(e) = fs::rename(&temp_filepath, &final_filepath) {
        let _ = fs::remove_file(&temp_filepath);
        let message = format!("Failed to finalize output: {}", e);
        emit_progress(app, "compression-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    let fp = dunce::canonicalize(&final_filepath)
        .unwrap_or_else(|_| final_filepath.clone())
        .to_string_lossy()
        .to_string();

    let insert = sqlx::query(
        "INSERT INTO compressed_images \
         (original_id, filepath, size, variant_label, is_primary, status, created_at) \
         VALUES (?, ?, ?, ?, 1, 'done', datetime('now'))",
    )
    .bind(id)
    .bind(&fp)
    .bind(size)
    .bind(format!("q{}", quality))
    .execute(pool)
    .await;

    if let Err(e) = insert {
        let message = format!("Failed to save result: {}", e);
        emit_progress(app, "compression-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    emit_progress(app, "compression-progress", id, 100, "done", "done", "Done");
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

async fn convert_one(
    app: &AppHandle,
    pool: &SqlitePool,
    id: i64,
    format: &str,
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

    let stem = orig_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let new_filename = format!("{}_{}.{}", stem, format, format);
    let final_filepath = match output_dir {
        Some(dir) if PathBuf::from(dir).exists() => PathBuf::from(dir).join(&new_filename),
        _ => orig_path.with_file_name(&new_filename),
    };
    let temp_filepath = final_filepath.with_file_name(format!("{}.tmp", new_filename));


    emit_progress(
        app,
        "image-conversion-progress",
        id,
        5,
        "encoding",
        "running",
        &format!("Converting to {}...", format.to_uppercase()),
    );

    let (_permit, queue_position) = limiter.acquire().await;
    if queue_position > 0 {
        emit_progress(
            app,
            "image-conversion-progress",
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
        emit_progress(app, "image-conversion-progress", id, 0, "cancelled", "cancelled", "Cancelled");
        return ImageFileResult::cancelled(id, source_size);
    }

    let orig_for_blocking = orig_path.clone();
    let temp_for_blocking = temp_filepath.clone();
    let format_owned = format.to_string();
    let encode = tauri::async_runtime::spawn_blocking(move || {
        encode_converted_image(&orig_for_blocking, &temp_for_blocking, &format_owned)
    })
    .await;

    match encode {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Conversion failed: {}", e);
            emit_progress(app, "image-conversion-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_filepath);
            let message = format!("Task failed: {}", e);
            emit_progress(app, "image-conversion-progress", id, 0, "error", "failed", "Failed");
            return ImageFileResult::failed(id, message);
        }
    }

    if final_filepath.exists() {
        let _ = fs::remove_file(&final_filepath);
    }
    if let Err(e) = fs::rename(&temp_filepath, &final_filepath) {
        let _ = fs::remove_file(&temp_filepath);
        let message = format!("Failed to finalize output: {}", e);
        emit_progress(app, "image-conversion-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    let size = fs::metadata(&final_filepath).map(|m| m.len() as i64).ok();
    let fp = dunce::canonicalize(&final_filepath)
        .unwrap_or_else(|_| final_filepath.clone())
        .to_string_lossy()
        .to_string();

    let insert = sqlx::query(
        "INSERT OR REPLACE INTO converted_images \
         (original_id, filepath, format, size, codec, status, source_size, completed_at, created_at) \
         VALUES (?, ?, ?, ?, ?, 'done', ?, datetime('now'), datetime('now'))",
    )
    .bind(id)
    .bind(&fp)
    .bind(format)
    .bind(size)
    .bind(format)
    .bind(source_size)
    .execute(pool)
    .await;

    if let Err(e) = insert {
        let message = format!("Failed to save result: {}", e);
        emit_progress(app, "image-conversion-progress", id, 0, "error", "failed", "Failed");
        return ImageFileResult::failed(id, message);
    }

    emit_progress(app, "image-conversion-progress", id, 100, "done", "done", "Done");
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

async fn compress_images_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    ids: Vec<i64>,
    quality: u8,
) -> Result<ImageBatchResult, String> {
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
            async move { compress_one(&app, &pool, id, quality, &output_dir_setting, limiter).await }
        })
        .buffer_unordered(num_cpus::get().max(2))
        .collect()
        .await;

    image_pipeline::clear_tokens(app, &ids_cleanup);
    Ok(build_batch(results))
}

async fn convert_images_inner(
    app: &AppHandle,
    pool: &SqlitePool,
    ids: Vec<i64>,
    format: String,
) -> Result<ImageBatchResult, String> {
    let output_dir_setting: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
            .fetch_optional(pool)
            .await
            .unwrap_or(None);

    let limiter = app.state::<ImageJobLimiter>().inner().clone();
    let ids_cleanup = ids.clone();
    let format = format.to_lowercase();
    image_pipeline::register_tokens(app, &ids);

    let results: Vec<ImageFileResult> = stream::iter(ids)
        .map(|id| {
            let app = app.clone();
            let pool = pool.clone();
            let output_dir_setting = output_dir_setting.clone();
            let limiter = limiter.clone();
            let format = format.clone();
            async move {
                convert_one(&app, &pool, id, &format, &output_dir_setting, limiter).await
            }
        })
        .buffer_unordered(num_cpus::get().max(2))
        .collect()
        .await;

    image_pipeline::clear_tokens(app, &ids_cleanup);
    Ok(build_batch(results))
}

#[tauri::command]
pub async fn compress_images_by_ids(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    quality: u8,
) -> Result<usize, String> {
    let result = compress_images_inner(&app, &state.0, ids, quality).await?;
    Ok(result.processed)
}

#[tauri::command]
pub async fn compress_images_by_ids_v2(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    quality: u8,
) -> Result<ImageBatchResult, String> {
    compress_images_inner(&app, &state.0, ids, quality).await
}

#[tauri::command]
pub async fn convert_images_by_ids(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    format: String,
) -> Result<usize, String> {
    let result = convert_images_inner(&app, &state.0, ids, format).await?;
    Ok(result.processed)
}

#[tauri::command]
pub async fn convert_images_by_ids_v2(
    app: AppHandle,
    state: State<'_, DbState>,
    ids: Vec<i64>,
    format: String,
) -> Result<ImageBatchResult, String> {
    convert_images_inner(&app, &state.0, ids, format).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_output_and_temp_paths() {
        let orig = PathBuf::from("C:/pics/clip.png");
        let (final_path, temp) = resolve_filepaths(&orig, "compressed", &None);
        assert!(final_path
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("clip_compressed.png"));
        assert!(temp
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with("clip_compressed.png.tmp"));
    }

    #[test]
    fn encodes_and_detects_smaller_output() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("a.png");
        let mut img = image::RgbaImage::new(64, 64);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x * 4) as u8, (y * 4) as u8, 128, 255]);
        }
        img.save(&src).expect("save source");

        let out = dir.path().join("out.png");
        encode_compressed_image(&src, &out, "png", 30).expect("encode");
        let size = fs::metadata(&out).expect("out size").len() as i64;
        assert!(size > 0);
        assert!(image::open(&out).is_ok(), "output must be a valid image");

        assert!(image_pipeline::output_is_smaller(&out, Some(size + 1)));
        assert!(!image_pipeline::output_is_smaller(&out, Some(1)));
    }

    #[test]
    fn converts_to_jpeg() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("a.png");
        image::RgbaImage::new(16, 16).save(&src).expect("save source");
        let out = dir.path().join("a.jpg");
        encode_converted_image(&src, &out, "jpg").expect("convert");
        assert!(image::open(&out).is_ok());
    }
}
