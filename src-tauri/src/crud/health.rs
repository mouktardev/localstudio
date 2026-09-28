use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

use crate::DbState;

/// Every table that stores a file path, tagged with its own primary key so an
/// orphaned row can be deleted directly. One query returns them all.
const ROW_PATH_UNION: &str = "\
    SELECT 'videos' AS t, id, filepath FROM videos \
    UNION ALL SELECT 'images', id, filepath FROM images \
    UNION ALL SELECT 'compressed_videos', id, filepath FROM compressed_videos \
    UNION ALL SELECT 'compressed_images', id, filepath FROM compressed_images \
    UNION ALL SELECT 'converted_videos', id, filepath FROM converted_videos \
    UNION ALL SELECT 'converted_images', id, filepath FROM converted_images \
    UNION ALL SELECT 'upscaled_images', id, filepath FROM upscaled_images \
    UNION ALL SELECT 'bg_removed_images', id, filepath FROM bg_removed_images \
    UNION ALL SELECT 'bg_removed_videos', id, filepath FROM bg_removed_videos";

const REFERENCED_UNION: &str = "\
    SELECT filepath FROM videos \
    UNION ALL SELECT thumbnail_path FROM videos \
    UNION ALL SELECT filepath FROM images \
    UNION ALL SELECT filepath FROM compressed_videos \
    UNION ALL SELECT filepath FROM compressed_images \
    UNION ALL SELECT filepath FROM converted_videos \
    UNION ALL SELECT filepath FROM converted_images \
    UNION ALL SELECT filepath FROM upscaled_images \
    UNION ALL SELECT filepath FROM bg_removed_images \
    UNION ALL SELECT filepath FROM bg_removed_videos";

#[derive(Debug, Clone, Serialize)]
pub struct OrphanRow {
    pub table: String,
    pub id: i64,
    pub filepath: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrphanFile {
    pub path: String,
    pub bytes: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OrphanScan {
    pub orphaned_rows: Vec<OrphanRow>,
    pub orphaned_files: Vec<OrphanFile>,
    pub deleted_rows: u64,
    pub deleted_files: u64,
    pub bytes_reclaimed: i64,
    pub statements: usize,
    pub dry_run: bool,
    pub skipped_files: Vec<String>,
}

const MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "avi", "mkv", "webm", "flv", "wmv", "m4v", "3gp", "png", "jpg", "jpeg", "webp",
    "bmp", "tiff", "tif", "avif", "gif",
];

fn has_media_extension(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => MEDIA_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

/// Only these kinds of files are ever considered app-generated leftovers.
fn generated_category(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".tmp") || n.contains(".tmp.") {
        return None;
    }
    if n.contains("_compressed_") || n.contains("_compressed.") {
        return Some("compressed");
    }
    if n.contains("_no_bg.") {
        return Some("bg_removed");
    }
    if n.contains("_upscaled_x2.") || n.contains("_upscaled_x4.") {
        return Some("upscaled");
    }
    for ext in MEDIA_EXTENSIONS {
        if n.ends_with(&format!("_{0}.{0}", ext)) {
            return Some("converted");
        }
    }
    None
}

fn is_passlog(name: &str) -> bool {
    name.starts_with("ffmpeg2pass") && name.ends_with(".log")
}

/// The thumbnails directory is app-owned, so any image file inside it is ours.
fn is_thumbnail(name: &str) -> bool {
    crate::crud::thumbnails::is_thumbnail_file(Path::new(name))
}

/// Defense-in-depth: refuse to remove a file unless it lives in our thumbnail
/// cache or its name is one we generate in the output folder.
fn deletion_allowed(path: &Path, thumb_dir: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if path.parent() == Some(thumb_dir) {
        return is_thumbnail(name);
    }
    is_passlog(name) || generated_category(name).is_some()
}

fn walk_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk_files(&path, depth - 1, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
}

async fn output_dir(pool: &SqlitePool) -> Option<PathBuf> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'output'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    value.map(PathBuf::from).filter(|p| p.is_dir())
}

async fn collect_orphans(
    pool: &SqlitePool,
    app: &AppHandle,
    scan_output: bool,
    categories: Option<&HashSet<String>>,
) -> Result<OrphanScan, String> {
    let mut scan = OrphanScan::default();
    let mut statements = 0usize;

    // 1) Integrity: rows whose file no longer exists.
    let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(ROW_PATH_UNION)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    statements += 1;
    for (table, id, filepath) in rows {
        if let Some(filepath) = filepath {
            if !Path::new(&filepath).exists() {
                scan.orphaned_rows.push(OrphanRow { table, id, filepath });
            }
        }
    }

    // 2) Storage: files with no owning row, but only inside folders we manage and
    // only names that match what we generate.
    let referenced: Vec<Option<String>> = sqlx::query_scalar(REFERENCED_UNION)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    statements += 1;
    let referenced: HashSet<String> = referenced.into_iter().flatten().collect();

    let thumb_dir = crate::crud::video_processing::get_thumbnails_dir(app);

    let mut candidates: Vec<PathBuf> = Vec::new();
    walk_files(&thumb_dir, 1, &mut candidates);
    if scan_output {
        if let Some(dir) = output_dir(pool).await {
            walk_files(&dir, 2, &mut candidates);
        }
    }

    for path in candidates {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if referenced.contains(&path_str) {
            continue;
        }

        let in_thumbnails = path.parent() == Some(thumb_dir.as_path());
        let eligible = if is_passlog(name) || (in_thumbnails && is_thumbnail(name)) {
            true
        } else if scan_output {
            match generated_category(name) {
                Some(category) => categories
                    .as_ref()
                    .map(|set| set.contains(category))
                    .unwrap_or(true),
                None => false,
            }
        } else {
            false
        };
        if !eligible || (!has_media_extension(name) && !is_passlog(name)) {
            continue;
        }

        let bytes = fs::metadata(&path).map(|m| m.len() as i64).unwrap_or(0);
        scan.orphaned_files.push(OrphanFile { path: path_str, bytes });
    }

    scan.bytes_reclaimed = scan.orphaned_files.iter().map(|f| f.bytes).sum();
    scan.statements = statements;
    Ok(scan)
}

/// Count only orphaned *rows* (a DB row whose file is gone). Safe and cheap.
pub async fn count_orphaned_rows(pool: &SqlitePool) -> Result<i64, String> {
    let rows: Vec<(String, i64, Option<String>)> = sqlx::query_as(ROW_PATH_UNION)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, filepath)| {
            filepath
                .as_ref()
                .map(|p| !Path::new(p).exists())
                .unwrap_or(false)
        })
        .count() as i64)
}

async fn delete_orphan_rows(pool: &SqlitePool, scan: &mut OrphanScan) -> Result<usize, String> {
    use std::collections::HashMap;
    let mut by_table: HashMap<String, Vec<i64>> = HashMap::new();
    for row in &scan.orphaned_rows {
        by_table.entry(row.table.clone()).or_default().push(row.id);
    }

    let mut statements = 0usize;
    for (table, ids) in by_table {
        // `table` only ever comes from our own whitelist above.
        if !ROW_PATH_UNION.contains(format!("'{}'", table).as_str()) {
            continue;
        }
        for chunk in ids.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let query = format!("DELETE FROM {} WHERE id IN ({})", table, placeholders);
            let mut q = sqlx::query(&query);
            for id in chunk {
                q = q.bind(id);
            }
            scan.deleted_rows += q
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?
                .rows_affected();
            statements += 1;
        }
    }
    Ok(statements)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_orphan_cleanup(
    pool: &SqlitePool,
    app: &AppHandle,
    dry_run: bool,
    delete_rows: bool,
    delete_files: bool,
    scan_output: bool,
    categories: Option<HashSet<String>>,
) -> Result<OrphanScan, String> {
    let mut scan = collect_orphans(pool, app, scan_output, categories.as_ref()).await?;
    scan.dry_run = dry_run;

    if dry_run {
        return Ok(scan);
    }

    if delete_rows {
        let extra_statements = delete_orphan_rows(pool, &mut scan).await?;
        scan.statements += extra_statements;
    }

    if delete_files {
        let thumb_dir = crate::crud::thumbnails::cache_dir(app);
        let files = std::mem::take(&mut scan.orphaned_files);
        for file in files {
            if !deletion_allowed(Path::new(&file.path), &thumb_dir) {
                scan.skipped_files.push(file.path);
                continue;
            }
            match fs::remove_file(&file.path) {
                Ok(()) => {
                    scan.deleted_files += 1;
                    scan.bytes_reclaimed += file.bytes;
                }
                Err(e) => log::warn!("Failed to remove orphaned file {}: {}", file.path, e),
            }
        }
    }

    Ok(scan)
}

/// Preview orphaned rows/generated files without deleting anything.
#[tauri::command]
pub async fn orphan_scan(
    app: AppHandle,
    state: State<'_, DbState>,
) -> Result<OrphanScan, String> {
    run_orphan_cleanup(&state.0, &app, true, false, false, true, None).await
}

/// Remove orphaned rows plus app-generated leftover files (pattern-scoped).
#[tauri::command]
pub async fn orphan_cleanup(
    app: AppHandle,
    state: State<'_, DbState>,
) -> Result<OrphanScan, String> {
    run_orphan_cleanup(&state.0, &app, false, true, true, true, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_generated_files_only() {
        assert_eq!(generated_category("clip_compressed.mp4"), Some("compressed"));
        assert_eq!(
            generated_category("photo_compressed_20MB.mp4"),
            Some("compressed")
        );
        assert_eq!(generated_category("img_no_bg.png"), Some("bg_removed"));
        assert_eq!(generated_category("img_upscaled_x4.jpg"), Some("upscaled"));
        assert_eq!(generated_category("img_webp.webp"), Some("converted"));
        // Unrelated files must never be classified as ours.
        assert_eq!(generated_category("holiday.jpg"), None);
        assert_eq!(generated_category("notes.txt"), None);
        assert_eq!(generated_category("x.mp4.tmp"), None);
        assert_eq!(generated_category("report_2024.pdf"), None);
    }

    #[test]
    fn thumbnails_passlogs_and_delete_guard() {
        assert!(is_thumbnail("123.jpg"));
        assert!(is_thumbnail("abc.png"));
        assert!(!is_thumbnail("notes.txt"));
        assert!(is_passlog("ffmpeg2pass-0.log"));
        assert!(!is_passlog("other.log"));

        let thumb_dir = Path::new("/thumbs");
        assert!(deletion_allowed(Path::new("/thumbs/12.jpg"), thumb_dir));
        assert!(deletion_allowed(Path::new("/x/clip_compressed.mp4"), thumb_dir));
        assert!(!deletion_allowed(Path::new("/x/holiday.jpg"), thumb_dir));
        assert!(!deletion_allowed(Path::new("/x/notes.txt"), thumb_dir));
    }
}
