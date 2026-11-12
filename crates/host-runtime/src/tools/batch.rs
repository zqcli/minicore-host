use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{
    ApprovalDecision, ApprovalRequest, EnabledTool, ToolContext, ToolDecision,
    ToolExecutionOutcome, ToolInputRequest, ToolInvocation, ToolName, ToolOutput, ToolPolicy,
    ToolPolicyRequest, ToolProgress, ToolProgressEmitter, ToolProgressSink, ToolResultOutcome,
    ToolSpec,
};
use crate::error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary};
use crate::execution::ExecutionConfig;
use crate::history::ToolResultHistory;
use crate::ids::{InteractionId, LoopId, ToolCallId};
use crate::interaction::{InteractionAnswer, InteractionKind, PendingInteraction};
use crate::model::ToolCall;
use crate::port_call::{PortCallOutcome, run_port_call};
use crate::time::DeadlineSource;
use crate::value::Text;

#[derive(Clone)]
pub struct ToolBatchContext {
    pub loop_id: LoopId,
    pub request_index: u32,
    pub cancellation: CancellationToken,
    pub deadline: Option<Instant>,
    pub policy_timeout: Duration,
    pub tool_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolBatchStop {
    Cancelled,
    Deadline,
    Policy(DiagnosticSummary),
    Interaction(DiagnosticSummary),
    Internal(DiagnosticSummary),
}

#[derive(Debug)]
pub struct ToolBatchReport {
    pub results: Vec<ToolResultHistory>,
    pub stop: Option<ToolBatchStop>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolBatchEvent {
    Started {
        call_id: ToolCallId,
        tool_name: ToolName,
    },
    Progress {
        call_id: ToolCallId,
        progress: ToolProgress,
    },
    Finished {
        call_id: ToolCallId,
        outcome: ToolResultOutcome,
        output_bytes: usize,
    },
}

pub trait ToolBatchHost: Send + Sync + 'static {
    fn observe(&self, event: ToolBatchEvent);
    /// Registers the exact waiting point before returning, with the owner validating and accepting its answer once.
    fn request_interaction(
        &self,
        pending: PendingInteraction,
        reply: oneshot::Sender<InteractionAnswer>,
    ) -> Result<(), ToolBatchStop>;
    /// Clears this waiting point without replacing an answer already accepted by the owner.
    fn finish_interaction(&self, interaction_id: InteractionId);
}

struct Batch {
    context: ToolBatchContext,
    host: Arc<dyn ToolBatchHost>,
}
impl Batch {
    fn observe(&self, event: ToolBatchEvent) {
        let _ = catch_unwind(AssertUnwindSafe(|| self.host.observe(event)));
    }
}

enum ToolStep {
    Result(ToolResultHistory),
    End(ToolBatchStop),
}

/// Executes only calls from a complete response validated against this same frozen snapshot.
pub async fn run_tool_batch(
    snapshot: &ExecutionConfig,
    calls: &[&ToolCall],
    context: ToolBatchContext,
    host: Arc<dyn ToolBatchHost>,
) -> ToolBatchReport {
    let ctx = Batch { context, host };
    let mut results = Vec::with_capacity(calls.len());
    for (index, call) in calls.iter().enumerate() {
        let step = if ctx.context.cancellation.is_cancelled() {
            ToolStep::End(ToolBatchStop::Cancelled)
        } else if ctx
            .context
            .deadline
            .is_some_and(|deadline| tokio::time::Instant::now() >= deadline.into())
        {
            ToolStep::End(ToolBatchStop::Deadline)
        } else {
            let enabled = snapshot
                .tools()
                .entry(call.name())
                .expect("model assembly accepts only this request's frozen tools");
            let invocation = ToolInvocation {
                tool_call_id: call.tool_call_id().clone(),
                tool_name: call.name().clone(),
                arguments: call.arguments().clone(),
            };
            ctx.observe(ToolBatchEvent::Started {
                call_id: call.tool_call_id().clone(),
                tool_name: call.name().clone(),
            });
            run_tool_call(
                &ctx,
                ctx.context.request_index,
                &invocation,
                enabled,
                snapshot.policy().cloned(),
            )
            .await
        };
        match step {
            ToolStep::Result(result) => {
                ctx.observe(ToolBatchEvent::Finished {
                    call_id: call.tool_call_id().clone(),
                    outcome: result.outcome,
                    output_bytes: result.output.content().byte_len(),
                });
                results.push(result);
            }
            ToolStep::End(stop) => {
                results.extend(calls[index..].iter().map(|call| {
                    terminal_tool_result(
                        ctx.context.loop_id,
                        ctx.context.request_index,
                        call.tool_call_id(),
                        call.name(),
                        ToolResultOutcome::Cancelled,
                        "tool batch interrupted",
                    )
                }));
                return ToolBatchReport {
                    results,
                    stop: Some(stop),
                };
            }
        }
    }
    ToolBatchReport {
        results,
        stop: None,
    }
}

fn internal_failure() -> ToolBatchStop {
    ToolBatchStop::Internal(DiagnosticSummary::static_text(
        DiagnosticCode::Internal,
        DiagnosticCategory::Internal,
        "tool execution failed internally",
        false,
    ))
}
fn policy_failure() -> ToolBatchStop {
    ToolBatchStop::Policy(DiagnosticSummary::static_text(
        DiagnosticCode::PolicyFailed,
        DiagnosticCategory::Policy,
        "tool policy returned an invalid decision",
        false,
    ))
}
fn interaction_failure() -> ToolBatchStop {
    ToolBatchStop::Interaction(DiagnosticSummary::static_text(
        DiagnosticCode::InteractionNotFound,
        DiagnosticCategory::Internal,
        "tool interaction did not settle",
        false,
    ))
}

/// Executes one tool call: policy decision, optional interaction, and the
/// tool port itself. Ordinary tool failures become `Failed` results so the
/// model can continue; only cancelled/deadline/broken interactions end the
/// batch.
async fn run_tool_call(
    ctx: &Batch,
    request_index: u32,
    invocation: &ToolInvocation,
    enabled: &EnabledTool,
    policy: Option<Arc<dyn ToolPolicy>>,
) -> ToolStep {
    let cancellation = ctx.context.cancellation.clone();

    let decision = match decide_policy(
        policy,
        invocation,
        &enabled.spec,
        ctx.context.deadline,
        &cancellation,
        ctx.context.policy_timeout,
    )
    .await
    {
        PolicyStep::Allow => None,
        PolicyStep::Denied(reason) => {
            return ToolStep::Result(denied_result(
                ctx.context.loop_id,
                request_index,
                invocation,
                reason,
            ));
        }
        PolicyStep::RequireApproval(request) => Some(request),
        PolicyStep::Cancelled => return ToolStep::End(ToolBatchStop::Cancelled),
        PolicyStep::TurnDeadline => return ToolStep::End(ToolBatchStop::Deadline),
        PolicyStep::Invalid => return ToolStep::End(policy_failure()),
    };

    if let Some(request) = decision {
        let interaction_id = match InteractionId::new() {
            Ok(id) => id,
            Err(_) => return ToolStep::End(internal_failure()),
        };
        let (reply, receiver) = oneshot::channel();
        let answer = match wait_interaction(
            ctx,
            invocation,
            interaction_id,
            InteractionKind::Approval(request),
            reply,
            receiver,
        )
        .await
        {
            Ok(answer) => answer,
            Err(path) => return ToolStep::End(path),
        };
        match answer {
            InteractionAnswer::Approval(ApprovalDecision::AllowOnce) => {}
            InteractionAnswer::Approval(ApprovalDecision::Deny) => {
                return ToolStep::Result(denied_result(
                    ctx.context.loop_id,
                    request_index,
                    invocation,
                    None,
                ));
            }
            _ => return ToolStep::End(internal_failure()),
        }
    }

    match run_tool_port(
        ctx,
        invocation,
        Arc::clone(&enabled.implementation),
        ctx.context.deadline,
    )
    .await
    {
        ToolPort::Completed(output) => ToolStep::Result(completed_result(
            ctx.context.loop_id,
            request_index,
            invocation,
            output,
        )),
        ToolPort::RequestInput(request) => {
            let interaction_id = match InteractionId::new() {
                Ok(id) => id,
                Err(_) => return ToolStep::End(internal_failure()),
            };
            let (reply, receiver) = oneshot::channel();
            let answer = match wait_interaction(
                ctx,
                invocation,
                interaction_id,
                InteractionKind::ToolInput(request.clone()),
                reply,
                receiver,
            )
            .await
            {
                Ok(answer) => answer,
                Err(path) => return ToolStep::End(path),
            };
            match answer {
                InteractionAnswer::ToolInput(answer) => {
                    let content = answer.encode_result(&request);
                    ToolStep::Result(provided_input_result(
                        ctx.context.loop_id,
                        request_index,
                        invocation,
                        content,
                    ))
                }
                _ => ToolStep::End(internal_failure()),
            }
        }
        ToolPort::Failed => ToolStep::Result(terminal_tool_result(
            ctx.context.loop_id,
            request_index,
            invocation.tool_call_id(),
            invocation.tool_name(),
            ToolResultOutcome::Failed,
            "tool failed",
        )),
        ToolPort::End(path) => ToolStep::End(path),
    }
}

enum PolicyStep {
    Allow,
    Denied(Option<Text>),
    RequireApproval(ApprovalRequest),
    Cancelled,
    TurnDeadline,
    Invalid,
}

async fn decide_policy(
    policy: Option<Arc<dyn ToolPolicy>>,
    invocation: &ToolInvocation,
    spec: &ToolSpec,
    turn_deadline: Option<Instant>,
    cancellation: &CancellationToken,
    policy_timeout: Duration,
) -> PolicyStep {
    let Some(policy) = policy else {
        // An absent policy permits direct execution under the bound tool capability.
        return PolicyStep::Allow;
    };
    let outcome = run_port_call(
        cancellation,
        turn_deadline,
        policy_timeout,
        |child, deadline| {
            let request = ToolPolicyRequest {
                invocation: invocation.clone(),
                spec: spec.clone(),
                cancellation: child,
                deadline,
            };
            policy.decide(request)
        },
    )
    .await;
    match outcome {
        PortCallOutcome::Returned(Ok(decision)) => {
            if decision.validate().is_err() {
                return PolicyStep::Invalid;
            }
            match decision {
                ToolDecision::Allow => PolicyStep::Allow,
                ToolDecision::Deny { reason } => PolicyStep::Denied(Some(reason)),
                ToolDecision::RequireApproval { request } => PolicyStep::RequireApproval(request),
            }
        }
        PortCallOutcome::Cancelled => PolicyStep::Cancelled,
        PortCallOutcome::DeadlineExceeded(DeadlineSource::Turn) => PolicyStep::TurnDeadline,
        // Fail-closed: an unavailable or panicking policy denies the call
        // rather than stopping the Turn.
        PortCallOutcome::Returned(Err(_))
        | PortCallOutcome::DeadlineExceeded(DeadlineSource::Port)
        | PortCallOutcome::InvalidDeadline(_)
        | PortCallOutcome::Panicked => PolicyStep::Denied(None),
    }
}

enum ToolPort {
    Completed(ToolOutput),
    RequestInput(ToolInputRequest),
    Failed,
    End(ToolBatchStop),
}

async fn run_tool_port(
    ctx: &Batch,
    invocation: &ToolInvocation,
    implementation: Arc<dyn crate::tools::Tool>,
    turn_deadline: Option<Instant>,
) -> ToolPort {
    let result = run_port_call(
        &ctx.context.cancellation,
        turn_deadline,
        ctx.context.tool_timeout,
        |child, deadline| {
            let context = ToolContext {
                cancellation: child,
                deadline,
                progress: ToolProgressSink::from_emitter(BatchProgress {
                    call_id: invocation.tool_call_id().clone(),
                    host: Arc::clone(&ctx.host),
                }),
            };
            implementation.execute(invocation.clone(), context)
        },
    )
    .await;
    match result {
        PortCallOutcome::Returned(Ok(ToolExecutionOutcome::Completed(output))) => {
            ToolPort::Completed(output)
        }
        PortCallOutcome::Returned(Ok(ToolExecutionOutcome::RequestInput(request)))
            if request.validate().is_ok() =>
        {
            ToolPort::RequestInput(request)
        }
        PortCallOutcome::Returned(Ok(ToolExecutionOutcome::RequestInput(_))) => ToolPort::Failed,
        PortCallOutcome::Cancelled => ToolPort::End(ToolBatchStop::Cancelled),
        PortCallOutcome::DeadlineExceeded(DeadlineSource::Turn) => {
            ToolPort::End(ToolBatchStop::Deadline)
        }
        PortCallOutcome::Returned(Err(_))
        | PortCallOutcome::DeadlineExceeded(DeadlineSource::Port)
        | PortCallOutcome::Panicked => ToolPort::Failed,
        PortCallOutcome::InvalidDeadline(_) => ToolPort::End(internal_failure()),
    }
}

struct BatchProgress {
    call_id: ToolCallId,
    host: Arc<dyn ToolBatchHost>,
}
impl ToolProgressEmitter for BatchProgress {
    fn emit(&self, progress: ToolProgress) -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            self.host.observe(ToolBatchEvent::Progress {
                call_id: self.call_id.clone(),
                progress,
            })
        }))
        .is_ok()
    }
}

async fn wait_interaction(
    ctx: &Batch,
    invocation: &ToolInvocation,
    interaction_id: InteractionId,
    kind: InteractionKind,
    reply: oneshot::Sender<InteractionAnswer>,
    mut receiver: oneshot::Receiver<InteractionAnswer>,
) -> Result<InteractionAnswer, ToolBatchStop> {
    let pending = PendingInteraction {
        interaction_id,
        tool_call_id: invocation.tool_call_id().clone(),
        tool_name: invocation.tool_name().clone(),
        kind,
    };
    catch_unwind(AssertUnwindSafe(|| {
        ctx.host.request_interaction(pending, reply)
    }))
    .map_err(|_| interaction_failure())??;
    let timeout = async {
        match ctx.context.deadline {
            Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        biased;
        _ = ctx.context.cancellation.cancelled() => Err(ToolBatchStop::Cancelled),
        _ = timeout => Err(ToolBatchStop::Deadline),
        answer = &mut receiver => answer.map_err(|_| interaction_failure()),
    };
    catch_unwind(AssertUnwindSafe(|| {
        ctx.host.finish_interaction(interaction_id)
    }))
    .map_err(|_| interaction_failure())?;
    result
}

fn completed_result(
    id: LoopId,
    request_index: u32,
    invocation: &ToolInvocation,
    output: ToolOutput,
) -> ToolResultHistory {
    ToolResultHistory {
        loop_id: id,
        request_index,
        call_id: invocation.tool_call_id().clone(),
        tool_name: invocation.tool_name().clone(),
        outcome: ToolResultOutcome::Success,
        output,
    }
}

fn provided_input_result(
    id: LoopId,
    request_index: u32,
    invocation: &ToolInvocation,
    content: String,
) -> ToolResultHistory {
    ToolResultHistory {
        loop_id: id,
        request_index,
        call_id: invocation.tool_call_id().clone(),
        tool_name: invocation.tool_name().clone(),
        outcome: ToolResultOutcome::InputProvided,
        output: ToolOutput::new(content).expect("validated answer has valid JSON text"),
    }
}

fn denied_result(
    id: LoopId,
    request_index: u32,
    invocation: &ToolInvocation,
    reason: Option<Text>,
) -> ToolResultHistory {
    ToolResultHistory {
        loop_id: id,
        request_index,
        call_id: invocation.tool_call_id().clone(),
        tool_name: invocation.tool_name().clone(),
        outcome: ToolResultOutcome::Denied,
        output: ToolOutput::new(reason.as_ref().map_or("tool denied", Text::as_str))
            .expect("validated denial reason has valid text"),
    }
}

fn terminal_tool_result(
    id: LoopId,
    request_index: u32,
    call_id: &ToolCallId,
    tool_name: &ToolName,
    outcome: ToolResultOutcome,
    message: &'static str,
) -> ToolResultHistory {
    ToolResultHistory {
        loop_id: id,
        request_index,
        call_id: call_id.clone(),
        tool_name: tool_name.clone(),
        outcome,
        output: ToolOutput::new(message).expect("static tool output has valid text"),
    }
}
