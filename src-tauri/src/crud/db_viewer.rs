use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, State};

use crate::DbState;

/// Read-only whitelist of tables the viewer may expose.
const TABLES: &[&str] = &[
    "images",
    "videos",
    "compressed_images",
    "compressed_videos",
    "converted_images",
    "converted_videos",
    "upscaled_images",
    "bg_removed_images",
    "bg_removed_videos",
    "settings",
    "notifications",
    "filters",
    "selections",
    "video_selections",
    "swatches",
];

#[derive(Debug, Clone, Serialize)]
pub struct DbTableInfo {
    pub name: String,
    pub rows: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbOverview {
    pub path: String,
    pub schema_version: i64,
    pub tables: Vec<DbTableInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbTableRows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub total: i64,
}

/// PRAGMA table_info columns: cid, name, type, notnull, dflt_value, pk.
type TableInfoRow = (i64, String, String, i64, Option<String>, i64);

fn validate_table(table: &str) -> Result<(), String> {
    if TABLES.contains(&table) {
        Ok(())
    } else {
        Err(format!("Unknown table: '{}'", table))
    }
}

fn cell(row: &sqlx::sqlite::SqliteRow, index: usize) -> serde_json::Value {
    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return value.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null);
    }
    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return value
            .map(|v| serde_json::json!(v))
            .unwrap_or(serde_json::Value::Null);
    }
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return value
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null);
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return value
            .map(|bytes| serde_json::Value::from(format!("<{} bytes>", bytes.len())))
            .unwrap_or(serde_json::Value::Null);
    }
    serde_json::Value::Null
}

async fn table_count(pool: &SqlitePool, table: &str) -> Result<i64, String> {
    sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {}", table))
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn db_overview(app: AppHandle, state: State<'_, DbState>) -> Result<DbOverview, String> {
    let path = crate::db::get_db_path(&app)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let schema_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&state.0)
        .await
        .map_err(|e| e.to_string())?;

    let mut tables = Vec::with_capacity(TABLES.len());
    for table in TABLES {
        let rows = table_count(&state.0, table).await?;
        tables.push(DbTableInfo {
            name: (*table).to_string(),
            rows,
        });
    }

    Ok(DbOverview {
        path,
        schema_version,
        tables,
    })
}

#[tauri::command]
pub async fn db_table_rows(
    state: State<'_, DbState>,
    table: String,
    limit: i64,
    offset: i64,
) -> Result<DbTableRows, String> {
    validate_table(&table)?;
    let limit = limit.clamp(1, 500);
    let offset = offset.max(0);

    let info: Vec<TableInfoRow> = sqlx::query_as(&format!("PRAGMA table_info({})", table))
            .fetch_all(&state.0)
            .await
            .map_err(|e| e.to_string())?;
    let columns: Vec<String> = info.into_iter().map(|row| row.1).collect();

    let query = format!("SELECT * FROM {} LIMIT ? OFFSET ?", table);
    let sql_rows = sqlx::query(&query)
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.0)
        .await
        .map_err(|e| e.to_string())?;

    let rows: Vec<Vec<serde_json::Value>> = sql_rows
        .iter()
        .map(|row| (0..columns.len()).map(|i| cell(row, i)).collect())
        .collect();

    Ok(DbTableRows {
        columns,
        rows,
        total: table_count(&state.0, &table).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_table() {
        assert!(validate_table("users; DROP TABLE images").is_err());
        assert!(validate_table("not_a_table").is_err());
        assert!(validate_table("images").is_ok());
    }
}
