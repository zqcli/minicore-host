//! Optional process-text capability bound by the Session, never by tool-supplied IDs.
use super::*;
use crate::observation::*;
use minicore_host_runtime::tools::{
    Tool, ToolContext, ToolError, ToolFuture, ToolInvocation, ToolSpec,
};

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
    /// Preserve the specification already frozen by the selected ToolSet.
    pub fn wrap(&self, tool: Arc<dyn Tool>, spec: ToolSpec) -> Arc<dyn Tool> {
        Arc::new(OutputTool {
            spec,
            tool,
            binding: self.clone(),
        })
    }
}
/// Immutable identity established once against accepted assistant history.
/// Sinks and worker leases share it without retaining invocation arguments or
/// the Session owner. Each output still checks this original execution's life.
struct ToolOutputContext {
    session: Weak<SessionController>,
    source: ToolCallIdentity,
    response_index: u64,
    name: String,
}
impl ToolOutputContext {
    fn admitted(&self, state: &SessionState, live: &Arc<AtomicBool>, owned: bool) -> bool {
        live.load(Ordering::SeqCst)
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
                        && record.data.state.name == self.name
                        && if owned {
                            record
                                .owned_output
                                .as_ref()
                                .is_some_and(|registered| Arc::ptr_eq(registered, live))
                        } else {
                            record.data.result.is_none()
                                && record
                                    .owned_output
                                    .as_ref()
                                    .is_none_or(|registered| registered.load(Ordering::SeqCst))
                        }
                })
    }
}
/// A tool obtains this only while its real invocation is executing. A cloned
/// capability does not retain the Session and stops accepting text on return.
#[derive(Clone)]
pub struct ToolOutputSink {
    context: Arc<ToolOutputContext>,
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
        let Some(session) = self.context.session.upgrade() else {
            return false;
        };
        session.append_tool_output(&self.context, &self.live, false, channel, text)
    }
    /// Transfer output admission to the registered actual worker owner. The
    /// returned lease is distinct from this future-scoped sink and is revoked
    /// when its actual per-call cleanup or the public Turn cleanup is confirmed.
    pub fn retain_owned(&self) -> Result<ToolOutputLease, Fault> {
        let session = self.context.session.upgrade().ok_or_else(inactive_output)?;
        session.retain_tool_output(self)
    }
}
/// An original invocation's output lease, held by a trusted execution adapter.
/// Its identity and lifetime are minted by the Session, never by tool arguments.
/// Cloning it cannot outlive its invocation's actual cleanup confirmation, or
/// the public Turn's cleanup barrier when no earlier per-call fact is supplied.
#[derive(Clone)]
pub struct ToolOutputLease {
    context: Arc<ToolOutputContext>,
    live: Arc<AtomicBool>,
}
impl ToolOutputLease {
    pub fn source(&self) -> &ToolCallIdentity {
        &self.context.source
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
            self.source(),
            &self.context.name,
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
        let session = self.context.session.upgrade().ok_or_else(inactive_output)?;
        session.record_tool_edit(self, path, before, after, outcome)
    }
    /// Confirm this invocation's actual owned work has ended, after all file
    /// work, process/group waits and pipe drains. This closes every clone's
    /// output admission and publishes finished only if its native result was
    /// already received. It does not create, save or replace a ToolResult.
    pub fn confirm_cleanup(&self) -> Result<(), Fault> {
        let session = self.context.session.upgrade().ok_or_else(inactive_output)?;
        let mut state = session.state.lock().expect("Session mutex poisoned");
        if !self.admitted(&state) {
            return Err(inactive_output());
        }
        self.live.store(false, Ordering::SeqCst);
        state.runner_revision = state
            .runner_revision
            .checked_add(1)
            .expect("runner revision exhausted");
        let revision = state.runner_revision;
        let record = state
            .output
            .get_mut(&self.source().turn.turn_id)
            .unwrap()
            .tools
            .get_mut(&self.source().tool_call_id)
            .unwrap();
        let diagnostic = session.publish_tool_finished(self.source().clone(), record, revision);
        drop(state);
        diagnostic::emit(diagnostic);
        Ok(())
    }

    /// Mark a real binary stream without replacing its bytes with fake text.
    /// The already-published UTF-8 prefix remains unchanged and range-readable.
    pub fn mark_non_utf8(&self, channel: &str) -> bool {
        let Some(session) = self.context.session.upgrade() else {
            return false;
        };
        let mut state = session.state.lock().expect("Session mutex poisoned");
        if channel.is_empty() || !self.admitted(&state) {
            return false;
        }
        state
            .output
            .get_mut(&self.source().turn.turn_id)
            .unwrap()
            .tools
            .get_mut(&self.source().tool_call_id)
            .unwrap()
            .output
            .entry(channel.into())
            .or_default()
            .non_utf8 = true;
        true
    }
    pub(super) fn admitted(&self, state: &SessionState) -> bool {
        self.context.admitted(state, &self.live, true)
    }

    pub fn emit(&self, channel: &str, text: &str) -> bool {
        if channel.is_empty() || !self.live.load(Ordering::SeqCst) {
            return false;
        }
        let Some(session) = self.context.session.upgrade() else {
            return false;
        };
        session.append_tool_output(&self.context, &self.live, true, channel, text)
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
        Box::pin(async move {
            let sink = self
                .binding
                .session
                .upgrade()
                .ok_or(ToolError::Internal)?
                .tool_output_sink(&self.binding, &invocation)
                .ok_or(ToolError::Internal)?;
            let _end = EndOutput(Arc::clone(&sink.live));
            CURRENT_OUTPUT
                .scope(sink, self.tool.execute(invocation, context))
                .await
        })
    }
}
impl SessionController {
    fn tool_output_sink(
        &self,
        binding: &ToolOutputBinding,
        invocation: &ToolInvocation,
    ) -> Option<ToolOutputSink> {
        let state = self.state.lock().expect("Session mutex poisoned");
        if state.reclaimed {
            return None;
        }
        let active = state
            .active
            .as_ref()
            .filter(|active| active.info.id == binding.turn_id)?;
        let call = ToolCallId(invocation.tool_call_id.as_str().to_owned());
        // Complete assistant history establishes the invocation before its real
        // tool starts. Check the full arguments exactly once when minting this
        // context, never on a chunk callback or a sink/worker-lease clone.
        let record = state
            .output
            .get(&binding.turn_id)?
            .tools
            .get(&call)
            .filter(|record| {
                record.data.result.is_none()
                    && record.data.state.name == invocation.tool_name.as_str()
                    && record.data.arguments == invocation.arguments
                    && record
                        .owned_output
                        .as_ref()
                        .is_none_or(|live| live.load(Ordering::SeqCst))
            })?;
        Some(ToolOutputSink {
            context: Arc::new(ToolOutputContext {
                session: binding.session.clone(),
                source: ToolCallIdentity {
                    turn: self.turn_identity(&active.info),
                    tool_call_id: call,
                },
                response_index: record.data.state.response_index,
                name: record.data.state.name.clone(),
            }),
            live: Arc::new(AtomicBool::new(true)),
        })
    }
    fn retain_tool_output(&self, sink: &ToolOutputSink) -> Result<ToolOutputLease, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if !sink.context.admitted(&state, &sink.live, false) {
            return Err(inactive_output());
        }
        let source = &sink.context.source;
        let record = state
            .output
            .get(&source.turn.turn_id)
            .unwrap()
            .tools
            .get(&source.tool_call_id)
            .unwrap();
        let newly_owned = record.owned_output.is_none();
        let live = record
            .owned_output
            .clone()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(true)));
        let active = state
            .active
            .as_mut()
            .filter(|active| matches!(active.info.state, TurnState::Starting | TurnState::Running))
            .ok_or_else(inactive_output)?;
        if newly_owned && !active.tool_cleanup.retain_output(Arc::clone(&live)) {
            return Err(inactive_output());
        }
        state
            .output
            .get_mut(&source.turn.turn_id)
            .unwrap()
            .tools
            .get_mut(&source.tool_call_id)
            .unwrap()
            .owned_output = Some(Arc::clone(&live));
        Ok(ToolOutputLease {
            context: Arc::clone(&sink.context),
            live,
        })
    }
    fn append_tool_output(
        &self,
        context: &ToolOutputContext,
        live: &Arc<AtomicBool>,
        owned: bool,
        channel: &str,
        text: &str,
    ) -> bool {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if !context.admitted(&state, live, owned) {
            return false;
        }
        let record = state
            .output
            .get_mut(&context.source.turn.turn_id)
            .unwrap()
            .tools
            .get_mut(&context.source.tool_call_id)
            .unwrap();
        let body = record.output.entry(channel.into()).or_default();
        if body.non_utf8 {
            return false;
        }
        let start = body.text.len() as u64;
        body.text.push_str(text);
        self.access
            .observation
            .publish(ObservationEvent::Activity(ActivityEvent::ToolOutput {
                source: context.source.clone(),
                data: OutputDelta {
                    channel: channel.into(),
                    start,
                    end: body.text.len() as u64,
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
