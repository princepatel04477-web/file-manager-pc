use crate::ops;
use std::fs;
use std::path::Path;

const IMAGE_EXTENSIONS: [&str; 12] = ["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "avif", "raw"];

/// Gate for future thumbnail decoding. It uses metadata only and must be checked
/// before any decoder or file handle is created.
pub fn is_safe_thumbnail_candidate(path: &Path, metadata: &fs::Metadata) -> bool {
    let extension_is_image = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .map(|extension| IMAGE_EXTENSIONS.contains(&extension.as_str()))
        .unwrap_or(false);
    metadata.is_file()
        && extension_is_image
        && !ops::is_reparse(metadata)
        && !ops::is_cloud(ops::attributes(metadata))
}

#[cfg(test)]
mod tests {
    use super::IMAGE_EXTENSIONS;

    #[test]
    fn thumbnail_allowlist_contains_only_image_formats() {
        assert!(IMAGE_EXTENSIONS.contains(&"png"));
        assert!(!IMAGE_EXTENSIONS.contains(&"exe"));
    }
}
