use std::{
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

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
    /// Path to the full saved output file when the model copy was truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Bytes the model sees verbatim from each end of one stream (8 KiB head + 8 KiB tail).
pub const MODEL_STREAM_EDGE: usize = 8 * 1024;

/// Maximum bytes to scan inward from the edge to land cleanly on a newline.
const NEWLINE_SNAP_WINDOW: usize = 512;

static SPILL_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Directory where truncated tool streams spill their complete raw output for later inspection.
pub fn output_log_dir() -> PathBuf {
    std::env::temp_dir().join("minicore-tool-output")
}

/// True when `path` resolves inside [`output_log_dir`].
pub fn is_in_output_log_dir(path: &Path) -> bool {
    let dir = crate::files::canonicalize_best_effort(&output_log_dir());
    let resolved = crate::files::canonicalize_best_effort(path);
    resolved.starts_with(&dir)
}

fn sanitize_call_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .take(64)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "call".to_owned()
    } else {
        cleaned
    }
}

fn spill_stream_output(
    call_id: &ToolCallId,
    channel: OutputChannel,
    bytes: &[u8],
) -> Option<String> {
    let dir = output_log_dir();
    fs::create_dir_all(&dir).ok()?;
    let channel_name = match channel {
        OutputChannel::Stdout => "stdout",
        OutputChannel::Stderr => "stderr",
    };
    let seq = SPILL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let filename = format!(
        "{}-{}-{}-{}.log",
        std::process::id(),
        seq,
        sanitize_call_id(call_id.as_str()),
        channel_name
    );
    let path = dir.join(filename);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).ok()?;
    file.write_all(bytes).ok()?;
    file.flush().ok()?;
    let canonical = fs::canonicalize(&path).unwrap_or(path);
    Some(canonical.to_string_lossy().into_owned())
}

impl CommandStream {
    /// The head and tail of a long stream. A command's complete output stays
    /// in its owner record; the model copy must not exhaust the context.
    pub fn for_model(&self) -> Self {
        self.truncate_for_model(None)
    }

    /// Same as [`Self::for_model`], additionally spilling the complete raw
    /// stream into [`output_log_dir`] when (and only when) bytes are omitted.
    pub fn for_model_spilled(&self, call_id: &ToolCallId, channel: OutputChannel) -> Self {
        self.truncate_for_model(Some((call_id, channel)))
    }

    fn truncate_for_model(&self, spill: Option<(&ToolCallId, OutputChannel)>) -> Self {
        let bytes = self.output.as_bytes();
        if bytes.len() <= 2 * MODEL_STREAM_EDGE {
            return self.clone();
        }
        let (head, tail) = match &self.output {
            OutputData::Utf8(text) => {
                let raw = text.as_bytes();
                let mut head = MODEL_STREAM_EDGE;
                while !text.is_char_boundary(head) {
                    head -= 1;
                }
                let head_scan_start = head.saturating_sub(NEWLINE_SNAP_WINDOW);
                if let Some(rel_nl) = raw[head_scan_start..head]
                    .iter()
                    .rposition(|&byte| byte == b'\n')
                {
                    let snapped = head_scan_start + rel_nl + 1;
                    if snapped > 0 {
                        head = snapped;
                    }
                }
                let mut tail = text.len() - MODEL_STREAM_EDGE;
                while !text.is_char_boundary(tail) {
                    tail += 1;
                }
                let tail_scan_end = (tail + NEWLINE_SNAP_WINDOW).min(text.len());
                if let Some(rel_nl) = raw[tail..tail_scan_end]
                    .iter()
                    .position(|&byte| byte == b'\n')
                {
                    let snapped = tail + rel_nl + 1;
                    if snapped < text.len() {
                        tail = snapped;
                    }
                }
                (head, tail)
            }
            OutputData::Bytes(bytes) => (MODEL_STREAM_EDGE, bytes.len() - MODEL_STREAM_EDGE),
        };
        let omitted = tail - head;
        let saved_path =
            spill.and_then(|(call_id, channel)| spill_stream_output(call_id, channel, bytes));
        let mut kept = bytes[..head].to_vec();
        if matches!(self.output, OutputData::Utf8(_)) {
            let notice = match &saved_path {
                Some(path) => format!(
                    "\n[… {omitted} bytes omitted; full output saved to {path} — use read(path, offset, limit) to inspect …]\n"
                ),
                None => format!(
                    "\n[… {omitted} bytes omitted; redirect to a file and read it in ranges …]\n"
                ),
            };
            kept.extend_from_slice(notice.as_bytes());
        }
        kept.extend_from_slice(&bytes[tail..]);
        Self {
            output: kept.into(),
            complete: self.complete,
            omitted_bytes: omitted as u64,
            full_output_path: saved_path,
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

    pub fn for_model_spilled(&self, call_id: &ToolCallId) -> Self {
        Self {
            stdout: self
                .stdout
                .for_model_spilled(call_id, OutputChannel::Stdout),
            stderr: self
                .stderr
                .for_model_spilled(call_id, OutputChannel::Stderr),
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
            full_output_path: None,
        };
        let copy = stream.for_model();
        let OutputData::Utf8(kept) = &copy.output else {
            panic!("UTF-8 output stays text");
        };
        assert!(kept.starts_with("开开") && kept.ends_with("开end"));
        assert!(kept.contains("bytes omitted"));
        assert!(copy.omitted_bytes > 0);
        assert!(copy.full_output_path.is_none());
        assert!(kept.len() < 2 * MODEL_STREAM_EDGE + 200);
        let short = CommandStream {
            output: OutputData::Utf8("ok".into()),
            complete: true,
            omitted_bytes: 0,
            full_output_path: None,
        };
        assert_eq!(short.for_model(), short);
        assert!(!serde_json::to_string(&short).unwrap().contains("omitted"));
        assert!(
            !serde_json::to_string(&short)
                .unwrap()
                .contains("full_output_path")
        );
    }

    #[test]
    fn spilled_stream_writes_full_log_only_when_truncated() {
        let call_id = ToolCallId::new("spill-test-call").unwrap();
        let short = CommandStream {
            output: OutputData::Utf8("small output\n".into()),
            complete: true,
            omitted_bytes: 0,
            full_output_path: None,
        };
        let short_copy = short.for_model_spilled(&call_id, OutputChannel::Stdout);
        assert_eq!(short_copy, short);
        assert!(short_copy.full_output_path.is_none());

        let mut lines = String::new();
        for idx in 0..1000 {
            lines.push_str(&format!("line-{idx:04}: payload-for-truncation-check\n"));
        }
        assert!(lines.len() > 2 * MODEL_STREAM_EDGE);
        let long = CommandStream {
            output: OutputData::Utf8(lines.clone()),
            complete: true,
            omitted_bytes: 0,
            full_output_path: None,
        };
        let long_copy = long.for_model_spilled(&call_id, OutputChannel::Stdout);
        let saved = long_copy
            .full_output_path
            .as_deref()
            .expect("truncated stream must record full_output_path");
        assert!(is_in_output_log_dir(Path::new(saved)));
        assert_eq!(fs::read_to_string(saved).unwrap(), lines);
        let OutputData::Utf8(kept) = &long_copy.output else {
            panic!("UTF-8 output stays text");
        };
        assert!(kept.contains(saved));
        assert!(kept.contains("use read(path, offset, limit)"));
        assert!(long_copy.omitted_bytes > 0);
        let _ = fs::remove_file(saved);
    }
}
