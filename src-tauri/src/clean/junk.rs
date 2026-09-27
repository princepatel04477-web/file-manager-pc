//! Junk resolution, scanning, and removal.
//!
//! Path resolution is driven by a [`SystemProbe`] so the exact set of folders Sift
//! offers to clean is decided by pure, unit-tested code. Nothing here deletes a file
//! permanently: every item goes through the platform Recycle Bin.

use crate::error::AppError;
use crate::ops;
use crate::ops::progress::{CancelToken, OperationProgress, OpsRegistry};
use serde::Serialize;
use specta::Type;
use std::fs;
use std::path::{Path, PathBuf};

pub const RECYCLE_BIN_ID: &str = "recycle-bin";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum JunkFilter {
    /// Every file under the target.
    Everything,
    /// Only files whose name starts with this prefix (thumbnail cache databases).
    Prefix(String),
}

impl JunkFilter {
    pub fn matches(&self, file_name: &str) -> bool {
        match self {
            JunkFilter::Everything => true,
            JunkFilter::Prefix(prefix) => file_name.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()),
        }
    }
}

/// One folder Sift is willing to clean.
#[derive(Clone, Debug)]
pub struct JunkTarget {
    pub id: &'static str,
    pub label: &'static str,
    pub path: PathBuf,
    pub filter: JunkFilter,
}

/// What the resolver needs from the machine. Kept as a trait so tests can describe a
/// machine without touching the real filesystem or environment.
pub trait SystemProbe {
    /// An environment variable holding a path (`TEMP`, `LOCALAPPDATA`, `SystemRoot`, …).
    fn variable(&self, name: &str) -> Option<PathBuf>;
    /// Immediate sub-directory names of `path`; empty when it does not exist.
    fn directories(&self, path: &Path) -> Vec<String>;
}

/// Chromium browsers store per-profile caches under `User Data`.
const CHROMIUM_BROWSERS: [(&str, &str, &str); 3] = [
    ("chrome", "Chrome cache", r"Google\Chrome\User Data"),
    ("edge", "Microsoft Edge cache", r"Microsoft\Edge\User Data"),
    ("brave", "Brave cache", r"BraveSoftware\Brave-Browser\User Data"),
];

const CHROMIUM_CACHE_DIRS: [&str; 4] = ["Cache", "Code Cache", "GPUCache", r"Service Worker\CacheStorage"];
const FIREFOX_PROFILES: &str = r"Mozilla\Firefox\Profiles";
const FIREFOX_CACHE_DIR: &str = "cache2";
const THUMBNAIL_CACHE_DIR: &str = r"Microsoft\Windows\Explorer";
const THUMBNAIL_PREFIX: &str = "thumbcache_";

/// Chromium keeps plenty of bookkeeping folders next to the real profiles; only
/// these hold caches worth removing.
pub fn is_chromium_profile_dir(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered == "default"
        || lowered.starts_with("profile ")
        || lowered == "guest profile"
        || lowered == "system profile"
}

/// Resolve every junk location available on this machine.
pub fn junk_targets(probe: &dyn SystemProbe) -> Vec<JunkTarget> {
    let mut targets = Vec::new();

    if let Some(temp) = probe.variable("TEMP").or_else(|| probe.variable("TMP")) {
        targets.push(JunkTarget { id: "user-temp", label: "Temporary files", path: temp, filter: JunkFilter::Everything });
    }

    let system_root = probe.variable("SystemRoot").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    targets.push(JunkTarget {
        id: "windows-temp",
        label: "Windows temporary files",
        path: system_root.join("Temp"),
        filter: JunkFilter::Everything,
    });

    if let Some(local) = probe.variable("LOCALAPPDATA") {
        for (id, label, relative) in CHROMIUM_BROWSERS {
            let user_data = local.join(relative);
            for profile in probe.directories(&user_data) {
                if !is_chromium_profile_dir(&profile) {
                    continue;
                }
                for cache in CHROMIUM_CACHE_DIRS {
                    targets.push(JunkTarget { id, label, path: user_data.join(&profile).join(cache), filter: JunkFilter::Everything });
                }
            }
        }

        let profiles = local.join(FIREFOX_PROFILES);
        for profile in probe.directories(&profiles) {
            targets.push(JunkTarget {
                id: "firefox",
                label: "Firefox cache",
                path: profiles.join(&profile).join(FIREFOX_CACHE_DIR),
                filter: JunkFilter::Everything,
            });
        }

        targets.push(JunkTarget {
            id: "thumbnails",
            label: "Windows thumbnail cache",
            path: local.join(THUMBNAIL_CACHE_DIR),
            filter: JunkFilter::Prefix(THUMBNAIL_PREFIX.to_owned()),
        });
    }

    targets
}

/// One row on the Junk card.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JunkGroup {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    pub item_count: u64,
    pub skipped: u64,
}

#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JunkScan {
    pub groups: Vec<JunkGroup>,
    pub total_bytes: u64,
    pub total_items: u64,
    pub skipped: u64,
}

/// Sum up what each junk location holds. Unreadable entries are counted, never fatal.
pub fn scan(targets: &[JunkTarget], cancel: &dyn Fn() -> bool) -> JunkScan {
    let mut scan = JunkScan::default();
    for target in targets {
        if cancel() {
            break;
        }
        let (bytes, items, skipped) = walk(target, cancel, &mut |_, _| {});
        let slot = scan.groups.iter_mut().find(|group| group.id == target.id);
        match slot {
            Some(group) => {
                group.bytes = group.bytes.saturating_add(bytes);
                group.item_count = group.item_count.saturating_add(items);
                group.skipped = group.skipped.saturating_add(skipped);
            }
            None => scan.groups.push(JunkGroup {
                id: target.id.to_owned(),
                label: target.label.to_owned(),
                bytes,
                item_count: items,
                skipped,
            }),
        }
        scan.total_bytes = scan.total_bytes.saturating_add(bytes);
        scan.total_items = scan.total_items.saturating_add(items);
        scan.skipped = scan.skipped.saturating_add(skipped);
    }
    scan
}

fn walk(
    target: &JunkTarget,
    cancel: &dyn Fn() -> bool,
    visit: &mut dyn FnMut(&Path, u64),
) -> (u64, u64, u64) {
    let mut bytes = 0_u64;
    let mut items = 0_u64;
    let mut skipped = 0_u64;
    let mut stack = vec![target.path.clone()];
    while let Some(directory) = stack.pop() {
        if cancel() {
            break;
        }
        let Ok(entries) = fs::read_dir(ops::io_path(&directory)) else {
            skipped += 1;
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            let path = ops::normal_path(&entry.path());
            let Ok(metadata) = fs::symlink_metadata(ops::io_path(&path)) else {
                skipped += 1;
                continue;
            };
            if ops::is_reparse(&metadata) {
                skipped += 1;
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() {
                skipped += 1;
                continue;
            }
            if !target.filter.matches(&ops::file_name(&path)) {
                continue;
            }
            bytes = bytes.saturating_add(metadata.len());
            items += 1;
            visit(&path, metadata.len());
        }
    }
    (bytes, items, skipped)
}

#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JunkDeleteResult {
    pub freed_bytes: u64,
    pub deleted: u64,
    pub skipped: u64,
    pub cancelled: bool,
    pub errors: Vec<String>,
}

pub struct JunkContext<'a> {
    pub registry: &'a OpsRegistry,
    pub job_id: &'a str,
    pub cancel: CancelToken,
}

/// Move every matching junk file to the Recycle Bin. Locked or in-use files are
/// skipped and reported; the folders themselves are left in place.
pub fn delete(
    targets: &[JunkTarget],
    context: &JunkContext<'_>,
    emit: &mut dyn FnMut(OperationProgress),
) -> JunkDeleteResult {
    let mut result = JunkDeleteResult::default();
    for target in targets {
        if context.cancel.requested() {
            result.cancelled = true;
            break;
        }
        let _ = walk(target, &|| context.cancel.requested(), &mut |path, size| {
            if let Some(progress) = context.registry.set_current(context.job_id, &ops::file_name(path)) {
                emit(progress);
            }
            match trash::delete(ops::io_path(path)) {
                Ok(()) => {
                    result.freed_bytes = result.freed_bytes.saturating_add(size);
                    result.deleted += 1;
                    if let Some(progress) = context.registry.finish_item(context.job_id, size) {
                        emit(progress);
                    }
                }
                Err(error) => {
                    result.skipped += 1;
                    if result.errors.len() < 5 {
                        result.errors.push(format!("{}: {error}", ops::file_name(path)));
                    }
                    if let Some(progress) = context.registry.skip_item(context.job_id) {
                        emit(progress);
                    }
                }
            }
        });
    }
    result
}

/// Recycle Bin totals from `SHQueryRecycleBinW`.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecycleBinInfo {
    pub bytes: u64,
    pub items: u64,
    pub available: bool,
}

pub fn recycle_bin_info() -> RecycleBinInfo {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRECYCLEBININFO};
        let mut info = SHQUERYRECYCLEBININFO {
            cbSize: std::mem::size_of::<SHQUERYRECYCLEBININFO>() as u32,
            ..Default::default()
        };
        let queried = unsafe { SHQueryRecycleBinW(windows::core::PCWSTR::null(), &mut info) };
        if queried.is_ok() {
            return RecycleBinInfo {
                bytes: info.i64Size.max(0) as u64,
                items: info.i64NumItems.max(0) as u64,
                available: true,
            };
        }
    }
    RecycleBinInfo { bytes: 0, items: 0, available: false }
}

/// Empty the Recycle Bin with `SHEmptyRecycleBinW`. Silent: the UI already confirmed.
pub fn empty_recycle_bin() -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::{SHEmptyRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND};
        let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
        let emptied = unsafe { SHEmptyRecycleBinW(None, windows::core::PCWSTR::null(), flags) };
        // An already-empty bin reports an error code; that is a success for us.
        if emptied.is_ok() {
            return Ok(());
        }
        return Err(AppError::Unavailable);
    }
    #[cfg(not(windows))]
    {
        Err(AppError::Unavailable)
    }
}

/// Live probe backed by the real environment and filesystem.
pub struct HostProbe;

impl SystemProbe for HostProbe {
    fn variable(&self, name: &str) -> Option<PathBuf> {
        std::env::var_os(name).map(PathBuf::from).filter(|value| !value.as_os_str().is_empty())
    }

    fn directories(&self, path: &Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(ops::io_path(path)) else { return Vec::new() };
        entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Describes a machine in memory.
    struct FakeMachine {
        variables: HashMap<String, PathBuf>,
        folders: HashMap<String, Vec<String>>,
    }

    impl FakeMachine {
        fn new() -> Self {
            Self { variables: HashMap::new(), folders: HashMap::new() }
        }

        fn with_variable(mut self, name: &str, value: &str) -> Self {
            self.variables.insert(name.to_owned(), PathBuf::from(value));
            self
        }

        fn with_folder(mut self, path: &str, children: &[&str]) -> Self {
            self.folders.insert(path.to_owned(), children.iter().map(|name| (*name).to_owned()).collect());
            self
        }
    }

    impl SystemProbe for FakeMachine {
        fn variable(&self, name: &str) -> Option<PathBuf> {
            self.variables.get(name).cloned()
        }

        fn directories(&self, path: &Path) -> Vec<String> {
            self.folders.get(&path.to_string_lossy().to_string()).cloned().unwrap_or_default()
        }
    }

    const LOCAL: &str = r"C:\Users\u\AppData\Local";

    #[test]
    fn resolves_the_two_temp_folders() {
        let machine = FakeMachine::new()
            .with_variable("TEMP", r"C:\Users\u\AppData\Local\Temp")
            .with_variable("SystemRoot", r"C:\Windows");
        let targets = junk_targets(&machine);
        let temp = targets.iter().find(|target| target.id == "user-temp").expect("user temp");
        assert_eq!(temp.path.to_string_lossy(), r"C:\Users\u\AppData\Local\Temp");
        let windows = targets.iter().find(|target| target.id == "windows-temp").expect("windows temp");
        assert_eq!(windows.path.to_string_lossy(), r"C:\Windows\Temp");
    }

    #[test]
    fn falls_back_to_tmp_and_the_default_windows_root() {
        let machine = FakeMachine::new().with_variable("TMP", r"D:\Scratch\Temp");
        let targets = junk_targets(&machine);
        assert!(targets.iter().any(|target| target.id == "user-temp" && target.path.to_string_lossy() == r"D:\Scratch\Temp"));
        assert!(targets.iter().any(|target| target.id == "windows-temp" && target.path.to_string_lossy() == r"C:\Windows\Temp"));
    }

    #[test]
    fn without_any_environment_there_is_nothing_to_offer() {
        let targets = junk_targets(&FakeMachine::new());
        // Only the fixed Windows temp folder is assumed to exist.
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "windows-temp");
    }

    #[test]
    fn finds_every_chromium_profile_and_its_cache_folders() {
        let machine = FakeMachine::new()
            .with_variable("LOCALAPPDATA", LOCAL)
            .with_folder(
                &format!(r"{LOCAL}\Google\Chrome\User Data"),
                &["Default", "Profile 1", "Profile 2", "Crashpad", "component_crx_cache"],
            )
            .with_folder(&format!(r"{LOCAL}\Microsoft\Edge\User Data"), &["Default"])
            .with_folder(&format!(r"{LOCAL}\BraveSoftware\Brave-Browser\User Data"), &["Profile 1"]);
        let targets = junk_targets(&machine);

        let chrome: Vec<&JunkTarget> = targets.iter().filter(|target| target.id == "chrome").collect();
        assert_eq!(chrome.len(), 3 * CHROMIUM_CACHE_DIRS.len(), "three profiles, four cache folders each");
        assert!(chrome.iter().any(|target| target.path.to_string_lossy().ends_with(r"Profile 2\GPUCache")));
        assert!(chrome.iter().any(|target| target.path.to_string_lossy().ends_with(r"Default\Service Worker\CacheStorage")));
        assert!(!targets.iter().any(|target| target.path.to_string_lossy().contains("Crashpad")), "bookkeeping folders are not caches");
        assert_eq!(targets.iter().filter(|target| target.id == "edge").count(), CHROMIUM_CACHE_DIRS.len());
        assert_eq!(targets.iter().filter(|target| target.id == "brave").count(), CHROMIUM_CACHE_DIRS.len());
    }

    #[test]
    fn finds_firefox_cache_two_for_every_profile() {
        let machine = FakeMachine::new()
            .with_variable("LOCALAPPDATA", LOCAL)
            .with_folder(&format!(r"{LOCAL}\Mozilla\Firefox\Profiles"), &["abc123.default-release", "xyz789.dev-edition"]);
        let firefox: Vec<String> = junk_targets(&machine)
            .iter()
            .filter(|target| target.id == "firefox")
            .map(|target| target.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(firefox.len(), 2);
        assert!(firefox.iter().all(|path| path.ends_with(r"\cache2")));
    }

    #[test]
    fn thumbnail_cache_only_matches_thumbcache_files() {
        let machine = FakeMachine::new().with_variable("LOCALAPPDATA", LOCAL);
        let thumbs = junk_targets(&machine)
            .into_iter()
            .find(|target| target.id == "thumbnails")
            .expect("thumbnail target");
        assert!(thumbs.path.to_string_lossy().ends_with(r"Microsoft\Windows\Explorer"));
        assert!(thumbs.filter.matches("thumbcache_1024.db"));
        assert!(thumbs.filter.matches("THUMBCACHE_IDX.db"));
        assert!(!thumbs.filter.matches("iconcache_32.db"), "icon caches are Explorer state, not thumbnails");
        assert!(!thumbs.filter.matches("desktop.ini"));
    }

    #[test]
    fn profile_detection_is_conservative() {
        for name in ["Default", "Profile 1", "Profile 12", "Guest Profile", "System Profile", "default"] {
            assert!(is_chromium_profile_dir(name), "{name} is a profile");
        }
        for name in ["Crashpad", "component_crx_cache", "extensions_crx_cache", "SwReporter", "ShaderCache", "profiles"] {
            assert!(!is_chromium_profile_dir(name), "{name} is not a profile");
        }
    }

    #[test]
    fn filters_match_case_insensitively() {
        assert!(JunkFilter::Everything.matches("anything.tmp"));
        assert!(JunkFilter::Prefix("thumbcache_".to_owned()).matches("ThumbCache_256.db"));
        assert!(!JunkFilter::Prefix("thumbcache_".to_owned()).matches("other.db"));
    }

    #[test]
    fn scanning_a_real_folder_counts_bytes_and_skips_what_it_cannot_read() {
        let root = std::env::temp_dir().join(format!(
            "sift-junk-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).expect("dirs");
        fs::write(root.join("a.tmp"), vec![0_u8; 100]).expect("write");
        fs::write(root.join("nested").join("b.log"), vec![0_u8; 40]).expect("write");
        fs::write(root.join("keep.txt"), b"not junk by filter").expect("write");

        let targets = vec![JunkTarget { id: "user-temp", label: "Temporary files", path: root.clone(), filter: JunkFilter::Prefix("".to_owned()) }];
        let scan = scan(&targets, &|| false);
        assert_eq!(scan.groups.len(), 1);
        assert_eq!(scan.total_items, 3, "every file counts with an empty prefix");
        assert_eq!(scan.total_bytes, 100 + 40 + 18);
        assert_eq!(scan.skipped, 0);

        let filtered = vec![JunkTarget { id: "user-temp", label: "Temporary files", path: root.clone(), filter: JunkFilter::Prefix("a.".to_owned()) }];
        let scan = scan(&filtered, &|| false);
        assert_eq!(scan.total_items, 1);
        assert_eq!(scan.total_bytes, 100);

        let cancelled = scan(&targets, &|| true);
        assert_eq!(cancelled.total_items, 0, "cancelling stops before any folder is read");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_junk_folders_are_skipped_not_fatal() {
        let targets = vec![JunkTarget {
            id: "chrome",
            label: "Chrome cache",
            path: std::env::temp_dir().join("sift-does-not-exist-anywhere"),
            filter: JunkFilter::Everything,
        }];
        let scan = scan(&targets, &|| false);
        assert_eq!(scan.groups.len(), 1);
        assert_eq!(scan.groups[0].bytes, 0);
        assert!(scan.groups[0].skipped >= 1, "the unreadable root is counted");
    }

    #[test]
    fn recycle_bin_calls_degrade_off_windows() {
        let info = recycle_bin_info();
        if cfg!(windows) {
            assert!(info.available);
        } else {
            assert!(!info.available);
            assert_eq!((info.bytes, info.items), (0, 0));
            assert!(empty_recycle_bin().is_err());
        }
    }
}
