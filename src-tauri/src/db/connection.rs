use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager};

const BUSY_TIMEOUT_MS: u64 = 5_000;

pub fn get_db_path(_app: &AppHandle) -> Result<PathBuf> {
    let path = if cfg!(debug_assertions) {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::current_dir().expect("Failed to get current directory")
            });
        let project_root = manifest_dir.parent().unwrap_or(&manifest_dir);
        project_root.join("data").join("db.sqlite")
    } else {
        let app_data_dir = _app
            .path()
            .app_data_dir()
            .context("Failed to get app data directory")?;
        app_data_dir.join("db.sqlite")
    };

    Ok(path)
}

pub fn ensure_database(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .context("Failed to create database directory")?;
        }
    }

    if !path.exists() {
        fs::File::create(path).context("Failed to create database file")?;
    }

    Ok(())
}

pub async fn create_pool(db_path: &Path) -> Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS))
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .context("Failed to create database pool")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_keys_cascade_deletes_children() {
        tauri::async_runtime::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let pool = create_pool(&dir.path().join("cascade.sqlite")).await.expect("pool");

            sqlx::query("CREATE TABLE parent (id INTEGER PRIMARY KEY NOT NULL)")
                .execute(&pool)
                .await
                .expect("parent table");
            sqlx::query(
                "CREATE TABLE child (
                    id INTEGER PRIMARY KEY NOT NULL,
                    parent_id INTEGER NOT NULL,
                    FOREIGN KEY (parent_id) REFERENCES parent(id) ON DELETE CASCADE
                )",
            )
            .execute(&pool)
            .await
            .expect("child table");

            sqlx::query("INSERT INTO parent (id) VALUES (1)")
                .execute(&pool)
                .await
                .expect("insert parent");
            sqlx::query("INSERT INTO child (id, parent_id) VALUES (1, 1)")
                .execute(&pool)
                .await
                .expect("insert child");

            sqlx::query("DELETE FROM parent WHERE id = 1")
                .execute(&pool)
                .await
                .expect("delete parent");

            let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM child")
                .fetch_one(&pool)
                .await
                .expect("count children");
            assert_eq!(remaining.0, 0, "ON DELETE CASCADE must remove children");

            let foreign_keys: (i64,) = sqlx::query_as("PRAGMA foreign_keys")
                .fetch_one(&pool)
                .await
                .expect("pragma foreign_keys");
            assert_eq!(foreign_keys.0, 1);
        });
    }

    #[test]
    fn pragmas_are_applied() {
        tauri::async_runtime::block_on(async {
            let dir = tempfile::tempdir().expect("tempdir");
            let pool = create_pool(&dir.path().join("pragmas.sqlite")).await.expect("pool");

            let journal: (String,) = sqlx::query_as("PRAGMA journal_mode")
                .fetch_one(&pool)
                .await
                .expect("journal_mode");
            assert_eq!(journal.0.to_lowercase(), "wal");

            let busy: (i64,) = sqlx::query_as("PRAGMA busy_timeout")
                .fetch_one(&pool)
                .await
                .expect("busy_timeout");
            assert_eq!(busy.0, BUSY_TIMEOUT_MS as i64);

            let sync: (i64,) = sqlx::query_as("PRAGMA synchronous")
                .fetch_one(&pool)
                .await
                .expect("synchronous");
            assert_eq!(sync.0, 1, "synchronous=NORMAL is 1");
        });
    }
}
