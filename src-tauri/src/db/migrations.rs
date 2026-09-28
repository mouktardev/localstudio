use anyhow::{Context, Result};
use sqlx::{Connection, SqliteConnection, SqlitePool};
use tauri::{AppHandle, Manager};

/// Current schema version. Bump this and add a matching `if current < N`
/// branch in `run_migrations` whenever the schema changes.
const SCHEMA_VERSION: i64 = 6;

pub async fn run_migrations(pool: &SqlitePool, app: &AppHandle) -> Result<()> {
    let picture_dir = app
        .path()
        .picture_dir()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()));
    run_migrations_with(pool, picture_dir.as_deref()).await
}

/// Runs the migration chain, taking the default output directory as a plain
/// value so the chain can be exercised without a Tauri `AppHandle`.
pub async fn run_migrations_with(pool: &SqlitePool, picture_dir: Option<&str>) -> Result<()> {
    let current: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await
        .context("Failed to read schema version")?;

    if current > SCHEMA_VERSION {
        log::warn!(
            "Database schema v{} is newer than this build (v{}); skipping migrations",
            current,
            SCHEMA_VERSION
        );
    }

    if current < 1 {
        let mut tx = pool.begin().await.context("Failed to begin v1 migration")?;
        migrate_v1(&mut tx, picture_dir).await?;
        set_user_version(&mut tx, 1).await?;
        tx.commit().await.context("Failed to commit v1 migration")?;
        log::info!("Applied schema migration v1");
    }

    if current < 2 {
        // `PRAGMA foreign_keys` must be toggled outside a transaction and on the
        // same connection that runs the migration, hence the pinned connection.
        let mut conn = pool
            .acquire()
            .await
            .context("Failed to acquire connection for migration")?;
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await
            .context("Failed to disable foreign_keys for migration")?;
        {
            let mut tx = conn.begin().await.context("Failed to begin v2 migration")?;
            migrate_v2(&mut tx).await?;
            set_user_version(&mut tx, 2).await?;
            tx.commit().await.context("Failed to commit v2 migration")?;
        }
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await
            .context("Failed to re-enable foreign_keys after migration")?;
        log::info!("Applied schema migration v2");
    }

    if current < 3 {
        let mut tx = pool.begin().await.context("Failed to begin v3 migration")?;
        migrate_v3(&mut tx).await?;
        set_user_version(&mut tx, 3).await?;
        tx.commit().await.context("Failed to commit v3 migration")?;
        log::info!("Applied schema migration v3");
    }

    if current < 4 {
        let mut tx = pool.begin().await.context("Failed to begin v4 migration")?;
        migrate_v4(&mut tx).await?;
        set_user_version(&mut tx, 4).await?;
        tx.commit().await.context("Failed to commit v4 migration")?;
        log::info!("Applied schema migration v4");
    }

    if current < 5 {
        let mut tx = pool.begin().await.context("Failed to begin v5 migration")?;
        migrate_v5(&mut tx).await?;
        set_user_version(&mut tx, 5).await?;
        tx.commit().await.context("Failed to commit v5 migration")?;
        log::info!("Applied schema migration v5");
    }

    if current < 6 {
        let mut tx = pool.begin().await.context("Failed to begin v6 migration")?;
        migrate_v6(&mut tx).await?;
        set_user_version(&mut tx, 6).await?;
        tx.commit().await.context("Failed to commit v6 migration")?;
        log::info!("Applied schema migration v6");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Schema v5/v6 — image thumbnails were added then removed (unused column).
// ---------------------------------------------------------------------------

async fn migrate_v5(conn: &mut SqliteConnection) -> Result<()> {
    add_column_if_missing(&mut *conn, "images", "thumbnail_path", "TEXT").await?;
    Ok(())
}

async fn migrate_v6(conn: &mut SqliteConnection) -> Result<()> {
    if column_exists(&mut *conn, "images", "thumbnail_path").await? {
        sqlx::query("ALTER TABLE images DROP COLUMN thumbnail_path")
            .execute(&mut *conn)
            .await
            .context("Failed to drop images.thumbnail_path")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema v4 — drop the unused `jobs` table.
// ---------------------------------------------------------------------------

async fn migrate_v4(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query("DROP TABLE IF EXISTS jobs")
        .execute(&mut *conn)
        .await
        .context("Failed to drop 'jobs' table")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema v3 — purge settings for features that were removed.
// ---------------------------------------------------------------------------

async fn migrate_v3(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        "DELETE FROM settings WHERE key IN (\
            'retention_days', 'retention_max_per_item', 'purge_cancelled_failed', \
            'delete_originals_after_compress'\
         )",
    )
    .execute(&mut *conn)
    .await
    .context("Failed to purge removed settings")?;
    Ok(())
}

async fn set_user_version(conn: &mut SqliteConnection, version: i64) -> Result<()> {
    sqlx::query(&format!("PRAGMA user_version = {}", version))
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to set user_version to {}", version))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema v1 — the original v0.2.3 baseline.
// ---------------------------------------------------------------------------

async fn migrate_v1(conn: &mut SqliteConnection, picture_dir: Option<&str>) -> Result<()> {
    create_images_table(&mut *conn).await?;
    create_settings_table(&mut *conn).await?;
    create_swatches_table(&mut *conn).await?;
    create_notifications_table(&mut *conn).await?;
    create_selections_table(&mut *conn).await?;
    create_compressed_images_table(&mut *conn).await?;
    create_upscaled_images_table(&mut *conn).await?;
    create_bg_removed_images_table(&mut *conn).await?;
    create_videos_table(&mut *conn).await?;
    alter_videos_add_thumbnail(&mut *conn).await?;
    create_bg_removed_videos_table(&mut *conn).await?;
    create_compressed_videos_table(&mut *conn).await?;
    create_converted_images_table(&mut *conn).await?;
    create_converted_videos_table(&mut *conn).await?;
    create_filters_table(&mut *conn).await?;
    insert_default_settings(&mut *conn, picture_dir).await?;
    insert_default_swatches(&mut *conn).await?;
    insert_default_filters(&mut *conn).await?;
    Ok(())
}

async fn create_images_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS images (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            filename TEXT NOT NULL,
            filepath TEXT NOT NULL,
            mimetype TEXT,
            size INTEGER,
            width INTEGER,
            height INTEGER
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'images' table")?;

    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_images_filepath ON images(filepath)")
        .execute(&mut *conn)
        .await
        .context("Failed to create unique index on filepath")?;

    Ok(())
}

async fn create_settings_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'settings' table")?;

    Ok(())
}

async fn create_swatches_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS swatches (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            hex TEXT NOT NULL UNIQUE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'swatches' table")?;

    Ok(())
}

async fn insert_default_settings(
    conn: &mut SqliteConnection,
    picture_dir: Option<&str>,
) -> Result<()> {
    let picture_dir = picture_dir.unwrap_or_default();

    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('output', ?)")
        .bind(picture_dir)
        .execute(&mut *conn)
        .await
        .context("Failed to insert default 'output' setting")?;

    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('upscale_model', 'realesrgan-x4')")
        .execute(&mut *conn)
        .await
        .context("Failed to insert default 'upscale_model' setting")?;

    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES ('bg_removal_model', 'bria-rmbg-1.4')")
        .execute(&mut *conn)
        .await
        .context("Failed to insert default 'bg_removal_model' setting")?;

    Ok(())
}

async fn insert_default_swatches(conn: &mut SqliteConnection) -> Result<()> {
    let default_swatches = ["#ff0000", "#00ff00", "#0000ff", "#ffffff", "#000000"];

    for color in default_swatches {
        sqlx::query("INSERT OR IGNORE INTO swatches (hex) VALUES (?)")
            .bind(color)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("Failed to insert default swatch '{}'", color))?;
    }

    Ok(())
}

async fn create_notifications_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            message TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'info',
            timestamp INTEGER NOT NULL,
            read INTEGER NOT NULL DEFAULT 0,
            action_label TEXT,
            action_payload TEXT
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'notifications' table")?;

    Ok(())
}

async fn create_selections_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS selections (
            image_id INTEGER PRIMARY KEY NOT NULL,
            selected_at INTEGER NOT NULL,
            FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'selections' table")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS video_selections (
            video_id INTEGER PRIMARY KEY NOT NULL,
            selected_at INTEGER NOT NULL,
            FOREIGN KEY (video_id) REFERENCES videos(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'video_selections' table")?;

    Ok(())
}

async fn create_compressed_images_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS compressed_images (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL UNIQUE,
            filepath TEXT NOT NULL,
            size INTEGER,
            FOREIGN KEY (original_id) REFERENCES images(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'compressed_images' table")?;

    Ok(())
}

async fn create_upscaled_images_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upscaled_images (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL,
            filepath TEXT NOT NULL,
            scale_factor INTEGER NOT NULL,
            model_used TEXT NOT NULL,
            size INTEGER,
            FOREIGN KEY (original_id) REFERENCES images(id) ON DELETE CASCADE,
            UNIQUE(original_id, scale_factor)
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'upscaled_images' table")?;

    Ok(())
}

async fn create_bg_removed_images_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS bg_removed_images (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL UNIQUE,
            filepath TEXT NOT NULL,
            size INTEGER,
            model_used TEXT NOT NULL,
            FOREIGN KEY (original_id) REFERENCES images(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'bg_removed_images' table")?;

    Ok(())
}

async fn create_filters_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS filters (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            page TEXT NOT NULL UNIQUE,
            search_query TEXT DEFAULT '',
            sort_field TEXT DEFAULT 'date',
            sort_order TEXT DEFAULT 'desc',
            output_type TEXT DEFAULT 'all',
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'filters' table")?;

    Ok(())
}

async fn create_videos_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS videos (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            filename TEXT NOT NULL,
            filepath TEXT NOT NULL UNIQUE,
            mimetype TEXT,
            size INTEGER,
            width INTEGER,
            height INTEGER,
            duration REAL,
            fps REAL
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'videos' table")?;

    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_videos_filepath ON videos(filepath)")
        .execute(&mut *conn)
        .await
        .context("Failed to create unique index on videos filepath")?;

    Ok(())
}

async fn alter_videos_add_thumbnail(conn: &mut SqliteConnection) -> Result<()> {
    match sqlx::query("ALTER TABLE videos ADD COLUMN thumbnail_path TEXT")
        .execute(&mut *conn)
        .await
    {
        Ok(_) => log::info!("Added thumbnail_path column to videos table"),
        Err(e) => {
            if e.to_string().contains("duplicate column") {
                // Column already exists, that's fine
            } else {
                return Err(e.into());
            }
        }
    }
    Ok(())
}

async fn create_bg_removed_videos_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS bg_removed_videos (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL UNIQUE,
            filepath TEXT NOT NULL,
            size INTEGER,
            model_used TEXT NOT NULL,
            FOREIGN KEY (original_id) REFERENCES videos(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'bg_removed_videos' table")?;

    Ok(())
}

async fn create_compressed_videos_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS compressed_videos (
            original_id INTEGER NOT NULL UNIQUE,
            filepath TEXT NOT NULL,
            size INTEGER,
            FOREIGN KEY (original_id) REFERENCES videos(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'compressed_videos' table")?;

    Ok(())
}

async fn create_converted_images_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS converted_images (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL,
            filepath TEXT NOT NULL,
            format TEXT NOT NULL,
            size INTEGER,
            FOREIGN KEY (original_id) REFERENCES images(id) ON DELETE CASCADE,
            UNIQUE(original_id, format)
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'converted_images' table")?;

    Ok(())
}

async fn create_converted_videos_table(conn: &mut SqliteConnection) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS converted_videos (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL,
            filepath TEXT NOT NULL,
            format TEXT NOT NULL,
            size INTEGER,
            FOREIGN KEY (original_id) REFERENCES videos(id) ON DELETE CASCADE,
            UNIQUE(original_id, format)
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create 'converted_videos' table")?;

    Ok(())
}

async fn insert_default_filters(conn: &mut SqliteConnection) -> Result<()> {
    // Default filters for index page
    sqlx::query(
        "INSERT OR IGNORE INTO filters (page, search_query, sort_field, sort_order, output_type) VALUES ('index', '', 'date', 'desc', 'all')"
    )
    .execute(&mut *conn)
    .await
    .context("Failed to insert default index filters")?;

    // Default filters for output page
    sqlx::query(
        "INSERT OR IGNORE INTO filters (page, search_query, sort_field, sort_order, output_type) VALUES ('output', '', 'date', 'desc', 'all')"
    )
    .execute(&mut *conn)
    .await
    .context("Failed to insert default output filters")?;

    // Default filters for videos page
    sqlx::query(
        "INSERT OR IGNORE INTO filters (page, search_query, sort_field, sort_order, output_type) VALUES ('videos', '', 'date', 'desc', 'all')"
    )
    .execute(&mut *conn)
    .await
    .context("Failed to insert default videos filters")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Schema v2 — additive timestamps, output metadata and indexes.
// ---------------------------------------------------------------------------

async fn migrate_v2(conn: &mut SqliteConnection) -> Result<()> {
    // Timestamps + content hash on library items.
    for (table, column, definition) in [
        ("videos", "created_at", "TEXT"),
        ("videos", "updated_at", "TEXT"),
        ("videos", "sha256", "TEXT"),
        ("images", "created_at", "TEXT"),
        ("images", "updated_at", "TEXT"),
        ("images", "sha256", "TEXT"),
    ] {
        add_column_if_missing(&mut *conn, table, column, definition).await?;
    }

    // Existing rows predate the column, so give them a stable timestamp.
    for table in ["videos", "images"] {
        sqlx::query(&format!(
            "UPDATE {} SET created_at = datetime('now') WHERE created_at IS NULL",
            table
        ))
        .execute(&mut *conn)
        .await
        .with_context(|| format!("Failed to backfill {}.created_at", table))?;
    }

    // Output metadata for conversion tables.
    for (table, column, definition) in [
        ("converted_videos", "codec", "TEXT"),
        ("converted_videos", "status", "TEXT NOT NULL DEFAULT 'done'"),
        ("converted_videos", "error", "TEXT"),
        ("converted_videos", "source_size", "INTEGER"),
        ("converted_videos", "completed_at", "TEXT"),
        ("converted_videos", "created_at", "TEXT"),
        ("converted_images", "codec", "TEXT"),
        ("converted_images", "status", "TEXT NOT NULL DEFAULT 'done'"),
        ("converted_images", "error", "TEXT"),
        ("converted_images", "source_size", "INTEGER"),
        ("converted_images", "completed_at", "TEXT"),
        ("converted_images", "created_at", "TEXT"),
        ("upscaled_images", "status", "TEXT NOT NULL DEFAULT 'done'"),
        ("upscaled_images", "error", "TEXT"),
        ("upscaled_images", "completed_at", "TEXT"),
        ("bg_removed_images", "status", "TEXT NOT NULL DEFAULT 'done'"),
        ("bg_removed_images", "error", "TEXT"),
        ("bg_removed_images", "completed_at", "TEXT"),
        ("bg_removed_videos", "status", "TEXT NOT NULL DEFAULT 'done'"),
        ("bg_removed_videos", "error", "TEXT"),
        ("bg_removed_videos", "completed_at", "TEXT"),
    ] {
        add_column_if_missing(&mut *conn, table, column, definition).await?;
    }

    // Rebuild the compressed tables so a user can keep multiple output variants
    // per original (drop the UNIQUE(original_id) constraint).
    rebuild_compressed_videos(&mut *conn).await?;
    rebuild_compressed_images(&mut *conn).await?;

    create_v2_indexes(&mut *conn).await?;
    insert_v2_default_settings(&mut *conn).await?;

    Ok(())
}

async fn rebuild_compressed_videos(conn: &mut SqliteConnection) -> Result<()> {
    if !table_exists(&mut *conn, "compressed_videos").await? {
        return Ok(());
    }
    if column_exists(&mut *conn, "compressed_videos", "id").await? {
        return Ok(());
    }

    sqlx::query(
        r#"
        CREATE TABLE compressed_videos_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL,
            filepath TEXT NOT NULL,
            size INTEGER,
            codec TEXT,
            target_kind TEXT,
            target_value REAL,
            crf INTEGER,
            target_bitrate_kbps INTEGER,
            achieved_bitrate_kbps INTEGER,
            preset TEXT,
            duration REAL,
            width INTEGER,
            height INTEGER,
            source_size INTEGER,
            status TEXT NOT NULL DEFAULT 'done',
            error TEXT,
            is_primary INTEGER NOT NULL DEFAULT 1,
            completed_at TEXT,
            created_at TEXT,
            FOREIGN KEY (original_id) REFERENCES videos(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create compressed_videos_new")?;

    sqlx::query(
        r#"
        INSERT INTO compressed_videos_new
            (original_id, filepath, size, status, is_primary, completed_at, created_at)
        SELECT original_id, filepath, size, 'done', 1, datetime('now'), datetime('now')
        FROM compressed_videos
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to copy compressed_videos rows")?;

    sqlx::query("DROP TABLE compressed_videos")
        .execute(&mut *conn)
        .await
        .context("Failed to drop old compressed_videos")?;

    sqlx::query("ALTER TABLE compressed_videos_new RENAME TO compressed_videos")
        .execute(&mut *conn)
        .await
        .context("Failed to rename compressed_videos_new")?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_compressed_videos_original_id ON compressed_videos(original_id)")
        .execute(&mut *conn)
        .await
        .context("Failed to index compressed_videos.original_id")?;

    Ok(())
}

async fn rebuild_compressed_images(conn: &mut SqliteConnection) -> Result<()> {
    if !table_exists(&mut *conn, "compressed_images").await? {
        return Ok(());
    }

    sqlx::query(
        r#"
        CREATE TABLE compressed_images_new (
            id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
            original_id INTEGER NOT NULL,
            filepath TEXT NOT NULL,
            size INTEGER,
            variant_label TEXT,
            is_primary INTEGER NOT NULL DEFAULT 1,
            status TEXT NOT NULL DEFAULT 'done',
            created_at TEXT,
            FOREIGN KEY (original_id) REFERENCES images(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to create compressed_images_new")?;

    sqlx::query(
        r#"
        INSERT INTO compressed_images_new (original_id, filepath, size, is_primary, status, created_at)
        SELECT original_id, filepath, size, 1, 'done', datetime('now')
        FROM compressed_images
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("Failed to copy compressed_images rows")?;

    sqlx::query("DROP TABLE compressed_images")
        .execute(&mut *conn)
        .await
        .context("Failed to drop old compressed_images")?;

    sqlx::query("ALTER TABLE compressed_images_new RENAME TO compressed_images")
        .execute(&mut *conn)
        .await
        .context("Failed to rename compressed_images_new")?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_compressed_images_original_id ON compressed_images(original_id)")
        .execute(&mut *conn)
        .await
        .context("Failed to index compressed_images.original_id")?;

    Ok(())
}

async fn create_v2_indexes(conn: &mut SqliteConnection) -> Result<()> {
    for statement in [
        "CREATE INDEX IF NOT EXISTS idx_videos_created_at ON videos(created_at)",
        "CREATE INDEX IF NOT EXISTS idx_images_created_at ON images(created_at)",
        "CREATE INDEX IF NOT EXISTS idx_converted_videos_original_id ON converted_videos(original_id)",
        "CREATE INDEX IF NOT EXISTS idx_converted_images_original_id ON converted_images(original_id)",
        "CREATE INDEX IF NOT EXISTS idx_notifications_read_timestamp ON notifications(read, timestamp)",
    ] {
        sqlx::query(statement)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("Failed to run: {}", statement))?;
    }
    Ok(())
}

async fn insert_v2_default_settings(conn: &mut SqliteConnection) -> Result<()> {
    for (key, value) in [
        ("max_concurrent_video_jobs", "0"),
        ("max_concurrent_image_jobs", "0"),
    ] {
        sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("Failed to insert default setting '{}'", key))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn table_exists(conn: &mut SqliteConnection, table: &str) -> Result<bool> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(table)
            .fetch_optional(&mut *conn)
            .await
            .with_context(|| format!("Failed to check existence of table '{}'", table))?;
    Ok(row.is_some())
}

async fn column_exists(conn: &mut SqliteConnection, table: &str, column: &str) -> Result<bool> {
    let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&format!("PRAGMA table_info({})", table))
            .fetch_all(&mut *conn)
            .await
            .with_context(|| format!("Failed to read table_info for '{}'", table))?;
    Ok(rows.iter().any(|r| r.1 == column))
}

async fn add_column_if_missing(
    conn: &mut SqliteConnection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    if column_exists(&mut *conn, table, column).await? {
        return Ok(());
    }
    sqlx::query(&format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        table, column, definition
    ))
    .execute(&mut *conn)
    .await
    .with_context(|| format!("Failed to add column {}.{}", table, column))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connection::create_pool;

    async fn fresh_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = create_pool(&dir.path().join("migrate.sqlite"))
            .await
            .expect("pool");
        (dir, pool)
    }

    async fn pool_table_exists(pool: &SqlitePool, table: &str) -> bool {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?")
                .bind(table)
                .fetch_optional(pool)
                .await
                .expect("sqlite_master query");
        row.is_some()
    }

    async fn pool_column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
        let rows: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as(&format!("PRAGMA table_info({})", table))
                .fetch_all(pool)
                .await
                .expect("table_info query");
        rows.iter().any(|r| r.1 == column)
    }

    async fn user_version(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(pool)
            .await
            .expect("user_version")
    }

    #[test]
    fn fresh_database_migrates_to_latest() {
        tauri::async_runtime::block_on(async {
            let (_dir, pool) = fresh_pool().await;
            run_migrations_with(&pool, None).await.expect("migrations");

            assert_eq!(user_version(&pool).await, SCHEMA_VERSION);

            for table in [
                "videos",
                "images",
                "compressed_videos",
                "compressed_images",
                "converted_videos",
                "converted_images",
                "settings",
                "filters",
            ] {
                assert!(pool_table_exists(&pool, table).await, "missing table {table}");
            }

            assert!(pool_column_exists(&pool, "videos", "created_at").await);
            assert!(pool_column_exists(&pool, "images", "sha256").await);
            assert!(pool_column_exists(&pool, "compressed_videos", "id").await);
            assert!(pool_column_exists(&pool, "compressed_videos", "target_bitrate_kbps").await);
            assert!(pool_column_exists(&pool, "converted_videos", "status").await);
            assert!(pool_column_exists(&pool, "bg_removed_videos", "completed_at").await);
        });
    }

    #[test]
    fn v1_database_upgrades_and_keeps_rows() {
        tauri::async_runtime::block_on(async {
            let (_dir, pool) = fresh_pool().await;

            // Simulate a v0.2.3 database: v1 schema only, user_version = 1.
            {
                let mut tx = pool.begin().await.expect("begin v1");
                migrate_v1(&mut tx, None).await.expect("migrate v1");
                set_user_version(&mut tx, 1).await.expect("set v1");
                tx.commit().await.expect("commit v1");
            }

            sqlx::query("INSERT INTO videos (filename, filepath, size) VALUES ('a.mp4', 'C:/a.mp4', 100)")
                .execute(&pool)
                .await
                .expect("seed video");
            sqlx::query(
                "INSERT INTO compressed_videos (original_id, filepath, size) VALUES (1, 'C:/a_compressed.mp4', 50)",
            )
            .execute(&pool)
            .await
            .expect("seed compressed video");

            run_migrations_with(&pool, None).await.expect("upgrade");

            assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
            let videos: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM videos")
                .fetch_one(&pool)
                .await
                .expect("count videos");
            assert_eq!(videos.0, 1, "existing rows must survive the upgrade");
            let compressed: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM compressed_videos")
                .fetch_one(&pool)
                .await
                .expect("count compressed");
            assert_eq!(compressed.0, 1);

            // The UNIQUE(original_id) constraint is gone: multiple variants allowed.
            sqlx::query(
                "INSERT INTO compressed_videos (original_id, filepath, size) VALUES (1, 'C:/a_20mb.mp4', 40)",
            )
            .execute(&pool)
            .await
            .expect("second variant");
            let variants: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM compressed_videos WHERE original_id = 1")
                    .fetch_one(&pool)
                    .await
                    .expect("count variants");
            assert_eq!(variants.0, 2, "multiple output variants must be allowed");
        });
    }

    #[test]
    fn migrations_are_idempotent() {
        tauri::async_runtime::block_on(async {
            let (_dir, pool) = fresh_pool().await;
            run_migrations_with(&pool, None).await.expect("first run");

            sqlx::query(
                "INSERT INTO videos (filename, filepath, size, created_at) VALUES ('a.mp4', 'C:/a.mp4', 100, datetime('now'))",
            )
            .execute(&pool)
            .await
            .expect("seed video");

            // A second app start must not re-run or double-apply migrations.
            run_migrations_with(&pool, None).await.expect("second run");

            assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
            let videos: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM videos")
                .fetch_one(&pool)
                .await
                .expect("count videos");
            assert_eq!(videos.0, 1);
        });
    }

    #[test]
    fn removed_settings_are_purged() {
        tauri::async_runtime::block_on(async {
            let (_dir, pool) = fresh_pool().await;
            run_migrations_with(&pool, None).await.expect("migrate");

            // Simulate a database created by the previous build, which had these.
            for key in [
                "retention_days",
                "retention_max_per_item",
                "purge_cancelled_failed",
                "delete_originals_after_compress",
            ] {
                sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, '0')")
                    .bind(key)
                    .execute(&pool)
                    .await
                    .expect("seed removed setting");
            }
            sqlx::query("PRAGMA user_version = 2")
                .execute(&pool)
                .await
                .expect("downgrade version");

            run_migrations_with(&pool, None).await.expect("re-migrate");

            let remaining: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM settings WHERE key LIKE 'retention%' \
                 OR key IN ('purge_cancelled_failed', 'delete_originals_after_compress')",
            )
            .fetch_one(&pool)
            .await
            .expect("count removed settings");
            assert_eq!(remaining, 0, "removed feature settings must be purged");
            assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
        });
    }

    /// Runs the real migration chain against a copy of an existing database.
    ///
    /// Set `LOCALSTUDIO_TEST_DB` to the path of a `db.sqlite` and run:
    /// `cargo test --lib migrate_real_database -- --ignored --nocapture`
    #[test]
    #[ignore = "set LOCALSTUDIO_TEST_DB to a db.sqlite path and run with --ignored"]
    fn migrate_real_database() {
        let source = match std::env::var_os("LOCALSTUDIO_TEST_DB") {
            Some(path) => std::path::PathBuf::from(path),
            None => return,
        };

        let seeded = std::env::var_os("LOCALSTUDIO_TEST_DB_SEED").is_some();

        tauri::async_runtime::block_on(async move {
            let dir = tempfile::tempdir().expect("tempdir");
            let dest = dir.path().join("db.sqlite");
            std::fs::copy(&source, &dest).expect("copy source db");
            let pool = create_pool(&dest).await.expect("pool");

            let mut seeded_video_id = 0i64;
            if seeded {
                // Populate the pre-migration (v1) schema so the rebuild path is
                // exercised with actual rows. AUTOINCREMENT counters on real
                // databases can start past 1, so use the generated ids.
                seeded_video_id = sqlx::query(
                    "INSERT INTO videos (filename, filepath, size, mimetype, width, height, duration, fps) VALUES ('seed.mp4', 'C:/seed.mp4', 1000, 'video/mp4', 1920, 1080, 10.0, 30.0)",
                )
                .execute(&pool)
                .await
                .expect("seed video")
                .last_insert_rowid();
                let image_id = sqlx::query(
                    "INSERT INTO images (filename, filepath, size, mimetype, width, height) VALUES ('seed.png', 'C:/seed.png', 500, 'image/png', 10, 10)",
                )
                .execute(&pool)
                .await
                .expect("seed image")
                .last_insert_rowid();

                for (sql, original_id) in [
                    (
                        "INSERT INTO compressed_videos (original_id, filepath, size) VALUES (?, 'C:/seed_compressed.mp4', 300)",
                        seeded_video_id,
                    ),
                    (
                        "INSERT INTO compressed_images (original_id, filepath, size) VALUES (?, 'C:/seed_compressed.png', 200)",
                        image_id,
                    ),
                    (
                        "INSERT INTO converted_videos (original_id, filepath, format, size) VALUES (?, 'C:/seed.webm', 'webm', 250)",
                        seeded_video_id,
                    ),
                    (
                        "INSERT INTO bg_removed_videos (original_id, filepath, size, model_used) VALUES (?, 'C:/seed_no_bg.webm', 150, 'bria-rmbg-1.4')",
                        seeded_video_id,
                    ),
                ] {
                    sqlx::query(sql)
                        .bind(original_id)
                        .execute(&pool)
                        .await
                        .unwrap_or_else(|e| panic!("seed `{sql}`: {e}"));
                }
            }

            let tables = [
                "videos",
                "images",
                "compressed_videos",
                "compressed_images",
                "converted_videos",
                "converted_images",
                "bg_removed_videos",
                "bg_removed_images",
                "upscaled_images",
                "settings",
                "filters",
                "notifications",
                "selections",
                "video_selections",
                "swatches",
            ];

            let mut before = Vec::new();
            for table in tables {
                let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {}", table))
                    .fetch_one(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("count {table}: {e}"));
                before.push((table, row.0));
            }
            let version_before: i64 = user_version(&pool).await;

            run_migrations_with(&pool, None)
                .await
                .expect("migrate real database");

            let version_after: i64 = user_version(&pool).await;
            println!("user_version: {} -> {}", version_before, version_after);
            for (table, count_before) in before {
                let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {}", table))
                    .fetch_one(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("recount {table}: {e}"));
                println!("{table}: {count_before} -> {}", row.0);
                if table == "settings" {
                    // v2 adds default keys via INSERT OR IGNORE, so it may grow.
                    assert!(
                        row.0 >= count_before,
                        "settings lost rows: {} -> {}",
                        count_before,
                        row.0
                    );
                } else {
                    assert_eq!(
                        row.0, count_before,
                        "row count changed for {table}: {} -> {}",
                        count_before, row.0
                    );
                }
            }
            assert_eq!(version_after, SCHEMA_VERSION);

            if seeded {
                // The old UNIQUE(original_id) must be gone after the rebuild.
                sqlx::query(
                    "INSERT INTO compressed_videos (original_id, filepath, size) VALUES (?, 'C:/seed_second.mp4', 280)",
                )
                .bind(seeded_video_id)
                .execute(&pool)
                .await
                .expect("second compressed variant");
                let variants: (i64,) =
                    sqlx::query_as("SELECT COUNT(*) FROM compressed_videos WHERE original_id = ?")
                        .bind(seeded_video_id)
                        .fetch_one(&pool)
                        .await
                        .expect("count variants");
                assert_eq!(variants.0, 2, "multiple variants must be allowed");
            }

            // Second start must be a no-op.
            run_migrations_with(&pool, None)
                .await
                .expect("second run");
            assert_eq!(user_version(&pool).await, SCHEMA_VERSION);
        });
    }
}
