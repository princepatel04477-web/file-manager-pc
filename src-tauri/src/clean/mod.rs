//! The Clean tab: six cards, each with a size, a confirm step, and a Recycle Bin delete.
//!
//! Everything the cards need is produced here as plain data. The heavy lifting lives in
//! the sibling modules, where the rules are pure and unit tested:
//! * [`rules`] — thresholds and the screenshot/age tests
//! * [`junk`] — temp, browser cache, thumbnail cache and the Recycle Bin
//! * [`duplicates`] — size grouping, preview hash, parallel full hash
//! * [`apps`] — Uninstall registry keys and their parsing

pub mod apps;
pub mod duplicates;
pub mod junk;
pub mod rules;

use crate::db::{IndexedEntry, ScreenshotRow};
use crate::indexer::IndexState;
use crate::ops::commands::{FileOpsState, ProgressEmitter};
use crate::ops::progress::{OpsKind, OpsState};
use crate::ops::{self, trash};
use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

/// How many size groups one duplicate scan considers. Keeps the generated `IN (...)`
/// list well inside SQLite's bound-parameter limit.
const DUPLICATE_GROUP_LIMIT: u32 = 900;
/// Rows fetched for the screenshot card before the precise name check runs. The SQL
/// filter is deliberately loose, so more rows are read than are kept.
const SCREENSHOT_FETCH_LIMIT: u32 = 1_000;

pub const CARD_JUNK: &str = "junk";
pub const CARD_DUPLICATES: &str = "duplicates";
pub const CARD_LARGE: &str = "large-files";
pub const CARD_DOWNLOADS: &str = "old-downloads";
pub const CARD_SCREENSHOTS: &str = "old-screenshots";
pub const CARD_APPS: &str = "unused-apps";

/// What the card's primary button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CardAction {
    /// Move the selected file paths to the Recycle Bin.
    DeletePaths,
    /// Move whole junk locations to the Recycle Bin, selected by group id.
    DeleteJunk,
    /// Empties the Recycle Bin.
    EmptyRecycleBin,
    /// Launches the app's own uninstaller.
    Uninstall,
    /// Nothing can be done here (unsupported platform, or a scan has not run yet).
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanItem {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified_unix: Option<u64>,
    pub days_old: Option<u64>,
    pub is_cloud: bool,
}

/// A selectable chunk of a card — a junk location, or the Recycle Bin.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanGroup {
    pub id: String,
    pub label: String,
    pub item_count: u64,
    pub reclaimable_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanCard {
    pub id: String,
    pub title: String,
    pub description: String,
    pub action: CardAction,
    pub item_count: u64,
    pub reclaimable_bytes: u64,
    /// Group-level selection, used by the Junk card.
    pub groups: Vec<CleanGroup>,
    /// File-level selection, capped at [`rules::CARD_LIST_LIMIT`].
    pub items: Vec<CleanItem>,
    /// True when the cap above cut the list short.
    pub truncated: bool,
    /// Entries that could not be read or are in use.
    pub skipped: u64,
    /// False until the card has something to show (duplicates need a scan first).
    pub ready: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanSummary {
    pub cards: Vec<CleanCard>,
    pub recycle_bin: junk::RecycleBinInfo,
    pub apps: Vec<apps::InstalledApp>,
    pub apps_bytes: u64,
    pub indexed_files: u64,
    pub generated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CleanDeleteResult {
    pub freed_bytes: u64,
    pub moved: u64,
    pub skipped: u64,
    pub cancelled: bool,
    pub errors: Vec<String>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn item_from(entry: &IndexedEntry, now: u64) -> CleanItem {
    CleanItem {
        path: entry.path.clone(),
        name: entry.name.clone(),
        size: entry.size,
        modified_unix: entry.mtime.map(|value| value.max(0) as u64),
        days_old: rules::days_old(entry.mtime, now as i64).map(|days| days.max(0) as u64),
        is_cloud: entry.is_cloud,
    }
}

fn empty_card(id: &str, title: &str, description: &str, action: CardAction) -> CleanCard {
    CleanCard {
        id: id.to_owned(),
        title: title.to_owned(),
        description: description.to_owned(),
        action,
        item_count: 0,
        reclaimable_bytes: 0,
        groups: Vec::new(),
        items: Vec::new(),
        truncated: false,
        skipped: 0,
        ready: false,
    }
}

/// Junk is reported per location: the walk can hold hundreds of thousands of files, so
/// the user picks locations, not individual files.
fn junk_card() -> (CleanCard, junk::RecycleBinInfo) {
    let recycle = junk::recycle_bin_info();
    let targets = junk::junk_targets(&junk::HostProbe);
    let scan = junk::scan(&targets, &|| false);
    let mut groups: Vec<CleanGroup> = scan
        .groups
        .iter()
        .filter(|group| group.bytes > 0 || group.item_count > 0)
        .map(|group| CleanGroup {
            id: group.id.clone(),
            label: group.label.clone(),
            item_count: group.item_count,
            reclaimable_bytes: group.bytes,
        })
        .collect();
    if recycle.available && (recycle.bytes > 0 || recycle.items > 0) {
        groups.push(CleanGroup {
            id: junk::RECYCLE_BIN_ID.to_owned(),
            label: "Recycle Bin".to_owned(),
            item_count: recycle.items,
            reclaimable_bytes: recycle.bytes,
        });
    }
    groups.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));

    let item_count = groups.iter().map(|group| group.item_count).sum::<u64>();
    let reclaimable_bytes = groups.iter().map(|group| group.reclaimable_bytes).sum::<u64>();
    let mut card = empty_card(
        CARD_JUNK,
        "Junk files",
        "Temporary files, browser caches, thumbnails and the Recycle Bin.",
        if groups.is_empty() { CardAction::Unavailable } else { CardAction::DeleteJunk },
    );
    card.groups = groups;
    card.item_count = item_count;
    card.reclaimable_bytes = reclaimable_bytes;
    card.skipped = scan.skipped;
    card.ready = !card.groups.is_empty();
    (card, recycle)
}

fn large_files_card(database: &crate::db::Database, now: u64) -> CleanCard {
    let mut card = empty_card(
        CARD_LARGE,
        "Large files",
        &format!("Files larger than {}.", rules::LARGE_FILE_BYTES / (1024 * 1024)),
        CardAction::DeletePaths,
    );
    let (total_count, total_bytes) = match database.large_file_totals(rules::LARGE_FILE_BYTES) {
        Ok(totals) => totals,
        Err(_) => return card,
    };
    let entries = match database.large_files(rules::LARGE_FILE_BYTES, rules::CARD_LIST_LIMIT) {
        Ok(entries) => entries,
        Err(_) => return card,
    };
    card.item_count = total_count;
    card.reclaimable_bytes = total_bytes;
    card.truncated = total_count > entries.len() as u64;
    card.items = entries.iter().map(|entry| item_from(entry, now)).collect();
    card.ready = total_count > 0;
    card
}

fn downloads_card(database: &crate::db::Database, now: u64) -> CleanCard {
    let mut card = empty_card(
        CARD_DOWNLOADS,
        "Old downloads",
        &format!("Downloads untouched for more than {} days.", rules::OLD_DOWNLOAD_DAYS),
        CardAction::DeletePaths,
    );
    let Some(downloads) = dirs::download_dir() else {
        card.description = "The Downloads folder could not be found.".to_owned();
        return card;
    };
    let cutoff = rules::old_download_cutoff(now as i64);
    let (total_count, total_bytes) = match database.older_than_under_totals(&downloads, cutoff) {
        Ok(totals) => totals,
        Err(_) => return card,
    };
    let entries = match database.files_older_than_under(&downloads, cutoff, rules::CARD_LIST_LIMIT) {
        Ok(entries) => entries,
        Err(_) => return card,
    };
    card.item_count = total_count;
    card.reclaimable_bytes = total_bytes;
    card.truncated = total_count > entries.len() as u64;
    card.items = entries.iter().map(|entry| item_from(entry, now)).collect();
    card.ready = total_count > 0;
    card
}

/// The SQL filter is deliberately loose; [`rules::is_screenshot_name`] has the final say
/// so a file called `capture-card-driver.zip` never ends up on this card.
fn screenshots_card(database: &crate::db::Database, now: u64) -> CleanCard {
    let mut card = empty_card(
        CARD_SCREENSHOTS,
        "Old screenshots",
        "Screenshots and screen captures.",
        CardAction::DeletePaths,
    );
    let folder = dirs::picture_dir().map(|pictures| rules::screenshot_folder(&pictures));
    let prefix = folder
        .as_ref()
        .map(|path| ops::display_path(path))
        .map(|path| descendant_prefix(&path))
        // A prefix nothing can match, so only the name patterns apply.
        .unwrap_or_else(|| "\u{0}no-screenshots-folder\u{0}".to_owned());
    let rows = match database.screenshot_candidates(&prefix, SCREENSHOT_FETCH_LIMIT) {
        Ok(rows) => rows,
        Err(_) => return card,
    };
    let kept: Vec<&ScreenshotRow> = rows
        .iter()
        .filter(|row| rules::is_screenshot_name(&row.entry.name, row.inside_folder))
        .collect();
    card.item_count = kept.len() as u64;
    card.reclaimable_bytes = kept.iter().map(|row| row.entry.size).sum();
    card.items = kept
        .iter()
        .take(rules::CARD_LIST_LIMIT as usize)
        .map(|row| item_from(&row.entry, now))
        .collect();
    card.truncated = rows.len() as u32 >= SCREENSHOT_FETCH_LIMIT || kept.len() > card.items.len();
    card.ready = !kept.is_empty();
    card
}

fn apps_card(installed: &[apps::InstalledApp]) -> CleanCard {
    let mut card = empty_card(
        CARD_APPS,
        "Unused apps",
        "Installed programs, biggest first.",
        CardAction::Uninstall,
    );
    card.item_count = installed.len() as u64;
    card.reclaimable_bytes = installed.iter().map(|app| app.size_bytes).sum();
    card.ready = !installed.is_empty();
    if card.item_count == 0 {
        card.description = "No installed programs were found.".to_owned();
    }
    card
}

fn duplicates_card() -> CleanCard {
    let mut card = empty_card(
        CARD_DUPLICATES,
        "Duplicate files",
        "Identical files found by hashing, one copy kept.",
        CardAction::Unavailable,
    );
    card.description = "Scan to find identical files. Only files above 1 KB are compared.".to_owned();
    card
}

fn summary_impl(database: Arc<crate::db::Database>) -> Result<CleanSummary, String> {
    let now = now_unix();
    let (junk, recycle_bin) = junk_card();
    let installed = apps::installed_apps().unwrap_or_default();
    let apps_bytes = installed.iter().map(|app| app.size_bytes).sum::<u64>();
    let indexed_files = database.counts().map(|counts| counts.indexed_files).unwrap_or(0);
    Ok(CleanSummary {
        cards: vec![
            junk,
            duplicates_card(),
            large_files_card(&database, now),
            downloads_card(&database, now),
            screenshots_card(&database, now),
            apps_card(&installed),
        ],
        recycle_bin,
        apps: installed,
        apps_bytes,
        indexed_files,
        generated_at_unix: now,
    })
}

/// `C:\Users\me\Downloads` → `C:\Users\me\Downloads\` so a prefix match cannot also
/// catch `Downloads 2`.
fn descendant_prefix(path: &str) -> String {
    let separator = if cfg!(windows) { '\\' } else { '/' };
    if path.ends_with('\\') || path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}{separator}")
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_clean_summary(state: State<'_, IndexState>) -> Result<CleanSummary, String> {
    let database = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || summary_impl(database))
        .await
        .map_err(|_| "The Clean summary was interrupted.".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn scan_duplicates(
    app: AppHandle,
    state: State<'_, IndexState>,
    ops: State<'_, FileOpsState>,
) -> Result<duplicates::DuplicateReport, String> {
    let database = state.db.clone();
    let registry = ops.registry_arc();
    tauri::async_runtime::spawn_blocking(move || {
        let sizes = database
            .duplicate_size_groups(rules::MIN_DUPLICATE_BYTES, DUPLICATE_GROUP_LIMIT)
            .map_err(|error| error.to_string())?;
        let entries = database.duplicate_candidates(&sizes).map_err(|error| error.to_string())?;
        let candidates: Vec<duplicates::Candidate> = entries
            .iter()
            .map(|entry| duplicates::Candidate {
                path: PathBuf::from(&entry.path),
                name: entry.name.clone(),
                size: entry.size,
                modified_unix: entry.mtime,
            })
            .collect();
        let bytes_total: u64 = candidates.iter().map(|candidate| candidate.size).sum();
        let (job_id, cancel, initial) = registry.start(
            OpsKind::Scan,
            candidates.len() as u64,
            bytes_total,
            "Duplicate scan".to_owned(),
        );
        let mut emitter = ProgressEmitter::new(&app);
        emitter.push(&initial);
        let report = duplicates::find_duplicates(candidates, &|| cancel.requested(), &mut |bytes| {
            if let Some(progress) = registry.advance_bytes(&job_id, bytes) {
                emitter.push(&progress);
            }
        });
        let cancelled = cancel.requested();
        let (state, message) = if cancelled {
            (OpsState::Cancelled, Some("Scan stopped.".to_owned()))
        } else {
            (OpsState::Completed, None)
        };
        if let Some(progress) = registry.finish(&job_id, state, message) {
            emitter.push(&progress);
        }
        registry.prune_finished(4);
        Ok(report)
    })
    .await
    .map_err(|_| "The duplicate scan was interrupted.".to_owned())?
}

/// Move junk locations to the Recycle Bin. `expected_bytes` only drives the progress bar;
/// the real total comes from the walk.
#[tauri::command]
#[specta::specta]
pub async fn clean_junk(
    group_ids: Vec<String>,
    expected_bytes: u64,
    app: AppHandle,
    ops: State<'_, FileOpsState>,
) -> Result<junk::JunkDeleteResult, String> {
    let registry = ops.registry_arc();
    tauri::async_runtime::spawn_blocking(move || {
        let wanted: Vec<String> = group_ids.iter().map(|id| id.to_lowercase()).collect();
        // The Recycle Bin is a pseudo-location: it is emptied, not moved into itself.
        let wants_bin = wanted.contains(&junk::RECYCLE_BIN_ID.to_lowercase());
        let targets: Vec<junk::JunkTarget> = junk::junk_targets(&junk::HostProbe)
            .into_iter()
            .filter(|target| wanted.contains(&target.id.to_lowercase()))
            .collect();
        if targets.is_empty() && !wants_bin {
            return Err("Select at least one location to clean.".to_owned());
        }
        let (job_id, cancel, initial) =
            registry.start(OpsKind::Clean, 0, expected_bytes, "Recycle Bin".to_owned());
        let mut emitter = ProgressEmitter::new(&app);
        emitter.push(&initial);
        let context = junk::JunkContext { registry: &registry, job_id: &job_id, cancel };
        let mut result = junk::delete(&targets, &context, &mut |progress| emitter.push(&progress));
        if wants_bin {
            let before = junk::recycle_bin_info();
            match junk::empty_recycle_bin() {
                Ok(()) => {
                    result.freed_bytes = result.freed_bytes.saturating_add(before.bytes);
                    result.deleted = result.deleted.saturating_add(before.items);
                }
                Err(error) => {
                    result.skipped += 1;
                    if result.errors.len() < 5 {
                        result.errors.push(format!("Recycle Bin: {error}"));
                    }
                }
            }
        }
        let (final_state, message) = if result.cancelled {
            (OpsState::Cancelled, Some("Stopped.".to_owned()))
        } else if result.skipped > 0 {
            (
                OpsState::Failed,
                Some(format!("{} file(s) are in use and were skipped.", result.skipped)),
            )
        } else {
            (OpsState::Completed, None)
        };
        if let Some(progress) = registry.finish(&job_id, final_state, message) {
            emitter.push(&progress);
        }
        registry.prune_finished(4);
        Ok(result)
    })
    .await
    .map_err(|_| "Cleaning was interrupted.".to_owned())?
}

/// Move a card's selected files to the Recycle Bin and drop them from the index's
/// favourites and recents so nothing points at a file that is gone.
#[tauri::command]
#[specta::specta]
pub async fn clean_paths(
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, IndexState>,
    ops: State<'_, FileOpsState>,
) -> Result<CleanDeleteResult, String> {
    let database = state.db.clone();
    let roots = state.roots().to_vec();
    let registry = ops.registry_arc();
    tauri::async_runtime::spawn_blocking(move || {
        let targets: Vec<PathBuf> = paths
            .iter()
            .map(|path| ops::normal_path(&PathBuf::from(path)))
            .collect();
        if targets.is_empty() {
            return Err("Select at least one file to remove.".to_owned());
        }
        let bytes_total = targets
            .iter()
            .map(|path| {
                std::fs::symlink_metadata(ops::io_path(path))
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            })
            .sum();
        let (job_id, cancel, initial) =
            registry.start(OpsKind::Clean, targets.len() as u64, bytes_total, "Recycle Bin".to_owned());
        let mut emitter = ProgressEmitter::new(&app);
        emitter.push(&initial);
        let context = trash::DeleteContext {
            registry: &registry,
            job_id: &job_id,
            cancel,
            roots: &roots,
        };
        let result = trash::delete_paths(&targets, &context, &mut |progress| emitter.push(&progress));
        let (final_state, message) = trash::finish_state(result.cancelled, result.skipped);
        if let Some(progress) = registry.finish(&job_id, final_state, message) {
            emitter.push(&progress);
        }
        registry.prune_finished(4);
        let _ = database.forget_paths(&paths);
        Ok(CleanDeleteResult {
            freed_bytes: result.moved_bytes,
            moved: result.moved,
            skipped: result.skipped,
            cancelled: result.cancelled,
            errors: result.errors,
        })
    })
    .await
    .map_err(|_| "The cleanup was interrupted.".to_owned())?
}

/// Empty the Recycle Bin and report how much that freed.
#[tauri::command]
#[specta::specta]
pub async fn empty_recycle_bin() -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let before = junk::recycle_bin_info();
        junk::empty_recycle_bin().map_err(|error| error.to_string())?;
        Ok(if before.available { before.bytes } else { 0 })
    })
    .await
    .map_err(|_| "The Recycle Bin could not be emptied.".to_owned())?
}

/// Launch an app's own uninstaller. Windows raises the elevation prompt if it needs one.
#[tauri::command]
#[specta::specta]
pub fn uninstall_app(command: apps::UninstallCommand) -> Result<(), String> {
    if command.executable.trim().is_empty() {
        return Err("This app has no uninstaller.".to_owned());
    }
    std::process::Command::new(&command.executable)
        .args(&command.arguments)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("The uninstaller could not be started: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, name: &str, size: u64, mtime: Option<i64>, is_cloud: bool) -> IndexedEntry {
        IndexedEntry {
            id: 1,
            path: path.to_owned(),
            parent_path: path.rsplit(['\\', '/']).nth(1).unwrap_or("").to_owned(),
            name: name.to_owned(),
            ext: name.rsplit('.').next().unwrap_or("").to_owned(),
            category: "other".to_owned(),
            size,
            mtime,
            ctime: mtime,
            is_hidden: false,
            is_cloud,
            is_directory: false,
            drive: "C:".to_owned(),
        }
    }

    #[test]
    fn card_items_carry_the_age_the_ui_shows() {
        let now = 1_800_000_000;
        let item = item_from(
            &entry("C:\\Users\\me\\Downloads\\old.zip", "old.zip", 4_096, Some(now - 100 * 86_400), false),
            now,
        );
        assert_eq!(item.size, 4_096);
        assert_eq!(item.days_old, Some(100));
        assert_eq!(item.modified_unix, Some((now - 100 * 86_400) as u64));
        assert!(!item.is_cloud);
    }

    #[test]
    fn a_file_from_the_future_is_zero_days_old() {
        let now = 1_800_000_000;
        let item = item_from(&entry("C:\\Users\\me\\a.bin", "a.bin", 10, Some(now + 500), false), now);
        assert_eq!(item.days_old, Some(0));
    }

    #[test]
    fn a_pre_epoch_timestamp_is_reported_as_zero_not_a_huge_unsigned_number() {
        let item = item_from(&entry("C:\\Users\\me\\a.bin", "a.bin", 10, Some(-5), false), 1_800_000_000);
        assert_eq!(item.modified_unix, Some(0));
        assert!(item.days_old.unwrap_or(0) > 20_000, "a 1969 stamp really is that old");
    }

    #[test]
    fn screenshots_survive_only_the_strict_name_test() {
        let shots = vec![
            ScreenshotRow {
                entry: entry("C:\\Users\\me\\Pictures\\Screenshots\\any-name.png", "any-name.png", 100, Some(1), false),
                inside_folder: true,
            },
            ScreenshotRow {
                entry: entry("C:\\Users\\me\\Desktop\\Screenshot 2026.png", "Screenshot 2026.png", 100, Some(1), false),
                inside_folder: false,
            },
            ScreenshotRow {
                entry: entry("C:\\Users\\me\\Desktop\\capture-card-driver.zip", "capture-card-driver.zip", 100, Some(1), false),
                inside_folder: false,
            },
        ];
        let kept: Vec<&str> = shots
            .iter()
            .filter(|row| rules::is_screenshot_name(&row.entry.name, row.inside_folder))
            .map(|row| row.entry.name.as_str())
            .collect();
        assert_eq!(kept, vec!["any-name.png", "Screenshot 2026.png"]);
    }

    #[test]
    fn a_card_with_nothing_in_it_is_not_actionable() {
        let card = apps_card(&[]);
        assert!(!card.ready);
        assert_eq!(card.item_count, 0);
        assert!(card.description.contains("No installed programs"));

        let card = apps_card(&[apps::InstalledApp {
            name: "Blender".to_owned(),
            publisher: "Blender Foundation".to_owned(),
            version: "4.2".to_owned(),
            install_date: Some("2026-01-01".to_owned()),
            size_bytes: 1_000_000,
            uninstall_command: None,
            per_user: false,
            source: apps::AppSource::Machine64,
        }]);
        assert!(card.ready);
        assert_eq!(card.reclaimable_bytes, 1_000_000);
    }

    #[test]
    fn the_summary_always_contains_the_same_six_cards_in_order() {
        let cards = vec![
            empty_card(CARD_JUNK, "Junk files", "", CardAction::DeleteJunk),
            duplicates_card(),
            empty_card(CARD_LARGE, "Large files", "", CardAction::DeletePaths),
            empty_card(CARD_DOWNLOADS, "Old downloads", "", CardAction::DeletePaths),
            empty_card(CARD_SCREENSHOTS, "Old screenshots", "", CardAction::DeletePaths),
            empty_card(CARD_APPS, "Unused apps", "", CardAction::Uninstall),
        ];
        let ids: Vec<&str> = cards.iter().map(|card| card.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["junk", "duplicates", "large-files", "old-downloads", "old-screenshots", "unused-apps"]
        );
        assert!(!cards[1].ready, "duplicates wait for their own scan");
        assert_eq!(cards[1].action, CardAction::Unavailable);
    }

    #[test]
    fn the_prefix_helper_cannot_match_a_sibling_folder() {
        assert_eq!(descendant_prefix("C:\\Users\\me\\Downloads"), "C:\\Users\\me\\Downloads\\");
        assert_eq!(descendant_prefix("C:\\Users\\me\\Downloads\\"), "C:\\Users\\me\\Downloads\\");
        assert!(!"C:\\Users\\me\\Downloads 2\\x.bin".starts_with(&descendant_prefix("C:\\Users\\me\\Downloads")));
    }
}
