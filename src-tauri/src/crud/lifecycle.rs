use serde::Serialize;
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;
use std::fs;
use tauri::State;

use crate::DbState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Image,
    Video,
}

impl ItemKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "image" | "images" => Ok(ItemKind::Image),
            "video" | "videos" => Ok(ItemKind::Video),
            other => Err(format!("Unknown item kind: '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeleteReport {
    pub rows_deleted: u64,
    pub files_deleted: u64,
    pub bytes_freed: i64,
    pub skipped: Vec<SkippedFile>,
}

fn placeholders(count: usize) -> String {
    vec!["?"; count].join(", ")
}

async fn query_paths(pool: &SqlitePool, sql: &str, ids: &[i64]) -> Result<Vec<String>, String> {
    let query = format!("{} ({})", sql, placeholders(ids.len()));
    let mut q = sqlx::query_scalar::<_, String>(&query);
    for id in ids {
        q = q.bind(id);
    }
    q.fetch_all(pool).await.map_err(|e| e.to_string())
}

async fn query_optional_paths(
    pool: &SqlitePool,
    sql: &str,
    ids: &[i64],
) -> Result<Vec<String>, String> {
    let query = format!("{} ({})", sql, placeholders(ids.len()));
    let mut q = sqlx::query_scalar::<_, Option<String>>(&query);
    for id in ids {
        q = q.bind(id);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().flatten().collect())
}

async fn delete_ids(
    tx: &mut Transaction<'_, Sqlite>,
    sql_prefix: &str,
    ids: &[i64],
) -> Result<u64, String> {
    let query = format!("{} ({})", sql_prefix, placeholders(ids.len()));
    let mut q = sqlx::query(&query);
    for id in ids {
        q = q.bind(id);
    }
    Ok(q.execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected())
}

/// Is this path still pointed at by any row (in any table)?
async fn path_referenced(pool: &SqlitePool, path: &str) -> Result<bool, String> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM (\
            SELECT filepath FROM videos \
            UNION ALL SELECT thumbnail_path FROM videos \
            UNION ALL SELECT filepath FROM images \
            UNION ALL SELECT filepath FROM compressed_videos \
            UNION ALL SELECT filepath FROM compressed_images \
            UNION ALL SELECT filepath FROM converted_videos \
            UNION ALL SELECT filepath FROM converted_images \
            UNION ALL SELECT filepath FROM upscaled_images \
            UNION ALL SELECT filepath FROM bg_removed_images \
            UNION ALL SELECT filepath FROM bg_removed_videos\
         ) WHERE filepath = ? LIMIT 1",
    )
    .bind(path)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.is_some())
}

fn remove_file_reporting(report: &mut DeleteReport, path: &str, must_exist: bool) {
    match fs::metadata(path) {
        Ok(meta) => match fs::remove_file(path) {
            Ok(()) => {
                report.files_deleted += 1;
                report.bytes_freed += meta.len() as i64;
            }
            Err(e) => report.skipped.push(SkippedFile {
                path: path.to_string(),
                reason: format!("could not delete: {}", e),
            }),
        },
        Err(_) => {
            if must_exist {
                report.skipped.push(SkippedFile {
                    path: path.to_string(),
                    reason: "file already missing".to_string(),
                });
            }
        }
    }
}

/// The one and only delete implementation for library items.
///
/// - Rows are deleted first, in a single transaction (children explicitly, so
///   it is safe even if foreign keys are off).
/// - App-managed files (video thumbnails) are always removed.
/// - Originals and outputs are only removed when `delete_files` is true, and
///   never when another row still references the same path.
pub async fn delete_items_impl(
    pool: &SqlitePool,
    kind: ItemKind,
    ids: &[i64],
    delete_files: bool,
) -> Result<DeleteReport, String> {
    let mut report = DeleteReport::default();
    if ids.is_empty() {
        return Ok(report);
    }

    // Collect candidate paths *before* deleting rows.
    let (managed, user_files): (Vec<String>, Vec<String>) = match kind {
        ItemKind::Image => (Vec::new(), {
            let mut files = Vec::new();
            for (table, key) in [
                ("images", "id"),
                ("compressed_images", "original_id"),
                ("upscaled_images", "original_id"),
                ("bg_removed_images", "original_id"),
                ("converted_images", "original_id"),
            ] {
                files.extend(
                    query_paths(
                        pool,
                        &format!("SELECT filepath FROM {} WHERE {} IN", table, key),
                        ids,
                    )
                    .await?,
                );
            }
            files
        }),
        ItemKind::Video => {
            let managed = query_optional_paths(
                pool,
                "SELECT thumbnail_path FROM videos WHERE id IN",
                ids,
            )
            .await?;
            let mut files = Vec::new();
            for (table, key) in [
                ("videos", "id"),
                ("compressed_videos", "original_id"),
                ("converted_videos", "original_id"),
                ("bg_removed_videos", "original_id"),
            ] {
                files.extend(
                    query_paths(
                        pool,
                        &format!("SELECT filepath FROM {} WHERE {} IN", table, key),
                        ids,
                    )
                    .await?,
                );
            }
            (managed, files)
        }
    };

    // Delete rows in one transaction.
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let rows_deleted = match kind {
        ItemKind::Image => {
            delete_ids(&mut tx, "DELETE FROM compressed_images WHERE original_id IN", ids).await?;
            delete_ids(&mut tx, "DELETE FROM upscaled_images WHERE original_id IN", ids).await?;
            delete_ids(&mut tx, "DELETE FROM bg_removed_images WHERE original_id IN", ids).await?;
            delete_ids(&mut tx, "DELETE FROM converted_images WHERE original_id IN", ids).await?;
            delete_ids(&mut tx, "DELETE FROM selections WHERE image_id IN", ids).await?;
            delete_ids(&mut tx, "DELETE FROM images WHERE id IN", ids).await?
        }
        ItemKind::Video => {
            delete_ids(
                &mut tx,
                "DELETE FROM compressed_videos WHERE original_id IN",
                ids,
            )
            .await?;
            delete_ids(
                &mut tx,
                "DELETE FROM converted_videos WHERE original_id IN",
                ids,
            )
            .await?;
            delete_ids(
                &mut tx,
                "DELETE FROM bg_removed_videos WHERE original_id IN",
                ids,
            )
            .await?;
            delete_ids(
                &mut tx,
                "DELETE FROM video_selections WHERE video_id IN",
                ids,
            )
            .await?;
            delete_ids(&mut tx, "DELETE FROM videos WHERE id IN", ids).await?
        }
    };
    tx.commit().await.map_err(|e| e.to_string())?;
    report.rows_deleted = rows_deleted;

    // App-managed thumbnails are removed unless another row still shares them
    // (identical content dedupes to one cached thumbnail).
    let mut seen: HashSet<String> = HashSet::new();
    for path in managed {
        if !seen.insert(path.clone()) {
            continue;
        }
        if path_referenced(pool, &path).await? {
            continue;
        }
        remove_file_reporting(&mut report, &path, false);
    }

    // Originals/outputs are opt-in and must not be referenced elsewhere.
    for path in user_files {
        if !seen.insert(path.clone()) {
            continue;
        }
        if !delete_files {
            report.skipped.push(SkippedFile {
                path,
                reason: "delete_files disabled".to_string(),
            });
            continue;
        }
        if path_referenced(pool, &path).await? {
            report.skipped.push(SkippedFile {
                path,
                reason: "still referenced by another row".to_string(),
            });
            continue;
        }
        remove_file_reporting(&mut report, &path, true);
    }

    Ok(report)
}

#[tauri::command]
pub async fn delete_items(
    state: State<'_, DbState>,
    kind: String,
    ids: Vec<i64>,
    delete_files: bool,
) -> Result<DeleteReport, String> {
    let kind = ItemKind::parse(&kind)?;
    delete_items_impl(&state.0, kind, &ids, delete_files).await
}

/// (category, table) pairs that hold generated outputs for a given item kind.
fn output_tables(kind: ItemKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        ItemKind::Image => &[
            ("compressed", "compressed_images"),
            ("converted", "converted_images"),
            ("upscaled", "upscaled_images"),
            ("bg_removed", "bg_removed_images"),
        ],
        ItemKind::Video => &[
            ("compressed", "compressed_videos"),
            ("converted", "converted_videos"),
            ("bg_removed", "bg_removed_videos"),
        ],
    }
}

/// Delete generated outputs (files + rows) for the given items while keeping the
/// original files and rows. `category` optionally limits to one output type.
pub async fn delete_outputs_impl(
    pool: &SqlitePool,
    kind: ItemKind,
    ids: &[i64],
    category: Option<&str>,
) -> Result<DeleteReport, String> {
    let mut report = DeleteReport::default();
    if ids.is_empty() {
        return Ok(report);
    }

    // Collect output files first.
    let mut files: Vec<String> = Vec::new();
    for (cat, table) in output_tables(kind) {
        if let Some(filter) = category {
            if filter != *cat {
                continue;
            }
        }
        files.extend(
            query_paths(
                pool,
                &format!("SELECT filepath FROM {} WHERE original_id IN", table),
                ids,
            )
            .await?,
        );
    }

    // Delete the output rows in one transaction (originals untouched).
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    for (cat, table) in output_tables(kind) {
        if let Some(filter) = category {
            if filter != *cat {
                continue;
            }
        }
        report.rows_deleted += delete_ids(
            &mut tx,
            &format!("DELETE FROM {} WHERE original_id IN", table),
            ids,
        )
        .await?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;

    // Then remove the files, unless another row still points at them.
    let mut seen: HashSet<String> = HashSet::new();
    for path in files {
        if !seen.insert(path.clone()) {
            continue;
        }
        if path_referenced(pool, &path).await? {
            report.skipped.push(SkippedFile {
                path,
                reason: "still referenced by another row".to_string(),
            });
            continue;
        }
        remove_file_reporting(&mut report, &path, true);
    }

    Ok(report)
}

/// Delete a single generated output by table + row id.
pub async fn delete_output_impl(
    pool: &SqlitePool,
    output_kind: &str,
    id: i64,
) -> Result<DeleteReport, String> {
    let table = match output_kind {
        "compressed_image" => "compressed_images",
        "compressed_video" => "compressed_videos",
        "converted_image" => "converted_images",
        "converted_video" => "converted_videos",
        "upscaled_image" => "upscaled_images",
        "bg_removed_image" => "bg_removed_images",
        "bg_removed_video" => "bg_removed_videos",
        other => return Err(format!("Unknown output kind: '{}'", other)),
    };

    let mut report = DeleteReport::default();
    let row: Option<(String,)> = sqlx::query_as(&format!(
        "SELECT filepath FROM {} WHERE id = ?",
        table
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    let Some((filepath,)) = row else {
        return Ok(report);
    };

    let affected = sqlx::query(&format!("DELETE FROM {} WHERE id = ?", table))
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected();
    report.rows_deleted = affected;

    if affected > 0 {
        if path_referenced(pool, &filepath).await? {
            report.skipped.push(SkippedFile {
                path: filepath,
                reason: "still referenced by another row".to_string(),
            });
        } else {
            remove_file_reporting(&mut report, &filepath, true);
        }
    }

    Ok(report)
}

#[tauri::command]
pub async fn delete_outputs(
    state: State<'_, DbState>,
    kind: String,
    ids: Vec<i64>,
    output_kind: Option<String>,
) -> Result<DeleteReport, String> {
    let kind = ItemKind::parse(&kind)?;
    delete_outputs_impl(&state.0, kind, &ids, output_kind.as_deref()).await
}

#[tauri::command]
pub async fn delete_output(
    state: State<'_, DbState>,
    output_kind: String,
    id: i64,
) -> Result<DeleteReport, String> {
    delete_output_impl(&state.0, &output_kind, id).await
}

/// One row per compressed variant so the UI can show every target-size output.
#[derive(Debug, Clone, Serialize)]
pub struct CompressedVariant {
    pub id: i64,
    pub filepath: String,
    pub size: Option<i64>,
    pub label: String,
}

type VideoVariantRow = (
    i64,
    String,
    Option<i64>,
    Option<String>,
    Option<f64>,
    Option<i64>,
    Option<String>,
);

#[tauri::command]
pub async fn get_compressed_variants(
    state: State<'_, DbState>,
    kind: String,
    id: i64,
) -> Result<Vec<CompressedVariant>, String> {
    match ItemKind::parse(&kind)? {
        ItemKind::Image => {
            let rows: Vec<(i64, String, Option<i64>, Option<String>)> = sqlx::query_as(
                "SELECT id, filepath, size, variant_label FROM compressed_images \
                 WHERE original_id = ? ORDER BY id DESC",
            )
            .bind(id)
            .fetch_all(&state.0)
            .await
            .map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|(id, filepath, size, label)| CompressedVariant {
                    id,
                    filepath,
                    size,
                    label: label.unwrap_or_else(|| "compressed".to_string()),
                })
                .collect())
        }
        ItemKind::Video => {
            let rows: Vec<VideoVariantRow> = sqlx::query_as(
                "SELECT id, filepath, size, target_kind, target_value, crf, preset \
                 FROM compressed_videos WHERE original_id = ? ORDER BY id DESC",
            )
            .bind(id)
            .fetch_all(&state.0)
            .await
            .map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|(id, filepath, size, target_kind, target_value, crf, preset)| {
                    let label = match target_kind.as_deref() {
                        Some("percent") => {
                            format!("{}%", target_value.unwrap_or(0.0).round() as i64)
                        }
                        Some("absolute") => format!(
                            "{} MB",
                            (target_value.unwrap_or(0.0) / (1024.0 * 1024.0)).round() as i64
                        ),
                        _ => crf
                            .map(|c| format!("CRF {}", c))
                            .or(preset)
                            .unwrap_or_else(|| "compressed".to_string()),
                    };
                    CompressedVariant { id, filepath, size, label }
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn test_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let options = SqliteConnectOptions::new()
            .filename(dir.path().join("lifecycle.sqlite"))
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("pool");

        for statement in [
            "CREATE TABLE videos (id INTEGER PRIMARY KEY, filepath TEXT, thumbnail_path TEXT)",
            "CREATE TABLE images (id INTEGER PRIMARY KEY, filepath TEXT, thumbnail_path TEXT)",
            "CREATE TABLE compressed_videos (id INTEGER PRIMARY KEY, original_id INTEGER, filepath TEXT)",
            "CREATE TABLE compressed_images (original_id INTEGER, filepath TEXT)",
            "CREATE TABLE converted_videos (original_id INTEGER, filepath TEXT)",
            "CREATE TABLE converted_images (original_id INTEGER, filepath TEXT)",
            "CREATE TABLE upscaled_images (original_id INTEGER, filepath TEXT)",
            "CREATE TABLE bg_removed_images (original_id INTEGER, filepath TEXT)",
            "CREATE TABLE bg_removed_videos (original_id INTEGER, filepath TEXT)",
            "CREATE TABLE selections (image_id INTEGER)",
            "CREATE TABLE video_selections (video_id INTEGER)",
        ] {
            sqlx::query(statement).execute(&pool).await.expect("table");
        }
        (dir, pool)
    }

    #[test]
    fn delete_without_files_keeps_files_but_removes_rows() {
        tauri::async_runtime::block_on(async {
            let (dir, pool) = test_pool().await;
            let original = dir.path().join("a.mp4");
            let output = dir.path().join("a_compressed.mp4");
            let thumb = dir.path().join("1.jpg");
            for p in [&original, &output, &thumb] {
                std::fs::write(p, b"x").expect("write");
            }

            sqlx::query("INSERT INTO videos (id, filepath, thumbnail_path) VALUES (1, ?, ?)")
                .bind(original.to_string_lossy().to_string())
                .bind(thumb.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed video");
            sqlx::query("INSERT INTO compressed_videos (original_id, filepath) VALUES (1, ?)")
                .bind(output.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed output");

            let report =
                delete_items_impl(&pool, ItemKind::Video, &[1], false).await.expect("delete");

            assert_eq!(report.rows_deleted, 1);
            // Thumbnail is app-managed and always removed.
            assert!(!thumb.exists());
            // Original and output survive, reported as skipped.
            assert!(original.exists());
            assert!(output.exists());
            assert!(report
                .skipped
                .iter()
                .any(|s| s.path.ends_with("a.mp4") && s.reason == "delete_files disabled"));
            let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM videos")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(remaining.0, 0);
        });
    }

    #[test]
    fn delete_outputs_keeps_the_original() {
        tauri::async_runtime::block_on(async {
            let (dir, pool) = test_pool().await;
            let original = dir.path().join("a.mp4");
            let output = dir.path().join("a_compressed.mp4");
            std::fs::write(&original, vec![0u8; 100]).expect("write original");
            std::fs::write(&output, vec![0u8; 40]).expect("write output");

            sqlx::query("INSERT INTO videos (id, filepath) VALUES (1, ?)")
                .bind(original.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed video");
            sqlx::query("INSERT INTO compressed_videos (id, original_id, filepath) VALUES (1, 1, ?)")
                .bind(output.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed output");

            let report = delete_outputs_impl(&pool, ItemKind::Video, &[1], None)
                .await
                .expect("delete outputs");

            assert_eq!(report.rows_deleted, 1);
            assert!(!output.exists(), "generated output removed");
            assert!(original.exists(), "original kept");
            let videos: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM videos")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(videos.0, 1, "original row kept");
        });
    }

    #[test]
    fn delete_single_variant_only() {
        tauri::async_runtime::block_on(async {
            let (dir, pool) = test_pool().await;
            let original = dir.path().join("b.mp4");
            let v1 = dir.path().join("b_20mb.mp4");
            let v2 = dir.path().join("b_50pct.mp4");
            std::fs::write(&original, b"x").unwrap();
            std::fs::write(&v1, b"x").unwrap();
            std::fs::write(&v2, b"x").unwrap();

            sqlx::query("INSERT INTO videos (id, filepath) VALUES (2, ?)")
                .bind(original.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO compressed_videos (id, original_id, filepath) VALUES (10, 2, ?)")
                .bind(v1.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO compressed_videos (id, original_id, filepath) VALUES (11, 2, ?)")
                .bind(v2.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .unwrap();

            let report = delete_output_impl(&pool, "compressed_video", 10)
                .await
                .expect("delete variant");
            assert_eq!(report.rows_deleted, 1);
            assert!(!v1.exists());
            assert!(v2.exists(), "other variant untouched");
            assert!(original.exists());
        });
    }

    #[test]
    fn delete_with_files_removes_outputs_and_reports_bytes() {
        tauri::async_runtime::block_on(async {
            let (dir, pool) = test_pool().await;
            let original = dir.path().join("b.mp4");
            let output = dir.path().join("b_compressed.mp4");
            std::fs::write(&original, vec![0u8; 100]).expect("write original");
            std::fs::write(&output, vec![0u8; 40]).expect("write output");

            sqlx::query("INSERT INTO videos (id, filepath) VALUES (2, ?)")
                .bind(original.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed video");
            sqlx::query("INSERT INTO compressed_videos (original_id, filepath) VALUES (2, ?)")
                .bind(output.to_string_lossy().to_string())
                .execute(&pool)
                .await
                .expect("seed output");

            let report =
                delete_items_impl(&pool, ItemKind::Video, &[2], true).await.expect("delete");

            assert_eq!(report.rows_deleted, 1);
            assert_eq!(report.files_deleted, 2);
            assert_eq!(report.bytes_freed, 140);
            assert!(!original.exists());
            assert!(!output.exists());
        });
    }
}
