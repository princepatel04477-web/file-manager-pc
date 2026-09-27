//! Copy, move, and rename execution.
//!
//! A [`TransferPlan`] (from [`crate::ops::conflict`]) decides *what* happens; this module
//! performs it, streaming bytes, reporting progress, and honouring cancellation.

use crate::error::AppError;
use crate::ops::conflict::{
    BlockedReason, FsView, ItemMeta, PlannedAction, PlannedItem, TransferPlan, TransferKind,
};
use crate::ops::progress::{CancelToken, OpsRegistry, OperationProgress, OpsState, CANCEL_CHECK_BYTES};
use crate::ops;
use serde::Serialize;
use specta::Type;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const BUFFER_BYTES: usize = 1024 * 1024;

/// Outcome of one copy/move/rename/delete request.
#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TransferResult {
    pub completed: u64,
    pub skipped: u64,
    pub failed: u64,
    pub bytes: u64,
    pub cancelled: bool,
    pub destinations: Vec<String>,
    pub errors: Vec<String>,
}

/// Everything the executor needs from the caller.
pub struct TransferContext<'a> {
    pub registry: &'a OpsRegistry,
    pub job_id: &'a str,
    pub cancel: CancelToken,
    pub roots: &'a [PathBuf],
}

/// Reads real metadata for planning, without following reparse points.
pub struct DiskView<'a> {
    roots: &'a [PathBuf],
}

impl<'a> DiskView<'a> {
    pub fn new(roots: &'a [PathBuf]) -> Self {
        Self { roots }
    }
}

impl FsView for DiskView<'_> {
    fn meta(&self, path: &Path) -> Option<ItemMeta> {
        let metadata = fs::symlink_metadata(ops::io_path(path)).ok()?;
        let attributes = ops::attributes(&metadata);
        Some(ItemMeta {
            is_directory: metadata.is_dir(),
            size: metadata.len(),
            modified_unix: modified_unix(&metadata),
            is_cloud: ops::is_cloud(attributes),
            is_reparse: ops::is_reparse(&metadata),
            in_user_scope: ops::root_for(path, self.roots).is_some(),
        })
    }
}

fn modified_unix(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
}

/// Run a planned copy or move batch.
pub fn execute(
    plan: &TransferPlan,
    context: &TransferContext<'_>,
    emit: &mut dyn FnMut(OperationProgress),
) -> TransferResult {
    let mut result = TransferResult::default();
    for item in &plan.items {
        if context.cancel.requested() {
            result.cancelled = true;
            break;
        }
        match item.action {
            PlannedAction::Skip => {
                result.skipped += 1;
                if let Some(progress) = context.registry.skip_item(context.job_id) {
                    emit(progress);
                }
                continue;
            }
            PlannedAction::Conflict => {
                // Planning left this undecided; the command layer must not reach here.
                result.skipped += 1;
                continue;
            }
            PlannedAction::Create | PlannedAction::Replace => {}
        }

        let source = PathBuf::from(&item.source);
        let destination = PathBuf::from(&item.destination);
        if let Some(progress) = context.registry.set_current(context.job_id, &item.name) {
            emit(progress);
        }

        let outcome = match plan.kind.as_str() {
            "move" => move_item(&source, &destination, item, context, emit),
            _ => copy_item(&source, &destination, item, context, emit),
        };

        match outcome {
            Ok(bytes) => {
                result.completed += 1;
                result.bytes = result.bytes.saturating_add(bytes);
                result.destinations.push(ops::display_path(&destination));
                if let Some(progress) = context.registry.finish_item(context.job_id, 0) {
                    emit(progress);
                }
            }
            Err(AppError::Cancelled) => {
                result.cancelled = true;
                break;
            }
            Err(error) => {
                result.failed += 1;
                result.errors.push(format!("{}: {}", item.name, error));
                if let Some(progress) = context.registry.skip_item(context.job_id) {
                    emit(progress);
                }
            }
        }
    }
    result
}

fn copy_item(
    source: &Path,
    destination: &Path,
    item: &PlannedItem,
    context: &TransferContext<'_>,
    emit: &mut dyn FnMut(OperationProgress),
) -> Result<u64, AppError> {
    if matches!(item.action, PlannedAction::Replace) {
        remove_destination(destination)?;
    }
    if item.is_directory {
        fs::create_dir_all(ops::io_path(destination))?;
        copy_tree(source, destination, context, emit)
    } else {
        copy_file(source, destination, context, emit)
    }
}

fn move_item(
    source: &Path,
    destination: &Path,
    item: &PlannedItem,
    context: &TransferContext<'_>,
    emit: &mut dyn FnMut(OperationProgress),
) -> Result<u64, AppError> {
    if matches!(item.action, PlannedAction::Replace) {
        remove_destination(destination)?;
    }
    // Same-volume renames are atomic and instant; only fall back to copy+delete when
    // Windows refuses (a different volume, or a folder crossing volumes).
    if fs::rename(ops::io_path(source), ops::io_path(destination)).is_ok() {
        return Ok(0);
    }
    let bytes = copy_item(source, destination, &PlannedItem { action: PlannedAction::Create, ..item.clone() }, context, emit)?;
    remove_source(source, item.is_directory)?;
    Ok(bytes)
}

fn remove_destination(destination: &Path) -> Result<(), AppError> {
    let metadata = match fs::symlink_metadata(ops::io_path(destination)) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(()),
    };
    if metadata.is_dir() && !ops::is_reparse(&metadata) {
        fs::remove_dir_all(ops::io_path(destination))?;
    } else {
        fs::remove_file(ops::io_path(destination))?;
    }
    Ok(())
}

fn remove_source(source: &Path, is_directory: bool) -> Result<(), AppError> {
    if is_directory {
        fs::remove_dir_all(ops::io_path(source))?;
    } else {
        fs::remove_file(ops::io_path(source))?;
    }
    Ok(())
}

/// Stream one file with progress and cancellation, preserving the modified time.
fn copy_file(
    source: &Path,
    destination: &Path,
    context: &TransferContext<'_>,
    emit: &mut dyn FnMut(OperationProgress),
) -> Result<u64, AppError> {
    let mut input = fs::File::open(ops::io_path(source))?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(ops::io_path(parent))?;
    }
    let mut output = fs::File::create(ops::io_path(destination))?;
    let mut buffer = vec![0_u8; BUFFER_BYTES];
    let mut copied = 0_u64;
    let mut pending_report = 0_u64;
    loop {
        if context.cancel.requested() {
            drop(output);
            let _ = fs::remove_file(ops::io_path(destination));
            return Err(AppError::Cancelled);
        }
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        copied += read as u64;
        pending_report += read as u64;
        if pending_report >= CANCEL_CHECK_BYTES {
            if let Some(progress) = context.registry.advance_bytes(context.job_id, pending_report) {
                emit(progress);
            }
            pending_report = 0;
        }
    }
    output.flush()?;
    if pending_report > 0 {
        if let Some(progress) = context.registry.advance_bytes(context.job_id, pending_report) {
            emit(progress);
        }
    }
    if let Ok(modified) = input.metadata().and_then(|metadata| metadata.modified()) {
        let _ = output.set_modified(modified);
    }
    Ok(copied)
}

/// Recursively copy a folder. Reparse points are skipped rather than followed.
fn copy_tree(
    source: &Path,
    destination: &Path,
    context: &TransferContext<'_>,
    emit: &mut dyn FnMut(OperationProgress),
) -> Result<u64, AppError> {
    let mut total = 0_u64;
    let entries = fs::read_dir(ops::io_path(source))?;
    for entry in entries {
        if context.cancel.requested() {
            return Err(AppError::Cancelled);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let child_source = ops::normal_path(&entry.path());
        let Ok(metadata) = fs::symlink_metadata(ops::io_path(&child_source)) else { continue };
        if ops::is_reparse(&metadata) {
            continue;
        }
        let name = ops::file_name(&child_source);
        let child_destination = destination.join(&name);
        if metadata.is_dir() {
            fs::create_dir_all(ops::io_path(&child_destination))?;
            total += copy_tree(&child_source, &child_destination, context, emit)?;
        } else {
            if let Some(progress) = context.registry.set_current(context.job_id, &name) {
                emit(progress);
            }
            total += copy_file(&child_source, &child_destination, context, emit)?;
        }
    }
    Ok(total)
}

/// Rename inside the same folder. The caller has already validated the new name.
pub fn rename(source: &Path, new_name: &str) -> Result<PathBuf, AppError> {
    let parent = source.parent().ok_or(AppError::InvalidRequest)?;
    let destination = parent.join(new_name);
    if ops::same_path(source, &destination) {
        return Ok(destination);
    }
    if fs::symlink_metadata(ops::io_path(&destination)).is_ok() {
        return Err(AppError::Conflict);
    }
    fs::rename(ops::io_path(source), ops::io_path(&destination))?;
    Ok(ops::normal_path(&destination))
}

/// Rename that overwrites an existing item, used after the user chose Replace.
pub fn rename_replace(source: &Path, new_name: &str) -> Result<PathBuf, AppError> {
    let parent = source.parent().ok_or(AppError::InvalidRequest)?;
    let destination = parent.join(new_name);
    if ops::same_path(source, &destination) {
        return Ok(destination);
    }
    remove_destination(&destination)?;
    fs::rename(ops::io_path(source), ops::io_path(&destination))?;
    Ok(ops::normal_path(&destination))
}

/// Map a planning failure onto a user-facing error.
pub fn error_for(reason: BlockedReason) -> AppError {
    match reason {
        BlockedReason::OutsideUserFiles => AppError::OutsideUserFiles,
        BlockedReason::ReparsePoint => AppError::ReparsePoint,
        BlockedReason::CloudOnly => AppError::CloudOnly,
        BlockedReason::Missing | BlockedReason::DestinationUnavailable => AppError::Unavailable,
        BlockedReason::InvalidName => AppError::InvalidRequest,
        BlockedReason::SameItem | BlockedReason::InsideItself => AppError::InvalidRequest,
    }
}

/// Convenience wrapper so callers do not have to build the context struct by hand.
pub fn finish_state(cancelled: bool, failed: u64) -> (OpsState, Option<String>) {
    if cancelled {
        (OpsState::Cancelled, Some("Stopped.".to_owned()))
    } else if failed > 0 {
        (OpsState::Failed, Some("Some items could not be finished.".to_owned()))
    } else {
        (OpsState::Completed, None)
    }
}

/// Kind used when planning a batch from the command layer.
pub fn transfer_kind(value: &str) -> TransferKind {
    if value == "move" { TransferKind::Move } else { TransferKind::Copy }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::conflict::{plan_transfer, ConflictAction, ConflictDecision};
    use std::collections::HashMap;

    struct MemoryFs {
        files: HashMap<String, ItemMeta>,
    }

    impl MemoryFs {
        fn with(paths: &[(&str, bool, u64)]) -> Self {
            let mut files = HashMap::new();
            for (path, is_directory, size) in paths {
                files.insert(
                    ops::path_key(&PathBuf::from(path)),
                    ItemMeta {
                        is_directory: *is_directory,
                        size: *size,
                        modified_unix: Some(1_700_000_000),
                        is_cloud: false,
                        is_reparse: false,
                        in_user_scope: true,
                    },
                );
            }
            Self { files }
        }
    }

    impl FsView for MemoryFs {
        fn meta(&self, path: &Path) -> Option<ItemMeta> {
            self.files.get(&ops::path_key(path)).copied()
        }
    }

    fn temp_root() -> PathBuf {
        let unique = format!(
            "sift-transfer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    fn write_file(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(path, contents).expect("write");
    }

    fn context_for<'a>(registry: &'a OpsRegistry, job_id: &'a str, cancel: CancelToken, roots: &'a [PathBuf]) -> TransferContext<'a> {
        TransferContext { registry, job_id, cancel, roots }
    }

    #[test]
    fn copies_files_and_folders_with_progress() {
        let root = temp_root();
        let source_dir = root.join("source");
        let target_dir = root.join("target");
        write_file(&source_dir.join("top.txt"), b"top level");
        write_file(&source_dir.join("nested").join("deep.txt"), b"deep content");
        write_file(&root.join("solo.txt"), b"just one file");
        fs::create_dir_all(&target_dir).expect("target dir");

        let roots = vec![root.clone()];
        let fs_view = MemoryFs::with(&[
            (&source_dir.to_string_lossy(), true, 0),
            (&root.join("solo.txt").to_string_lossy(), false, 13),
            (&target_dir.to_string_lossy(), true, 0),
        ]);
        let plan = plan_transfer(
            TransferKind::Copy,
            &[source_dir.clone(), root.join("solo.txt")],
            &target_dir,
            &[],
            ConflictAction::Ask,
            &fs_view,
        );
        assert!(plan.blocked.is_empty(), "nothing should be blocked: {:?}", plan.blocked);

        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(
            crate::ops::progress::OpsKind::Copy,
            plan.items.len() as u64,
            plan.bytes_total,
            ops::display_path(&target_dir),
        );
        let context = context_for(&registry, &job_id, cancel, &roots);
        let mut emissions = 0_usize;
        let result = execute(&plan, &context, &mut |_| emissions += 1);

        assert_eq!(result.completed, 2, "errors: {:?}", result.errors);
        assert!(!result.cancelled);
        assert!(target_dir.join("source").join("top.txt").exists());
        assert!(target_dir.join("source").join("nested").join("deep.txt").exists());
        assert_eq!(fs::read(target_dir.join("solo.txt")).expect("copied"), b"just one file");
        assert!(emissions > 0, "progress was emitted");
        let snapshot = registry.snapshot(&job_id).expect("job");
        assert_eq!(snapshot.items_done, 2);
    }

    #[test]
    fn keep_both_copy_leaves_the_original_in_place() {
        let root = temp_root();
        write_file(&root.join("notes.txt"), b"original");
        let roots = vec![root.clone()];
        let fs_view = MemoryFs::with(&[
            (&root.join("notes.txt").to_string_lossy(), false, 8),
            (&root.to_string_lossy(), true, 0),
        ]);
        let plan = plan_transfer(
            TransferKind::Copy,
            &[root.join("notes.txt")],
            &root,
            &[],
            ConflictAction::Ask,
            &fs_view,
        );
        assert_eq!(plan.items[0].name, "notes (1).txt");

        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(crate::ops::progress::OpsKind::Copy, 1, 8, ops::display_path(&root));
        let context = context_for(&registry, &job_id, cancel, &roots);
        let result = execute(&plan, &context, &mut |_| {});

        assert_eq!(result.completed, 1);
        assert_eq!(fs::read(root.join("notes.txt")).expect("original"), b"original");
        assert_eq!(fs::read(root.join("notes (1).txt")).expect("copy"), b"original");
    }

    #[test]
    fn replace_overwrites_and_skip_leaves_the_destination_alone() {
        let root = temp_root();
        write_file(&root.join("src").join("data.bin"), b"new bytes");
        write_file(&root.join("dst").join("data.bin"), b"old");
        write_file(&root.join("src").join("skip.bin"), b"leave me");
        write_file(&root.join("dst").join("skip.bin"), b"do not touch");
        let roots = vec![root.clone()];
        let fs_view = MemoryFs::with(&[
            (&root.join("src").join("data.bin").to_string_lossy(), false, 9),
            (&root.join("dst").join("data.bin").to_string_lossy(), false, 3),
            (&root.join("src").join("skip.bin").to_string_lossy(), false, 8),
            (&root.join("dst").join("skip.bin").to_string_lossy(), false, 12),
            (&root.join("src").to_string_lossy(), true, 0),
            (&root.join("dst").to_string_lossy(), true, 0),
        ]);
        let sources = vec![root.join("src").join("data.bin"), root.join("src").join("skip.bin")];
        let decisions = vec![
            ConflictDecision {
                source: ops::display_path(&root.join("src").join("data.bin")),
                action: ConflictAction::Replace,
            },
            ConflictDecision {
                source: ops::display_path(&root.join("src").join("skip.bin")),
                action: ConflictAction::Skip,
            },
        ];
        let plan = plan_transfer(TransferKind::Copy, &sources, &root.join("dst"), &decisions, ConflictAction::Ask, &fs_view);
        assert!(plan.conflicts.is_empty());

        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(crate::ops::progress::OpsKind::Copy, 2, plan.bytes_total, ops::display_path(&root.join("dst")));
        let context = context_for(&registry, &job_id, cancel, &roots);
        let result = execute(&plan, &context, &mut |_| {});

        assert_eq!(result.completed, 1, "errors: {:?}", result.errors);
        assert_eq!(result.skipped, 1);
        assert_eq!(fs::read(root.join("dst").join("data.bin")).expect("replaced"), b"new bytes");
        assert_eq!(fs::read(root.join("dst").join("skip.bin")).expect("skipped"), b"do not touch");
    }

    #[test]
    fn move_removes_the_source_after_copying() {
        let root = temp_root();
        write_file(&root.join("src").join("move me.txt"), b"payload");
        fs::create_dir_all(root.join("dst")).expect("dst");
        let roots = vec![root.clone()];
        let fs_view = MemoryFs::with(&[
            (&root.join("src").join("move me.txt").to_string_lossy(), false, 7),
            (&root.join("src").to_string_lossy(), true, 0),
            (&root.join("dst").to_string_lossy(), true, 0),
        ]);
        let plan = plan_transfer(
            TransferKind::Move,
            &[root.join("src").join("move me.txt")],
            &root.join("dst"),
            &[],
            ConflictAction::Ask,
            &fs_view,
        );
        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(crate::ops::progress::OpsKind::Move, 1, 7, ops::display_path(&root.join("dst")));
        let context = context_for(&registry, &job_id, cancel, &roots);
        let result = execute(&plan, &context, &mut |_| {});

        assert_eq!(result.completed, 1, "errors: {:?}", result.errors);
        assert_eq!(fs::read(root.join("dst").join("move me.txt")).expect("moved"), b"payload");
        assert!(!root.join("src").join("move me.txt").exists(), "source should be gone");
    }

    #[test]
    fn cancellation_stops_the_batch_and_reports_it() {
        let root = temp_root();
        write_file(&root.join("src").join("one.txt"), b"1");
        write_file(&root.join("src").join("two.txt"), b"2");
        fs::create_dir_all(root.join("dst")).expect("dst");
        let roots = vec![root.clone()];
        let fs_view = MemoryFs::with(&[
            (&root.join("src").join("one.txt").to_string_lossy(), false, 1),
            (&root.join("src").join("two.txt").to_string_lossy(), false, 1),
            (&root.join("src").to_string_lossy(), true, 0),
            (&root.join("dst").to_string_lossy(), true, 0),
        ]);
        let sources = vec![root.join("src").join("one.txt"), root.join("src").join("two.txt")];
        let plan = plan_transfer(TransferKind::Copy, &sources, &root.join("dst"), &[], ConflictAction::Ask, &fs_view);
        assert_eq!(plan.items.len(), 2);

        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(crate::ops::progress::OpsKind::Copy, 2, 2, ops::display_path(&root.join("dst")));
        assert!(registry.request_cancel(&job_id));
        let context = context_for(&registry, &job_id, cancel, &roots);
        let result = execute(&plan, &context, &mut |_| {});

        assert!(result.cancelled);
        assert_eq!(result.completed, 0);
        let (state, message) = finish_state(result.cancelled, result.failed);
        assert_eq!(state, OpsState::Cancelled);
        assert!(message.is_some());
    }

    #[test]
    fn rename_creates_the_new_name_and_refuses_collisions() {
        let root = temp_root();
        write_file(&root.join("draft.md"), b"content");
        write_file(&root.join("taken.md"), b"occupied");

        let renamed = rename(&root.join("draft.md"), "final.md").expect("rename succeeds");
        assert_eq!(ops::file_name(&renamed), "final.md");
        assert!(root.join("final.md").exists());
        assert!(!root.join("draft.md").exists());

        let collision = rename(&root.join("final.md"), "taken.md");
        assert!(matches!(collision, Err(AppError::Conflict)));

        let forced = rename_replace(&root.join("final.md"), "taken.md").expect("replace rename");
        assert_eq!(ops::file_name(&forced), "taken.md");
        assert_eq!(fs::read(root.join("taken.md")).expect("replaced"), b"content");
    }

    #[test]
    fn planning_errors_map_to_user_facing_app_errors() {
        assert!(matches!(error_for(BlockedReason::CloudOnly), AppError::CloudOnly));
        assert!(matches!(error_for(BlockedReason::OutsideUserFiles), AppError::OutsideUserFiles));
        assert!(matches!(error_for(BlockedReason::InsideItself), AppError::InvalidRequest));
        assert!(matches!(transfer_kind("move"), TransferKind::Move));
        assert!(matches!(transfer_kind("copy"), TransferKind::Copy));
        assert!(matches!(transfer_kind("anything"), TransferKind::Copy));
    }
}
