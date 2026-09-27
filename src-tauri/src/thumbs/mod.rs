//! Thumbnail generation and its on-disk cache.
//!
//! Pixels come from the Windows shell (`IShellItemImageFactory`), so anything Windows
//! itself can preview gets a thumbnail. The cache lives in `%LOCALAPPDATA%\Sift\thumbs`
//! and is keyed by path + modified time + size, so an edited file never shows a stale
//! image. Cloud placeholders are never thumbnailed: asking the shell for their pixels
//! would silently hydrate them from the network.

pub mod png;

use crate::error::AppError;
use crate::indexer::IndexState;
use crate::ops;
use base64::Engine;
use serde::Serialize;
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tauri::State;

const IMAGE_EXTENSIONS: [&str; 12] = [
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "avif", "raw",
];
const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mov", "m4v", "mkv", "avi", "wmv"];
const DOCUMENT_EXTENSIONS: [&str; 1] = ["pdf"];

/// Largest edge, in pixels, of a stored thumbnail.
pub const MAX_DIMENSION: u32 = 256;
/// Files above this size are never sent to the shell extractor.
pub const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
/// Largest bitmap the shell may hand back; anything bigger is treated as a failure.
pub const MAX_BITMAP_EDGE: u32 = 8_192;
const CACHE_DIR_NAME: &str = "thumbs";
const CACHE_SHARDS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailKind {
    Image,
    Video,
    Document,
}

/// Thumbnail payload sent to the webview as a data URL, so the cache folder never has
/// to be exposed through the asset protocol.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Thumbnail {
    pub data_url: String,
    pub key: String,
    pub from_cache: bool,
}

pub fn kind_for(path: &Path) -> Option<ThumbnailKind> {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())?;
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        return Some(ThumbnailKind::Image);
    }
    if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        return Some(ThumbnailKind::Video);
    }
    if DOCUMENT_EXTENSIONS.contains(&extension.as_str()) {
        return Some(ThumbnailKind::Document);
    }
    None
}

/// Pure eligibility rule, kept separate from metadata collection so the cloud and
/// reparse exclusions are directly testable.
pub fn is_eligible(kind: Option<ThumbnailKind>, is_file: bool, size: u64, attributes: u32, is_symlink: bool) -> bool {
    kind.is_some()
        && is_file
        && size <= MAX_SOURCE_BYTES
        && !is_symlink
        && attributes & ops::FILE_ATTRIBUTE_REPARSE_POINT == 0
        && !ops::is_cloud(attributes)
}

/// Metadata-only gate. Callers must check this *before* opening a handle or asking the
/// shell for pixels; it rejects cloud placeholders, reparse points, and huge files.
pub fn is_safe_thumbnail_candidate(path: &Path, metadata: &fs::Metadata) -> bool {
    is_eligible(
        kind_for(path),
        metadata.is_file(),
        metadata.len(),
        ops::attributes(metadata),
        metadata.file_type().is_symlink(),
    )
}

/// `%LOCALAPPDATA%\Sift\thumbs`, falling back to the platform cache directory.
pub fn cache_root() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|value| value.is_absolute())
        .or_else(dirs::cache_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Sift").join(CACHE_DIR_NAME)
}

/// Stable cache key: normalised path + modified time + size.
pub fn cache_key(path: &Path, modified_unix: Option<u64>, size: u64) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ops::path_key(path).as_bytes());
    hasher.update(b"|");
    hasher.update(modified_unix.unwrap_or(0).to_le_bytes().as_slice());
    hasher.update(b"|");
    hasher.update(size.to_le_bytes().as_slice());
    hasher.finalize().to_hex().to_string()
}

/// Sharded cache file so a single folder never holds tens of thousands of entries.
pub fn cache_path(root: &Path, key: &str) -> PathBuf {
    root.join(&key[..2.min(key.len())]).join(format!("{key}.png"))
}

pub fn shard_count() -> u32 {
    CACHE_SHARDS
}

fn modified_unix(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
}

/// Read a cached thumbnail, if one exists for this exact path/mtime/size.
pub fn read_cached(path: &Path, metadata: &fs::Metadata, root: &Path) -> Option<(String, String)> {
    let key = cache_key(path, modified_unix(metadata), metadata.len());
    let bytes = fs::read(ops::io_path(&cache_path(root, &key))).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    Some((key, data_url(&bytes)))
}

/// Generate (or reuse) a thumbnail and return the cached PNG path.
pub fn ensure(path: &Path, metadata: &fs::Metadata, root: &Path) -> Result<(PathBuf, String, bool), AppError> {
    if !is_safe_thumbnail_candidate(path, metadata) {
        return Err(AppError::InvalidRequest);
    }
    let key = cache_key(path, modified_unix(metadata), metadata.len());
    let target = cache_path(root, &key);
    if fs::metadata(ops::io_path(&target)).is_ok() {
        return Ok((target, key, true));
    }
    let (pixels, width, height) = platform::extract(path, MAX_DIMENSION).ok_or(AppError::Unavailable)?;
    let encoded = png::encode_rgba(width, height, &pixels).map_err(|_| AppError::Unavailable)?;
    fs::create_dir_all(ops::io_path(target.parent().unwrap_or(root)))?;
    // Write to a temporary name first so an interrupted write cannot poison the cache.
    let temporary = target.with_extension("part");
    fs::write(ops::io_path(&temporary), &encoded)?;
    if fs::rename(ops::io_path(&temporary), ops::io_path(&target)).is_err() {
        let _ = fs::remove_file(ops::io_path(&target));
        fs::rename(ops::io_path(&temporary), ops::io_path(&target))?;
    }
    Ok((target, key, false))
}

fn data_url(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// Encode RGBA pixels as PNG bytes.
pub fn encode_png(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    png::encode_rgba(width, height, pixels).ok()
}

#[cfg(windows)]
mod platform {
    use super::{png, MAX_BITMAP_EDGE};
    use std::path::Path;

    /// Ask the Windows shell for an image and return top-down RGBA pixels.
    pub fn extract(path: &Path, max_dimension: u32) -> Option<(Vec<u8>, u32, u32)> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::SIZE;
        use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
        use windows::Win32::System::Com::{CoInitializeEx, IBindCtx, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::Shell::{
            IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
        };

        unsafe {
            // A worker thread may already be in another COM apartment; that is harmless.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            // This binding hands back the interface directly rather than filling a void
            // pointer, and naming the bind-context type keeps the `None` from resolving
            // to the never type.
            let factory: IShellItemImageFactory =
                SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None::<&IBindCtx>).ok()?;
            let bitmap = factory
                .GetImage(
                    SIZE { cx: max_dimension as i32, cy: max_dimension as i32 },
                    SIIGBF_BIGGERSIZEOK,
                )
                .ok()?;
            let result = read_pixels(bitmap);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            result
        }
    }

    unsafe fn read_pixels(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<(Vec<u8>, u32, u32)> {
        use windows::Win32::Graphics::Gdi::{
            GetDIBits, GetDC, ReleaseDC, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS,
        };

        let device = GetDC(None);
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        // First pass: let Windows report the bitmap's dimensions.
        let probed = GetDIBits(device, bitmap, 0, 0, None, &mut info, DIB_RGB_COLORS);
        let width = info.bmiHeader.biWidth.unsigned_abs();
        let height = info.bmiHeader.biHeight.unsigned_abs();
        let usable = probed != 0
            && width > 0
            && height > 0
            && width <= MAX_BITMAP_EDGE
            && height <= MAX_BITMAP_EDGE;
        if !usable {
            let _ = ReleaseDC(None, device);
            return None;
        }
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;
        // Negative height requests top-down rows, avoiding a manual flip.
        info.bmiHeader.biHeight = -(height as i32);
        let mut pixels = vec![0_u8; (width as usize) * (height as usize) * 4];
        let copied = GetDIBits(
            device,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut std::ffi::c_void),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = ReleaseDC(None, device);
        if copied == 0 {
            return None;
        }
        png::bgra_to_rgba(&mut pixels);
        Some((pixels, width, height))
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::Path;

    /// Thumbnail extraction uses the Windows shell and is unavailable elsewhere.
    pub fn extract(_path: &Path, _max_dimension: u32) -> Option<(Vec<u8>, u32, u32)> {
        None
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_thumbnail(path: String, state: State<'_, IndexState>) -> Result<Thumbnail, String> {
    let roots = state.roots().to_vec();
    tauri::async_runtime::spawn_blocking(move || {
        let requested = PathBuf::from(&path);
        if ops::root_for(&requested, &roots).is_none() {
            return Err(AppError::OutsideUserFiles.to_string());
        }
        let metadata =
            fs::symlink_metadata(ops::io_path(&requested)).map_err(|_| AppError::Unavailable.to_string())?;
        let root = cache_root();
        if let Some((key, data_url)) = read_cached(&requested, &metadata, &root) {
            return Ok(Thumbnail { data_url, key, from_cache: true });
        }
        let (file, key, _) = ensure(&requested, &metadata, &root).map_err(|error| error.to_string())?;
        let bytes = fs::read(ops::io_path(&file)).map_err(|_| AppError::Unavailable.to_string())?;
        Ok(Thumbnail { data_url: data_url(&bytes), key, from_cache: false })
    })
    .await
    .map_err(|_| "The thumbnail could not be generated.".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("sift-thumbs-{tag}-{unique}"))
    }

    #[test]
    fn thumbnail_allowlist_contains_only_image_formats() {
        assert!(IMAGE_EXTENSIONS.contains(&"png"));
        assert!(!IMAGE_EXTENSIONS.contains(&"exe"));
        assert!(VIDEO_EXTENSIONS.contains(&"mp4"));
        assert!(DOCUMENT_EXTENSIONS.contains(&"pdf"));
    }

    #[test]
    fn extensions_map_to_preview_kinds() {
        assert_eq!(kind_for(Path::new("C:\\u\\a.JPG")), Some(ThumbnailKind::Image));
        assert_eq!(kind_for(Path::new("/u/clip.mp4")), Some(ThumbnailKind::Video));
        assert_eq!(kind_for(Path::new("/u/paper.pdf")), Some(ThumbnailKind::Document));
        assert_eq!(kind_for(Path::new("/u/app.exe")), None);
        assert_eq!(kind_for(Path::new("/u/noext")), None);
    }

    #[test]
    fn cloud_placeholders_are_never_thumbnail_candidates() {
        let image = Some(ThumbnailKind::Image);
        assert!(is_eligible(image, true, 1_024, 0, false));
        assert!(!is_eligible(image, true, 1_024, ops::FILE_ATTRIBUTE_OFFLINE, false));
        assert!(!is_eligible(image, true, 1_024, ops::FILE_ATTRIBUTE_RECALL_ON_OPEN, false));
        assert!(!is_eligible(image, true, 1_024, ops::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, false));
    }

    #[test]
    fn reparse_points_folders_and_huge_files_are_excluded() {
        let image = Some(ThumbnailKind::Image);
        assert!(!is_eligible(image, true, 10, ops::FILE_ATTRIBUTE_REPARSE_POINT, false));
        assert!(!is_eligible(image, true, 10, 0, true), "symlinks are not followed");
        assert!(!is_eligible(image, false, 10, 0, false), "folders have no thumbnail");
        assert!(!is_eligible(image, true, MAX_SOURCE_BYTES + 1, 0, false));
        assert!(is_eligible(image, true, MAX_SOURCE_BYTES, 0, false), "threshold is inclusive");
        assert!(!is_eligible(None, true, 10, 0, false), "extension must be on the allowlist");
    }

    #[test]
    fn a_real_file_on_disk_passes_the_metadata_gate() {
        let path = temp_root("gate").join("picture.png");
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(&path, b"fake image bytes").expect("write");
        let metadata = fs::symlink_metadata(&path).expect("metadata");
        assert!(is_safe_thumbnail_candidate(&path, &metadata));

        let binary = path.with_extension("exe");
        fs::write(&binary, b"MZ").expect("write");
        let metadata = fs::symlink_metadata(&binary).expect("metadata");
        assert!(!is_safe_thumbnail_candidate(&binary, &metadata));
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn cache_key_changes_with_path_mtime_and_size() {
        let path = PathBuf::from("/u/photos/cat.png");
        let base = cache_key(&path, Some(1_700_000_000), 1_024);
        assert_eq!(base, cache_key(&path, Some(1_700_000_000), 1_024), "stable for identical input");
        assert_ne!(base, cache_key(&path, Some(1_700_000_001), 1_024), "mtime matters");
        assert_ne!(base, cache_key(&path, Some(1_700_000_000), 2_048), "size matters");
        assert_ne!(
            base,
            cache_key(&PathBuf::from("/u/photos/dog.png"), Some(1_700_000_000), 1_024),
            "path matters"
        );
        assert_eq!(base.len(), 64, "blake3 hex digest");
    }

    #[test]
    fn cache_paths_are_sharded_and_png_named() {
        let root = PathBuf::from("/cache/thumbs");
        let key = cache_key(&PathBuf::from("/u/a.png"), Some(5), 10);
        let file = cache_path(&root, &key);
        assert!(file.starts_with(&root));
        assert_eq!(file.extension().and_then(|value| value.to_str()), Some("png"));
        assert_eq!(
            file.parent().and_then(|parent| parent.file_name()).and_then(|name| name.to_str()),
            Some(&key[..2])
        );
        assert!(shard_count() > 1);
    }

    #[test]
    fn a_cached_thumbnail_is_returned_and_invalidated_by_new_content() {
        let root = temp_root("cache");
        let source = root.join("source.png");
        fs::create_dir_all(&root).expect("dir");
        fs::write(&source, b"pretend png bytes").expect("write");
        let metadata = fs::symlink_metadata(&source).expect("metadata");
        let key = cache_key(&source, modified_unix(&metadata), metadata.len());
        let cached = cache_path(&root, &key);
        fs::create_dir_all(cached.parent().expect("parent")).expect("shard dir");
        fs::write(&cached, b"\x89PNG\r\n\x1a\nrest").expect("cache write");

        let (hit_key, url) = read_cached(&source, &metadata, &root).expect("cache hit");
        assert_eq!(hit_key, key);
        assert!(url.starts_with("data:image/png;base64,"));

        // New content changes the size, so the old entry must not be reused.
        fs::write(&source, b"a much longer pretend png payload").expect("rewrite");
        let updated = fs::symlink_metadata(&source).expect("metadata");
        assert_ne!(updated.len(), metadata.len());
        assert!(read_cached(&source, &updated, &root).is_none(), "stale entry must not be reused");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn truncated_cache_entries_are_ignored() {
        let root = temp_root("truncated");
        let source = root.join("b.png");
        fs::create_dir_all(&root).expect("dir");
        fs::write(&source, b"image").expect("write");
        let metadata = fs::symlink_metadata(&source).expect("metadata");
        let key = cache_key(&source, modified_unix(&metadata), metadata.len());
        let cached = cache_path(&root, &key);
        fs::create_dir_all(cached.parent().expect("parent")).expect("shard dir");
        fs::write(&cached, b"tiny").expect("cache write");
        assert!(read_cached(&source, &metadata, &root).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn data_url_wraps_base64_png_bytes() {
        let encoded = encode_png(&[255, 0, 0, 255, 0, 255, 0, 255], 2, 1).expect("png");
        let url = data_url(&encoded);
        assert!(url.starts_with("data:image/png;base64,"));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(url.trim_start_matches("data:image/png;base64,"))
            .expect("base64");
        assert_eq!(decoded, encoded);
    }

    #[test]
    fn extraction_is_unavailable_off_windows() {
        if !cfg!(windows) {
            assert!(platform::extract(Path::new("/tmp/anything.png"), MAX_DIMENSION).is_none());
        }
    }
}
