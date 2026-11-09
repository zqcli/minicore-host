use std::fmt;

use minicore_host_workspace::{EditOutcome, FileSnapshot};
use minicore_runtime::ToolCallId;
use serde::{Deserialize, Serialize};

/// Owner-bound recording failed. This is never a shell exit code or a file error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordError;
impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the bound local-tool record owner failed")
    }
}
impl std::error::Error for RecordError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputChannel {
    Stdout,
    Stderr,
}

/// Snapshots of this tool's actual target, never inferred from Git status.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapturedEdit {
    pub path: String,
    pub before: FileSnapshot,
    pub after: FileSnapshot,
    pub outcome: EditOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Exited,
    Cancelled,
    TimedOut,
    Failed,
}

/// Byte-exact output. Non-UTF-8 bytes are not silently replaced or omitted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub enum OutputData {
    Utf8(String),
    Bytes(Vec<u8>),
}
impl OutputData {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Utf8(text) => text.as_bytes(),
            Self::Bytes(bytes) => bytes,
        }
    }
}
impl From<Vec<u8>> for OutputData {
    fn from(bytes: Vec<u8>) -> Self {
        match String::from_utf8(bytes) {
            Ok(text) => Self::Utf8(text),
            Err(error) => Self::Bytes(error.into_bytes()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandStream {
    pub output: OutputData,
    /// True only after a real EOF, not after a read failure or stopped reader.
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandFileChanges {
    /// Arbitrary shell scripts have no known file target. No Git-dirty heuristic
    /// claims that unrelated edits belong to this call.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandResult {
    pub status: CommandStatus,
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    /// True only when the actual direct child wait returned its exit status.
    pub process_waited: bool,
    /// The direct child was waited, group stop did not fail, and no owned pipe
    /// reader remains unfinished. Escaped process groups are not a sandboxed scope.
    pub cleanup_confirmed: bool,
    pub stdout: CommandStream,
    pub stderr: CommandStream,
    pub file_changes: CommandFileChanges,
    pub cleanup_errors: Vec<String>,
}

/// A capability already bound by the real Session owner to one Turn/execution.
/// Only Runtime-supplied call IDs enter these methods. Implementations must be
/// short, nonblocking, nonpanicking record updates, not subscription delivery.
/// A disconnected observer must not cause a recording error.
pub trait LocalToolSink: Send + Sync + 'static {
    fn output(
        &self,
        call: &ToolCallId,
        channel: OutputChannel,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), RecordError>;
    fn edit(&self, call: &ToolCallId, edit: CapturedEdit) -> Result<(), RecordError>;
    fn command(&self, call: &ToolCallId, result: &CommandResult) -> Result<(), RecordError>;
}

/// Explicit standalone execution without retained owner records. This is not a
/// production Session adapter and does not claim persistent observation.
#[derive(Default)]
pub struct NoopLocalToolSink;
impl LocalToolSink for NoopLocalToolSink {
    fn output(
        &self,
        _: &ToolCallId,
        _: OutputChannel,
        _: u64,
        _: &[u8],
    ) -> Result<(), RecordError> {
        Ok(())
    }
    fn edit(&self, _: &ToolCallId, _: CapturedEdit) -> Result<(), RecordError> {
        Ok(())
    }
    fn command(&self, _: &ToolCallId, _: &CommandResult) -> Result<(), RecordError> {
        Ok(())
    }
}
