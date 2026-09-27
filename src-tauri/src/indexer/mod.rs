pub mod categorize;

use crate::db::{Database, FileRecord};
use crate::error::AppError;
use crate::ops;
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use specta::Type;
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

const INSERT_BATCH_SIZE: usize = 5_000;
const PROGRESS_EVERY_FILES: u64 = 500;
const DEBOUNCE: Duration = Duration::from_millis(500);
/// How many skipped folders the Settings screen can list. Past this the count
/// keeps rising but the list stops growing, so one broken drive cannot fill
/// memory or make the report unreadable.
pub const MAX_SKIPPED_FOLDERS: usize = 250;

#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub files_scanned: u64,
    pub skipped: u64,
    pub current_dir: String,
    pub drive: String,
    pub drives: Vec<String>,
    pub scanning: bool,
    pub complete: bool,
    pub watching: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct IndexState {
    pub db: Arc<Database>,
    roots: Arc<Vec<PathBuf>>,
    progress: Arc<Mutex<IndexProgress>>,
    watched: Arc<Mutex<HashSet<String>>>,
    /// Folders the user excluded in Settings, already in comparable form.
    excluded: Arc<RwLock<Vec<String>>>,
    /// Folders this scan passed over, with the reason, newest first.
    skipped_folders: Arc<Mutex<Vec<(String, String)>>>,
    /// Every folder passed over, including any past `MAX_SKIPPED_FOLDERS`.
    skipped_total: Arc<AtomicU64>,
}

impl IndexState {
    pub fn new(db: Arc<Database>, roots: Vec<PathBuf>) -> Self {
        let excluded = db
            .exclusions()
            .unwrap_or_default()
            .into_iter()
            .map(|(path, _, _)| crate::settings::exclusion_key(Path::new(&path)))
            .collect();
        Self {
            db,
            roots: Arc::new(roots),
            progress: Arc::new(Mutex::new(IndexProgress::default())),
            watched: Arc::new(Mutex::new(HashSet::new())),
            excluded: Arc::new(RwLock::new(excluded)),
            skipped_folders: Arc::new(Mutex::new(Vec::new())),
            skipped_total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn roots(&self) -> &[PathBuf] { self.roots.as_slice() }

    pub fn progress(&self) -> IndexProgress {
        self.progress.lock().map(|value| value.clone()).unwrap_or_default()
    }

    /// True when Settings excludes this path, or anything above it. The stored
    /// keys all end in a separator, so a plain prefix match is the whole test.
    pub fn is_excluded(&self, path: &Path) -> bool {
        let key = format!("{}\\", ops::path_key(path));
        self.excluded
            .read()
            .map(|excluded| excluded.iter().any(|value| key.starts_with(value.as_str())))
            .unwrap_or(false)
    }

    pub fn set_excluded_paths(&self, values: Vec<String>) {
        if let Ok(mut excluded) = self.excluded.write() {
            *excluded = values;
        }
    }

    /// Note a folder the scan could not read, or that Settings told it to skip.
    /// The same folder is only counted once, and the list stops growing at
    /// `MAX_SKIPPED_FOLDERS` while the total keeps counting.
    pub fn record_skipped_folder(&self, path: String, reason: &str) {
        let Ok(mut folders) = self.skipped_folders.lock() else { return };
        if folders.iter().any(|(value, _)| value == &path) { return; }
        if folders.len() < MAX_SKIPPED_FOLDERS {
            folders.push((path, reason.to_owned()));
        }
        self.skipped_total.fetch_add(1, Ordering::Relaxed);
    }

    /// The recorded folders, capped at `MAX_SKIPPED_FOLDERS`. Compare with
    /// `skipped_folders_total` to know whether the list is the whole story.
    pub fn skipped_folders(&self) -> Vec<(String, String)> {
        self.skipped_folders
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    pub fn skipped_folders_total(&self) -> u64 {
        self.skipped_total.load(Ordering::Relaxed)
    }

    /// Forget the skipped-folder record and the counter that goes with it. The
    /// next scan fills both in again.
    pub fn clear_skipped_folders(&self) {
        if let Ok(mut folders) = self.skipped_folders.lock() {
            folders.clear();
        }
        self.skipped_total.store(0, Ordering::Relaxed);
        self.mutate(|progress| progress.skipped = 0);
    }

    fn mutate(&self, edit: impl FnOnce(&mut IndexProgress)) {
        if let Ok(mut progress) = self.progress.lock() { edit(&mut progress); }
    }

    fn update(&self, app: &AppHandle, edit: impl FnOnce(&mut IndexProgress)) {
        if let Ok(mut progress) = self.progress.lock() {
            edit(&mut progress);
            let _ = app.emit("index://progress", progress.clone());
        }
    }
}

#[derive(Clone, Debug)]
pub struct DriveRoot {
    pub path: PathBuf,
    pub label: String,
}

pub fn start(app: AppHandle, state: IndexState) {
    state.update(&app, |progress| { progress.scanning = true; progress.complete = false; });
    let worker_app = app.clone();
    let worker_state = state.clone();
    if thread::Builder::new()
        .name("sift-indexer".to_owned())
        .spawn(move || run_indexer(worker_app, worker_state))
        .is_err()
    {
        state.update(&app, |progress| {
            progress.error = Some("The background indexer could not be started.".to_owned());
            progress.scanning = false;
        });
    }
}

fn run_indexer(app: AppHandle, state: IndexState) {
    let drives = discover_fixed_and_removable_drives();
    let drive_labels = drives.iter().map(|drive| drive.label.clone()).collect::<Vec<_>>();
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128) as i64;
    state.update(&app, |progress| {
        progress.files_scanned = 0;
        progress.skipped = 0;
        progress.drives = drive_labels.clone();
        progress.scanning = true;
        progress.complete = false;
        progress.watching = false;
        progress.error = None;
    });

    let (event_sender, event_receiver) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = match notify::recommended_watcher(move |event: notify::Result<Event>| {
        let _ = event_sender.send(event);
    }) {
        Ok(watcher) => Some(watcher),
        Err(_) => {
            state.update(&app, |progress| progress.error = Some("Live file change monitoring is unavailable.".to_owned()));
            None
        }
    };

    let mut seen_paths = HashSet::new();
    // The Settings scan schedule decides whether this launch walks the profile.
    // `on_launch` (the default) and a never-scanned profile always scan, so the
    // only launches that skip the walk are the ones the user asked to skip.
    if crate::settings::scan_is_due(&state.db, crate::settings::now_unix()) {
        let roots = minimal_roots(state.roots());
        for root in &roots {
            if !drives.iter().any(|drive| ops::drive_for(root).eq_ignore_ascii_case(&drive.label)) {
                increment_skipped(&state);
            }
        }
        let mut batch = Vec::with_capacity(INSERT_BATCH_SIZE);
        for drive in &drives {
            state.update(&app, |progress| {
                progress.drive = drive.label.clone();
                progress.current_dir = ops::display_path(&drive.path);
            });
            let drive_roots = roots
                .iter()
                .filter(|root| ops::drive_for(root).eq_ignore_ascii_case(&drive.label))
                .cloned()
                .collect::<Vec<_>>();
            for root in drive_roots {
                scan_tree(
                    &app,
                    &state,
                    &mut watcher,
                    &root,
                    epoch,
                    &mut batch,
                    &mut seen_paths,
                );
            }
        }
        if flush_batch(&state.db, &mut batch).is_err() {
            state.update(&app, |progress| progress.error = Some("Some file metadata could not be saved to the index.".to_owned()));
            increment_skipped(&state);
        }
        if state.progress().skipped == 0 && state.db.complete_scan(epoch).is_err() {
            state.update(&app, |progress| progress.error = Some("The index could not finish reconciling old entries.".to_owned()));
        }
        crate::settings::record_scan(&state.db, crate::settings::now_unix());
    } else {
        // The index is still current. Watch the known folders so ordinary changes
        // still land, and report the size the index already holds.
        let indexed = state.db.counts().map(|counts| counts.indexed_files).unwrap_or(0);
        for root in minimal_roots(state.roots()) {
            register_directory_watch(&mut watcher, &root, &state);
        }
        state.update(&app, |progress| progress.files_scanned = indexed);
    }
    state.update(&app, |progress| {
        progress.scanning = false;
        progress.complete = true;
        progress.watching = watcher.is_some();
        progress.current_dir.clear();
    });

    if watcher.is_none() { return; }
    while let Ok(first_event) = event_receiver.recv() {
        let mut events = vec![first_event];
        loop {
            match event_receiver.recv_timeout(DEBOUNCE) {
                Ok(event) => events.push(event),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        apply_events(&app, &state, &mut watcher, events, epoch, &mut seen_paths);
    }
}

fn minimal_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut values = roots.iter().map(|path| ops::normal_path(path)).collect::<Vec<_>>();
    values.sort_by_key(|path| path.components().count());
    let mut output: Vec<PathBuf> = Vec::new();
    for candidate in values {
        if !output.iter().any(|root| ops::is_within(&candidate, root)) {
            output.push(candidate);
        }
    }
    output
}

fn scan_tree(
    app: &AppHandle,
    state: &IndexState,
    watcher: &mut Option<RecommendedWatcher>,
    root: &Path,
    epoch: i64,
    batch: &mut Vec<FileRecord>,
    seen_paths: &mut HashSet<String>,
) {
    let mut directories = VecDeque::from([ops::normal_path(root)]);
    while let Some(directory) = directories.pop_front() {
        let normalized = ops::normal_path(&directory);
        let display = ops::display_path(&normalized);
        state.mutate(|progress| progress.current_dir = display.clone());
        if ops::is_excluded(&normalized) { continue; }
        if state.is_excluded(&normalized) {
            state.record_skipped_folder(display, "Excluded in Settings");
            continue;
        }
        let metadata = match ops::validate_path(&normalized, state.roots()) {
            Ok(metadata) if metadata.is_dir() => metadata,
            Ok(_) => { increment_skipped(state); state.record_skipped_folder(display, "No longer a folder"); continue; }
            Err(AppError::ReparsePoint) => { increment_skipped(state); state.record_skipped_folder(display, "A shortcut or link"); continue; }
            Err(AppError::OutsideUserFiles) => { increment_skipped(state); state.record_skipped_folder(display, "Outside your user profile"); continue; }
            Err(_) => { increment_skipped(state); state.record_skipped_folder(display, "Sift needs permission to read this folder"); continue; }
        };
        if ops::is_cloud(ops::attributes(&metadata)) {
            increment_skipped(state);
            state.record_skipped_folder(display, "Stored in the cloud, not on this PC");
            continue;
        }
        register_directory_watch(watcher, &normalized, state);
        let iterator = match fs::read_dir(ops::io_path(&normalized)) {
            Ok(iterator) => iterator,
            Err(_) => {
                increment_skipped(state);
                state.record_skipped_folder(display, "Sift needs permission to read this folder");
                continue;
            }
        };
        for entry_result in iterator {
            let entry = match entry_result {
                Ok(entry) => entry,
                Err(_) => { increment_skipped(state); continue; }
            };
            let path = ops::normal_path(&entry.path());
            if ops::is_excluded(&path) { continue; }
            if state.is_excluded(&path) {
                if fs::symlink_metadata(ops::io_path(&path)).map(|metadata| metadata.is_dir()).unwrap_or(false) {
                    state.record_skipped_folder(ops::display_path(&path), "Excluded in Settings");
                }
                continue;
            }
            let metadata = match fs::symlink_metadata(ops::io_path(&path)) {
                Ok(metadata) => metadata,
                Err(_) => { increment_skipped(state); continue; }
            };
            if ops::is_reparse(&metadata) {
                increment_skipped(state);
                continue;
            }
            let attributes = ops::attributes(&metadata);
            let record = make_record(&path, &metadata, attributes, epoch);
            let is_new_path = seen_paths.insert(record.path.clone());
            if is_new_path { batch.push(record); }
            if metadata.is_dir() && !ops::is_cloud(attributes) {
                directories.push_back(path);
            } else if metadata.is_file() && is_new_path {
                state.mutate(|progress| progress.files_scanned = progress.files_scanned.saturating_add(1));
            }
            if batch.len() >= INSERT_BATCH_SIZE && flush_batch(&state.db, batch).is_err() {
                increment_skipped(state);
            }
            let files_scanned = state.progress().files_scanned;
            if files_scanned > 0 && files_scanned.is_multiple_of(PROGRESS_EVERY_FILES) {
                emit_current_progress(app, state);
            }
        }
    }
}

fn apply_events(
    app: &AppHandle,
    state: &IndexState,
    watcher: &mut Option<RecommendedWatcher>,
    events: Vec<notify::Result<Event>>,
    epoch: i64,
    seen_paths: &mut HashSet<String>,
) {
    let mut paths = HashSet::new();
    for event in events {
        match event {
            Ok(event) => paths.extend(event.paths.into_iter().map(|path| ops::normal_path(&path))),
            Err(_) => increment_skipped(state),
        }
    }
    for path in paths {
        if ops::root_for(&path, state.roots()).is_none() { continue; }
        if ops::is_excluded(&path) || state.is_excluded(&path) {
            let _ = state.db.remove_path(&path);
            unregister_watches(watcher, state, &path);
            continue;
        }
        let metadata = match ops::validate_path(&path, state.roots()) {
            Ok(metadata) => metadata,
            Err(_) => {
                let _ = state.db.remove_path(&path);
                unregister_watches(watcher, state, &path);
                seen_paths.remove(&ops::display_path(&path));
                continue;
            }
        };
        if ops::is_reparse(&metadata) {
            let _ = state.db.remove_path(&path);
            unregister_watches(watcher, state, &path);
            continue;
        }
        let record = make_record(&path, &metadata, ops::attributes(&metadata), epoch);
        seen_paths.insert(record.path.clone());
        if metadata.is_dir() && !record.is_cloud {
            let mut batch = vec![record];
            scan_tree(app, state, watcher, &path, epoch, &mut batch, seen_paths);
            if flush_batch(&state.db, &mut batch).is_err() { increment_skipped(state); }
        } else {
            if state.db.upsert(&record).is_err() { increment_skipped(state); }
            if !metadata.is_dir() {
                state.mutate(|progress| progress.files_scanned = progress.files_scanned.saturating_add(1));
            }
        }
    }
    emit_current_progress(app, state);
}

fn register_directory_watch(watcher: &mut Option<RecommendedWatcher>, path: &Path, state: &IndexState) {
    let Some(watcher) = watcher.as_mut() else { return; };
    let key = ops::display_path(path);
    let Ok(mut watched) = state.watched.lock() else { return; };
    if watched.contains(&key) { return; }
    if watcher.watch(&ops::io_path(path), RecursiveMode::NonRecursive).is_ok() {
        watched.insert(key);
    } else {
        increment_skipped(state);
    }
}

fn unregister_watches(watcher: &mut Option<RecommendedWatcher>, state: &IndexState, root: &Path) {
    let Some(watcher) = watcher.as_mut() else { return; };
    let Ok(mut watched) = state.watched.lock() else { return; };
    let removed = watched
        .iter()
        .filter(|value| ops::is_within(Path::new(value.as_str()), root))
        .cloned()
        .collect::<Vec<_>>();
    for path in removed {
        let _ = watcher.unwatch(&ops::io_path(Path::new(&path)));
        watched.remove(&path);
    }
}

fn make_record(path: &Path, metadata: &fs::Metadata, attributes: u32, epoch: i64) -> FileRecord {
    let name = path.file_name().map(|value| value.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|value| value.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let is_directory = metadata.is_dir();
    FileRecord {
        path: ops::display_path(path),
        parent_path: path.parent().map(ops::display_path).unwrap_or_default(),
        name,
        ext: ext.clone(),
        category: if is_directory { "other".to_owned() } else { categorize::category_for_extension(&ext).to_owned() },
        size: if is_directory { 0 } else { metadata.len() },
        mtime: unix_time(metadata.modified().ok()),
        ctime: unix_time(metadata.created().ok()),
        is_hidden: ops::is_hidden(path, metadata),
        is_cloud: ops::is_cloud(attributes),
        is_directory,
        drive: ops::drive_for(path),
        last_seen: epoch,
    }
}

fn unix_time(time: Option<SystemTime>) -> Option<i64> {
    time.and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
}

fn flush_batch(db: &Database, batch: &mut Vec<FileRecord>) -> Result<(), AppError> {
    if batch.is_empty() { return Ok(()); }
    db.insert_batch(batch)?;
    batch.clear();
    Ok(())
}

fn increment_skipped(state: &IndexState) {
    state.mutate(|progress| progress.skipped = progress.skipped.saturating_add(1));
}

fn emit_current_progress(app: &AppHandle, state: &IndexState) {
    let progress = state.progress();
    let _ = app.emit("index://progress", progress);
}

pub fn discover_fixed_and_removable_drives() -> Vec<DriveRoot> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        use windows::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};
        let bitmask = unsafe { GetLogicalDrives() };
        let mut output = Vec::new();
        for index in 0..26 {
            if bitmask & (1 << index) == 0 { continue; }
            let letter = char::from(b'A' + index as u8);
            let root = format!("{letter}:\\");
            let wide = std::ffi::OsStr::new(&root).encode_wide().chain(std::iter::once(0)).collect::<Vec<_>>();
            let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
            if drive_type == DRIVE_FIXED || drive_type == DRIVE_REMOVABLE {
                output.push(DriveRoot { path: PathBuf::from(&root), label: format!("{letter}:") });
            }
        }
        output
    }
    #[cfg(not(windows))]
    {
        let home = ops::user_home().unwrap_or_else(|_| PathBuf::from("/"));
        vec![DriveRoot { path: PathBuf::from("/"), label: ops::drive_for(&home) }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_roots_remove_nested_known_folders() {
        let roots = vec![PathBuf::from("C:\\Users\\alex\\Documents"), PathBuf::from("C:\\Users\\alex")];
        assert_eq!(minimal_roots(&roots), vec![PathBuf::from("C:\\Users\\alex")]);
    }
}
