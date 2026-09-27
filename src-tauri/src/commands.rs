use crate::db::{CategorySummary, FavoriteItem, IndexedEntry, RecentItem, SearchFilter};
use crate::error::AppError;
use crate::indexer::{self, IndexProgress, IndexState};
use crate::ops;
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::Disks;
use tauri::State;

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DriveStorage {
    pub drive: String,
    pub root: String,
    pub used: u64,
    pub free: u64,
    pub total: u64,
    pub indexed_size: u64,
}

#[tauri::command]
#[specta::specta]
pub fn get_index_status(state: State<'_, IndexState>) -> Result<IndexProgress, String> {
    Ok(state.progress())
}

#[tauri::command]
#[specta::specta]
pub fn get_category_summary(state: State<'_, IndexState>) -> Result<Vec<CategorySummary>, String> {
    state.db.categories().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn get_drive_storage(state: State<'_, IndexState>) -> Result<Vec<DriveStorage>, String> {
    let supported = indexer::discover_fixed_and_removable_drives()
        .into_iter()
        .map(|drive| (drive.label.to_ascii_lowercase(), ops::display_path(&drive.path)))
        .collect::<HashMap<_, _>>();
    let indexed = state.db.drive_sizes().map_err(|error| error.to_string())?;
    let indexed = indexed
        .into_iter()
        .map(|(drive, size)| (drive.to_ascii_lowercase(), size))
        .collect::<HashMap<_, _>>();
    let disks = Disks::new_with_refreshed_list();
    let mut output = Vec::new();
    for disk in &disks {
        let root_path = disk.mount_point();
        let drive = ops::drive_for(root_path);
        if !supported.contains_key(&drive.to_ascii_lowercase()) { continue; }
        let total = disk.total_space();
        let free = disk.available_space().min(total);
        output.push(DriveStorage {
            drive: drive.clone(),
            root: ops::display_path(root_path),
            used: total.saturating_sub(free),
            free,
            total,
            indexed_size: indexed.get(&drive.to_ascii_lowercase()).copied().unwrap_or(0),
        });
    }
    output.sort_by(|left, right| left.drive.to_ascii_lowercase().cmp(&right.drive.to_ascii_lowercase()));
    Ok(output)
}

#[tauri::command]
#[specta::specta]
pub fn list_index_directory(
    path: String,
    sort: String,
    descending: bool,
    limit: u32,
    offset: u32,
    state: State<'_, IndexState>,
) -> Result<Vec<IndexedEntry>, String> {
    let path = std::path::PathBuf::from(path);
    let metadata = ops::validate_path(&path, state.roots()).map_err(|error| error.to_string())?;
    if !metadata.is_dir() { return Err("The selected path is not a folder.".to_owned()); }
    if ops::is_cloud(ops::attributes(&metadata)) { return Err(AppError::CloudOnly.to_string()); }
    state.db
        .children(&path, &sort, descending, limit.clamp(1, 1_000), offset)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn search_index(filter: SearchFilter, state: State<'_, IndexState>) -> Result<Vec<IndexedEntry>, String> {
    if filter.min_size.zip(filter.max_size).is_some_and(|(min, max)| min > max) {
        return Err(AppError::InvalidRequest.to_string());
    }
    state.db.search(&filter).map_err(|error| error.to_string())
}

/// Everything the Properties sheet shows, gathered without reading file contents.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileProperties {
    pub name: String,
    pub path: String,
    pub parent: String,
    pub extension: String,
    pub kind: String,
    pub is_directory: bool,
    pub is_cloud_placeholder: bool,
    pub is_hidden: bool,
    pub size: u64,
    pub child_count: Option<u64>,
    pub modified_unix: Option<u64>,
    pub created_unix: Option<u64>,
    pub accessed_unix: Option<u64>,
    pub drive: String,
    pub attribute_labels: Vec<String>,
    pub favorite: bool,
}

/// A capped, lossy text read used by the built-in text/code preview.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TextPreview {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub size: u64,
    pub encoding: String,
}

/// Extensions Sift is willing to render as text. Anything else falls back to the
/// shell viewer so a binary is never dumped into the webview.
pub const TEXT_EXTENSIONS: [&str; 34] = [
    "txt", "md", "markdown", "log", "json", "jsonc", "csv", "tsv", "xml", "yml", "yaml", "toml",
    "ini", "cfg", "conf", "env", "gitignore", "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "css",
    "scss", "html", "htm", "svg", "py", "sh", "ps1", "sql", "java",
];
/// Hard cap on how much of a file the text preview will read.
pub const TEXT_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;

pub fn is_text_preview(path: &Path) -> bool {
    path.extension()
        .map(|extension| {
            TEXT_EXTENSIONS.contains(&extension.to_string_lossy().to_ascii_lowercase().as_str())
        })
        .unwrap_or(false)
}

fn unix(value: Option<SystemTime>) -> Option<u64> {
    value
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
}

fn attribute_labels(metadata: &fs::Metadata) -> Vec<String> {
    let attributes = ops::attributes(metadata);
    let mut labels = Vec::new();
    if attributes & ops::FILE_ATTRIBUTE_HIDDEN != 0 {
        labels.push("Hidden".to_owned());
    }
    if ops::is_reparse(metadata) {
        labels.push("Link".to_owned());
    }
    if ops::is_cloud(attributes) {
        labels.push("Online only".to_owned());
    }
    if attributes == 0 || labels.is_empty() {
        labels.push(if metadata.is_dir() { "Folder".to_owned() } else { "File".to_owned() });
    }
    labels
}

fn kind_label(path: &Path, metadata: &fs::Metadata) -> String {
    if metadata.is_dir() {
        return "Folder".to_owned();
    }
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "avif" => "Image".to_owned(),
        "mp4" | "mov" | "mkv" | "avi" | "webm" | "m4v" | "wmv" => "Video".to_owned(),
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" => "Audio".to_owned(),
        "pdf" => "PDF document".to_owned(),
        "zip" | "7z" | "rar" | "tar" | "gz" => "Archive".to_owned(),
        _ if is_text_preview(path) => "Text document".to_owned(),
        _ if extension.is_empty() => "File".to_owned(),
        _ => format!("{} file", extension.to_uppercase()),
    }
}

#[tauri::command]
#[specta::specta]
pub fn describe_path(path: String, state: State<'_, IndexState>) -> Result<FileProperties, String> {
    let roots = state.roots().to_vec();
    let target = ops::normal_path(&PathBuf::from(&path));
    let metadata = ops::validate_path(&target, &roots).map_err(|error| error.to_string())?;
    let child_count = if metadata.is_dir() {
        fs::read_dir(ops::io_path(&target)).ok().map(|entries| entries.count() as u64)
    } else {
        None
    };
    let favorite = state
        .db
        .is_favorite(&ops::display_path(&target))
        .unwrap_or(false);
    Ok(FileProperties {
        name: ops::file_name(&target),
        path: ops::display_path(&target),
        parent: target.parent().map(ops::display_path).unwrap_or_default(),
        extension: target
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default(),
        kind: kind_label(&target, &metadata),
        is_directory: metadata.is_dir(),
        is_cloud_placeholder: ops::is_cloud(ops::attributes(&metadata)),
        is_hidden: ops::is_hidden(&target, &metadata),
        size: if metadata.is_dir() { 0 } else { metadata.len() },
        child_count,
        modified_unix: unix(metadata.modified().ok()),
        created_unix: unix(metadata.created().ok()),
        accessed_unix: unix(metadata.accessed().ok()),
        drive: ops::drive_for(&target),
        attribute_labels: attribute_labels(&metadata),
        favorite,
    })
}

#[tauri::command]
#[specta::specta]
pub fn read_text_preview(path: String, state: State<'_, IndexState>) -> Result<TextPreview, String> {
    let roots = state.roots().to_vec();
    let target = ops::normal_path(&PathBuf::from(&path));
    let metadata = ops::validate_path(&target, &roots).map_err(|error| error.to_string())?;
    if metadata.is_dir() {
        return Err(AppError::InvalidRequest.to_string());
    }
    if ops::is_cloud(ops::attributes(&metadata)) {
        return Err(AppError::CloudOnly.to_string());
    }
    if !is_text_preview(&target) {
        return Err("Sift previews text and code files only. Open this file in its own app.".to_owned());
    }

    use std::io::Read;
    let mut file = fs::File::open(ops::io_path(&target)).map_err(|_| AppError::Unavailable.to_string())?;
    let mut buffer = vec![0_u8; TEXT_PREVIEW_BYTES as usize];
    let read = file.read(&mut buffer).map_err(|_| AppError::Unavailable.to_string())?;
    buffer.truncate(read);
    if buffer.contains(&0) {
        return Err("This file looks like a binary, so Sift will not render it as text.".to_owned());
    }
    let truncated = metadata.len() > read as u64;
    Ok(TextPreview {
        path: ops::display_path(&target),
        content: String::from_utf8_lossy(&buffer).into_owned(),
        truncated,
        size: metadata.len(),
        encoding: "UTF-8".to_owned(),
    })
}

#[tauri::command]
#[specta::specta]
pub fn list_favorites(state: State<'_, IndexState>) -> Result<Vec<FavoriteItem>, String> {
    state.db.favorites().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn add_favorite(path: String, name: String, is_directory: bool, state: State<'_, IndexState>) -> Result<Vec<FavoriteItem>, String> {
    let roots = state.roots().to_vec();
    let target = ops::normal_path(&PathBuf::from(&path));
    if ops::root_for(&target, &roots).is_none() {
        return Err(AppError::OutsideUserFiles.to_string());
    }
    let display = ops::display_path(&target);
    let label = if name.trim().is_empty() { ops::file_name(&target) } else { name };
    state
        .db
        .add_favorite(&display, &label, is_directory, now_unix())
        .map_err(|error| error.to_string())?;
    state.db.favorites().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn remove_favorite(path: String, state: State<'_, IndexState>) -> Result<Vec<FavoriteItem>, String> {
    let display = ops::display_path(&ops::normal_path(&PathBuf::from(&path)));
    state.db.remove_favorite(&display).map_err(|error| error.to_string())?;
    state.db.favorites().map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn list_recents(limit: u32, state: State<'_, IndexState>) -> Result<Vec<RecentItem>, String> {
    state.db.recents(limit).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn clear_recents(state: State<'_, IndexState>) -> Result<(), String> {
    state.db.clear_recents().map_err(|error| error.to_string())
}

/// Record an open in the Recents list. Called by the command layer after a file opens
/// or a preview is shown; failures never block the user's action.
pub fn note_recent(state: &IndexState, path: &Path, name: &str) {
    let display = ops::display_path(path);
    let label = if name.trim().is_empty() { ops::file_name(path) } else { name.to_owned() };
    let _ = state.db.push_recent(&display, &label, now_unix());
}

/// Recents entry point for the webview (used when a file is previewed inside Sift).
#[tauri::command]
#[specta::specta]
pub fn record_recent(path: String, name: String, state: State<'_, IndexState>) -> Result<(), String> {
    let roots = state.roots().to_vec();
    let target = ops::normal_path(&PathBuf::from(&path));
    if ops::root_for(&target, &roots).is_none() {
        return Err(AppError::OutsideUserFiles.to_string());
    }
    note_recent(state.inner(), &target, &name);
    Ok(())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_text_extensions_are_previewable() {
        assert!(is_text_preview(Path::new("/u/notes.md")));
        assert!(is_text_preview(Path::new("C:\\u\\src\\main.RS")));
        assert!(!is_text_preview(Path::new("/u/movie.mp4")));
        assert!(!is_text_preview(Path::new("/u/archive.zip")));
        assert!(!is_text_preview(Path::new("/u/no-extension")));
    }

    #[test]
    fn kind_labels_are_human_readable() {
        let path = std::env::temp_dir().join("sift-kind-sample.png");
        std::fs::write(&path, b"x").expect("write");
        let metadata = std::fs::symlink_metadata(&path).expect("metadata");
        assert_eq!(kind_label(&path, &metadata), "Image");
        let binary = path.with_extension("exe");
        std::fs::write(&binary, b"MZ").expect("write");
        let metadata = std::fs::symlink_metadata(&binary).expect("metadata");
        assert_eq!(kind_label(&binary, &metadata), "EXE file");
        assert_eq!(kind_label(&path.with_extension("rs"), &metadata), "Text document");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&binary);
    }

    #[test]
    fn attribute_labels_name_the_windows_flags() {
        let path = std::env::temp_dir().join("sift-attribute-sample.txt");
        std::fs::write(&path, b"data").expect("write");
        let metadata = std::fs::symlink_metadata(&path).expect("metadata");
        let labels = attribute_labels(&metadata);
        assert!(!labels.is_empty());
        assert!(labels.iter().any(|label| label == "File"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn timestamps_convert_to_unix_seconds() {
        assert_eq!(unix(None), None);
        assert_eq!(unix(Some(UNIX_EPOCH)), Some(0));
        assert_eq!(unix(Some(UNIX_EPOCH + std::time::Duration::from_secs(42))), Some(42));
    }
}
