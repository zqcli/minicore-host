use std::fmt;

use minicore_host_runtime::ToolCallId;
use minicore_host_workspace::{EditOutcome, FileSnapshot};
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
    /// Bytes left out of the middle of `output` in the model's copy. Owner
    /// records always carry the complete output and leave this at zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_bytes: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Bytes the model sees verbatim from each end of one stream.
const MODEL_STREAM_EDGE: usize = 16 * 1024;

impl CommandStream {
    /// The head and tail of a long stream. A command's complete output stays
    /// in its owner record; the model copy must not exhaust the context.
    pub fn for_model(&self) -> Self {
        let bytes = self.output.as_bytes();
        if bytes.len() <= 2 * MODEL_STREAM_EDGE {
            return self.clone();
        }
        let (head, tail) = match &self.output {
            OutputData::Utf8(text) => {
                let mut head = MODEL_STREAM_EDGE;
                while !text.is_char_boundary(head) {
                    head -= 1;
                }
                let mut tail = text.len() - MODEL_STREAM_EDGE;
                while !text.is_char_boundary(tail) {
                    tail += 1;
                }
                (head, tail)
            }
            OutputData::Bytes(bytes) => (MODEL_STREAM_EDGE, bytes.len() - MODEL_STREAM_EDGE),
        };
        let omitted = tail - head;
        let mut kept = bytes[..head].to_vec();
        if matches!(self.output, OutputData::Utf8(_)) {
            kept.extend_from_slice(
                format!(
                    "\n[… {omitted} bytes omitted; redirect to a file and read it in ranges …]\n"
                )
                .as_bytes(),
            );
        }
        kept.extend_from_slice(&bytes[tail..]);
        Self {
            output: kept.into(),
            complete: self.complete,
            omitted_bytes: omitted as u64,
        }
    }
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
    /// The real wrapper wait finished, the original process group was observed
    /// gone, and no owned pipe reader remains unfinished. A successful signal
    /// request or pipe EOF alone cannot confirm cleanup. Escaped groups are
    /// outside the controlled scope.
    pub cleanup_confirmed: bool,
    pub stdout: CommandStream,
    pub stderr: CommandStream,
    pub file_changes: CommandFileChanges,
    pub cleanup_errors: Vec<String>,
}

impl CommandResult {
    pub fn for_model(&self) -> Self {
        Self {
            stdout: self.stdout.for_model(),
            stderr: self.stderr.for_model(),
            ..self.clone()
        }
    }
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
    /// Actual work has returned after its file operation or process/group/pipe
    /// cleanup. This is independent of Runtime's result/future lifetime. False
    /// must never be treated as confirmed completion. Standalone sinks may
    /// ignore this fact; a Session adapter uses it to close its original lease.
    fn complete(&self, _call: &ToolCallId, _cleanup_confirmed: bool) -> Result<(), RecordError> {
        Ok(())
    }
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

#[cfg(test)]
mod model_copy_tests {
    use super::*;

    #[test]
    fn long_streams_keep_head_and_tail_on_character_boundaries() {
        let text = format!("{}{}", "开".repeat(20_000), "end");
        let stream = CommandStream {
            output: OutputData::Utf8(text.clone()),
            complete: true,
            omitted_bytes: 0,
        };
        let copy = stream.for_model();
        let OutputData::Utf8(kept) = &copy.output else {
            panic!("UTF-8 output stays text");
        };
        assert!(kept.starts_with("开开") && kept.ends_with("开end"));
        assert!(kept.contains("bytes omitted"));
        assert!(copy.omitted_bytes > 0);
        assert!(kept.len() < 2 * MODEL_STREAM_EDGE + 200);
        let short = CommandStream {
            output: OutputData::Utf8("ok".into()),
            complete: true,
            omitted_bytes: 0,
        };
        assert_eq!(short.for_model(), short);
        assert!(!serde_json::to_string(&short).unwrap().contains("omitted"));
    }
}
