use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

/// Identifies the current app run so the log panel can show only this session.
pub static SESSION_ID: OnceLock<String> = OnceLock::new();

/// Called once at startup; the returned id is logged as a `[session] <id>`
/// marker that `read_app_log` uses to slice off earlier runs.
pub fn start_session() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let id = format!("{}-{}", std::process::id(), millis);
    let _ = SESSION_ID.set(id.clone());
    id
}

/// The tail of the current app log file — the same text that is written to the
/// terminal — so the log panel can show startup and pre-attach logs too.
#[derive(Debug, Clone, Serialize)]
pub struct AppLog {
    pub path: String,
    pub lines: Vec<String>,
}

#[tauri::command]
pub async fn read_app_log(
    app: AppHandle,
    max_lines: usize,
    all_sessions: bool,
) -> Result<AppLog, String> {
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let dir_str = dir.to_string_lossy().to_string();

    let mut newest: Option<(SystemTime, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_log = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("log"))
                .unwrap_or(false);
            if !is_log {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            if newest.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
                newest = Some((modified, path));
            }
        }
    }

    let Some((_, path)) = newest else {
        return Ok(AppLog {
            path: dir_str,
            lines: Vec::new(),
        });
    };

    let content = fs::read_to_string(&path).unwrap_or_default();
    let all: Vec<&str> = content.lines().collect();

    // Only this run's lines: slice from the last `[session] <id>` marker.
    // `all_sessions` bypasses the slice for debugging crashes from earlier runs.
    let session_start = if all_sessions {
        0
    } else {
        SESSION_ID
            .get()
            .and_then(|id| {
                let marker = format!("[session] {}", id);
                all.iter().rposition(|line| line.contains(&marker))
            })
            .unwrap_or(0)
    };
    let session_lines = &all[session_start..];

    let max = max_lines.clamp(1, 5000);
    let start = session_lines.len().saturating_sub(max);
    let lines = session_lines[start..].iter().map(|s| s.to_string()).collect();

    Ok(AppLog {
        path: path.to_string_lossy().to_string(),
        lines,
    })
}
