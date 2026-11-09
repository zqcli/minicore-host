//! Production recording is a thin handoff to the original Session owner.
//! This adapter owns only invocation leases and incomplete UTF-8 code points.
use crate::LocalToolInvocationSink;
use minicore_host_core::ToolOutputLease;
use minicore_host_local_tools::{
    CapturedEdit, CommandResult, LocalToolSink, OutputChannel, RecordError,
};
use minicore_runtime::{ToolCallId, tools::ToolInvocation};
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
pub(crate) struct SessionLocalToolSink(Mutex<BTreeMap<String, Invocation>>);
struct Invocation {
    output: ToolOutputLease,
    stdout: TextStream,
    stderr: TextStream,
}

/// `received` is the actual raw byte offset, including an incomplete code point
/// or binary suffix. Public positions count only the unchanged UTF-8 prefix.
#[derive(Default)]
struct TextStream {
    received: u64,
    pending: Vec<u8>,
    binary: bool,
}
impl TextStream {
    fn accept(&mut self, offset: u64, bytes: &[u8]) -> Result<(String, bool), RecordError> {
        if offset != self.received {
            return Err(RecordError);
        }
        self.received += bytes.len() as u64;
        if self.binary {
            return Ok((String::new(), false));
        }
        self.pending.extend_from_slice(bytes);
        let (length, binary) = match std::str::from_utf8(&self.pending) {
            Ok(text) => (text.len(), false),
            Err(error) => (error.valid_up_to(), error.error_len().is_some()),
        };
        let text = std::str::from_utf8(&self.pending[..length])
            .expect("validated prefix")
            .to_owned();
        self.pending.drain(..length);
        if binary {
            self.binary = true;
            self.pending.clear();
        }
        Ok((text, binary))
    }
    fn finish(&mut self, bytes: &[u8]) -> Result<bool, RecordError> {
        if self.received != bytes.len() as u64 {
            return Err(RecordError);
        }
        let incomplete = !self.pending.is_empty();
        self.pending.clear();
        self.binary |= incomplete;
        Ok(incomplete)
    }
}

impl LocalToolInvocationSink for SessionLocalToolSink {
    fn begin(
        &self,
        invocation: &ToolInvocation,
        output: ToolOutputLease,
    ) -> Result<(), RecordError> {
        let mut calls = self.0.lock().map_err(|_| RecordError)?;
        let id = invocation.tool_call_id.as_str();
        if output.source().tool_call_id.0 != id || calls.contains_key(id) {
            return Err(RecordError);
        }
        calls.insert(
            id.into(),
            Invocation {
                output,
                stdout: TextStream::default(),
                stderr: TextStream::default(),
            },
        );
        Ok(())
    }
}
impl LocalToolSink for SessionLocalToolSink {
    fn complete(&self, call: &ToolCallId, cleanup_confirmed: bool) -> Result<(), RecordError> {
        let output = self
            .0
            .lock()
            .map_err(|_| RecordError)?
            .get(call.as_str())
            .ok_or(RecordError)?
            .output
            .clone();
        if cleanup_confirmed {
            output.confirm_cleanup().map_err(|_| RecordError)?;
        }
        Ok(())
    }
    fn output(
        &self,
        call: &ToolCallId,
        channel: OutputChannel,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), RecordError> {
        let (output, channel, text, became_binary) = {
            let mut calls = self.0.lock().map_err(|_| RecordError)?;
            let invocation = calls.get_mut(call.as_str()).ok_or(RecordError)?;
            let (channel, stream) = match channel {
                OutputChannel::Stdout => ("stdout", &mut invocation.stdout),
                OutputChannel::Stderr => ("stderr", &mut invocation.stderr),
            };
            let (text, became_binary) = stream.accept(offset, bytes)?;
            (invocation.output.clone(), channel, text, became_binary)
        };
        // The actual command owner calls these callbacks in stream order.
        // Do not hold adapter state while entering the Session owner.
        if !text.is_empty() && !output.emit(channel, &text) {
            return Err(RecordError);
        }
        if became_binary && !output.mark_non_utf8(channel) {
            return Err(RecordError);
        }
        Ok(())
    }
    fn edit(&self, call: &ToolCallId, edit: CapturedEdit) -> Result<(), RecordError> {
        let output = self
            .0
            .lock()
            .map_err(|_| RecordError)?
            .get(call.as_str())
            .ok_or(RecordError)?
            .output
            .clone();
        output
            .record_edit(edit.path, edit.before, edit.after, edit.outcome)
            .map_err(|_| RecordError)
    }
    fn command(&self, call: &ToolCallId, result: &CommandResult) -> Result<(), RecordError> {
        let (output, channels) = {
            let mut calls = self.0.lock().map_err(|_| RecordError)?;
            let invocation = calls.get_mut(call.as_str()).ok_or(RecordError)?;
            let mut channels = Vec::new();
            for (channel, stream, bytes) in [
                (
                    "stdout",
                    &mut invocation.stdout,
                    result.stdout.output.as_bytes(),
                ),
                (
                    "stderr",
                    &mut invocation.stderr,
                    result.stderr.output.as_bytes(),
                ),
            ] {
                channels.push((channel, stream.finish(bytes)?, stream.received == 0));
            }
            (invocation.output.clone(), channels)
        };
        for (channel, incomplete, empty) in channels {
            if incomplete && !output.mark_non_utf8(channel) {
                return Err(RecordError);
            }
            // An actual empty stream is readable as empty, not missing.
            if empty && !output.emit(channel, "") {
                return Err(RecordError);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_utf8_keeps_original_offsets_and_complete_codepoints() {
        let mut stream = TextStream::default();
        assert_eq!(stream.accept(0, b"a\xe4").unwrap(), ("a".into(), false));
        assert_eq!(stream.accept(2, b"\xb8").unwrap(), ("".into(), false));
        assert_eq!(
            stream.accept(3, b"\xad\xf0\x9f").unwrap(),
            ("中".into(), false)
        );
        assert_eq!(
            stream.accept(6, b"\x99\x82z").unwrap(),
            ("🙂z".into(), false)
        );
        assert!(!stream.finish("a中🙂z".as_bytes()).unwrap());
    }
    #[test]
    fn binary_suffix_and_incomplete_eof_are_explicit_not_lossy_text() {
        let mut stream = TextStream::default();
        assert_eq!(
            stream.accept(0, b"real\xfflater").unwrap(),
            ("real".into(), true)
        );
        assert_eq!(stream.accept(10, b"more").unwrap(), ("".into(), false));
        assert!(!stream.finish(b"real\xfflatermore").unwrap());
        let mut stream = TextStream::default();
        assert_eq!(
            stream.accept(0, b"real\xe4\xb8").unwrap(),
            ("real".into(), false)
        );
        assert!(stream.finish(b"real\xe4\xb8").unwrap());
    }
    #[test]
    fn reordered_or_missing_raw_chunks_fail_recording() {
        let mut stream = TextStream::default();
        assert!(stream.accept(1, b"a").is_err());
        stream.accept(0, b"a").unwrap();
        assert!(stream.accept(0, b"a").is_err());
        assert!(stream.finish(b"ab").is_err());
    }
}
