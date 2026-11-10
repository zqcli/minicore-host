//! Optional process-text capability bound by the Session, never by tool-supplied IDs.
use super::*;
use crate::observation::*;
use minicore_runtime::tools::{Tool, ToolContext, ToolFuture, ToolInvocation, ToolSpec};

tokio::task_local! { static CURRENT_OUTPUT: ToolOutputSink; }

/// Passed only to the trusted resource assembler for the original Turn. Wrap
/// tools which produce process text; ordinary Runtime tools need not change.
#[derive(Clone)]
pub struct ToolOutputBinding {
    session: Weak<SessionController>,
    turn_id: TurnId,
}
impl ToolOutputBinding {
    pub(super) fn new(session: &Arc<SessionController>, turn_id: TurnId) -> Self {
        Self {
            session: Arc::downgrade(session),
            turn_id,
        }
    }
    pub fn wrap(&self, tool: Arc<dyn Tool>) -> Arc<dyn Tool> {
        Arc::new(OutputTool {
            spec: tool.spec().clone(),
            tool,
            binding: self.clone(),
        })
    }
}
/// A tool obtains this only while its real invocation is executing. A cloned
/// capability does not retain the Session and stops accepting text on return.
#[derive(Clone)]
pub struct ToolOutputSink {
    binding: ToolOutputBinding,
    invocation: ToolInvocation,
    live: Arc<AtomicBool>,
}
impl ToolOutputSink {
    pub fn current() -> Option<Self> {
        CURRENT_OUTPUT.try_with(Clone::clone).ok()
    }
    pub fn emit(&self, channel: &str, text: &str) -> bool {
        if channel.is_empty() || !self.live.load(Ordering::SeqCst) {
            return false;
        }
        let Some(session) = self.binding.session.upgrade() else {
            return false;
        };
        session.append_tool_output(
            &self.binding.turn_id,
            &self.invocation,
            channel,
            text,
            &self.live,
            None,
        )
    }
    /// Transfer output admission to the registered actual worker owner. The
    /// returned lease is distinct from this future-scoped sink and is revoked
    /// when its actual per-call cleanup or the public Turn cleanup is confirmed.
    pub fn retain_owned(&self) -> Result<ToolOutputLease, Fault> {
        let session = self.binding.session.upgrade().ok_or_else(inactive_output)?;
        session.retain_tool_output(self)
    }
}
/// An original invocation's output lease, held by a trusted execution adapter.
/// Its identity and lifetime are minted by the Session, never by tool arguments.
/// Cloning it cannot outlive its invocation's actual cleanup confirmation, or
/// the public Turn's cleanup barrier when no earlier per-call fact is supplied.
#[derive(Clone)]
pub struct ToolOutputLease {
    sink: ToolOutputSink,
    source: ToolCallIdentity,
    response_index: u64,
}
impl ToolOutputLease {
    pub fn source(&self) -> &ToolCallIdentity {
        &self.source
    }
    /// Observe the trusted command owner's actual wait/cleanup facts. This is
    /// diagnostic-only: it grants no admission, confirms no cleanup and cannot
    /// alter the original command result. Call outside the adapter's lock.
    pub fn record_process_exit(
        &self,
        pid: u32,
        exit_code: Option<i32>,
        signal: Option<i32>,
        process_waited: bool,
        cleanup_confirmed: bool,
    ) {
        diagnostics::process_exit(
            &self.source,
            self.sink.invocation.tool_name.as_str(),
            pid,
            exit_code,
            signal,
            process_waited,
            cleanup_confirmed,
        )
        .emit();
    }
    /// Record an actual mutation observation under this original invocation.
    /// This retained Session journal is not a new durable history database.
    pub fn record_edit(
        &self,
        path: String,
        before: minicore_host_workspace::FileSnapshot,
        after: minicore_host_workspace::FileSnapshot,
        outcome: minicore_host_workspace::EditOutcome,
    ) -> Result<(), Fault> {
        let session = self
            .sink
            .binding
            .session
            .upgrade()
            .ok_or_else(inactive_output)?;
        session.record_tool_edit(self, path, before, after, outcome)
    }
    /// Confirm this invocation's actual owned work has ended, after all file
    /// work, process/group waits and pipe drains. This closes every clone's
    /// output admission and publishes finished only if its native result was
    /// already received. It does not create, save or replace a ToolResult.
    pub fn confirm_cleanup(&self) -> Result<(), Fault> {
        let session = self
            .sink
            .binding
            .session
            .upgrade()
            .ok_or_else(inactive_output)?;
        let mut state = session.state.lock().expect("Session mutex poisoned");
        if !self.admitted(&state) {
            return Err(inactive_output());
        }
        self.sink.live.store(false, Ordering::SeqCst);
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        let record = state
            .output
            .get_mut(&self.source.turn.turn_id)
            .unwrap()
            .tools
            .get_mut(&self.source.tool_call_id)
            .unwrap();
        let diagnostic = session.publish_tool_finished(self.source.clone(), record, revision);
        drop(state);
        diagnostic::emit(diagnostic);
        Ok(())
    }

    /// Mark a real binary stream without replacing its bytes with fake text.
    /// The already-published UTF-8 prefix remains unchanged and range-readable.
    pub fn mark_non_utf8(&self, channel: &str) -> bool {
        let Some(session) = self.sink.binding.session.upgrade() else {
            return false;
        };
        let mut state = session.state.lock().expect("Session mutex poisoned");
        if channel.is_empty() || !self.admitted(&state) {
            return false;
        }
        state
            .output
            .get_mut(&self.source.turn.turn_id)
            .unwrap()
            .tools
            .get_mut(&self.source.tool_call_id)
            .unwrap()
            .output
            .entry(channel.into())
            .or_default()
            .non_utf8 = true;
        true
    }
    pub(super) fn admitted(&self, state: &SessionState) -> bool {
        self.sink.live.load(Ordering::SeqCst)
            && !state.reclaimed
            && state.active.as_ref().is_some_and(|active| {
                active.info.id == self.source.turn.turn_id
                    && Some(&active.info.load_id) == self.source.turn.load_id.as_ref()
            })
            && state
                .output
                .get(&self.source.turn.turn_id)
                .and_then(|turn| turn.tools.get(&self.source.tool_call_id))
                .is_some_and(|record| {
                    record.data.state.response_index == self.response_index
                        && record.data.state.name == self.sink.invocation.tool_name.as_str()
                        && record.data.arguments == self.sink.invocation.arguments
                })
    }

    pub fn emit(&self, channel: &str, text: &str) -> bool {
        if channel.is_empty() || !self.sink.live.load(Ordering::SeqCst) {
            return false;
        }
        let Some(session) = self.sink.binding.session.upgrade() else {
            return false;
        };
        session.append_tool_output(
            &self.sink.binding.turn_id,
            &self.sink.invocation,
            channel,
            text,
            &self.sink.live,
            Some((&self.source, self.response_index)),
        )
    }
}
struct OutputTool {
    tool: Arc<dyn Tool>,
    spec: ToolSpec,
    binding: ToolOutputBinding,
}
struct EndOutput(Arc<AtomicBool>);
impl Drop for EndOutput {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
impl Tool for OutputTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, context: ToolContext) -> ToolFuture<'a> {
        let sink = ToolOutputSink {
            binding: self.binding.clone(),
            invocation: invocation.clone(),
            live: Arc::new(AtomicBool::new(true)),
        };
        Box::pin(async move {
            let _end = EndOutput(Arc::clone(&sink.live));
            CURRENT_OUTPUT
                .scope(sink, self.tool.execute(invocation, context))
                .await
        })
    }
}
impl SessionController {
    fn retain_tool_output(&self, sink: &ToolOutputSink) -> Result<ToolOutputLease, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if !sink.live.load(Ordering::SeqCst) || state.reclaimed {
            return Err(inactive_output());
        }
        let call = ToolCallId(sink.invocation.tool_call_id.as_str().to_owned());
        // Complete assistant history establishes the invocation before its real
        // tool starts. Never wait for a ToolStarted observation event to do so.
        let record = state
            .output
            .get(&sink.binding.turn_id)
            .and_then(|turn| turn.tools.get(&call))
            .filter(|record| {
                record.data.result.is_none()
                    && record.data.state.name == sink.invocation.tool_name.as_str()
                    && record.data.arguments == sink.invocation.arguments
            })
            .ok_or_else(inactive_output)?;
        let response_index = record.data.state.response_index;
        let newly_owned = record.owned_output.is_none();
        let live = record
            .owned_output
            .clone()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(true)));
        if !live.load(Ordering::SeqCst) {
            return Err(inactive_output());
        }
        let active = state
            .active
            .as_mut()
            .filter(|active| {
                active.info.id == sink.binding.turn_id
                    && matches!(active.info.state, TurnState::Starting | TurnState::Running)
            })
            .ok_or_else(inactive_output)?;
        if newly_owned && !active.tool_cleanup.retain_output(Arc::clone(&live)) {
            return Err(inactive_output());
        }
        let load_id = active.info.load_id.clone();
        state
            .output
            .get_mut(&sink.binding.turn_id)
            .unwrap()
            .tools
            .get_mut(&call)
            .unwrap()
            .owned_output = Some(Arc::clone(&live));
        Ok(ToolOutputLease {
            sink: ToolOutputSink {
                binding: sink.binding.clone(),
                invocation: sink.invocation.clone(),
                live,
            },
            source: ToolCallIdentity {
                turn: TurnIdentity {
                    session: self.identity(),
                    turn_id: sink.binding.turn_id.clone(),
                    load_id: Some(load_id),
                },
                tool_call_id: call,
            },
            response_index,
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn append_tool_output(
        &self,
        turn_id: &TurnId,
        invocation: &ToolInvocation,
        channel: &str,
        text: &str,
        live: &AtomicBool,
        owned: Option<(&ToolCallIdentity, u64)>,
    ) -> bool {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if !live.load(Ordering::SeqCst) || state.reclaimed {
            return false;
        }
        let Some(active) = state.active.as_ref().filter(|a| &a.info.id == turn_id) else {
            return false;
        };
        let source = ToolCallIdentity {
            turn: TurnIdentity {
                session: self.identity(),
                turn_id: turn_id.clone(),
                load_id: Some(active.info.load_id.clone()),
            },
            tool_call_id: ToolCallId(invocation.tool_call_id.as_str().to_owned()),
        };
        if owned.is_some_and(|(original, _)| original != &source) {
            return false;
        }
        let Some(record) = state
            .output
            .get_mut(turn_id)
            .and_then(|o| o.tools.get_mut(&source.tool_call_id))
        else {
            return false;
        };
        if record
            .owned_output
            .as_ref()
            .is_some_and(|live| !live.load(Ordering::SeqCst))
            || (owned.is_none() && record.data.result.is_some())
            || owned.is_some_and(|(_, index)| record.data.state.response_index != index)
            || record.data.state.name != invocation.tool_name.as_str()
            || record.data.arguments != invocation.arguments
        {
            return false;
        }
        let body = record.output.entry(channel.into()).or_default();
        if body.non_utf8 {
            return false;
        }
        let start = body.live.len() as u64;
        body.live.push_str(text);
        self.access
            .observation
            .publish(ObservationEvent::Activity(ActivityEvent::ToolOutput {
                source,
                data: OutputDelta {
                    channel: channel.into(),
                    start,
                    end: body.live.len() as u64,
                    text: text.into(),
                },
            }));
        true
    }
}
fn inactive_output() -> Fault {
    Fault::new(
        FaultKind::TurnInternalUnexpected,
        "output lease requires the original active invocation and registered tool cleanup owner",
    )
}
