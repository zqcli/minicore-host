use std::io;

/// Local operation errors. The owning Session maps these into its public Fault
/// registry; this crate does not invent a second public error-code registry.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("no workspace is configured")]
    NotConfigured,
    #[error("the configured workspace root must be an absolute directory")]
    InvalidRoot,
    #[error("a workspace path must be relative and must not contain '..' or NUL")]
    InvalidPath,
    #[error("the path leaves the configured workspace")]
    OutsideWorkspace,
    #[error("the requested path does not exist")]
    NotFound,
    #[error("the requested path is not a regular file")]
    NotFile,
    #[error("the requested path is not a directory")]
    NotDirectory,
    #[error("the requested range is invalid or cannot advance at a UTF-8 boundary")]
    InvalidRange,
    #[error("the continuation does not belong to this query or is no longer valid")]
    InvalidCursor,
    #[error("the requested content is binary or is not UTF-8")]
    Binary,
    #[error("the file changed while it was being read")]
    Changed,
    #[error("the path cannot be represented as UTF-8")]
    NonUtf8Path,
    #[error("the filesystem operation failed")]
    Io(#[source] io::Error),
    #[error("Git is not installed or could not be started")]
    GitUnavailable(#[source] io::Error),
    #[error("the configured workspace is not in a Git work tree")]
    NotRepository,
    #[error("Git failed during {operation} (exit code {code:?})")]
    GitFailed {
        operation: &'static str,
        code: Option<i32>,
    },
    #[error("Git returned an unrecognized {0} record")]
    GitOutput(&'static str),
    #[error("Git status for this scope can invoke a configured external content filter")]
    UnsupportedFilter,
    #[error("the requested change no longer exists")]
    ChangeNotFound,
    #[error("the change is unmerged or uses an unsupported Git object type")]
    UnsupportedChange,
    #[error("the tool edit does not retain both observed snapshots")]
    SnapshotUnavailable,
    #[error("the query was cancelled")]
    Cancelled,
    #[error("the caller's query deadline elapsed")]
    Deadline,
    #[error("the query worker failed")]
    WorkerFailed,
}

impl From<io::Error> for WorkspaceError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::NotADirectory => Self::NotDirectory,
            _ => Self::Io(error),
        }
    }
}

pub type Result<T> = std::result::Result<T, WorkspaceError>;
