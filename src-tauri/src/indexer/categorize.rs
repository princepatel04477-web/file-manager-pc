pub const CATEGORIES: [&str; 7] = [
    "images",
    "videos",
    "audio",
    "documents",
    "archives",
    "installers",
    "other",
];

pub fn category_for_extension(extension: &str) -> &'static str {
    match extension.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "avif" | "raw" | "cr2" | "nef" | "arw" => "images",
        "mp4" | "m4v" | "mov" | "mkv" | "avi" | "wmv" | "webm" | "mpeg" | "mpg" | "3gp" => "videos",
        "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "wma" | "aiff" => "audio",
        "pdf" | "doc" | "docx" | "odt" | "rtf" | "txt" | "md" | "csv" | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" => "documents",
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "cab" | "iso" => "archives",
        "exe" | "msi" => "installers",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::category_for_extension;

    #[test]
    fn categorizes_every_requested_group() {
        for (extension, expected) in [
            ("jpg", "images"),
            ("MP4", "videos"),
            ("flac", "audio"),
            ("pdf", "documents"),
            ("tar", "archives"),
            ("exe", "installers"),
            ("msi", "installers"),
            ("unknown-extension", "other"),
        ] {
            assert_eq!(category_for_extension(extension), expected);
        }
    }

    #[test]
    fn accepts_a_leading_dot_and_case_insensitively() {
        assert_eq!(category_for_extension(".JpEg"), "images");
    }
}
