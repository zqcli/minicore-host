use std::{io, path::PathBuf};
use thiserror::Error;

/// A location in the original file, never a public history pagination cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilePosition {
    pub line: u64,
    pub byte_offset: u64,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("data root is already in use")]
    RootInUse,
    #[error("history already has an active writer")]
    HistoryInUse,
    #[error("unsafe storage path: {reason}")]
    UnsafePath { reason: &'static str },
    #[error("storage I/O failed during {operation}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("unsupported storage schema version {found}; expected {supported}")]
    UnsupportedSchema {
        found: u64,
        supported: u32,
        position: Option<FilePosition>,
    },
    #[error("object format is invalid: {reason}")]
    InvalidObject { reason: String },
    #[error("stored object identity or direct owner conflicts with its directory")]
    OwnershipMismatch,
    #[error("persistent data cannot belong to a memory object")]
    InvalidStorageMode,
    #[error("history validation failed at {position:?}: {problem}")]
    InvalidHistory {
        position: FilePosition,
        problem: HistoryProblem,
    },
    #[error("accepted history has an unresolved failed save; reconcile that exact range first")]
    ReconciliationRequired,
    #[error("file differs from the original save range; no automatic repair is permitted")]
    UncertainWrite,
    #[error("a save batch is already pending")]
    PendingBatch,
    #[error("history position overflow")]
    PositionOverflow,
    #[error("explicit deletion would include unexpected files or child objects")]
    DirectoryNotEmpty,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryProblem {
    #[error("invalid or incomplete JSON: {0}")]
    Json(String),
    #[error("blank JSONL record")]
    BlankRecord,
    #[error("expected position {expected}, found duplicate or old position {found}")]
    DuplicatePosition { expected: u64, found: u64 },
    #[error("expected position {expected}, found {found}")]
    UnexpectedPosition { expected: u64, found: u64 },
    #[error("record belongs to another Session")]
    WrongSession,
    #[error("invalid record: {0}")]
    InvalidRecord(String),
}

pub(crate) fn io_error(
    operation: &'static str,
    path: impl Into<PathBuf>,
    source: impl Into<io::Error>,
) -> StorageError {
    StorageError::Io {
        operation,
        path: path.into(),
        source: source.into(),
    }
}
