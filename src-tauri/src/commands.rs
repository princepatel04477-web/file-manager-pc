use crate::db::{CategorySummary, IndexedEntry, SearchFilter};
use crate::error::AppError;
use crate::indexer::{self, IndexProgress, IndexState};
use crate::ops;
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
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
