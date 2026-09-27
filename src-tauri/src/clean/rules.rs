//! Pure classification rules for the Clean tab: what counts as junk, duplicate,
//! large, old, or a screenshot. No filesystem access lives here, so every rule is
//! covered by a unit test.

use std::path::{Path, PathBuf};

/// 100 MB — the "worth a look" threshold for the Large files card.
pub const LARGE_FILE_BYTES: u64 = 100 * 1024 * 1024;
/// Files smaller than this are never treated as duplicate candidates.
pub const MIN_DUPLICATE_BYTES: u64 = 1024;
/// Bytes read for the cheap first-pass hash before the full BLAKE3 pass.
pub const DUPLICATE_PREVIEW_BYTES: usize = 64 * 1024;
/// Downloads older than this are offered on the Old downloads card.
pub const OLD_DOWNLOAD_DAYS: i64 = 90;
/// How many entries each list card shows.
pub const CARD_LIST_LIMIT: u32 = 250;

pub fn is_large_file(size: u64) -> bool {
    size >= LARGE_FILE_BYTES
}

/// Extra copies are the only ones that can be reclaimed; one copy always stays.
pub fn duplicate_reclaimable_bytes(size: u64, copies: usize) -> u64 {
    size.saturating_mul(copies.saturating_sub(1) as u64)
}

/// Size grouping is only worth doing for real, local, non-trivial files.
pub fn is_duplicate_candidate(size: u64, is_directory: bool, is_cloud: bool) -> bool {
    !is_directory && !is_cloud && size > MIN_DUPLICATE_BYTES
}

/// Unix-second cutoff for "older than 90 days".
pub fn old_download_cutoff(now_unix: i64) -> i64 {
    now_unix.saturating_sub(OLD_DOWNLOAD_DAYS * 86_400)
}

pub fn is_older_than(mtime_unix: Option<i64>, cutoff_unix: i64) -> bool {
    matches!(mtime_unix, Some(value) if value > 0 && value < cutoff_unix)
}

/// `Pictures\Screenshots` — everything inside counts, whatever it is named.
pub fn screenshot_folder(pictures: &Path) -> PathBuf {
    pictures.join("Screenshots")
}

const SCREENSHOT_PREFIXES: [&str; 7] = [
    "screenshot",
    "screen shot",
    "screen-shot",
    "screen_shot",
    "snipping",
    "snip ",
    "capture",
];

/// Windows screenshot names: `Screenshot (3).png`, `Screenshot 2024-01-01 120000.png`,
/// `Snipping Tool 2024...`, `Capture.PNG`. Anything inside `Pictures\Screenshots`
/// counts regardless of its name.
pub fn is_screenshot_name(name: &str, inside_screenshot_folder: bool) -> bool {
    if inside_screenshot_folder {
        return true;
    }
    let stem = name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(name)
        .to_ascii_lowercase();
    SCREENSHOT_PREFIXES.iter().any(|prefix| stem.starts_with(prefix))
}

/// Days between two unix timestamps, floored; used for "93 days old" labels.
pub fn days_old(mtime_unix: Option<i64>, now_unix: i64) -> Option<i64> {
    mtime_unix.map(|value| (now_unix.saturating_sub(value)).max(0) / 86_400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_file_threshold_is_inclusive() {
        assert!(!is_large_file(LARGE_FILE_BYTES - 1));
        assert!(is_large_file(LARGE_FILE_BYTES));
    }

    #[test]
    fn duplicate_savings_preserve_one_copy() {
        assert_eq!(duplicate_reclaimable_bytes(10, 0), 0);
        assert_eq!(duplicate_reclaimable_bytes(10, 1), 0);
        assert_eq!(duplicate_reclaimable_bytes(10, 3), 20);
    }

    #[test]
    fn duplicate_candidates_exclude_small_cloud_and_folder_entries() {
        assert!(is_duplicate_candidate(MIN_DUPLICATE_BYTES + 1, false, false));
        assert!(!is_duplicate_candidate(MIN_DUPLICATE_BYTES, false, false), "1 KB and under is ignored");
        assert!(!is_duplicate_candidate(10_000, true, false), "folders are not hashed");
        assert!(!is_duplicate_candidate(10_000, false, true), "cloud placeholders are never read");
    }

    #[test]
    fn old_download_cutoff_is_ninety_days_back() {
        let now = 1_800_000_000_i64;
        let cutoff = old_download_cutoff(now);
        assert_eq!(cutoff, now - 90 * 86_400);
        assert!(is_older_than(Some(cutoff - 1), cutoff));
        assert!(!is_older_than(Some(cutoff), cutoff), "exactly 90 days is not old yet");
        assert!(!is_older_than(Some(cutoff + 1), cutoff));
        assert!(!is_older_than(None, cutoff));
        assert!(!is_older_than(Some(0), cutoff), "an unknown timestamp is never flagged");
    }

    #[test]
    fn screenshot_folder_lives_under_pictures() {
        assert_eq!(
            screenshot_folder(Path::new("C:\\Users\\u\\Pictures")).to_string_lossy(),
            "C:\\Users\\u\\Pictures\\Screenshots"
        );
    }

    #[test]
    fn screenshot_names_cover_the_windows_conventions() {
        for name in [
            "Screenshot (3).png",
            "screenshot 2024-01-01 120000.png",
            "Screen Shot 2024-01-01 at 12.00.00.png",
            "Screen-Shot.png",
            "Snipping Tool 2024-01-01.png",
            "Snip 2024.png",
            "Capture.PNG",
        ] {
            assert!(is_screenshot_name(name, false), "{name} should match");
        }
    }

    #[test]
    fn ordinary_photos_are_not_screenshots() {
        for name in ["IMG_2041.jpg", "holiday.png", "my capture device.log", "captured-value.json"] {
            assert!(!is_screenshot_name(name, false), "{name} should not match");
        }
        // The extension is not what makes it a screenshot; the stem is.
        assert!(is_screenshot_name("screenshot.md.txt", false));
    }

    #[test]
    fn everything_inside_the_screenshots_folder_counts() {
        assert!(is_screenshot_name("IMG_2041.jpg", true));
        assert!(is_screenshot_name("holiday.png", true));
    }

    #[test]
    fn age_labels_are_whole_days() {
        let now = 1_800_000_000_i64;
        assert_eq!(days_old(Some(now - 3 * 86_400), now), Some(3));
        assert_eq!(days_old(Some(now + 60), now), Some(0), "clock skew never goes negative");
        assert_eq!(days_old(None, now), None);
    }
}
