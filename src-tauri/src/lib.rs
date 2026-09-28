mod db;
mod crud;

use db::{get_db_url, init_db, get_setting as db_get_setting, set_setting as db_set_setting};
use tauri_plugin_log::{Target, TargetKind};

use crud::images::{get_all_images, get_all_compressed_images, get_all_converted_images, add_image, import_images_bulk, delete_image, delete_images_by_ids, get_image_metadata, get_image_by_id};
use crud::db_maintenance::{sync_database, check_db_health};
use crud::notifications::{get_all_notifications, add_notification, mark_notification_read, delete_notification, mark_all_notifications_read, clear_all_notifications};
use crud::selections::{get_selections, set_selections, add_selection, remove_selection, clear_selections};
use crud::video_selections::{get_video_selections, set_video_selections, add_video_selection, remove_video_selection, clear_video_selections};
use crud::compression::{
    compress_images_by_ids, compress_images_by_ids_v2, convert_images_by_ids,
    convert_images_by_ids_v2,
};
use crud::image_pipeline::{
    cancel_image_jobs, get_image_job_limit, set_image_job_limit, default_image_job_limit,
    ImageCancelTokens, ImageJobLimiter,
};
use crud::upscaling::{
    upscale_images_by_ids, upscale_images_by_ids_v2, get_all_upscaled_images, get_model_status, 
    download_model, get_upscale_settings, set_upscale_settings
};
use crud::background_removal::{
    remove_background_by_ids, remove_background_by_ids_v2, get_all_bg_removed_images,
    get_bg_removal_model_status, download_bg_removal_model
};
use crud::video_processing::{
    import_videos, get_all_videos, get_video_by_id, delete_videos_by_ids, remove_video_bg,
    get_all_bg_removed_videos, get_all_compressed_videos, get_all_converted_videos,
    check_ffmpeg_status, download_ffmpeg,
    cancel_video_bg_removal, cancel_video_compression, cancel_video_conversion,
    compress_videos_by_ids, compress_videos_by_ids_v2,
    convert_videos_by_ids, convert_videos_by_ids_v2, get_compression_presets,
    get_encoder_capabilities, cleanup_stale_temp_files,
    generate_video_thumbnails, CancelTokens, VideoJobLimiter, default_video_job_limit
};
use crud::lifecycle::{
    delete_items, delete_outputs, delete_output, get_compressed_variants,
};
use crud::health::{orphan_scan, orphan_cleanup};
use crud::db_viewer::{db_overview, db_table_rows};
use crud::app_log::{read_app_log, start_session};
use crud::filters::{get_filters, update_filters, reset_filters};
use sqlx::SqlitePool;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager, State};

pub struct DbState(pub SqlitePool);

static DB_INITIALIZED: OnceLock<()> = OnceLock::new();

#[tauri::command]
async fn init_database(app: AppHandle) -> Result<String, String> {
    let (pool, path) = init_db(&app).await.map_err(|e| e.to_string())?;
    app.manage(DbState(pool));
    Ok(format!("Database initialized at: {:?}", path))
}

#[tauri::command]
async fn db_exists(app: AppHandle) -> Result<bool, String> {
    let path = db::get_db_path(&app).map_err(|e| e.to_string())?;
    Ok(path.exists())
}

#[tauri::command]
fn get_db_path_cmd(app: AppHandle) -> Result<String, String> {
    get_db_url(&app).map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_setting(key: String, state: State<'_, DbState>) -> Result<Option<String>, String> {
    db_get_setting(&state.0, &key)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_setting(key: String, value: String, state: State<'_, DbState>) -> Result<(), String> {
    db_set_setting(&state.0, &key, &value)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_video_job_limit(limiter: State<'_, VideoJobLimiter>) -> Result<usize, String> {
    Ok(limiter.limit())
}

#[tauri::command]
async fn set_video_job_limit(
    limit: usize,
    state: State<'_, DbState>,
    limiter: State<'_, VideoJobLimiter>,
) -> Result<(), String> {
    db_set_setting(&state.0, "max_concurrent_video_jobs", &limit.to_string())
        .await
        .map_err(|e| e.to_string())?;
    limiter.set_limit(limit);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: Some("app".to_string()) }),
                    Target::new(TargetKind::Webview),
                ])
                .level(log::LevelFilter::Info)
                .level_for("tao", log::LevelFilter::Error)
                .level_for("ort", log::LevelFilter::Warn)
                .level_for("tracing", log::LevelFilter::Warn)
                .max_file_size(5_000_000)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            if DB_INITIALIZED.get().is_some() {
                return Ok(());
            }

            // Mark the start of this run so the log panel only shows this session.
            log::info!("[session] {}", start_session());

            // Ensure ffmpeg cache dir exists and register in OnceLock
            if let Ok(app_data) = app.path().app_data_dir() {
                let ffmpeg_dir = app_data.join("ffmpeg");
                let _ = std::fs::create_dir_all(&ffmpeg_dir);
                let _ = crate::crud::video_processing::FFMPEG_DIR.set(ffmpeg_dir);
            }

            #[cfg(desktop)]
            let _ = app.handle().plugin(tauri_plugin_updater::Builder::new().build());

            app.handle().manage(CancelTokens(std::sync::Mutex::new(std::collections::HashMap::new())));
            app.handle().manage(ImageCancelTokens(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )));

            let handle = app.handle().clone();
            let (pool, path) = tauri::async_runtime::block_on(init_db(&handle))
                .map_err(|e| {
                    log::error!("[DB] Failed to initialize: {}", e);
                    e
                })?;

            // Remove leftovers from a previous run that did not finish cleanly.
            let _ = tauri::async_runtime::block_on(cleanup_stale_temp_files(&pool));

            // Process-wide video encode ceiling (0 / unset => auto).
            let job_limit = tauri::async_runtime::block_on(db_get_setting(
                &pool,
                "max_concurrent_video_jobs",
            ))
            .ok()
            .flatten()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or_else(default_video_job_limit);
            handle.manage(VideoJobLimiter::new(job_limit));
            log::info!("[jobs] Video concurrency limit: {}", job_limit);

            let image_job_limit = tauri::async_runtime::block_on(db_get_setting(
                &pool,
                "max_concurrent_image_jobs",
            ))
            .ok()
            .flatten()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or_else(default_image_job_limit);
            handle.manage(ImageJobLimiter::new(image_job_limit));
            log::info!("[jobs] Image concurrency limit: {}", image_job_limit);

            handle.manage(DbState(pool));
            let _ = DB_INITIALIZED.set(());
            log::info!("[DB] Initialized at: {:?}", path);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            init_database,
            db_exists,
            get_db_path_cmd,
            get_setting,
            set_setting,
            get_video_job_limit,
            set_video_job_limit,
            get_image_job_limit,
            set_image_job_limit,
            cancel_image_jobs,
            delete_items,
            delete_outputs,
            delete_output,
            get_compressed_variants,
            db_overview,
            db_table_rows,
            read_app_log,
            orphan_scan,
            orphan_cleanup,
            get_all_images,
            get_all_compressed_images,
            add_image,
            import_images_bulk,
            delete_image,
            delete_images_by_ids,
            sync_database,
            check_db_health,
            get_image_metadata,
            get_all_notifications,
            add_notification,
            mark_notification_read,
            delete_notification,
            mark_all_notifications_read,
            clear_all_notifications,
            get_selections,
            set_selections,
            add_selection,
            remove_selection,
            clear_selections,
            get_video_selections,
            set_video_selections,
            add_video_selection,
            remove_video_selection,
            clear_video_selections,
            compress_images_by_ids,
            compress_images_by_ids_v2,
            convert_images_by_ids,
            convert_images_by_ids_v2,
            get_all_converted_images,
            get_image_by_id,
            upscale_images_by_ids,
            upscale_images_by_ids_v2,
            get_all_upscaled_images,
            get_model_status,
            download_model,
            get_upscale_settings,
            set_upscale_settings,
            remove_background_by_ids,
            remove_background_by_ids_v2,
            get_all_bg_removed_images,
            get_bg_removal_model_status,
            download_bg_removal_model,
            import_videos,
            get_all_videos,
            get_video_by_id,
            delete_videos_by_ids,
            remove_video_bg,
            get_all_bg_removed_videos,
            get_all_compressed_videos,
            check_ffmpeg_status,
            download_ffmpeg,
            cancel_video_bg_removal,
            compress_videos_by_ids,
            compress_videos_by_ids_v2,
            convert_videos_by_ids,
            convert_videos_by_ids_v2,
            cancel_video_compression,
            cancel_video_conversion,
            get_encoder_capabilities,
            get_all_converted_videos,
            get_compression_presets,
            generate_video_thumbnails,
            get_filters,
            update_filters,
            reset_filters
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                crate::crud::video_processing::kill_all_ffmpeg();
            }
        });
}
