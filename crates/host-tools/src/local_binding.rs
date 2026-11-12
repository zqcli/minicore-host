//! Frozen local-tool execution and recording against the original Session owner.
//! Ordinary tools receive a Workspace and owner-minted output leases, never the
//! agent tool's business port or a model-supplied source identity. The recording
//! adapter retains only invocation leases and UTF-8 stream state.
use minicore_host_core::{
    OwnerFuture, PreparedTurnBinding, ToolEnvironmentExclusions, ToolOutputLease, ToolOutputSink,
    TurnCapability, TurnToolCleanup,
};
use minicore_host_local_tools::{
    CapturedEdit, CommandEnvironment, CommandResult, LocalToolSink, LocalTools, OutputChannel,
    RecordError,
};
use minicore_host_protocol::{Fault, FaultKind};
use minicore_host_runtime::{
    ToolCallId,
    tools::{Tool, ToolContext, ToolError, ToolFuture, ToolInvocation, ToolSpec},
};
use minicore_host_workspace::Workspace;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// Actual Session-bound recording, distinct from an observer or a standalone
/// no-op sink. `begin` runs before the local worker starts, from the real Runtime
/// invocation. The output lease carries its owner-minted Turn and Load identity.
/// All callbacks are short, synchronous ownership handoffs; necessary saving
/// remains owned by the receiving Session rather than by event subscribers.
pub trait LocalToolInvocationSink: LocalToolSink {
    fn begin(
        &self,
        invocation: &ToolInvocation,
        output: ToolOutputLease,
    ) -> Result<(), RecordError>;
}

pub struct BoundLocalTools {
    tools: Arc<LocalTools>,
    capability: TurnCapability,
    sink: Arc<dyn LocalToolInvocationSink>,
}
impl BoundLocalTools {
    /// Production uses the original Session owner, never a standalone no-op
    /// sink. The adapter retains no second result store or execution scheduler.
    pub fn production(binding: PreparedTurnBinding, workspace: Workspace) -> Result<Self, Fault> {
        Self::prepare(
            binding,
            workspace,
            Arc::new(SessionLocalToolSink::default()),
        )
    }

    pub fn prepare(
        binding: PreparedTurnBinding,
        workspace: Workspace,
        sink: Arc<dyn LocalToolInvocationSink>,
    ) -> Result<Self, Fault> {
        let environment = Arc::new(HostCommandEnvironment(
            binding.tool_environment_exclusions()?,
        ));
        let tools = Arc::new(LocalTools::new(workspace, sink.clone(), environment));
        binding.register_tool_cleanup(Arc::new(LocalCleanup(tools.clone())))?;
        Ok(Self {
            tools,
            capability: binding.capability(),
            sink,
        })
    }
    pub(crate) fn tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(BoundLocalTool {
            tool: self.tools.tool(name)?,
            capability: self.capability.clone(),
            sink: self.sink.clone(),
        }))
    }
}
struct HostCommandEnvironment(ToolEnvironmentExclusions);
impl CommandEnvironment for HostCommandEnvironment {
    fn excluded_names(&self) -> Vec<String> {
        self.0.names()
    }
}
struct BoundLocalTool {
    tool: Arc<dyn Tool>,
    capability: TurnCapability,
    sink: Arc<dyn LocalToolInvocationSink>,
}
impl Tool for BoundLocalTool {
    fn spec(&self) -> &ToolSpec {
        self.tool.spec()
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, context: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            self.capability
                .check_active()
                .map_err(|_| ToolError::Internal)?;
            let output = ToolOutputSink::current()
                .ok_or(ToolError::Internal)?
                .retain_owned()
                .map_err(|_| ToolError::Internal)?;
            self.sink
                .begin(&invocation, output)
                .map_err(|_| ToolError::Internal)?;
            self.tool.execute(invocation, context).await
        })
    }
}
struct LocalCleanup(Arc<LocalTools>);
impl TurnToolCleanup for LocalCleanup {
    fn finish(&self) -> OwnerFuture<'_, Vec<Fault>> {
        Box::pin(async move {
            let completions = self.0.finish().await;
            if let Some(completion) = completions.iter().find(|result| !result.cleanup_confirmed) {
                return Err(Fault::new(
                    FaultKind::TurnInternalUnexpected,
                    format!(
                        "tool call {} did not confirm actual worker cleanup",
                        completion.tool_call_id
                    ),
                ));
            }
            Ok(completions
                .iter()
                .filter(|result| result.error == Some(ToolError::Internal))
                .map(|result| {
                    Fault::new(
                        FaultKind::TurnInternalUnexpected,
                        format!(
                            "tool call {} returned an internal fault after actual cleanup",
                            result.tool_call_id
                        ),
                    )
                })
                .collect())
        })
    }
}

// Production recording hands results to the original Session owner. It does
// not retain a second result store or make observer delivery authoritative.
#[derive(Default)]
struct SessionLocalToolSink(Mutex<BTreeMap<String, Invocation>>);
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
        output.record_process_exit(
            result.pid,
            result.exit_code,
            result.signal,
            result.process_waited,
            result.cleanup_confirmed,
        );
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
