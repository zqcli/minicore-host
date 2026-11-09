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
    fn append_tool_output(
        &self,
        turn_id: &TurnId,
        invocation: &ToolInvocation,
        channel: &str,
        text: &str,
        live: &AtomicBool,
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
        let Some(record) = state
            .output
            .get_mut(turn_id)
            .and_then(|o| o.tools.get_mut(&source.tool_call_id))
        else {
            return false;
        };
        if record.data.result.is_some()
            || record.data.state.name != invocation.tool_name.as_str()
            || record.data.arguments != invocation.arguments
        {
            return false;
        }
        let body = record.output.entry(channel.into()).or_default();
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
