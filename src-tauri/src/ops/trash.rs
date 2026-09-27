//! Deletion always goes through the platform Recycle Bin, with progress and cancel.

use crate::error::AppError;
use crate::ops::progress::{CancelToken, OperationProgress, OpsRegistry, OpsState};
use crate::ops;
use serde::Serialize;
use specta::Type;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    pub moved: u64,
    pub skipped: u64,
    pub cancelled: bool,
    pub errors: Vec<String>,
}

pub struct DeleteContext<'a> {
    pub registry: &'a OpsRegistry,
    pub job_id: &'a str,
    pub cancel: CancelToken,
    pub roots: &'a [PathBuf],
}

/// True when the path *is* one of the user roots; deleting those would be catastrophic.
pub fn is_user_root(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| ops::same_path(root, path))
}

/// Move each path to the Recycle Bin. Inaccessible or protected items are skipped and
/// counted instead of aborting the batch.
pub fn delete_paths(paths: &[PathBuf], context: &DeleteContext<'_>, emit: &mut dyn FnMut(OperationProgress)) -> DeleteResult {
    let mut result = DeleteResult::default();
    for path in paths {
        if context.cancel.requested() {
            result.cancelled = true;
            break;
        }
        let name = ops::file_name(path);
        if let Some(progress) = context.registry.set_current(context.job_id, &name) {
            emit(progress);
        }
        // Same guard the browser path uses: inside a user root, not a root itself, and
        // no reparse point anywhere along the way.
        if is_user_root(path, context.roots) || ops::validate_path(path, context.roots).is_err() {
            result.skipped += 1;
            if let Some(progress) = context.registry.skip_item(context.job_id) {
                emit(progress);
            }
            continue;
        }
        // `trash` hands the item to the Windows Recycle Bin; nothing is shredded.
        match trash::delete(ops::io_path(path)) {
            Ok(()) => {
                result.moved += 1;
                if let Some(progress) = context.registry.finish_item(context.job_id, 0) {
                    emit(progress);
                }
            }
            Err(error) => {
                result.skipped += 1;
                result.errors.push(format!("{name}: {error}"));
                if let Some(progress) = context.registry.skip_item(context.job_id) {
                    emit(progress);
                }
            }
        }
    }
    result
}

pub fn finish_state(cancelled: bool, skipped: u64) -> (OpsState, Option<String>) {
    if cancelled {
        (OpsState::Cancelled, Some("Stopped.".to_owned()))
    } else if skipped > 0 {
        (OpsState::Failed, Some("Some items could not be moved to the Recycle Bin.".to_owned()))
    } else {
        (OpsState::Completed, None)
    }
}

/// Map a planning/validation failure onto the error type used by the command layer.
pub fn error_for(reason: &str) -> AppError {
    match reason {
        "cloud" => AppError::CloudOnly,
        "reparse" => AppError::ReparsePoint,
        "outside" => AppError::OutsideUserFiles,
        _ => AppError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const DELETE_KIND: crate::ops::progress::OpsKind = crate::ops::progress::OpsKind::Delete;

    fn temp_dir() -> PathBuf {
        let unique = format!(
            "sift-trash-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn user_roots_are_never_deletable() {
        let roots = vec![PathBuf::from("/home/u"), PathBuf::from("/home/u/Documents")];
        assert!(is_user_root(&PathBuf::from("/home/u"), &roots));
        assert!(is_user_root(&PathBuf::from("/home/u/Documents"), &roots));
        assert!(!is_user_root(&PathBuf::from("/home/u/Documents/report.pdf"), &roots));
    }

    #[test]
    fn cancelling_before_the_batch_starts_deletes_nothing() {
        let root = temp_dir();
        let victim = root.join("victim.txt");
        fs::write(&victim, b"still here").expect("write");

        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(DELETE_KIND, 1, 0, ops::display_path(&root));
        assert!(registry.request_cancel(&job_id));
        let context = DeleteContext { registry: &registry, job_id: &job_id, cancel, roots: &[root.clone()] };
        let result = delete_paths(&[victim.clone()], &context, &mut |_| {});

        assert!(result.cancelled);
        assert_eq!(result.moved, 0);
        assert!(victim.exists(), "a cancelled delete must not touch the file");
    }

    #[test]
    fn paths_outside_the_user_roots_are_skipped_and_counted() {
        let root = temp_dir();
        let victim = root.join("victim.txt");
        fs::write(&victim, b"data").expect("write");
        let outside = PathBuf::from(if cfg!(windows) { "C:\\Windows\\system32\\drivers\\etc\\hosts" } else { "/etc/hosts" });

        let registry = OpsRegistry::default();
        let (job_id, cancel, _) = registry.start(DELETE_KIND, 2, 0, String::new());
        let context = DeleteContext { registry: &registry, job_id: &job_id, cancel, roots: &[root.clone()] };
        let result = delete_paths(&[outside.clone(), root.clone()], &context, &mut |_| {});

        assert_eq!(result.skipped, 2, "outside path and the root itself");
        assert_eq!(result.moved, 0);
        assert!(outside.exists());
        assert!(root.exists());
        assert!(result.errors.is_empty(), "skips are not errors");
        let snapshot = registry.snapshot(&job_id).expect("job");
        assert_eq!(snapshot.items_skipped, 2);
    }

    #[test]
    fn finish_state_reflects_the_outcome() {
        assert!(matches!(finish_state(true, 0), (OpsState::Cancelled, Some(_))));
        assert!(matches!(finish_state(false, 2), (OpsState::Failed, Some(_))));
        assert!(matches!(finish_state(false, 0), (OpsState::Completed, None)));
    }

    #[test]
    fn error_mapping_covers_every_reason() {
        assert!(matches!(error_for("cloud"), AppError::CloudOnly));
        assert!(matches!(error_for("reparse"), AppError::ReparsePoint));
        assert!(matches!(error_for("outside"), AppError::OutsideUserFiles));
        assert!(matches!(error_for("unknown"), AppError::Unavailable));
    }

}
