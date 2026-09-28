use tauri::{AppHandle, State};

use crate::crud::health::{count_orphaned_rows, run_orphan_cleanup};
use crate::DbState;

/// Count DB rows whose file is gone (the real integrity signal).
#[tauri::command]
pub async fn check_db_health(_app: AppHandle, state: State<'_, DbState>) -> Result<i64, String> {
    count_orphaned_rows(&state.0).await
}

/// Remove orphaned rows plus app-generated leftover files (pattern-scoped).
#[tauri::command]
pub async fn sync_database(app: AppHandle, state: State<'_, DbState>) -> Result<i64, String> {
    let scan = run_orphan_cleanup(&state.0, &app, false, true, true, true, None).await?;
    Ok((scan.deleted_rows + scan.deleted_files) as i64)
}
