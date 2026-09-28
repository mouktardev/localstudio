mod connection;
mod migrations;

pub use connection::{create_pool, ensure_database, get_db_path};
pub use migrations::run_migrations;

use anyhow::{Context, Result};
use sqlx::SqlitePool;
use std::path::PathBuf;
use tauri::AppHandle;

pub fn get_db_url(app: &AppHandle) -> Result<String> {
    let path = get_db_path(app)?;
    path.into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("Database path contains invalid UTF-8"))
}

pub async fn init_db(app: &AppHandle) -> Result<(SqlitePool, PathBuf)> {
    let db_path = get_db_path(app)?;

    ensure_database(&db_path)?;
    let pool = create_pool(&db_path).await?;
    run_migrations(&pool, app).await?;

    println!("[DB] Initialized at: {:?}", db_path);

    Ok((pool, db_path))
}

pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .context("Failed to get setting")?;

    Ok(row.map(|r| r.0))
}

pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await
        .context("Failed to set setting")?;

    Ok(())
}
