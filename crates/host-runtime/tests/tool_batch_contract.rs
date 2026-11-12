use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use minicore_host_runtime::model::{
    Model, ModelCallContext, ModelDescriptor, ModelRequest, ModelStartFuture, ReasoningPreference,
    ToolCall,
};
use minicore_host_runtime::prompt::DefaultPromptProvider;
use minicore_host_runtime::tools::{
    ApprovalDecision, ApprovalRequest, ApprovalRisk, Tool, ToolBatchContext, ToolBatchEvent,
    ToolBatchHost, ToolBatchReport, ToolBatchStop, ToolContext, ToolDecision, ToolError,
    ToolExecutionOutcome, ToolFuture, ToolInputAnswer, ToolInputAnswerKind, ToolInputRequest,
    ToolInvocation, ToolOutput, ToolPolicy, ToolPolicyFuture, ToolPolicyRequest, ToolResultOutcome,
    ToolSet, ToolSpec, run_tool_batch,
};
use minicore_host_runtime::{
    ExecutionConfig, InteractionAnswer, InteractionId, LoopId, PendingInteraction,
};
use serde_json::json;
use tokio::sync::{Notify, oneshot};
use tokio_util::sync::CancellationToken;

struct UnusedModel(ModelDescriptor);
impl Model for UnusedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.0
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        panic!("tool batch must not call the model")
    }
}
#[derive(Clone)]
enum Action {
    Success,
    Failed,
    Hold,
    Input,
}
struct TestTool {
    spec: ToolSpec,
    actions: Mutex<VecDeque<Action>>,
    calls: Mutex<Vec<String>>,
    tokens: Mutex<Vec<CancellationToken>>,
    started: Notify,
}
impl TestTool {
    fn new(actions: Vec<Action>) -> Arc<Self> {
        Arc::new(Self {
            spec: ToolSpec::new(
                "test".parse().unwrap(),
                "test tool",
                json!({"type":"object"}),
            )
            .unwrap(),
            actions: Mutex::new(actions.into()),
            calls: Mutex::new(Vec::new()),
            tokens: Mutex::new(Vec::new()),
            started: Notify::new(),
        })
    }
}
impl Tool for TestTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, context: ToolContext) -> ToolFuture<'a> {
        self.calls
            .lock()
            .unwrap()
            .push(invocation.tool_call_id.to_string());
        self.tokens.lock().unwrap().push(context.cancellation);
        let action = self
            .actions
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted tool call");
        Box::pin(async move {
            self.started.notify_one();
            match action {
                Action::Success => Ok(ToolExecutionOutcome::Completed(
                    ToolOutput::new("done").unwrap(),
                )),
                Action::Failed => Err(ToolError::Failed),
                Action::Hold => std::future::pending().await,
                Action::Input => Ok(ToolExecutionOutcome::RequestInput(
                    ToolInputRequest::new("enter text", vec![], ToolInputAnswerKind::Text).unwrap(),
                )),
            }
        })
    }
}
struct FixedPolicy(ToolDecision);
impl ToolPolicy for FixedPolicy {
    fn decide<'a>(&'a self, _: ToolPolicyRequest) -> ToolPolicyFuture<'a> {
        Box::pin(async { Ok(self.0.clone()) })
    }
}
#[derive(Default)]
struct TestOwner {
    pending: Mutex<Option<(PendingInteraction, oneshot::Sender<InteractionAnswer>)>>,
    waiting: Notify,
    accepted: Mutex<Vec<InteractionId>>,
    finished: Mutex<Vec<InteractionId>>,
    events: Mutex<Vec<ToolBatchEvent>>,
}
impl TestOwner {
    fn answer(&self, answer: InteractionAnswer) {
        let (pending, sender) = self
            .pending
            .lock()
            .unwrap()
            .take()
            .expect("pending test interaction");
        pending.validate_answer(&answer).unwrap();
        self.accepted.lock().unwrap().push(pending.interaction_id);
        sender.send(answer).unwrap();
    }
}
impl ToolBatchHost for TestOwner {
    fn observe(&self, event: ToolBatchEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn request_interaction(
        &self,
        pending: PendingInteraction,
        reply: oneshot::Sender<InteractionAnswer>,
    ) -> Result<(), ToolBatchStop> {
        assert!(
            self.pending
                .lock()
                .unwrap()
                .replace((pending, reply))
                .is_none()
        );
        self.waiting.notify_one();
        Ok(())
    }
    fn finish_interaction(&self, id: InteractionId) {
        self.pending.lock().unwrap().take();
        self.finished.lock().unwrap().push(id);
    }
}
fn config(tool: Arc<TestTool>, policy: Option<Arc<dyn ToolPolicy>>) -> ExecutionConfig {
    let mut tools = ToolSet::builder();
    tools.register(tool);
    ExecutionConfig::new(
        Arc::new(UnusedModel(
            ModelDescriptor::new(
                "test/model".parse().unwrap(),
                128_000,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
        )),
        ReasoningPreference::Auto,
        tools.build().unwrap(),
        policy,
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap()
}
fn calls(count: u32) -> Vec<ToolCall> {
    (0..count)
        .map(|index| {
            ToolCall::new(
                format!("call_{index}").parse().unwrap(),
                "test".parse().unwrap(),
                json!({}),
                index,
            )
            .unwrap()
        })
        .collect()
}
fn context(cancellation: CancellationToken) -> ToolBatchContext {
    ToolBatchContext {
        loop_id: LoopId::new().unwrap(),
        request_index: 7,
        cancellation,
        deadline: None,
        policy_timeout: Duration::from_secs(30),
        tool_timeout: Duration::from_secs(30),
    }
}
async fn run(
    config: ExecutionConfig,
    calls: Vec<ToolCall>,
    context: ToolBatchContext,
    owner: Arc<TestOwner>,
) -> ToolBatchReport {
    run_tool_batch(&config, &calls.iter().collect::<Vec<_>>(), context, owner).await
}
fn approval() -> Arc<dyn ToolPolicy> {
    Arc::new(FixedPolicy(
        ToolDecision::require_approval(
            ApprovalRequest::new("allow tool?", ApprovalRisk::Low).unwrap(),
        )
        .unwrap(),
    ))
}

#[tokio::test]
async fn batch_preserves_order_and_continues_after_an_ordinary_tool_failure() {
    let tool = TestTool::new(vec![Action::Failed, Action::Success]);
    let owner = Arc::new(TestOwner::default());
    let report = run(
        config(tool.clone(), None),
        calls(2),
        context(CancellationToken::new()),
        owner.clone(),
    )
    .await;
    assert!(report.stop.is_none());
    assert_eq!(*tool.calls.lock().unwrap(), ["call_0", "call_1"]);
    assert_eq!(
        report.results.iter().map(|r| r.outcome).collect::<Vec<_>>(),
        [ToolResultOutcome::Failed, ToolResultOutcome::Success]
    );
    assert!(report.results.iter().all(|r| r.request_index == 7));
    assert!(
        tool.tokens
            .lock()
            .unwrap()
            .iter()
            .all(CancellationToken::is_cancelled)
    );
    assert!(matches!(
        owner.events.lock().unwrap().last(),
        Some(ToolBatchEvent::Finished {
            outcome: ToolResultOutcome::Success,
            ..
        })
    ));
}

#[tokio::test]
async fn cancellation_stops_the_actual_call_and_settles_unstarted_calls_without_execution() {
    let tool = TestTool::new(vec![Action::Hold, Action::Success]);
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(run(
        config(tool.clone(), None),
        calls(2),
        context(cancellation.clone()),
        Arc::new(TestOwner::default()),
    ));
    tool.started.notified().await;
    cancellation.cancel();
    let report = task.await.unwrap();
    assert_eq!(report.stop, Some(ToolBatchStop::Cancelled));
    assert_eq!(tool.calls.lock().unwrap().len(), 1);
    assert!(tool.tokens.lock().unwrap()[0].is_cancelled());
    assert_eq!(report.results.len(), 2);
    assert!(
        report
            .results
            .iter()
            .all(|r| r.outcome == ToolResultOutcome::Cancelled)
    );
}

#[tokio::test]
async fn policy_denial_never_enters_the_tool() {
    let tool = TestTool::new(vec![]);
    let policy = Arc::new(FixedPolicy(
        ToolDecision::deny("denied by test policy").unwrap(),
    ));
    let report = run(
        config(tool.clone(), Some(policy)),
        calls(1),
        context(CancellationToken::new()),
        Arc::new(TestOwner::default()),
    )
    .await;
    assert!(report.stop.is_none());
    assert!(tool.calls.lock().unwrap().is_empty());
    assert_eq!(report.results[0].outcome, ToolResultOutcome::Denied);
    assert_eq!(
        report.results[0].output.content().as_str(),
        "denied by test policy"
    );
}

#[tokio::test(start_paused = true)]
async fn approval_has_no_implicit_turn_deadline_and_executes_only_after_answer() {
    let tool = TestTool::new(vec![Action::Success]);
    let owner = Arc::new(TestOwner::default());
    let task = tokio::spawn(run(
        config(tool.clone(), Some(approval())),
        calls(1),
        context(CancellationToken::new()),
        owner.clone(),
    ));
    owner.waiting.notified().await;
    tokio::time::advance(Duration::from_secs(48 * 60 * 60)).await;
    assert!(!task.is_finished());
    assert!(tool.calls.lock().unwrap().is_empty());
    owner.answer(InteractionAnswer::Approval(ApprovalDecision::AllowOnce));
    let report = task.await.unwrap();
    assert!(report.stop.is_none());
    assert_eq!(report.results[0].outcome, ToolResultOutcome::Success);
    assert_eq!(owner.finished.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn tool_input_is_encoded_as_one_result_without_reexecuting_the_tool() {
    let tool = TestTool::new(vec![Action::Input]);
    let owner = Arc::new(TestOwner::default());
    let task = tokio::spawn(run(
        config(tool.clone(), None),
        calls(1),
        context(CancellationToken::new()),
        owner.clone(),
    ));
    owner.waiting.notified().await;
    owner.answer(InteractionAnswer::ToolInput(ToolInputAnswer::Text(
        minicore_host_runtime::Text::new("provided"),
    )));
    let report = task.await.unwrap();
    assert!(report.stop.is_none());
    assert_eq!(report.results[0].outcome, ToolResultOutcome::InputProvided);
    assert!(
        report.results[0]
            .output
            .content()
            .as_str()
            .contains("provided")
    );
    assert_eq!(tool.calls.lock().unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn port_timeout_is_an_ordinary_result_but_turn_deadline_stops_the_batch() {
    for turn_deadline in [false, true] {
        let tool = TestTool::new(vec![Action::Hold, Action::Success]);
        let mut context = context(CancellationToken::new());
        if turn_deadline {
            context.deadline =
                Some((tokio::time::Instant::now() + Duration::from_secs(1)).into_std());
        } else {
            context.tool_timeout = Duration::from_secs(1);
        }
        let report = run(
            config(tool.clone(), None),
            calls(2),
            context,
            Arc::new(TestOwner::default()),
        )
        .await;
        if turn_deadline {
            assert_eq!(report.stop, Some(ToolBatchStop::Deadline));
            assert!(
                report
                    .results
                    .iter()
                    .all(|r| r.outcome == ToolResultOutcome::Cancelled)
            );
            assert_eq!(tool.calls.lock().unwrap().len(), 1);
        } else {
            assert!(report.stop.is_none());
            assert_eq!(report.results[0].outcome, ToolResultOutcome::Failed);
            assert_eq!(report.results[1].outcome, ToolResultOutcome::Success);
        }
    }
}

#[tokio::test]
async fn accepted_answer_is_retained_when_cancellation_wins_before_tool_execution() {
    let tool = TestTool::new(vec![Action::Success]);
    let owner = Arc::new(TestOwner::default());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(run(
        config(tool.clone(), Some(approval())),
        calls(1),
        context(cancellation.clone()),
        owner.clone(),
    ));
    owner.waiting.notified().await;
    owner.answer(InteractionAnswer::Approval(ApprovalDecision::AllowOnce));
    cancellation.cancel();
    let report = task.await.unwrap();
    assert_eq!(report.stop, Some(ToolBatchStop::Cancelled));
    assert_eq!(owner.accepted.lock().unwrap().len(), 1);
    assert_eq!(owner.finished.lock().unwrap().len(), 1);
    assert!(tool.calls.lock().unwrap().is_empty());
}
