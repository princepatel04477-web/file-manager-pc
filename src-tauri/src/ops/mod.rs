pub mod commands;
pub mod conflict;
pub mod progress;
pub mod shell;
pub mod transfer;
pub mod trash;

use crate::error::AppError;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

pub fn io_path(path: &Path) -> PathBuf {
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

pub fn normal_path(path: &Path) -> PathBuf {
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

pub fn display_path(path: &Path) -> String {
    normal_path(path).to_string_lossy().into_owned()
}

fn compare_path(path: &Path) -> String {
    let value = display_path(path).replace('/', "\\");
    #[cfg(windows)]
    { value.to_lowercase() }
    #[cfg(not(windows))]
    { value }
}

/// Platform comparison key for a path (lower-case, backslash separated on Windows).
/// Exposed so planning logic and its tests share one definition of path equality.
pub fn path_key(path: &Path) -> String {
    compare_path(path)
}

/// True when both paths address the same item, using the platform comparison rules.
pub fn same_path(left: &Path, right: &Path) -> bool {
    compare_path(left) == compare_path(right)
}

/// File name of a path, as displayed to the user.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn is_within(path: &Path, root: &Path) -> bool {
    let target = compare_path(path);
    let base = compare_path(root).trim_end_matches('\\').to_owned();
    target == base || target.starts_with(&format!("{base}\\"))
}

pub fn user_home() -> Result<PathBuf, AppError> {
    #[cfg(windows)]
    let value = std::env::var_os("USERPROFILE");
    #[cfg(not(windows))]
    let value = std::env::var_os("HOME");
    value
        .map(PathBuf::from)
        .map(|path| normal_path(&path))
        .ok_or(AppError::Unavailable)
}

/// Roots are resolved from the current Windows user's known folders. They may be
/// redirected to another fixed/removable volume, but never expand to a drive root.
pub fn user_roots() -> Vec<PathBuf> {
    let mut roots = vec![user_home().unwrap_or_default()];
    for known in [
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::audio_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
    ]
    .into_iter()
    .flatten()
    {
        let known = normal_path(&known);
        if !roots.iter().any(|root| compare_path(root) == compare_path(&known)) {
            roots.push(known);
        }
    }
    roots.retain(|root| root.is_absolute());
    roots
}

pub fn root_for(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| is_within(path, root))
        .max_by_key(|root| root.components().count())
        .cloned()
}

pub fn attributes(metadata: &fs::Metadata) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
    }
    #[cfg(not(windows))]
    {
        if metadata.file_type().is_symlink() { FILE_ATTRIBUTE_REPARSE_POINT } else { 0 }
    }
}

pub fn is_reparse(metadata: &fs::Metadata) -> bool {
    attributes(metadata) & FILE_ATTRIBUTE_REPARSE_POINT != 0 || metadata.file_type().is_symlink()
}

pub fn is_cloud(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

/// Validate user scope and inspect each path component with symlink_metadata. This
/// check never canonicalizes or follows a symlink/reparse point.
pub fn validate_path(path: &Path, roots: &[PathBuf]) -> Result<fs::Metadata, AppError> {
    let path = normal_path(path);
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir)) {
        return Err(AppError::OutsideUserFiles);
    }
    let root = root_for(&path, roots).ok_or(AppError::OutsideUserFiles)?;
    let relative = path.strip_prefix(&root).map_err(|_| AppError::OutsideUserFiles)?;
    let mut current = root.clone();
    let mut metadata = fs::symlink_metadata(io_path(&root)).map_err(|_| AppError::Unavailable)?;
    if is_reparse(&metadata) {
        return Err(AppError::ReparsePoint);
    }
    for component in relative.components() {
        match component {
            Component::Normal(part) => current.push(part),
            Component::CurDir => continue,
            _ => return Err(AppError::OutsideUserFiles),
        }
        metadata = fs::symlink_metadata(io_path(&current)).map_err(|_| AppError::Unavailable)?;
        if is_reparse(&metadata) {
            return Err(AppError::ReparsePoint);
        }
    }
    Ok(metadata)
}

pub fn drive_for(path: &Path) -> String {
    let value = display_path(path);
    #[cfg(windows)]
    {
        value.get(..2).filter(|prefix| prefix.ends_with(':')).unwrap_or("?").to_owned()
    }
    #[cfg(not(windows))]
    {
        let _ = value;
        "/".to_owned()
    }
}

/// Privacy- and performance-oriented scan exclusions. Windows Temp is the sole
/// exception inside the Windows directory; only current-user allowed roots are ever scanned.
pub fn is_excluded(path: &Path) -> bool {
    let rendered = compare_path(path);
    let windows_dir = rendered.starts_with("c:\\windows\\") || rendered == "c:\\windows";
    let windows_temp = rendered == "c:\\windows\\temp" || rendered.starts_with("c:\\windows\\temp\\");
    if windows_dir && !windows_temp { return true; }

    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_lowercase();
        name == "$recycle.bin"
            || name == "system volume information"
            || name == "node_modules"
            || name == ".git"
            || name.starts_with("program files")
    })
}

pub fn is_hidden(path: &Path, metadata: &fs::Metadata) -> bool {
    attributes(metadata) & FILE_ATTRIBUTE_HIDDEN != 0
        || path.file_name().map(|name| name.to_string_lossy().starts_with('.')).unwrap_or(false)
}
