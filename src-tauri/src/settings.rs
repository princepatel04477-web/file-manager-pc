//! Everything the Settings screen owns: the folders Sift never indexes, the
//! appearance choice, the scan schedule, the Windows autostart entry, and the
//! two caches Settings can inspect (the thumbnail cache and the folders the
//! indexer skipped).
//!
//! Preferences live in the same SQLite file as the index, so they are per-user,
//! per-machine, and never leave the device. The autostart entry is the one
//! setting Windows itself owns, so it is read back from the registry rather than
//! trusted from the database.

use crate::db::Database;
use crate::error::AppError;
use crate::indexer::IndexState;
use crate::ops;
use crate::thumbs;
use serde::Serialize;
use specta::Type;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const THEME_SYSTEM: &str = "system";
pub const THEME_LIGHT: &str = "light";
pub const THEME_DARK: &str = "dark";

pub const SCHEDULE_ON_LAUNCH: &str = "on_launch";
pub const SCHEDULE_DAILY: &str = "daily";
pub const SCHEDULE_WEEKLY: &str = "weekly";
pub const SCHEDULE_MANUAL: &str = "manual";

const THEME_KEY: &str = "theme";
const SCHEDULE_KEY: &str = "scan_schedule";
const LAST_SCAN_KEY: &str = "last_scan_unix";

/// A folder Sift will not index, search, or show as a result.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Exclusion {
    pub path: String,
    pub label: String,
    pub added_at_unix: i64,
}

/// A folder the indexer passed over, and why.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFolder {
    pub path: String,
    pub reason: String,
}

/// What the Settings screen reads and writes.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,
    /// Read from the Windows Run key, not from the database, so the toggle can
    /// never drift away from what Windows will actually do at sign-in.
    pub start_with_windows: bool,
    pub scan_schedule: String,
    pub last_scan_unix: Option<i64>,
    pub next_scan_unix: Option<i64>,
    pub exclusions: Vec<Exclusion>,
}

/// The two caches Settings can inspect and clear.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CacheReport {
    pub thumbnail_cache_path: String,
    pub thumbnail_cache_bytes: u64,
    pub thumbnail_cache_files: u64,
    pub skipped_folders: Vec<SkippedFolder>,
    /// True when more folders were skipped than the report keeps.
    pub skipped_folders_truncated: bool,
    pub skipped_total: u64,
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or_default()
}

fn normalize_theme(value: Option<String>) -> String {
    match value.as_deref() {
        Some(THEME_LIGHT) => THEME_LIGHT.to_owned(),
        Some(THEME_DARK) => THEME_DARK.to_owned(),
        _ => THEME_SYSTEM.to_owned(),
    }
}

fn normalize_schedule(value: Option<String>) -> String {
    match value.as_deref() {
        Some(SCHEDULE_DAILY) => SCHEDULE_DAILY.to_owned(),
        Some(SCHEDULE_WEEKLY) => SCHEDULE_WEEKLY.to_owned(),
        Some(SCHEDULE_MANUAL) => SCHEDULE_MANUAL.to_owned(),
        _ => SCHEDULE_ON_LAUNCH.to_owned(),
    }
}

fn interval_seconds(schedule: &str) -> Option<i64> {
    match schedule {
        SCHEDULE_DAILY => Some(24 * 60 * 60),
        SCHEDULE_WEEKLY => Some(7 * 24 * 60 * 60),
        _ => None,
    }
}

/// When the next full scan is due. `on_launch` scans at every start and `manual`
/// never scans on its own, so neither has a next date.
pub fn next_scan_unix(schedule: &str, last_scan_unix: Option<i64>) -> Option<i64> {
    let interval = interval_seconds(schedule)?;
    Some(last_scan_unix.unwrap_or_default().saturating_add(interval))
}

/// Whether the indexer should walk the profile at this launch. A profile that has
/// never been scanned is always due, whatever the schedule says.
pub fn scan_is_due(db: &Database, now: i64) -> bool {
    let schedule = normalize_schedule(db.setting(SCHEDULE_KEY).ok().flatten());
    let Some(last) = last_scan_unix(db) else { return true };
    match next_scan_unix(&schedule, Some(last)) {
        Some(due) => now >= due,
        // `on_launch` and `manual` have no interval: only `on_launch` scans here.
        None => schedule == SCHEDULE_ON_LAUNCH,
    }
}

pub fn last_scan_unix(db: &Database) -> Option<i64> {
    db.setting(LAST_SCAN_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
}

pub fn record_scan(db: &Database, now: i64) {
    let _ = db.set_setting(LAST_SCAN_KEY, &now.to_string());
}

fn exclusions(db: &Database) -> Vec<Exclusion> {
    db.exclusions()
        .unwrap_or_default()
        .into_iter()
        .map(|(path, label, added_at)| Exclusion { path, label, added_at_unix: added_at })
        .collect()
}

pub fn load(db: &Database) -> Settings {
    let schedule = normalize_schedule(db.setting(SCHEDULE_KEY).ok().flatten());
    let last_scan_unix = last_scan_unix(db);
    Settings {
        theme: normalize_theme(db.setting(THEME_KEY).ok().flatten()),
        start_with_windows: autostart::is_enabled(),
        next_scan_unix: next_scan_unix(&schedule, last_scan_unix),
        scan_schedule: schedule,
        last_scan_unix,
        exclusions: exclusions(db),
    }
}

pub fn set_theme(db: &Database, theme: &str) -> Result<Settings, String> {
    let theme = match theme {
        THEME_LIGHT => THEME_LIGHT,
        THEME_DARK => THEME_DARK,
        THEME_SYSTEM => THEME_SYSTEM,
        _ => return Err("Appearance can only be set to system, light, or dark.".to_owned()),
    };
    db.set_setting(THEME_KEY, theme)
        .map_err(|error| error.to_string())?;
    Ok(load(db))
}

pub fn set_scan_schedule(db: &Database, schedule: &str) -> Result<Settings, String> {
    let schedule = match schedule {
        SCHEDULE_ON_LAUNCH => SCHEDULE_ON_LAUNCH,
        SCHEDULE_DAILY => SCHEDULE_DAILY,
        SCHEDULE_WEEKLY => SCHEDULE_WEEKLY,
        SCHEDULE_MANUAL => SCHEDULE_MANUAL,
        _ => return Err("The scan schedule can only be on launch, daily, weekly, or manual.".to_owned()),
    };
    db.set_setting(SCHEDULE_KEY, schedule)
        .map_err(|error| error.to_string())?;
    Ok(load(db))
}

/// Excluding a folder drops whatever the index already holds for it, so the
/// choice takes effect in Browse straight away instead of at the next scan.
pub fn add_exclusion(state: &IndexState, path: &str) -> Result<Settings, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("Enter the full path of a folder to exclude.".to_owned());
    }
    let candidate = ops::normal_path(Path::new(path));
    if !candidate.is_absolute() {
        return Err("Use a full path, for example C:\\Users\\you\\Videos.".to_owned());
    }
    // Reuses the same profile boundary as every other command: the folder has to
    // exist, sit inside the current user's own roots, and not be a reparse point.
    if let Err(error) = ops::validate_path(&candidate, &ops::user_roots()) {
        return Err(
            match error {
                AppError::OutsideUserFiles => {
                    "Sift only manages folders inside your own user profile."
                }
                AppError::ReparsePoint => "That is a shortcut or link, not a real folder.",
                _ => "Sift could not find that folder. Check the path and try again.",
            }
            .to_owned(),
        );
    }
    let display = ops::display_path(&candidate);
    let label = ops::file_name(&candidate);
    let label = if label.is_empty() { display.clone() } else { label };
    state
        .db
        .add_exclusion(&display, &label, now_unix())
        .map_err(|error| error.to_string())?;
    state.db.remove_path(&candidate).map_err(|error| error.to_string())?;
    refresh_exclusions(state);
    Ok(load(&state.db))
}

pub fn remove_exclusion(state: &IndexState, path: &str) -> Result<Settings, String> {
    state
        .db
        .remove_exclusion(path.trim())
        .map_err(|error| error.to_string())?;
    refresh_exclusions(state);
    Ok(load(&state.db))
}

/// Push the stored list into the live indexer so the next folder it looks at
/// already knows about it.
pub fn refresh_exclusions(state: &IndexState) {
    state.set_excluded_paths(
        state
            .db
            .exclusions()
            .unwrap_or_default()
            .into_iter()
            .map(|(path, _, _)| exclusion_key(Path::new(&path)))
            .collect(),
    );
}

/// A comparable form of an excluded path: the platform key, always ending in a
/// separator so a prefix match also covers everything inside the folder.
pub fn exclusion_key(path: &Path) -> String {
    format!("{}\\", ops::path_key(path).trim_end_matches('\\'))
}

/// `%LOCALAPPDATA%\Sift\thumbs`, walked one shard at a time. The walk is
/// best-effort: a shard that cannot be read is counted as empty rather than
/// failing the whole report.
fn thumbnail_cache_usage() -> (u64, u64) {
    fn walk(directory: &Path, bytes: &mut u64, files: &mut u64) {
        let Ok(entries) = fs::read_dir(ops::io_path(directory)) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.is_dir() {
                walk(&path, bytes, files);
            } else {
                *bytes = bytes.saturating_add(metadata.len());
                *files = files.saturating_add(1);
            }
        }
    }
    let mut bytes = 0_u64;
    let mut files = 0_u64;
    walk(&thumbs::cache_root(), &mut bytes, &mut files);
    (bytes, files)
}

/// The cache is disposable, so clearing it is a directory removal. Thumbnail
/// generation recreates any shard it needs on the next request.
pub fn clear_thumbnail_cache() -> Result<(), String> {
    let root = thumbs::cache_root();
    if !root.exists() {
        return Ok(());
    }
    fs::remove_dir_all(ops::io_path(&root))
        .map_err(|error| format!("The thumbnail cache could not be cleared ({error})."))
}

pub fn cache_report(state: &IndexState) -> CacheReport {
    let (bytes, files) = thumbnail_cache_usage();
    let skipped_folders = state.skipped_folders();
    let skipped_total = state.skipped_folders_total();
    CacheReport {
        thumbnail_cache_path: ops::display_path(&thumbs::cache_root()),
        thumbnail_cache_bytes: bytes,
        thumbnail_cache_files: files,
        skipped_total,
        // The list is capped, so it is only the whole story while it is as long
        // as the count.
        skipped_folders_truncated: skipped_total > skipped_folders.len() as u64,
        skipped_folders: skipped_folders
            .into_iter()
            .map(|(path, reason)| SkippedFolder { path, reason })
            .collect(),
    }
}

/// Whether Windows is currently set to start Sift at sign-in.
pub fn start_with_windows() -> bool {
    autostart::is_enabled()
}

/// Ask Windows to start Sift at sign-in, or to stop. The Run key is per-user, so
/// this never needs elevation and never affects another account.
pub fn set_start_with_windows(enabled: bool) -> Result<(), String> {
    autostart::set_enabled(enabled)
}

/// Windows autostart, read and written through the per-user Run key so Sift never
/// asks for elevation and never touches another account.
#[cfg(windows)]
mod autostart {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
    };

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const VALUE_NAME: &str = "Sift";

    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Open the Run key for the current user and run `edit` against the handle.
    /// `None` means the key could not be opened at all.
    fn with_run_key<T>(access: REG_SAM_FLAGS, edit: impl FnOnce(HKEY) -> T) -> Option<T> {
        unsafe {
            let mut handle = HKEY::default();
            let key = to_wide(RUN_KEY);
            if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), None, access, &mut handle)
                .is_err()
            {
                return None;
            }
            let value = edit(handle);
            let _ = RegCloseKey(handle);
            Some(value)
        }
    }

    pub fn is_enabled() -> bool {
        with_run_key(KEY_READ, |handle| unsafe {
            let name = to_wide(VALUE_NAME);
            RegQueryValueExW(handle, PCWSTR(name.as_ptr()), None, None, None, None).is_ok()
        })
        .unwrap_or(false)
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        if enabled {
            let executable = std::env::current_exe().map_err(|_| {
                "Sift could not work out where it is installed, so it cannot start with Windows."
                    .to_owned()
            })?;
            // Quoted so a path containing spaces still starts the right program.
            let command = format!("\"{}\"", executable.display());
            let result = with_run_key(KEY_SET_VALUE, |handle| unsafe {
                let name = to_wide(VALUE_NAME);
                let data = command
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect::<Vec<u16>>();
                let bytes =
                    std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
                RegSetValueExW(
                    handle,
                    PCWSTR(name.as_ptr()),
                    None,
                    REG_SZ,
                    Some(bytes),
                    bytes.len() as u32,
                )
            });
            match result {
                Some(status) if status.is_ok() => Ok(()),
                _ => Err(
                    "Windows would not let Sift add itself to your startup list. You can add it manually in Task Manager > Startup apps."
                        .to_owned(),
                ),
            }
        } else {
            // Already off: Windows reports "value not found", which is the state asked for.
            if !is_enabled() {
                return Ok(());
            }
            let result = with_run_key(KEY_SET_VALUE, |handle| unsafe {
                let name = to_wide(VALUE_NAME);
                RegDeleteValueW(handle, PCWSTR(name.as_ptr()))
            });
            match result {
                Some(status) if status.is_ok() => Ok(()),
                _ => Err(
                    "Windows would not let Sift remove itself from your startup list. You can remove it manually in Task Manager > Startup apps."
                        .to_owned(),
                ),
            }
        }
    }
}

#[cfg(not(windows))]
mod autostart {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn set_enabled(_enabled: bool) -> Result<(), String> {
        Err("Starting with Windows is only available in the Windows app.".to_owned())
    }
}
