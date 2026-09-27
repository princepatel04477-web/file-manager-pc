pub mod clean;
pub mod commands;
pub mod db;
pub mod error;
pub mod indexer;
pub mod ops;
pub mod share;
pub mod thumbs;
pub mod vault;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State as AxumState};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use crate::clean::{
    clean_junk, clean_paths, empty_recycle_bin, get_clean_summary, scan_duplicates, uninstall_app,
};
use crate::commands::{
    add_favorite, clear_recents, describe_path, get_category_summary, get_drive_storage,
    get_index_status, list_favorites, list_index_directory, list_recents, read_text_preview,
    record_recent, remove_favorite, search_index,
};
use crate::ops::commands::{
    cancel_operation, copy_paths, delete_paths, list_operations, move_paths, open_with,
    plan_transfer, rename_path, reveal_in_explorer, show_properties, FileOpsState,
};
use crate::thumbs::get_thumbnail;
use crate::indexer::IndexState;
use tauri::Manager;
use qrcode::render::svg;
use qrcode::QrCode;
use serde::Serialize;
use specta::Type;
use std::fs;
use std::net::UdpSocket;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::oneshot;
use tokio_util::io::ReaderStream;

#[derive(Debug, Error)]
enum SiftError {
    #[error("The requested location is outside your user folder.")]
    OutsideHome,
    #[error("This location cannot be opened. It may be offline or unavailable.")]
    Unavailable,
    #[error("This location is protected by Windows and cannot be opened.")]
    ReparsePoint,
    #[error("The location is not a folder.")]
    NotDirectory,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeLocation {
    pub id: String,
    pub label: String,
    pub path: String,
    pub category: String,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub extension: String,
    pub kind: String,
    pub is_directory: bool,
    pub is_cloud_placeholder: bool,
    pub size: u64,
    pub modified_unix: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryListing {
    pub path: String,
    pub label: String,
    pub parent_path: Option<String>,
    pub entries: Vec<FileEntry>,
    pub skipped: u64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub entries: Vec<FileEntry>,
    pub scanned: u64,
    pub skipped: u64,
    pub truncated: bool,
}

fn user_home() -> Result<PathBuf, SiftError> {
    #[cfg(windows)]
    let value = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let value = std::env::var_os("HOME");
    value
        .map(PathBuf::from)
        .map(|path| normal_path(&path))
        .ok_or(SiftError::Unavailable)
}

/// Add the Win32 extended-length prefix to every path passed to filesystem APIs.
/// The prefix is left alone when it is already present and is a no-op off Windows.
fn io_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if raw.starts_with(r"\\?\") {
            return path.to_path_buf();
        }
        if raw.starts_with(r"\\") {
            return PathBuf::from(format!(r"\\?\UNC\{}", raw.trim_start_matches('\\')));
        }
        return PathBuf::from(format!(r"\\?\{}", raw));
    }
    #[cfg(not(windows))]
    {
        path.to_path_buf()
    }
}

fn normal_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{}", rest));
        }
        if let Some(rest) = raw.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

fn display_path(path: &Path) -> String {
    normal_path(path).to_string_lossy().into_owned()
}

#[cfg(windows)]
fn windows_attributes(metadata: &fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes()
}

#[cfg(not(windows))]
fn windows_attributes(metadata: &fs::Metadata) -> u32 {
    if metadata.file_type().is_symlink() { 0x400 } else { 0 }
}

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

fn cloud_placeholder(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

/// Open file data only after validating the user-profile boundary, reparse attributes,
/// and cloud recall flags. On Windows the handle targets the reparse point itself rather
/// than transparently following it, so the handle's attributes can be checked before reads.
fn open_regular_file(path: &Path) -> Result<fs::File, SiftError> {
    let home = user_home()?;
    let before = validate_user_path(path, &home)?;
    if !before.is_file() || cloud_placeholder(windows_attributes(&before)) {
        return Err(SiftError::Unavailable);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(io_path(path)).map_err(|_| SiftError::Unavailable)?;
    let opened = file.metadata().map_err(|_| SiftError::Unavailable)?;
    let attributes = windows_attributes(&opened);
    if !opened.is_file()
        || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || cloud_placeholder(attributes)
    {
        return Err(if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            SiftError::ReparsePoint
        } else {
            SiftError::Unavailable
        });
    }
    Ok(file)
}

/// Validate the path lexically and inspect every component without following links.
/// This prevents a crafted command argument from escaping the current user's profile
/// through parent segments, symlinks, or Windows junctions.
fn validate_user_path(path: &Path, _home: &Path) -> Result<fs::Metadata, SiftError> {
    if !path.is_absolute() || path.components().any(|component| matches!(component, Component::ParentDir)) {
        return Err(SiftError::OutsideHome);
    }
    let roots = ops::user_roots();
    let root = ops::root_for(path, &roots).ok_or(SiftError::OutsideHome)?;
    let relative = path.strip_prefix(&root).map_err(|_| SiftError::OutsideHome)?;
    let mut current = root.clone();
    let mut result = fs::symlink_metadata(io_path(&root)).map_err(|_| SiftError::Unavailable)?;
    if windows_attributes(&result) & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(SiftError::ReparsePoint);
    }

    for component in relative.components() {
        match component {
            Component::Normal(part) => current.push(part),
            Component::CurDir => continue,
            _ => return Err(SiftError::OutsideHome),
        }
        result = fs::symlink_metadata(io_path(&current)).map_err(|_| SiftError::Unavailable)?;
        if windows_attributes(&result) & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(SiftError::ReparsePoint);
        }
    }
    Ok(result)
}

fn location_specs(home: &Path) -> Vec<HomeLocation> {
    let mut locations = vec![HomeLocation {
        id: "home".to_owned(),
        label: "Home".to_owned(),
        path: display_path(home),
        category: "home".to_owned(),
    }];
    let candidates = [
        ("desktop", "Desktop", "personal", dirs::desktop_dir()),
        ("documents", "Documents", "personal", dirs::document_dir()),
        ("downloads", "Downloads", "personal", dirs::download_dir()),
        ("pictures", "Pictures", "media", dirs::picture_dir()),
        ("music", "Music", "media", dirs::audio_dir()),
        ("videos", "Videos", "media", dirs::video_dir()),
    ];
    let roots = ops::user_roots();
    for (id, label, category, candidate) in candidates {
        let Some(path) = candidate.map(|value| normal_path(&value)) else { continue; };
        if ops::root_for(&path, &roots).is_none() { continue; }
        let available = fs::symlink_metadata(io_path(&path))
            .map(|metadata| metadata.is_dir() && !ops::is_reparse(&metadata))
            .unwrap_or(false);
        if available {
            locations.push(HomeLocation {
                id: id.to_owned(),
                label: label.to_owned(),
                path: display_path(&path),
                category: category.to_owned(),
            });
        }
    }
    locations
}

fn file_kind(path: &Path, is_directory: bool, is_cloud: bool) -> (String, String) {
    if is_directory {
        return ("folder".to_owned(), String::new());
    }
    let extension = path
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if is_cloud {
        return ("cloud".to_owned(), extension);
    }
    let kind = match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "bmp" => "image",
        "mp4" | "mov" | "mkv" | "avi" | "webm" => "video",
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" => "audio",
        "pdf" | "doc" | "docx" | "txt" | "rtf" | "xls" | "xlsx" | "ppt" | "pptx" => "document",
        "zip" | "7z" | "rar" | "tar" | "gz" => "archive",
        _ => "file",
    };
    (kind.to_owned(), extension)
}

fn file_entry(path: &Path, metadata: &fs::Metadata) -> FileEntry {
    let attributes = windows_attributes(metadata);
    let is_directory = metadata.is_dir();
    let is_cloud = cloud_placeholder(attributes);
    let (kind, extension) = file_kind(path, is_directory, is_cloud);
    FileEntry {
        name: path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "Item".to_owned()),
        path: display_path(path),
        extension,
        kind,
        is_directory,
        is_cloud_placeholder: is_cloud,
        size: if is_directory { 0 } else { metadata.len() },
        modified_unix: metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
    }
}

fn sorted_entries(mut entries: Vec<FileEntry>) -> Vec<FileEntry> {
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    entries
}

fn list_home_locations_impl() -> Result<Vec<HomeLocation>, String> {
    let home = user_home().map_err(|error| error.to_string())?;
    Ok(location_specs(&home))
}

#[tauri::command]
#[specta::specta]
async fn list_home_locations() -> Result<Vec<HomeLocation>, String> {
    tauri::async_runtime::spawn_blocking(list_home_locations_impl)
        .await
        .map_err(|_| "Could not load your personal folders.".to_owned())?
}

fn list_directory_impl(path: String) -> Result<DirectoryListing, String> {
    let home = user_home().map_err(|error| error.to_string())?;
    let requested = PathBuf::from(&path);
    let metadata = validate_user_path(&requested, &home).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err(SiftError::NotDirectory.to_string());
    }
    if cloud_placeholder(windows_attributes(&metadata)) {
        return Err("This cloud folder is online-only and cannot be browsed while offline.".to_owned());
    }

    let mut entries = Vec::new();
    let mut skipped = 0_u64;
    let iterator = fs::read_dir(io_path(&requested)).map_err(|_| SiftError::Unavailable.to_string())?;
    for result in iterator {
        let entry = match result {
            Ok(entry) => entry,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let entry_path = normal_path(&entry.path());
        let entry_metadata = match fs::symlink_metadata(io_path(&entry_path)) {
            Ok(value) => value,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if windows_attributes(&entry_metadata) & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            skipped += 1;
            continue;
        }
        entries.push(file_entry(&entry_path, &entry_metadata));
    }
    let parent_path = requested
        .parent()
        .filter(|parent| ops::root_for(parent, &ops::user_roots()).is_some())
        .map(display_path);
    Ok(DirectoryListing {
        path: display_path(&requested),
        label: requested.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "Home".to_owned()),
        parent_path,
        entries: sorted_entries(entries),
        skipped,
    })
}

#[tauri::command]
#[specta::specta]
async fn list_directory(path: String) -> Result<DirectoryListing, String> {
    tauri::async_runtime::spawn_blocking(move || list_directory_impl(path))
        .await
        .map_err(|_| "This folder could not be loaded.".to_owned())?
}

/// Walk only user folders, using a stack so each directory can be rejected before
/// its children are enumerated. Inaccessible entries are deliberately counted, not surfaced.
fn walk_personal_files<F>(home: &Path, mut visit: F) -> (u64, u64)
where
    F: FnMut(&Path, &fs::Metadata),
{
    let mut stack: Vec<PathBuf> = location_specs(home)
        .into_iter()
        .filter(|location| location.id != "home")
        .map(|location| PathBuf::from(location.path))
        .collect();
    let mut scanned = 0_u64;
    let mut skipped = 0_u64;
    while let Some(directory) = stack.pop() {
        if ops::is_excluded(&directory) { continue; }
        let directory_metadata = match validate_user_path(&directory, home) {
            Ok(metadata) if metadata.is_dir() => metadata,
            _ => {
                skipped += 1;
                continue;
            }
        };
        if cloud_placeholder(windows_attributes(&directory_metadata)) {
            skipped += 1;
            continue;
        }
        let contents = match fs::read_dir(io_path(&directory)) {
            Ok(contents) => contents,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        for item in contents {
            let entry = match item {
                Ok(entry) => entry,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let path = normal_path(&entry.path());
            if ops::is_excluded(&path) { continue; }
            let metadata = match fs::symlink_metadata(io_path(&path)) {
                Ok(metadata) => metadata,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let attributes = windows_attributes(&metadata);
            if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                skipped += 1;
                continue;
            }
            if metadata.is_dir() {
                if cloud_placeholder(attributes) {
                    skipped += 1;
                } else {
                    stack.push(path);
                }
            } else if metadata.is_file() {
                scanned += 1;
                visit(&path, &metadata);
            }
        }
    }
    (scanned, skipped)
}

fn search_files_impl(query: String) -> Result<SearchResults, String> {
    let term = query.trim().to_lowercase();
    if term.is_empty() {
        return Ok(SearchResults { entries: Vec::new(), scanned: 0, skipped: 0, truncated: false });
    }
    let home = user_home().map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    let mut truncated = false;
    let (scanned, skipped) = walk_personal_files(&home, |path, metadata| {
        if entries.len() >= 250 {
            truncated = true;
            return;
        }
        if path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default().contains(&term) {
            entries.push(file_entry(path, metadata));
        }
    });
    entries.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(SearchResults { entries, scanned, skipped, truncated })
}

#[tauri::command]
#[specta::specta]
async fn search_files(query: String) -> Result<SearchResults, String> {
    tauri::async_runtime::spawn_blocking(move || search_files_impl(query))
        .await
        .map_err(|_| "File search was interrupted.".to_owned())?
}

fn open_file_impl(value: String) -> Result<(), String> {
    let home = user_home().map_err(|error| error.to_string())?;
    let path = PathBuf::from(value);
    let metadata = validate_user_path(&path, &home).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a file, not a folder.".to_owned());
    }
    if cloud_placeholder(windows_attributes(&metadata)) {
        return Err("This file is online-only. Download it in Windows before opening it.".to_owned());
    }
    let verified_handle = open_regular_file(&path).map_err(|error| error.to_string())?;
    drop(verified_handle);

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
        let target = io_path(&path);
        let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target_wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err("Windows could not open this file.".to_owned());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Opening files is available in the Windows desktop app.".to_owned())
    }
}

#[tauri::command]
#[specta::specta]
async fn open_file(path: String, state: tauri::State<'_, IndexState>) -> Result<(), String> {
    let index = state.inner().clone();
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || open_file_impl(target))
        .await
        .map_err(|_| "Windows could not open this file.".to_owned())??;
    // Opening a file is what makes it "recent"; a failure above returns early.
    commands::note_recent(&index, &PathBuf::from(&path), "");
    Ok(())
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ShareLink {
    pub url: String,
    pub qr_svg: String,
    pub file_name: String,
    pub expires_in_seconds: u64,
}

#[derive(Clone)]
struct SharePayload {
    token: String,
    path: PathBuf,
    file_name: String,
}

struct ShareSession {
    stop: oneshot::Sender<()>,
}

#[derive(Default)]
struct ShareState {
    active: Mutex<Option<ShareSession>>,
}

async fn download_shared_file(
    AxumState(payload): AxumState<Arc<SharePayload>>,
    AxumPath(token): AxumPath<String>,
) -> impl IntoResponse {
    if token != payload.token {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let home = match user_home() {
        Ok(home) => home,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let metadata = match validate_user_path(&payload.path, &home) {
        Ok(metadata)
            if metadata.is_file()
                && !cloud_placeholder(windows_attributes(&metadata)) => metadata,
        _ => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let file = match open_regular_file(&payload.path) {
        Ok(file) => tokio::fs::File::from_std(file),
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let safe_name = share::safe_attachment_name(&payload.file_name);
    let mut response = Body::from_stream(ReaderStream::new(file)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{safe_name}\"")) {
        response.headers_mut().insert(header::CONTENT_DISPOSITION, value);
    }
    if let Ok(value) = HeaderValue::from_str(&metadata.len().to_string()) {
        response.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    response
}

#[tauri::command]
#[specta::specta]
async fn start_share(path: String, state: tauri::State<'_, ShareState>) -> Result<ShareLink, String> {
    let home = user_home().map_err(|error| error.to_string())?;
    let file_path = PathBuf::from(&path);
    let metadata = validate_user_path(&file_path, &home).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a file to share; folders are not shared.".to_owned());
    }
    if cloud_placeholder(windows_attributes(&metadata)) {
        return Err("This file is online-only. Download it in Windows before sharing.".to_owned());
    }

    let route = UdpSocket::bind("0.0.0.0:0").map_err(|_| "Could not find a network connection.".to_owned())?;
    route.connect("8.8.8.8:80").map_err(|_| "Connect both devices to the same Wi-Fi network.".to_owned())?;
    let host = route.local_addr().map_err(|_| "Could not determine this PC's network address.".to_owned())?.ip();
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 0))
        .await
        .map_err(|_| "Sift could not open a local sharing port. Check your network permissions.".to_owned())?;
    let port = listener.local_addr().map_err(|_| "Could not open a local sharing port.".to_owned())?.port();

    let mut random = [0_u8; 24];
    getrandom::getrandom(&mut random).map_err(|_| "Could not create a secure sharing link.".to_owned())?;
    let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let url = format!("http://{host}:{port}/{token}");
    let qr = QrCode::new(url.as_bytes()).map_err(|_| "Could not create a QR code for this link.".to_owned())?;
    let qr_svg = qr.render::<svg::Color>().min_dimensions(220, 220).build();
    let file_name = file_path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "Shared file".to_owned());
    let link = ShareLink {
        url,
        qr_svg,
        file_name: file_name.clone(),
        expires_in_seconds: share::SESSION_LIFETIME_SECONDS,
    };
    let payload = Arc::new(SharePayload { token, path: file_path, file_name });
    let router = Router::new().route("/{token}", get(download_shared_file)).with_state(payload);
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let timeout = tokio::time::sleep(Duration::from_secs(share::SESSION_LIFETIME_SECONDS));
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            tokio::select! {
                _ = stopped => {},
                _ = timeout => {},
            }
        });
        let _ = server.await;
    });

    let mut active = state.active.lock().map_err(|_| "Sharing could not be started.".to_owned())?;
    if let Some(previous) = active.take() {
        let _ = previous.stop.send(());
    }
    *active = Some(ShareSession { stop });
    Ok(link)
}

#[tauri::command]
#[specta::specta]
fn stop_share(state: tauri::State<'_, ShareState>) -> Result<(), String> {
    let mut active = state.active.lock().map_err(|_| "Sharing could not be stopped.".to_owned())?;
    if let Some(session) = active.take() {
        let _ = session.stop.send(());
    }
    Ok(())
}

fn configure_specta() -> tauri_specta::Builder<tauri::Wry> {
    let builder = tauri_specta::Builder::<tauri::Wry>::new().commands(
        tauri_specta::collect_commands![
            list_home_locations,
            list_directory,
            search_files,
            get_clean_summary,
            scan_duplicates,
            clean_junk,
            clean_paths,
            empty_recycle_bin,
            uninstall_app,
            open_file,
            start_share,
            stop_share,
            get_index_status,
            get_category_summary,
            get_drive_storage,
            list_index_directory,
            search_index,
            plan_transfer,
            copy_paths,
            move_paths,
            rename_path,
            delete_paths,
            cancel_operation,
            list_operations,
            reveal_in_explorer,
            open_with,
            show_properties,
            get_thumbnail,
            describe_path,
            read_text_preview,
            record_recent,
            list_favorites,
            add_favorite,
            remove_favorite,
            list_recents,
            clear_recents
        ],
    );
    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/specta-bindings.ts"),
        )
        .expect("failed to export Rust command and data types");
    builder
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let specta = configure_specta();
    tauri::Builder::default()
        .manage(ShareState::default())
        .manage(FileOpsState::default())
        .invoke_handler(specta.invoke_handler())
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            let database = Arc::new(db::Database::open(&db::database_path(&data_dir))?);
            let index_state = IndexState::new(database, ops::user_roots());
            let _ = app.manage(index_state.clone());
            indexer::start(app.handle().clone(), index_state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Sift");
}
