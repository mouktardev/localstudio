use dunce::canonicalize;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::State;

use crate::DbState;

const SUPPORTED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "avif"];

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConvertedImage {
    pub filepath: String,
    pub size: Option<i64>,
    pub format: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Image {
    pub id: i64,
    pub filename: String,
    pub filepath: String,
    pub mimetype: Option<String>,
    pub size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub compressed_filepath: Option<String>,
    pub compressed_size: Option<i64>,
    #[serde(default)]
    pub upscaled_versions: String,  // JSON array of upscaled versions
    #[serde(default)]
    pub bg_removed_filepath: Option<String>,
    #[serde(default)]
    pub bg_removed_size: Option<i64>,
    #[serde(default)]
    pub converted_images: Vec<ConvertedImage>,
}

#[derive(Debug, Deserialize)]
pub struct ImageQueryParams {
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub sort_field: String, // 'name', 'size', 'date'
    #[serde(default)]
    pub sort_order: String, // 'asc', 'desc'
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AddImageData {
    pub filename: String,
    pub filepath: String,
    pub mimetype: Option<String>,
    pub size: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub width: u32,
    pub height: u32,
    pub size: u64,
    pub mimetype: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: i64,
    pub duplicates: i64,
    pub failed: i64,
}

#[tauri::command]
pub async fn get_all_images(
    state: State<'_, DbState>,
    params: Option<ImageQueryParams>,
) -> Result<Vec<Image>, String> {
    let pool = &state.0;

    let order_clause = if let Some(ref p) = params {
        let sort_field = match p.sort_field.as_str() {
            "name" => "i.filename",
            "size" => "i.size",
            _ => "i.id",
        };
        format!(
            "ORDER BY {} {}",
            sort_field,
            crate::crud::query::sort_direction(Some(p.sort_order.as_str()))
        )
    } else {
        "ORDER BY i.id DESC".to_string()
    };

    let search = params
        .as_ref()
        .and_then(|p| p.search.clone())
        .unwrap_or_default();
    let (search_clause, search_bind) = if search.trim().is_empty() {
        (String::new(), None)
    } else {
        (
            "WHERE LOWER(i.filename) LIKE ? ESCAPE '\\'".to_string(),
            Some(crate::crud::query::like_contains(&search)),
        )
    };

    let (limit_clause, limit_bind) = params
        .as_ref()
        .map(|p| crate::crud::query::limit_offset(p.limit, p.offset))
        .unwrap_or((String::new(), None));

    let query = format!(
        "SELECT 
            i.id, i.filename, i.filepath, i.mimetype, i.size, i.width, i.height,
            ci.filepath as compressed_filepath, ci.size as compressed_size,
            COALESCE(json_group_array(
                json_object(
                    'scale', u.scale_factor,
                    'filepath', u.filepath,
                    'size', u.size,
                    'model', u.model_used
                )
            ), '[]') as upscaled_versions,
            bi.filepath as bg_removed_filepath, bi.size as bg_removed_size
         FROM images i
         LEFT JOIN compressed_images ci ON ci.original_id = i.id
         LEFT JOIN upscaled_images u ON u.original_id = i.id
         LEFT JOIN bg_removed_images bi ON bi.original_id = i.id
         {}
         GROUP BY i.id
         {}{}",
        search_clause, order_clause, limit_clause
    );

    let mut q = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<i64>, String, Option<String>, Option<i64>)>(&query);
    if let Some(bind) = search_bind {
        q = q.bind(bind);
    }
    if let Some((limit, offset)) = limit_bind {
        q = q.bind(limit).bind(offset);
    }
    let rows = q.fetch_all(pool).await.map_err(|e| e.to_string())?;

    let conv_rows = sqlx::query_as::<_, (i64, String, Option<i64>, String)>(
        "SELECT original_id, filepath, size, format FROM converted_images ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut conv_map: std::collections::HashMap<i64, Vec<ConvertedImage>> =
        std::collections::HashMap::new();
    for (orig_id, fp, sz, fmt) in conv_rows {
        conv_map.entry(orig_id).or_default().push(ConvertedImage {
            filepath: fp,
            size: sz,
            format: fmt,
        });
    }

    let images: Vec<Image> = rows
        .into_iter()
        .map(|(id, filename, filepath, mimetype, size, width, height, compressed_filepath, compressed_size, upscaled_versions, bg_removed_filepath, bg_removed_size)| {
            let converted_images = conv_map.remove(&id).unwrap_or_default();
            Image {
                id, filename, filepath, mimetype, size, width, height,
                compressed_filepath, compressed_size,
                upscaled_versions,
                bg_removed_filepath,
                bg_removed_size,
                converted_images,
            }
        })
        .collect();

    Ok(images)
}

#[tauri::command]
pub async fn get_all_compressed_images(state: State<'_, DbState>) -> Result<Vec<Image>, String> {
    let pool = &state.0;

    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<i64>, String, Option<String>, Option<i64>)>(
        "SELECT 
            i.id, i.filename, ci.filepath, i.mimetype, ci.size, i.width, i.height,
            ci.filepath as compressed_filepath, ci.size as compressed_size,
            COALESCE(json_group_array(
                json_object(
                    'scale', u.scale_factor,
                    'filepath', u.filepath,
                    'size', u.size,
                    'model', u.model_used
                )
            ), '[]') as upscaled_versions,
            bi.filepath as bg_removed_filepath, bi.size as bg_removed_size
         FROM images i
         INNER JOIN compressed_images ci ON ci.original_id = i.id
         LEFT JOIN upscaled_images u ON u.original_id = i.id
         LEFT JOIN bg_removed_images bi ON bi.original_id = i.id
         GROUP BY i.id, ci.id
         ORDER BY ci.id DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let conv_rows = sqlx::query_as::<_, (i64, String, Option<i64>, String)>(
        "SELECT original_id, filepath, size, format FROM converted_images ORDER BY id DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut conv_map: std::collections::HashMap<i64, Vec<ConvertedImage>> = std::collections::HashMap::new();
    for (orig_id, fp, sz, fmt) in conv_rows {
        conv_map.entry(orig_id).or_default().push(ConvertedImage { filepath: fp, size: sz, format: fmt });
    }

    let images: Vec<Image> = rows
        .into_iter()
        .map(|(id, filename, filepath, mimetype, size, width, height, compressed_filepath, compressed_size, upscaled_versions, bg_removed_filepath, bg_removed_size)| {
            let converted_images = conv_map.remove(&id).unwrap_or_default();
            Image { 
                id, filename, filepath, mimetype, size, width, height, 
                compressed_filepath, compressed_size,
                upscaled_versions,
                bg_removed_filepath,
                bg_removed_size,
                converted_images,
            }
        })
        .collect();

    Ok(images)
}

#[tauri::command]
pub async fn get_all_converted_images(state: State<'_, DbState>) -> Result<Vec<Image>, String> {
    let pool = &state.0;

    let rows = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<i64>, String, Option<String>, Option<i64>)>(
        "SELECT 
            i.id, i.filename, i.filepath, i.mimetype, i.size, i.width, i.height,
            ci.filepath as compressed_filepath, ci.size as compressed_size,
            COALESCE(json_group_array(
                json_object(
                    'scale', u.scale_factor,
                    'filepath', u.filepath,
                    'size', u.size,
                    'model', u.model_used
                )
            ), '[]') as upscaled_versions,
            bi.filepath as bg_removed_filepath, bi.size as bg_removed_size
         FROM images i
         INNER JOIN converted_images cvi ON cvi.original_id = i.id
         LEFT JOIN compressed_images ci ON ci.original_id = i.id
         LEFT JOIN upscaled_images u ON u.original_id = i.id
         LEFT JOIN bg_removed_images bi ON bi.original_id = i.id
         GROUP BY i.id
         ORDER BY MAX(cvi.id) DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let conv_rows = sqlx::query_as::<_, (i64, String, Option<i64>, String)>(
        "SELECT original_id, filepath, size, format FROM converted_images ORDER BY id DESC"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut conv_map: std::collections::HashMap<i64, Vec<ConvertedImage>> = std::collections::HashMap::new();
    for (orig_id, fp, sz, fmt) in conv_rows {
        conv_map.entry(orig_id).or_default().push(ConvertedImage { filepath: fp, size: sz, format: fmt });
    }

    let images: Vec<Image> = rows
        .into_iter()
        .map(|(id, filename, filepath, mimetype, size, width, height, compressed_filepath, compressed_size, upscaled_versions, bg_removed_filepath, bg_removed_size)| {
            let converted_images = conv_map.remove(&id).unwrap_or_default();
            Image { 
                id, filename, filepath, mimetype, size, width, height, 
                compressed_filepath, compressed_size,
                upscaled_versions,
                bg_removed_filepath,
                bg_removed_size,
                converted_images,
            }
        })
        .collect();

    Ok(images)
}

#[tauri::command]
pub async fn add_image(data: AddImageData, state: State<'_, DbState>) -> Result<Image, String> {
    let path = PathBuf::from(&data.filepath);
    let canonical_path = canonicalize(&path)
        .map_err(|e| format!("Failed to canonicalize path: {}", e))?;
    let filepath = canonical_path
        .to_str()
        .map(|s| s.to_string())
        .unwrap_or(data.filepath);

    let result = sqlx::query(
        "INSERT INTO images (filename, filepath, mimetype, size, width, height) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&data.filename)
    .bind(&filepath)
    .bind(&data.mimetype)
    .bind(data.size)
    .bind(data.width)
    .bind(data.height)
    .execute(&state.0)
    .await
    .map_err(|e| e.to_string())?;

    let id = result.last_insert_rowid();

    Ok(Image {
        id,
        filename: data.filename,
        filepath,
        mimetype: data.mimetype,
        size: data.size,
        width: data.width,
        height: data.height,
        compressed_filepath: None,
        compressed_size: None,
        upscaled_versions: "[]".to_string(),
        bg_removed_filepath: None,
        bg_removed_size: None,
        converted_images: vec![],
    })
}

#[tauri::command]
pub async fn import_images_bulk(
    filepaths: Vec<String>,
    state: State<'_, DbState>,
) -> Result<ImportResult, String> {
    let mut imported: i64 = 0;
    let mut duplicates: i64 = 0;
    let mut failed: i64 = 0;

    // Prepare all image data before touching the DB
    struct ImageRow {
        filename: String,
        filepath: String,
        mimetype: String,
        size: i64,
        width: i64,
        height: i64,
        sha256: Option<String>,
    }

    let mut rows: Vec<ImageRow> = Vec::with_capacity(filepaths.len());

    for filepath in &filepaths {
        let path = PathBuf::from(filepath);

        // Validate file extension first
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        if let Some(extension) = ext.as_deref() {
            if !SUPPORTED_IMAGE_EXTENSIONS.contains(&extension) {
                failed += 1;
                continue;
            }
        } else {
            // No extension - skip
            failed += 1;
            continue;
        }

        let canonical_path = match canonicalize(&path) {
            Ok(p) => p,
            Err(_) => { failed += 1; continue; }
        };

        let filename = canonical_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mimetype = match ext.as_deref() {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            Some("tiff") | Some("tif") => "image/tiff",
            Some("avif") => "image/avif",
            _ => "application/octet-stream",
        }.to_string();

        let size = match fs::metadata(&canonical_path) {
            Ok(m) => m.len() as i64,
            Err(_) => 0,
        };

        // imagesize only reads the image header - much faster than full decode
        let (width, height) = match imagesize::size(&canonical_path) {
            Ok(dim) => (dim.width as i64, dim.height as i64),
            Err(_) => (0, 0),
        };

        let fp = canonical_path
            .to_str()
            .unwrap_or(filepath)
            .to_string();

        let sha256 = crate::crud::hashing::hash_file(&canonical_path)
            .ok()
            .filter(|hash| !hash.is_empty());

        rows.push(ImageRow { filename, filepath: fp, mimetype, size, width, height, sha256 });
    }

    // Bulk insert in a single transaction
    let mut tx = state.0.begin().await.map_err(|e| e.to_string())?;

    let mut seen_hashes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for row in &rows {
        // Content duplicate: same bytes under a different name.
        if let Some(hash) = row.sha256.as_ref() {
            if !seen_hashes.insert(hash.clone()) {
                duplicates += 1;
                continue;
            }
            let existing: Option<(i64,)> = sqlx::query_as("SELECT id FROM images WHERE sha256 = ?")
                .bind(hash)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            if existing.is_some() {
                duplicates += 1;
                continue;
            }
        }

        let result = sqlx::query(
            "INSERT OR IGNORE INTO images (filename, filepath, mimetype, size, width, height, sha256, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))"
        )
        .bind(&row.filename)
        .bind(&row.filepath)
        .bind(&row.mimetype)
        .bind(row.size)
        .bind(row.width)
        .bind(row.height)
        .bind(&row.sha256)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

        if result.rows_affected() == 0 {
            duplicates += 1;
        } else {
            imported += 1;
        }
    }

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(ImportResult { imported, duplicates, failed })
}

#[tauri::command]
pub async fn delete_image(id: i64, state: State<'_, DbState>) -> Result<(), String> {
    // Single delete path; library files are kept (delete_files = false).
    crate::crud::lifecycle::delete_items_impl(
        &state.0,
        crate::crud::lifecycle::ItemKind::Image,
        &[id],
        false,
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn delete_images_by_ids(ids: Vec<i64>, state: State<'_, DbState>) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }

    // Single delete path; library files are kept (delete_files = false).
    crate::crud::lifecycle::delete_items_impl(
        &state.0,
        crate::crud::lifecycle::ItemKind::Image,
        &ids,
        false,
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn get_image_by_id(
    id: i64,
    state: State<'_, DbState>,
) -> Result<Image, String> {
    let pool = &state.0;

    let row = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<i64>, String, Option<String>, Option<i64>)>(
        "SELECT 
            i.id, i.filename, i.filepath, i.mimetype, i.size, i.width, i.height,
            ci.filepath as compressed_filepath, ci.size as compressed_size,
            COALESCE(json_group_array(
                json_object(
                    'scale', u.scale_factor,
                    'filepath', u.filepath,
                    'size', u.size,
                    'model', u.model_used
                )
            ), '[]') as upscaled_versions,
            bi.filepath as bg_removed_filepath, bi.size as bg_removed_size
         FROM images i
         LEFT JOIN compressed_images ci ON ci.original_id = i.id
         LEFT JOIN upscaled_images u ON u.original_id = i.id
         LEFT JOIN bg_removed_images bi ON bi.original_id = i.id
         WHERE i.id = ?
         GROUP BY i.id"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "Image not found".to_string())?;

    let (id, filename, filepath, mimetype, size, width, height, compressed_filepath, compressed_size, upscaled_versions, bg_removed_filepath, bg_removed_size) = row;

    let conv_rows = sqlx::query_as::<_, (String, Option<i64>, String)>(
        "SELECT filepath, size, format FROM converted_images WHERE original_id = ? ORDER BY id DESC"
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let converted_images: Vec<ConvertedImage> = conv_rows
        .into_iter()
        .map(|(fp, sz, fmt)| ConvertedImage { filepath: fp, size: sz, format: fmt })
        .collect();

    Ok(Image {
        id,
        filename,
        filepath,
        mimetype,
        size,
        width,
        height,
        compressed_filepath,
        compressed_size,
        upscaled_versions,
        bg_removed_filepath,
        bg_removed_size,
        converted_images,
    })
}

/// Backfill/repair the thumbnail cache for every image (self-healing).
/// Backfill/repair the thumbnail cache. Pass `ids` to limit it to just-imported
/// rows. Generation runs with bounded concurrency so it never pins the CPU.
#[tauri::command]
pub async fn get_image_metadata(filepath: String) -> Result<ImageMetadata, String> {
    let path = PathBuf::from(&filepath);
    let canonical_path = canonicalize(&path)
        .map_err(|e| format!("Failed to canonicalize path: {}", e))?;

    let metadata = fs::metadata(&canonical_path).map_err(|e| e.to_string())?;
    let size = metadata.len();

    let dim = imagesize::size(&canonical_path).map_err(|e| e.to_string())?;

    let mimetype = match canonical_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("tiff") | Some("tif") => "image/tiff",
        Some("avif") => "image/avif",
        _ => "application/octet-stream",
    }
    .to_string();

    Ok(ImageMetadata {
        width: dim.width as u32,
        height: dim.height as u32,
        size,
        mimetype,
    })
}
