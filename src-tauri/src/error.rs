use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("The requested path is outside the current user's files.")]
    OutsideUserFiles,
    #[error("The location is a Windows reparse point and cannot be followed.")]
    ReparsePoint,
    #[error("The location is online-only and cannot be read locally.")]
    CloudOnly,
    #[error("The location is not available.")]
    Unavailable,
    #[error("The request is invalid.")]
    InvalidRequest,
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("The indexer worker stopped unexpectedly.")]
    WorkerStopped,
}

impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.to_string()
    }
}
