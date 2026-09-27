//! Tauri commands for file operations: plan, copy, move, rename, delete, shell verbs.

use crate::error::AppError;
use crate::indexer::IndexState;
use crate::ops::conflict::{
    self, BlockedReason, ConflictAction, ConflictDecision, TransferKind, TransferPlan,
};
use crate::ops::progress::{OperationProgress, OpsKind, OpsRegistry, PROGRESS_EVENT};
use crate::ops::transfer::{self, DiskView, TransferContext, TransferResult};
use crate::ops::{shell, trash};
use serde::Serialize;
use specta::Type;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// Managed state: the live job table behind the progress card and Cancel button.
#[derive(Default)]
pub struct FileOpsState {
    registry: std::sync::Arc<OpsRegistry>,
}

impl FileOpsState {
    pub fn registry(&self) -> &OpsRegistry {
        &self.registry
    }

    /// Owned handle for work that runs on a blocking task.
    pub fn registry_arc(&self) -> std::sync::Arc<OpsRegistry> {
        self.registry.clone()
    }
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameOutcome {
    pub previous_path: String,
    pub new_path: String,
    pub replaced: bool,
}

/// Throttles progress events so a large copy does not flood the webview.
pub(crate) struct ProgressEmitter<'a> {
    app: &'a AppHandle,
    last: Option<Instant>,
    minimum: Duration,
}

impl<'a> ProgressEmitter<'a> {
    pub(crate) fn new(app: &'a AppHandle) -> Self {
        Self { app, last: None, minimum: Duration::from_millis(90) }
    }

    pub(crate) fn push(&mut self, progress: &OperationProgress) {
        let now = Instant::now();
        let due = progress.state.is_finished()
            || matches!(self.last, Some(last) if now.duration_since(last) >= self.minimum)
            || self.last.is_none();
        if due {
            self.last = Some(now);
            let _ = self.app.emit(PROGRESS_EVENT, progress.clone());
        }
    }
}

fn require_destination(path: &str, roots: &[PathBuf]) -> Result<PathBuf, AppError> {
    let destination = crate::ops::normal_path(&PathBuf::from(path));
    if destination.to_string_lossy().trim().is_empty() {
        return Err(AppError::InvalidRequest);
    }
    if crate::ops::root_for(&destination, roots).is_none() {
        return Err(AppError::OutsideUserFiles);
    }
    Ok(destination)
}

fn require_source(path: &str, roots: &[PathBuf]) -> Result<PathBuf, AppError> {
    let source = crate::ops::normal_path(&PathBuf::from(path));
    if crate::ops::root_for(&source, roots).is_none() {
        return Err(AppError::OutsideUserFiles);
    }
    if conflict::validate_name(&crate::ops::file_name(&source)).is_err() {
        return Err(AppError::InvalidName);
    }
    Ok(source)
}

/// Work out what a copy/move would do, including every name collision, without
/// touching the filesystem. The UI shows a conflict dialog from this result.
#[tauri::command]
#[specta::specta]
pub fn plan_transfer(
    paths: Vec<String>,
    destination: String,
    kind: String,
    state: State<'_, IndexState>,
) -> Result<TransferPlan, String> {
    let roots = state.roots().to_vec();
    let destination = require_destination(&destination, &roots).map_err(|error| error.to_string())?;
    let mut sources = Vec::with_capacity(paths.len());
    for path in &paths {
        sources.push(require_source(path, &roots).map_err(|error| error.to_string())?);
    }
    if sources.is_empty() {
        return Err(AppError::InvalidRequest.to_string());
    }
    let view = DiskView::new(&roots);
    Ok(conflict::plan_transfer(
        transfer::transfer_kind(&kind),
        &sources,
        &destination,
        &[],
        ConflictAction::Ask,
        &view,
    ))
}

fn run_transfer(
    app: &AppHandle,
    state: &FileOpsState,
    kind: TransferKind,
    sources: Vec<PathBuf>,
    destination: PathBuf,
    decisions: Vec<ConflictDecision>,
    default_action: ConflictAction,
    roots: &[PathBuf],
) -> Result<TransferResult, String> {
    let view = DiskView::new(roots);
    let planned = conflict::plan_transfer(kind, &sources, &destination, &decisions, default_action, &view);
    if let Some(unresolved) = planned.conflicts.first() {
        return Err(format!("{} already exists at the destination.", unresolved.name));
    }
    let resolved = conflict::resolve_conflicts(&planned, &decisions, default_action);
    if resolved.items.is_empty() {
        return Ok(TransferResult {
            skipped: planned.blocked.len() as u64,
            ..TransferResult::default()
        });
    }

    let ops_kind = match kind {
        TransferKind::Copy => OpsKind::Copy,
        TransferKind::Move => OpsKind::Move,
    };
    let registry = state.registry();
    let (job_id, cancel, initial) = registry.start(
        ops_kind,
        resolved.items.len() as u64,
        resolved.bytes_total,
        crate::ops::display_path(&destination),
    );
    let mut emitter = ProgressEmitter::new(app);
    emitter.push(&initial);

    let context = TransferContext { registry, job_id: &job_id, cancel, roots };
    let result = transfer::execute(&resolved, &context, &mut |progress| emitter.push(&progress));
    let blocked = resolved.blocked.iter().map(|item| item.reason).collect::<Vec<BlockedReason>>();
    let mut errors = result.errors.clone();
    for reason in blocked {
        errors.push(reason.message().to_owned());
    }
    let (final_state, message) = transfer::finish_state(result.cancelled, result.failed + blocked.len() as u64);
    if let Some(progress) = registry.finish(&job_id, final_state, message) {
        emitter.push(&progress);
    }
    registry.prune_finished(4);
    Ok(TransferResult { skipped: result.skipped + blocked.len() as u64, errors, ..result })
}

#[tauri::command]
#[specta::specta]
pub fn copy_paths(
    paths: Vec<String>,
    destination: String,
    decisions: Vec<ConflictDecision>,
    default_action: ConflictAction,
    app: AppHandle,
    state: State<'_, IndexState>,
    ops: State<'_, FileOpsState>,
) -> Result<TransferResult, String> {
    let roots = state.roots().to_vec();
    let destination = require_destination(&destination, &roots).map_err(|error| error.to_string())?;
    let sources = paths
        .iter()
        .map(|path| require_source(path, &roots))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    run_transfer(
        &app,
        ops.inner(),
        TransferKind::Copy,
        sources,
        destination,
        decisions,
        default_action,
        &roots,
    )
}

#[tauri::command]
#[specta::specta]
pub fn move_paths(
    paths: Vec<String>,
    destination: String,
    decisions: Vec<ConflictDecision>,
    default_action: ConflictAction,
    app: AppHandle,
    state: State<'_, IndexState>,
    ops: State<'_, FileOpsState>,
) -> Result<TransferResult, String> {
    let roots = state.roots().to_vec();
    let destination = require_destination(&destination, &roots).map_err(|error| error.to_string())?;
    let sources = paths
        .iter()
        .map(|path| require_source(path, &roots))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    run_transfer(
        &app,
        ops.inner(),
        TransferKind::Move,
        sources,
        destination,
        decisions,
        default_action,
        &roots,
    )
}

#[tauri::command]
#[specta::specta]
pub fn rename_path(path: String, new_name: String, replace: bool, state: State<'_, IndexState>) -> Result<RenameOutcome, String> {
    let roots = state.roots().to_vec();
    let source = require_source(&path, &roots).map_err(|error| error.to_string())?;
    conflict::validate_name(&new_name).map_err(|_| AppError::InvalidName.to_string())?;
    let parent = source.parent().ok_or_else(|| AppError::InvalidRequest.to_string())?;
    let destination = parent.join(&new_name);
    if crate::ops::root_for(&destination, &roots).is_none() {
        return Err(AppError::OutsideUserFiles.to_string());
    }
    let replaced = std::fs::symlink_metadata(crate::ops::io_path(&destination)).is_ok();
    if replaced && !replace {
        return Err(AppError::Conflict.to_string());
    }
    let renamed = if replace {
        transfer::rename_replace(&source, &new_name)
    } else {
        transfer::rename(&source, &new_name)
    };
    let new_path = renamed.map_err(|error| error.to_string())?;
    Ok(RenameOutcome {
        previous_path: crate::ops::display_path(&source),
        new_path: crate::ops::display_path(&new_path),
        replaced,
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_paths(
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, IndexState>,
    ops: State<'_, FileOpsState>,
) -> Result<trash::DeleteResult, String> {
    let roots = state.roots().to_vec();
    let targets = paths
        .iter()
        .map(|path| crate::ops::normal_path(&PathBuf::from(path)))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err(AppError::InvalidRequest.to_string());
    }
    let registry = ops.registry();
    let (job_id, cancel, initial) = registry.start(OpsKind::Delete, targets.len() as u64, 0, String::new());
    let mut emitter = ProgressEmitter::new(&app);
    emitter.push(&initial);
    let context = trash::DeleteContext { registry, job_id: &job_id, cancel, roots: &roots };
    let result = trash::delete_paths(&targets, &context, &mut |progress| emitter.push(&progress));
    let (final_state, message) = trash::finish_state(result.cancelled, result.skipped);
    if let Some(progress) = registry.finish(&job_id, final_state, message) {
        emitter.push(&progress);
    }
    registry.prune_finished(4);
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub fn cancel_operation(job_id: String, ops: State<'_, FileOpsState>) -> Result<bool, String> {
    Ok(ops.registry().request_cancel(&job_id))
}

#[tauri::command]
#[specta::specta]
pub fn list_operations(ops: State<'_, FileOpsState>) -> Result<Vec<OperationProgress>, String> {
    Ok(ops.registry().active())
}

#[tauri::command]
#[specta::specta]
pub fn reveal_in_explorer(path: String, state: State<'_, IndexState>) -> Result<(), String> {
    let roots = state.roots().to_vec();
    let target = require_source(&path, &roots).map_err(|error| error.to_string())?;
    shell::reveal(&target).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn open_with(path: String, state: State<'_, IndexState>) -> Result<(), String> {
    let roots = state.roots().to_vec();
    let target = require_source(&path, &roots).map_err(|error| error.to_string())?;
    shell::open_with(&target).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn show_properties(path: String, state: State<'_, IndexState>) -> Result<(), String> {
    let roots = state.roots().to_vec();
    let target = require_source(&path, &roots).map_err(|error| error.to_string())?;
    shell::properties(&target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_outside_the_user_roots_are_refused() {
        let roots = vec![PathBuf::from("/home/u")];
        assert!(require_destination("/home/u/Documents", &roots).is_ok());
        assert!(require_destination("/etc", &roots).is_err());
        assert!(require_destination("   ", &roots).is_err());
    }

    #[test]
    fn sources_must_be_inside_the_roots_and_well_named() {
        let roots = vec![PathBuf::from("/home/u")];
        assert!(require_source("/home/u/Documents/a.txt", &roots).is_ok());
        assert!(require_source("/home/u/Documents/con.txt", &roots).is_err());
        assert!(require_source("/tmp/outside.txt", &roots).is_err());
    }

    #[test]
    fn transfer_kind_parsing_defaults_to_copy() {
        assert!(matches!(transfer::transfer_kind("move"), TransferKind::Move));
        assert!(matches!(transfer::transfer_kind("copy"), TransferKind::Copy));
    }
}
