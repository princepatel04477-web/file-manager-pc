//! Pure destination/conflict resolution for copy, move, and rename.
//!
//! Everything in this module is free of filesystem calls: the caller supplies an
//! [`FsView`], so the planning rules are exercised directly by unit tests.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Longest allowed single path component on Windows, measured in UTF-16 code units.
pub const MAX_COMPONENT_LEN: usize = 255;

const ILLEGAL_NAME_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED_STEMS: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// What the user chose for one destination that already holds an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ConflictAction {
    Replace,
    Skip,
    KeepBoth,
    /// Planning only: surface the conflict to the user before touching anything.
    Ask,
}

impl ConflictAction {
    pub fn is_terminal(self) -> bool {
        !matches!(self, ConflictAction::Ask)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TransferKind {
    Copy,
    Move,
}

/// One resolved source item inside a [`TransferPlan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PlannedAction {
    /// Destination is free; create it.
    Create,
    /// Destination exists and the user chose Replace.
    Replace,
    /// Destination exists and the user chose Skip.
    Skip,
    /// Destination exists and no decision was supplied yet.
    Conflict,
}

/// Why a source item is excluded from a transfer before any conflict is considered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BlockedReason {
    OutsideUserFiles,
    Missing,
    ReparsePoint,
    CloudOnly,
    SameItem,
    InsideItself,
    InvalidName,
    DestinationUnavailable,
}

impl BlockedReason {
    pub fn message(self) -> &'static str {
        match self {
            BlockedReason::OutsideUserFiles => "This item is outside your user folders.",
            BlockedReason::Missing => "This item no longer exists.",
            BlockedReason::ReparsePoint => "This item is a Windows link and is not followed.",
            BlockedReason::CloudOnly => "This item is online-only. Download it in Windows first.",
            BlockedReason::SameItem => "The source and destination are the same item.",
            BlockedReason::InsideItself => "A folder cannot be copied or moved into itself.",
            BlockedReason::InvalidName => "The destination name is not valid on Windows.",
            BlockedReason::DestinationUnavailable => "The destination folder is not available.",
        }
    }
}

/// Metadata for a path, provided by the caller so planning stays side-effect free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemMeta {
    pub is_directory: bool,
    pub size: u64,
    pub modified_unix: Option<u64>,
    pub is_cloud: bool,
    pub is_reparse: bool,
    pub in_user_scope: bool,
}

pub trait FsView {
    /// `None` when the path does not exist or cannot be inspected.
    fn meta(&self, path: &Path) -> Option<ItemMeta>;
}

/// One source/destination pair, with its resolved action.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlannedItem {
    pub source: String,
    pub destination: String,
    pub name: String,
    pub is_directory: bool,
    pub size: u64,
    pub action: PlannedAction,
}

/// A destination collision the user still has to decide on.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlannedConflict {
    pub source: String,
    pub destination: String,
    pub name: String,
    pub source_is_directory: bool,
    pub source_size: u64,
    pub source_modified_unix: Option<u64>,
    pub existing_is_directory: bool,
    pub existing_size: u64,
    pub existing_modified_unix: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockedItem {
    pub source: String,
    pub reason: BlockedReason,
}

/// The full result of planning: what will happen, what needs a decision, what cannot run.
#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferPlan {
    pub kind: String,
    pub destination: String,
    pub items: Vec<PlannedItem>,
    pub conflicts: Vec<PlannedConflict>,
    pub blocked: Vec<BlockedItem>,
    pub bytes_total: u64,
}

impl TransferPlan {
    pub fn actionable(&self) -> impl Iterator<Item = &PlannedItem> {
        self.items
            .iter()
            .filter(|item| !matches!(item.action, PlannedAction::Skip | PlannedAction::Conflict))
    }

    pub fn pending_decisions(&self) -> usize {
        self.conflicts.len()
    }

    pub fn transferable_bytes(&self) -> u64 {
        self.actionable().map(|item| item.size).sum()
    }
}

/// A user decision for one specific source path.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDecision {
    pub source: String,
    pub action: ConflictAction,
}

/// Look up the decision recorded for `source`, if any.
pub fn decision_for(decisions: &[ConflictDecision], source: &str) -> Option<ConflictAction> {
    decisions
        .iter()
        .find(|decision| same_name(&decision.source, source))
        .map(|decision| decision.action)
}

/// Case-insensitive on Windows, byte-exact elsewhere. Used for name/decision matching.
pub fn same_name(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

/// Windows rejects these names outright; rename and paste must refuse them before
/// the filesystem returns a confusing OS error.
pub fn validate_name(name: &str) -> Result<(), BlockedReason> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(BlockedReason::InvalidName);
    }
    if utf16_len(name) > MAX_COMPONENT_LEN {
        return Err(BlockedReason::InvalidName);
    }
    if name.chars().any(|character| {
        ILLEGAL_NAME_CHARS.contains(&character) || (character as u32) < 0x20 || character as u32 == 0x7f
    }) {
        return Err(BlockedReason::InvalidName);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(BlockedReason::InvalidName);
    }
    if name.chars().all(|character| character == '.' || character == ' ') {
        return Err(BlockedReason::InvalidName);
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    if RESERVED_STEMS.contains(&stem.as_str()) {
        return Err(BlockedReason::InvalidName);
    }
    Ok(())
}

/// `report.pdf` + `report.pdf` becomes `report (1).pdf`; the counter keeps climbing
/// while the name is taken. `taken` holds names already claimed by this batch.
pub fn unique_file_name(name: &str, taken: &HashSet<String>) -> String {
    let (stem, extension) = split_name(name);
    let mut counter = 1_u32;
    loop {
        let candidate = compose_name(stem, extension, Some(counter));
        if !taken.iter().any(|existing| same_name(existing, &candidate)) {
            return candidate;
        }
        counter = counter.saturating_add(1);
        if counter > 10_000 {
            // Pathological case: fall back to something that cannot collide further.
            return compose_name(&format!("{stem}-{}", std::process::id()), extension, Some(counter));
        }
    }
}

/// Split a file name into the part that gets the counter and the extension.
/// Matches Windows Explorer: `archive.tar.gz` -> (`archive.tar`, `.gz`),
/// `.env` -> (`.env`, ``), `report` -> (`report`, ``).
pub fn split_name(name: &str) -> (&str, &str) {
    let bytes = name.as_bytes();
    match bytes.iter().rposition(|byte| *byte == b'.') {
        // A leading dot is part of the name, not an extension separator.
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    }
}

fn compose_name(stem: &str, extension: &str, counter: Option<u32>) -> String {
    let budget = MAX_COMPONENT_LEN.saturating_sub(utf16_len(extension));
    let body = match counter {
        None => truncate_stem(stem, budget).to_owned(),
        Some(value) => {
            let suffix = format!(" ({value})");
            let budget = budget.saturating_sub(utf16_len(&suffix));
            format!("{}{}", truncate_stem(stem, budget), suffix)
        }
    };
    let mut name = String::with_capacity(body.len() + extension.len());
    name.push_str(&body);
    name.push_str(extension);
    name
}

/// Keep the component inside the Windows per-component limit by trimming the stem
/// from the end; the counter suffix and extension always survive.
fn truncate_stem(stem: &str, budget: usize) -> &str {
    if utf16_len(stem) <= budget {
        return stem;
    }
    let mut used = 0_usize;
    let mut cut = 0_usize;
    for (index, character) in stem.char_indices() {
        let width = character.len_utf16();
        if used + width > budget {
            break;
        }
        used += width;
        cut = index + character.len_utf8();
    }
    let trimmed = stem[..cut].trim_end();
    if trimmed.is_empty() { "file" } else { trimmed }
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

/// Destination for `source` inside `destination_dir`, i.e. `destination_dir\<name>`.
pub fn destination_in(destination_dir: &Path, source: &Path) -> Option<PathBuf> {
    let name = source.file_name()?;
    Some(destination_dir.join(name))
}

/// True when `candidate` is `ancestor` or lives inside it. Blocks folder-into-itself moves.
pub fn is_within_or_equal(candidate: &Path, ancestor: &Path) -> bool {
    crate::ops::is_within(candidate, ancestor)
}

/// Plan a copy/move batch. No filesystem writes happen here.
pub fn plan_transfer(
    kind: TransferKind,
    sources: &[PathBuf],
    destination_dir: &Path,
    decisions: &[ConflictDecision],
    default_action: ConflictAction,
    view: &dyn FsView,
) -> TransferPlan {
    let kind_label = match kind {
        TransferKind::Copy => "copy",
        TransferKind::Move => "move",
    };
    let destination = crate::ops::display_path(destination_dir);
    let mut plan = TransferPlan {
        kind: kind_label.to_owned(),
        destination,
        ..TransferPlan::default()
    };

    let destination_meta = view.meta(destination_dir);
    let destination_ready = matches!(destination_meta, Some(meta) if meta.is_directory && !meta.is_reparse);

    // Names already claimed in this batch, so `a.txt` from two folders cannot both
    // resolve to the same "keep both" name.
    let mut taken: HashSet<String> = HashSet::new();

    for source in sources {
        let source_text = crate::ops::display_path(source);
        let name = source
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default();

        if validate_name(&name).is_err() {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::InvalidName });
            continue;
        }
        let Some(meta) = view.meta(source) else {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::Missing });
            continue;
        };
        if !meta.in_user_scope {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::OutsideUserFiles });
            continue;
        }
        if meta.is_reparse {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::ReparsePoint });
            continue;
        }
        if meta.is_cloud {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::CloudOnly });
            continue;
        }
        if !destination_ready {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::DestinationUnavailable });
            continue;
        }
        let Some(target) = destination_in(destination_dir, source) else {
            plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::InvalidName });
            continue;
        };

        if crate::ops::same_path(source, &target) {
            // Pasting into the folder the item already lives in.
            if matches!(kind, TransferKind::Move) {
                plan.blocked.push(BlockedItem { source: source_text, reason: BlockedReason::SameItem });
            } else {
                // Copy-in-place is a legitimate "keep both" duplication.
                let renamed = unique_file_name(&name, &taken);
                taken.insert(renamed.clone());
                let destination_path = destination_dir.join(&renamed);
                plan.items.push(PlannedItem {
                    source: source_text.clone(),
                    destination: crate::ops::display_path(&destination_path),
                    name: renamed,
                    is_directory: meta.is_directory,
                    size: meta.size,
                    action: PlannedAction::Create,
                });
            }
            continue;
        }
        if meta.is_directory && is_within_or_equal(destination_dir, source) {
            // Dropping a folder onto itself is a no-op worth naming plainly; only a
            // destination strictly inside the source is a containment error.
            let reason = if crate::ops::same_path(destination_dir, source) {
                BlockedReason::SameItem
            } else {
                BlockedReason::InsideItself
            };
            plan.blocked.push(BlockedItem { source: source_text, reason });
            continue;
        }

        let target_text = crate::ops::display_path(&target);
        let existing = view.meta(&target);
        let collision = existing.is_some()
            || taken.iter().any(|claimed| same_name(claimed, &name));

        if !collision {
            taken.insert(name.clone());
            plan.items.push(PlannedItem {
                source: source_text,
                destination: target_text,
                name,
                is_directory: meta.is_directory,
                size: meta.size,
                action: PlannedAction::Create,
            });
            continue;
        }

        let action = decision_for(decisions, &source_text).unwrap_or(default_action);
        match action {
            ConflictAction::Skip => {
                plan.items.push(PlannedItem {
                    source: source_text,
                    destination: target_text,
                    name,
                    is_directory: meta.is_directory,
                    size: meta.size,
                    action: PlannedAction::Skip,
                });
            }
            ConflictAction::Replace => {
                taken.insert(name.clone());
                plan.items.push(PlannedItem {
                    source: source_text,
                    destination: target_text,
                    name,
                    is_directory: meta.is_directory,
                    size: meta.size,
                    action: PlannedAction::Replace,
                });
            }
            ConflictAction::KeepBoth => {
                let renamed = unique_file_name(&name, &taken);
                taken.insert(renamed.clone());
                let destination_path = destination_dir.join(&renamed);
                plan.items.push(PlannedItem {
                    source: source_text,
                    destination: crate::ops::display_path(&destination_path),
                    name: renamed,
                    is_directory: meta.is_directory,
                    size: meta.size,
                    action: PlannedAction::Create,
                });
            }
            ConflictAction::Ask => {
                let existing = existing.unwrap_or(ItemMeta {
                    is_directory: false,
                    size: 0,
                    modified_unix: None,
                    is_cloud: false,
                    is_reparse: false,
                    in_user_scope: true,
                });
                plan.conflicts.push(PlannedConflict {
                    source: source_text.clone(),
                    destination: target_text,
                    name,
                    source_is_directory: meta.is_directory,
                    source_size: meta.size,
                    source_modified_unix: meta.modified_unix,
                    existing_is_directory: existing.is_directory,
                    existing_size: existing.size,
                    existing_modified_unix: existing.modified_unix,
                });
                plan.items.push(PlannedItem {
                    source: source_text,
                    destination: crate::ops::display_path(&target),
                    name: source
                        .file_name()
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    is_directory: meta.is_directory,
                    size: meta.size,
                    action: PlannedAction::Conflict,
                });
            }
        }
    }

    plan.bytes_total = plan.transferable_bytes();
    plan
}

/// Re-plan with the decisions the user just made, turning every `Ask` into a real action.
pub fn resolve_conflicts(
    plan: &TransferPlan,
    decisions: &[ConflictDecision],
    default_action: ConflictAction,
) -> TransferPlan {
    let mut resolved = plan.clone();
    resolved.conflicts.clear();
    let mut taken: HashSet<String> = resolved
        .items
        .iter()
        .filter(|item| !matches!(item.action, PlannedAction::Conflict))
        .map(|item| item.name.clone())
        .collect();
    let parent = PathBuf::from(&resolved.destination);
    let pending = std::mem::take(&mut resolved.items);

    resolved.items = pending
        .into_iter()
        .map(|mut item| {
            if !matches!(item.action, PlannedAction::Conflict) {
                return item;
            }
            let action = decision_for(decisions, &item.source).unwrap_or(default_action);
            match action {
                ConflictAction::Skip => item.action = PlannedAction::Skip,
                ConflictAction::Replace => item.action = PlannedAction::Replace,
                _ => {
                    let renamed = unique_file_name(&item.name, &taken);
                    taken.insert(renamed.clone());
                    item.destination = crate::ops::display_path(&parent.join(&renamed));
                    item.name = renamed;
                    item.action = PlannedAction::Create;
                }
            }
            item
        })
        .collect();
    resolved.bytes_total = resolved.transferable_bytes();
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory filesystem for planning tests.
    struct FakeFs {
        entries: HashMap<String, ItemMeta>,
    }

    impl FakeFs {
        fn new() -> Self {
            Self { entries: HashMap::new() }
        }

        fn file(mut self, path: &str, size: u64) -> Self {
            self.entries.insert(
                key(path),
                ItemMeta {
                    is_directory: false,
                    size,
                    modified_unix: Some(1_700_000_000),
                    is_cloud: false,
                    is_reparse: false,
                    in_user_scope: true,
                },
            );
            self
        }

        fn dir(mut self, path: &str) -> Self {
            self.entries.insert(
                key(path),
                ItemMeta {
                    is_directory: true,
                    size: 0,
                    modified_unix: Some(1_700_000_000),
                    is_cloud: false,
                    is_reparse: false,
                    in_user_scope: true,
                },
            );
            self
        }

        fn cloud(mut self, path: &str) -> Self {
            self.entries.insert(
                key(path),
                ItemMeta {
                    is_directory: false,
                    size: 10,
                    modified_unix: None,
                    is_cloud: true,
                    is_reparse: false,
                    in_user_scope: true,
                },
            );
            self
        }

        fn reparse(mut self, path: &str) -> Self {
            self.entries.insert(
                key(path),
                ItemMeta {
                    is_directory: false,
                    size: 0,
                    modified_unix: None,
                    is_cloud: false,
                    is_reparse: true,
                    in_user_scope: true,
                },
            );
            self
        }
    }

    fn key(path: &str) -> String {
        crate::ops::path_key(&PathBuf::from(path))
    }

    impl FsView for FakeFs {
        fn meta(&self, path: &Path) -> Option<ItemMeta> {
            self.entries.get(&key(&path.to_string_lossy())).copied()
        }
    }

    fn path(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[test]
    fn unique_names_increment_while_taken() {
        let mut taken: HashSet<String> = HashSet::new();
        let first = unique_file_name("report.pdf", &taken);
        assert_eq!(first, "report (1).pdf");
        taken.insert("report.pdf".to_owned());
        taken.insert(first.clone());
        let second = unique_file_name("report.pdf", &taken);
        assert_eq!(second, "report (2).pdf");
    }

    #[test]
    fn unique_names_keep_compound_and_hidden_names_intact() {
        let taken = HashSet::new();
        assert_eq!(unique_file_name("archive.tar.gz", &taken), "archive.tar (1).gz");
        assert_eq!(unique_file_name(".env", &taken), ".env (1)");
        assert_eq!(unique_file_name("README", &taken), "README (1)");
        assert_eq!(unique_file_name("photo.min.jpeg", &taken), "photo.min (1).jpeg");
    }

    #[test]
    fn unique_names_stay_inside_the_windows_component_limit() {
        let long_stem = "a".repeat(400);
        let name = format!("{long_stem}.txt");
        let unique = unique_file_name(&name, &HashSet::new());
        assert!(utf16_len(&unique) <= MAX_COMPONENT_LEN, "component was {} units", utf16_len(&unique));
        assert!(unique.ends_with(" (1).txt"), "suffix survived: {unique}");
    }

    #[test]
    fn name_validation_rejects_windows_illegal_names() {
        for invalid in ["", ".", "..", "con", "CON.txt", "lpt9", "bad<>name", "tail.", "trail ", "a\rb", "nul.log"] {
            assert_eq!(validate_name(invalid), Err(BlockedReason::InvalidName), "{invalid:?} should be rejected");
        }
        for valid in ["report.pdf", ".env", "my file (1).txt", "consoles", "a".repeat(200).as_str()] {
            assert_eq!(validate_name(valid), Ok(()), "{valid:?} should be accepted");
        }
    }

    #[test]
    fn copy_into_a_busy_folder_plans_one_conflict_per_collision() {
        let fs = FakeFs::new()
            .dir("/home/u/docs")
            .dir("/home/u/pics")
            .file("/home/u/docs/report.pdf", 1_024)
            .file("/home/u/pics/report.pdf", 2_048)
            // notes.txt lives beside the source and has no twin at the destination,
            // which is what makes it the non-colliding case this test checks.
            .file("/home/u/docs/notes.txt", 12);
        let plan = plan_transfer(
            TransferKind::Copy,
            &[path("/home/u/docs/report.pdf"), path("/home/u/docs/notes.txt")],
            &path("/home/u/pics"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert_eq!(plan.conflicts.len(), 1, "only report.pdf collides");
        assert_eq!(plan.conflicts[0].source, "/home/u/docs/report.pdf");
        assert_eq!(plan.conflicts[0].source_size, 1_024);
        assert_eq!(plan.conflicts[0].existing_size, 2_048);
        assert_eq!(plan.pending_decisions(), 1);
        // The non-colliding file is still ready to go.
        assert_eq!(plan.items.iter().filter(|item| matches!(item.action, PlannedAction::Create)).count(), 1);
        assert_eq!(plan.actionable().count(), 1);
    }

    #[test]
    fn keep_both_gives_each_colliding_file_its_own_number() {
        let fs = FakeFs::new()
            .dir("/src/a").dir("/src/b").dir("/dst")
            .file("/src/a/photo.jpg", 10)
            .file("/src/b/photo.jpg", 20)
            .file("/dst/photo.jpg", 30);
        let plan = plan_transfer(
            TransferKind::Copy,
            &[path("/src/a/photo.jpg"), path("/src/b/photo.jpg")],
            &path("/dst"),
            &[],
            ConflictAction::KeepBoth,
            &fs,
        );
        let names: Vec<String> = plan.items.iter().map(|item| item.name.clone()).collect();
        assert_eq!(names, vec!["photo (1).jpg".to_owned(), "photo (2).jpg".to_owned()]);
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.actionable().count(), 2);
        assert_eq!(plan.transferable_bytes(), 30);
    }

    #[test]
    fn replace_and_skip_are_applied_per_source() {
        let fs = FakeFs::new()
            .dir("/src").dir("/dst")
            .file("/src/keep.txt", 5)
            .file("/src/drop.txt", 7)
            .file("/dst/keep.txt", 1)
            .file("/dst/drop.txt", 1);
        let decisions = vec![
            ConflictDecision { source: "/src/keep.txt".to_owned(), action: ConflictAction::Replace },
            ConflictDecision { source: "/src/drop.txt".to_owned(), action: ConflictAction::Skip },
        ];
        let plan = plan_transfer(
            TransferKind::Copy,
            &[path("/src/keep.txt"), path("/src/drop.txt")],
            &path("/dst"),
            &decisions,
            ConflictAction::Ask,
            &fs,
        );
        let keep = plan.items.iter().find(|item| item.source == "/src/keep.txt").expect("keep planned");
        let drop = plan.items.iter().find(|item| item.source == "/src/drop.txt").expect("drop planned");
        assert_eq!(keep.action, PlannedAction::Replace);
        assert_eq!(drop.action, PlannedAction::Skip);
        assert!(plan.conflicts.is_empty(), "every conflict had a decision");
        assert_eq!(plan.transferable_bytes(), 5);
    }

    #[test]
    fn moving_a_folder_into_itself_or_its_children_is_blocked() {
        let fs = FakeFs::new().dir("/u/docs").dir("/u/docs/2024").dir("/u/docs/2024/jan");
        let into_child = plan_transfer(
            TransferKind::Move,
            &[path("/u/docs")],
            &path("/u/docs/2024/jan"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert_eq!(into_child.blocked.len(), 1);
        assert_eq!(into_child.blocked[0].reason, BlockedReason::InsideItself);
        assert!(into_child.items.is_empty());

        let into_self = plan_transfer(
            TransferKind::Move,
            &[path("/u/docs/2024")],
            &path("/u/docs/2024"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert_eq!(into_self.blocked[0].reason, BlockedReason::SameItem);
    }

    #[test]
    fn copy_in_place_duplicates_instead_of_erroring() {
        let fs = FakeFs::new().dir("/u/docs").file("/u/docs/report.pdf", 64);
        let plan = plan_transfer(
            TransferKind::Copy,
            &[path("/u/docs/report.pdf")],
            &path("/u/docs"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert!(plan.blocked.is_empty());
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].name, "report (1).pdf");
        assert_eq!(plan.items[0].action, PlannedAction::Create);

        let moved = plan_transfer(
            TransferKind::Move,
            &[path("/u/docs/report.pdf")],
            &path("/u/docs"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert_eq!(moved.blocked[0].reason, BlockedReason::SameItem);
    }

    #[test]
    fn cloud_placeholders_and_reparse_points_never_enter_the_plan() {
        let fs = FakeFs::new()
            .dir("/src").dir("/dst")
            .cloud("/src/online.docx")
            .reparse("/src/link.txt")
            .file("/src/real.txt", 3);
        let plan = plan_transfer(
            TransferKind::Copy,
            &[path("/src/online.docx"), path("/src/link.txt"), path("/src/real.txt")],
            &path("/dst"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        let reasons: Vec<BlockedReason> = plan.blocked.iter().map(|item| item.reason).collect();
        assert_eq!(reasons, vec![BlockedReason::CloudOnly, BlockedReason::ReparsePoint]);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].name, "real.txt");
    }

    #[test]
    fn missing_sources_and_unusable_destinations_are_reported_not_fatal() {
        // gone.txt is deliberately absent: the point is that a source which is not
        // there is reported rather than panicking the plan.
        let fs = FakeFs::new().dir("/dst");
        let plan = plan_transfer(
            TransferKind::Copy,
            &[path("/src/gone.txt")],
            &path("/dst"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert_eq!(plan.blocked[0].reason, BlockedReason::Missing);

        let no_destination = plan_transfer(
            TransferKind::Copy,
            &[path("/src/gone.txt")],
            &path("/nowhere"),
            &[],
            ConflictAction::Ask,
            &FakeFs::new(),
        );
        assert_eq!(no_destination.blocked[0].reason, BlockedReason::Missing);
    }

    #[test]
    fn resolve_conflicts_turns_ask_into_concrete_actions() {
        let fs = FakeFs::new()
            .dir("/src").dir("/dst")
            .file("/src/a.txt", 10)
            .file("/src/b.txt", 20)
            .file("/dst/a.txt", 1)
            .file("/dst/b.txt", 2);
        let pending = plan_transfer(
            TransferKind::Copy,
            &[path("/src/a.txt"), path("/src/b.txt")],
            &path("/dst"),
            &[],
            ConflictAction::Ask,
            &fs,
        );
        assert_eq!(pending.conflicts.len(), 2);

        let resolved = resolve_conflicts(
            &pending,
            &[ConflictDecision { source: "/src/a.txt".to_owned(), action: ConflictAction::Replace }],
            ConflictAction::KeepBoth,
        );
        assert!(resolved.conflicts.is_empty());
        let a = resolved.items.iter().find(|item| item.source == "/src/a.txt").expect("a planned");
        let b = resolved.items.iter().find(|item| item.source == "/src/b.txt").expect("b planned");
        assert_eq!(a.action, PlannedAction::Replace);
        assert_eq!(a.name, "a.txt");
        assert_eq!(b.action, PlannedAction::Create);
        assert_eq!(b.name, "b (1).txt");
        assert_eq!(resolved.transferable_bytes(), 30);
    }

    #[test]
    fn skip_all_leaves_the_destination_untouched_in_the_plan() {
        let fs = FakeFs::new().dir("/src").dir("/dst").file("/src/x.bin", 9).file("/dst/x.bin", 9);
        let plan = plan_transfer(
            TransferKind::Move,
            &[path("/src/x.bin")],
            &path("/dst"),
            &[],
            ConflictAction::Skip,
            &fs,
        );
        assert_eq!(plan.items[0].action, PlannedAction::Skip);
        assert_eq!(plan.transferable_bytes(), 0);
        assert_eq!(plan.actionable().count(), 0);
    }

    #[test]
    fn blocked_reasons_all_carry_user_copy() {
        for reason in [
            BlockedReason::OutsideUserFiles,
            BlockedReason::Missing,
            BlockedReason::ReparsePoint,
            BlockedReason::CloudOnly,
            BlockedReason::SameItem,
            BlockedReason::InsideItself,
            BlockedReason::InvalidName,
            BlockedReason::DestinationUnavailable,
        ] {
            assert!(!reason.message().is_empty());
        }
    }
}
