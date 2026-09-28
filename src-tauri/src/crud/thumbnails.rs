//! Thumbnail cache (used for video posters).
//!
//! Files live in one app-owned cache directory and are keyed by content hash
//! (the `sha256` computed at import), falling back to size+mtime. Identical
//! content therefore shares one thumbnail, and a changed source regenerates.

use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const THUMB_EXTENSIONS: &[&str] = &["jpg", "png", "webp"];

/// One cache directory for dev and release.
pub fn cache_dir(app: &AppHandle) -> PathBuf {
    let base = app
        .path()
        .app_cache_dir()
        .or_else(|_| app.path().app_data_dir())
        .unwrap_or_else(|_| PathBuf::from("."));
    let dir = base.join("thumbnails");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Stable, filesystem-safe key for a source file.
pub fn key_for(sha256: Option<&str>, path: &Path, id: i64) -> String {
    if let Some(hash) = sha256.filter(|h| !h.is_empty()) {
        return hash.replace(':', "_");
    }
    if let Ok(meta) = fs::metadata(path) {
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return format!("s{}_{}", size, mtime);
    }
    format!("id{}", id)
}

pub fn target_base(dir: &Path, key: &str) -> PathBuf {
    dir.join(key)
}

/// A cached thumbnail for this key, whichever extension it was written with.
pub fn existing(dir: &Path, key: &str) -> Option<PathBuf> {
    THUMB_EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{}.{}", key, ext)))
        .find(|p| p.is_file())
}

/// Any file the cache may own (used for orphan/deletion guards).
pub fn is_thumbnail_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| THUMB_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_prefers_content_hash() {
        let p = Path::new("does-not-exist");
        assert_eq!(key_for(Some("abc:def"), p, 1), "abc_def");
        assert_eq!(key_for(Some(""), p, 1), "id1");
    }
}
