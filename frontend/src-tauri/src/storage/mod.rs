//! Motiv: where recordings are stored, and automatic expiration of old
//! recording audio so it doesn't fill the local disk (or a synced cloud
//! folder such as iCloud Drive, OneDrive or Google Drive).
//!
//! Expiration only deletes audio files inside meeting folders Motiv knows
//! about (from the database). Transcripts, summaries, chat, action items and
//! the meeting folder's metadata are kept. The folder of a recording in
//! progress is never touched.

use crate::state::AppState;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Manager, Runtime};

const AUDIO_EXTENSIONS: &[&str] = &["mp4", "m4a", "wav", "mp3", "aac", "ogg", "webm", "flac", "opus"];
const CLEANUP_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const GB: f64 = 1024.0 * 1024.0 * 1024.0;

// ---------------------------------------------------------------------------
// Recordings folder
// ---------------------------------------------------------------------------

static ROOT: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));

/// Remembers the folder chosen in Recording settings (empty = default).
pub fn set_recordings_root(path: &Path) {
    let p = (!path.as_os_str().is_empty()).then(|| path.to_path_buf());
    *ROOT.write().unwrap() = p;
}

/// Folder new recordings go into: the one chosen in settings, or the default.
pub fn recordings_root() -> PathBuf {
    ROOT.read()
        .unwrap()
        .clone()
        .unwrap_or_else(crate::audio::recording_preferences::get_default_recordings_folder)
}

// ---------------------------------------------------------------------------
// Retention settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    /// Delete recording audio older than this many days. None = keep forever.
    pub max_age_days: Option<u32>,
    /// Keep total recording audio under this many GB, oldest deleted first. None = no cap.
    pub max_total_gb: Option<f64>,
}

impl Retention {
    pub fn is_active(&self) -> bool {
        self.max_age_days.is_some() || self.max_total_gb.is_some()
    }
}

fn retention_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("storage.json"))
}

pub fn load_retention<R: Runtime>(app: &AppHandle<R>) -> Retention {
    retention_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Planning (pure, unit tested)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct AudioFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}

/// Indexes of `files` to delete: everything older than the age limit, then
/// the oldest remaining files until the total fits the size cap.
pub fn plan(files: &[AudioFile], now: SystemTime, r: &Retention) -> Vec<usize> {
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by_key(|&i| files[i].modified);

    let mut delete: Vec<usize> = Vec::new();
    if let Some(days) = r.max_age_days {
        let limit = Duration::from_secs(days as u64 * 86_400);
        for &i in &order {
            if now.duration_since(files[i].modified).unwrap_or_default() > limit {
                delete.push(i);
            }
        }
    }
    if let Some(gb) = r.max_total_gb {
        let cap = (gb.max(0.0) * GB) as u64;
        let mut total: u64 = order.iter().filter(|i| !delete.contains(i)).map(|&i| files[i].bytes).sum();
        for &i in &order {
            if total <= cap {
                break;
            }
            if !delete.contains(&i) {
                delete.push(i);
                total = total.saturating_sub(files[i].bytes);
            }
        }
    }
    delete.sort_unstable();
    delete
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Audio files directly inside the given meeting folders.
pub fn scan(folders: &[PathBuf], skip: &HashSet<PathBuf>) -> Vec<AudioFile> {
    let mut out = Vec::new();
    for folder in folders {
        if skip.contains(folder) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(folder) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_file() && is_audio(&path) {
                out.push(AudioFile { path, bytes: meta.len(), modified: meta.modified().unwrap_or(SystemTime::now()) });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct StorageReport {
    pub root: String,
    pub default_root: String,
    pub retention: Retention,
    pub recordings: usize,
    pub total_bytes: u64,
    pub oldest: Option<String>,
    /// What a cleanup would delete now under the current rules.
    pub would_delete: usize,
    pub would_free_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupResult {
    pub deleted: usize,
    pub freed_bytes: u64,
    pub failed: usize,
}

async fn meeting_folders(pool: &SqlitePool) -> Vec<PathBuf> {
    sqlx::query_as::<_, (String,)>("SELECT folder_path FROM meetings WHERE folder_path IS NOT NULL AND folder_path != ''")
        .fetch_all(pool)
        .await
        .map(|rows| rows.into_iter().map(|(p,)| PathBuf::from(p)).collect())
        .unwrap_or_default()
}

async fn protected_folders() -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    if let Ok(Some(p)) = crate::audio::recording_commands::get_meeting_folder_path().await {
        set.insert(PathBuf::from(p));
    }
    set
}

async fn current_files(pool: &SqlitePool) -> Vec<AudioFile> {
    let folders = meeting_folders(pool).await;
    let skip = protected_folders().await;
    tokio::task::spawn_blocking(move || scan(&folders, &skip)).await.unwrap_or_default()
}

fn fmt_time(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d").to_string()
}

#[tauri::command]
pub async fn storage_report<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<StorageReport, String> {
    let retention = load_retention(&app);
    let files = current_files(state.db_manager.pool()).await;
    let doomed = plan(&files, SystemTime::now(), &retention);
    Ok(StorageReport {
        root: recordings_root().to_string_lossy().into_owned(),
        default_root: crate::audio::recording_preferences::get_default_recordings_folder().to_string_lossy().into_owned(),
        recordings: files.len(),
        total_bytes: files.iter().map(|f| f.bytes).sum(),
        oldest: files.iter().map(|f| f.modified).min().map(fmt_time),
        would_delete: doomed.len(),
        would_free_bytes: doomed.iter().map(|&i| files[i].bytes).sum(),
        retention,
    })
}

#[tauri::command]
pub async fn storage_save_retention<R: Runtime>(app: AppHandle<R>, retention: Retention) -> Result<(), String> {
    let path = retention_path(&app).ok_or("No config folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&retention).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Applies the retention rules now.
#[tauri::command]
pub async fn storage_cleanup_now<R: Runtime>(app: AppHandle<R>, state: tauri::State<'_, AppState>) -> Result<CleanupResult, String> {
    Ok(cleanup(&app, state.db_manager.pool()).await)
}

async fn cleanup<R: Runtime>(app: &AppHandle<R>, pool: &SqlitePool) -> CleanupResult {
    let retention = load_retention(app);
    let mut result = CleanupResult { deleted: 0, freed_bytes: 0, failed: 0 };
    if !retention.is_active() {
        return result;
    }
    let files = current_files(pool).await;
    for i in plan(&files, SystemTime::now(), &retention) {
        let f = &files[i];
        match std::fs::remove_file(&f.path) {
            Ok(()) => {
                result.deleted += 1;
                result.freed_bytes += f.bytes;
            }
            Err(e) => {
                result.failed += 1;
                warn!("storage: could not delete expired recording: {}", e);
            }
        }
    }
    if result.deleted > 0 || result.failed > 0 {
        info!(
            "storage: expired {} recording(s), freed {:.1} MB, {} failed",
            result.deleted,
            result.freed_bytes as f64 / 1_048_576.0,
            result.failed
        );
    }
    result
}

/// Opens a folder picker and returns the chosen folder (None if cancelled).
#[tauri::command]
pub async fn storage_pick_folder<R: Runtime>(app: AppHandle<R>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Choose where to save recordings").pick_folder(move |f| {
        let _ = tx.send(f);
    });
    let picked = rx.await.map_err(|e| e.to_string())?;
    Ok(picked.and_then(|f| f.into_path().ok()).map(|p| p.to_string_lossy().into_owned()))
}

/// Loads the chosen recordings folder and runs expiration on startup and
/// every few hours. Call once from app setup.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        if let Ok(prefs) = crate::audio::recording_preferences::load_recording_preferences(&app).await {
            set_recordings_root(&prefs.save_folder);
        }
        // Give the database a moment to finish migrating on first launch.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                cleanup(&app, state.db_manager.pool()).await;
            }
            tokio::time::sleep(CLEANUP_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, mb: u64, days_ago: u64, now: SystemTime) -> AudioFile {
        AudioFile {
            path: PathBuf::from(name),
            bytes: mb * 1_048_576,
            modified: now - Duration::from_secs(days_ago * 86_400),
        }
    }

    #[test]
    fn no_rules_deletes_nothing() {
        let now = SystemTime::now();
        let files = vec![file("a", 100, 400, now)];
        assert!(plan(&files, now, &Retention::default()).is_empty());
    }

    #[test]
    fn age_rule() {
        let now = SystemTime::now();
        let files = vec![file("new", 1, 2, now), file("old", 1, 40, now), file("edge", 1, 30, now)];
        let r = Retention { max_age_days: Some(30), max_total_gb: None };
        assert_eq!(plan(&files, now, &r), vec![1]);
    }

    #[test]
    fn size_cap_deletes_oldest_first() {
        let now = SystemTime::now();
        // 3 x 512 MB = 1.5 GB; cap 1 GB -> delete the oldest one only.
        let files = vec![file("mid", 512, 5, now), file("oldest", 512, 9, now), file("newest", 512, 1, now)];
        let r = Retention { max_age_days: None, max_total_gb: Some(1.0) };
        assert_eq!(plan(&files, now, &r), vec![1]);
    }

    #[test]
    fn both_rules_combine_without_double_counting() {
        let now = SystemTime::now();
        let files = vec![file("old", 600, 100, now), file("b", 600, 10, now), file("c", 600, 3, now)];
        let r = Retention { max_age_days: Some(30), max_total_gb: Some(1.0) };
        // "old" goes by age; the remaining 1.2 GB still exceeds 1 GB, so "b" goes too.
        assert_eq!(plan(&files, now, &r), vec![0, 1]);
    }

    #[test]
    fn scan_finds_only_audio_and_skips_protected() {
        let dir = std::env::temp_dir().join(format!("motiv-storage-test-{}", std::process::id()));
        let (a, b) = (dir.join("m1"), dir.join("m2"));
        std::fs::create_dir_all(a.join(".checkpoints")).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("audio.mp4"), b"x").unwrap();
        std::fs::write(a.join("metadata.json"), b"{}").unwrap();
        std::fs::write(a.join(".checkpoints").join("c.mp4"), b"x").unwrap();
        std::fs::write(b.join("audio.MP4"), b"xx").unwrap();

        let all = scan(&[a.clone(), b.clone(), dir.join("missing")], &HashSet::new());
        assert_eq!(all.len(), 2);
        let skip: HashSet<PathBuf> = [b.clone()].into_iter().collect();
        let some = scan(&[a.clone(), b], &skip);
        assert_eq!(some.len(), 1);
        assert_eq!(some[0].path, a.join("audio.mp4"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn root_falls_back_to_default() {
        set_recordings_root(Path::new(""));
        assert_eq!(recordings_root(), crate::audio::recording_preferences::get_default_recordings_folder());
        set_recordings_root(Path::new("/tmp/elsewhere"));
        assert_eq!(recordings_root(), PathBuf::from("/tmp/elsewhere"));
        set_recordings_root(Path::new(""));
    }
}
