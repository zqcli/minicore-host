use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use minicore_host_runtime::error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary};
use minicore_host_runtime::model::{
    AssistantPart, DeliveryState, Model, ModelCallContext, ModelCallEvent, ModelCallParams,
    ModelDescriptor, ModelDriver, ModelDriverConfig, ModelError, ModelErrorKind, ModelEvent,
    ModelFinishReason, ModelLimits, ModelMessage, ModelPartial, ModelRequest, ModelStartFuture,
    ModelStream, ProviderReplay, ReasoningPreference, Usage,
};
use minicore_host_runtime::tools::ToolSpec;
use minicore_host_runtime::{DeadlineSource, LoopId};
use serde_json::json;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn interrupted() -> ModelError {
    ModelError::started(ModelErrorKind::StreamInterrupted, diagnostic())
}
fn diagnostic() -> DiagnosticSummary {
    DiagnosticSummary::new(
        DiagnosticCode::ModelUnavailable,
        DiagnosticCategory::Model,
        minicore_host_runtime::Text::new("test stream failed"),
        false,
    )
    .unwrap()
}

enum Behavior {
    Events(Vec<Result<ModelEvent, ModelError>>),
    Hold(Vec<ModelEvent>, Arc<Notify>),
    StartError(ModelError),
}
struct TestModel {
    descriptor: ModelDescriptor,
    behavior: Mutex<VecDeque<Behavior>>,
    starts: AtomicUsize,
}
impl TestModel {
    fn new(behavior: Vec<Behavior>) -> Self {
        Self {
            descriptor: ModelDescriptor::new(
                "test/model".parse().unwrap(),
                128_000,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
            behavior: Mutex::new(behavior.into()),
            starts: AtomicUsize::new(0),
        }
    }
}
impl Model for TestModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let behavior = self
            .behavior
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted attempt");
        Box::pin(async move {
            match behavior {
                Behavior::StartError(error) => Err(error),
                Behavior::Events(events) => Ok(Box::pin(stream::iter(events)) as ModelStream),
                Behavior::Hold(events, drained) => Ok(Box::pin(
                    stream::iter(events.into_iter().map(Ok)).chain(stream::poll_fn(move |_| {
                        drained.notify_one();
                        std::task::Poll::Pending
                    })),
                ) as ModelStream),
            }
        })
    }
}
fn driver(model: &dyn Model) -> ModelDriver<'_> {
    ModelDriver::new(
        model,
        ModelDriverConfig::new(Duration::from_secs(60), 2, Duration::from_millis(100)),
    )
    .unwrap()
}
fn request() -> ModelRequest {
    ModelRequest::new(
        vec![ModelMessage::user("request").unwrap()],
        vec![ToolSpec::new("echo".parse().unwrap(), "echo", json!({"type":"object"})).unwrap()],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap()
}
fn context(index: u32, cancellation: CancellationToken, after: Duration) -> ModelCallParams {
    ModelCallParams {
        loop_id: LoopId::new().unwrap(),
        request_index: index,
        cancellation,
        deadline: Some((tokio::time::Instant::now() + after).into_std()),
    }
}
fn text_success() -> Behavior {
    Behavior::Events(vec![
        Ok(ModelEvent::text_delta("complete")),
        Ok(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        }),
    ])
}

#[tokio::test]
async fn failure_returns_assembled_partial_and_unfinished_tool_arguments_without_execution() {
    let usage = Usage::new(5, 7, 2);
    let model = TestModel::new(vec![Behavior::Events(vec![
        Ok(ModelEvent::reasoning_delta("think\r")),
        Ok(ModelEvent::text_delta("Hello\r")),
        Ok(ModelEvent::ToolCallStart {
            tool_call_id: "complete-call".parse().unwrap(),
            tool_name: "echo".parse().unwrap(),
        }),
        Ok(ModelEvent::tool_call_arguments_delta(
            "complete-call".parse().unwrap(),
            "{}",
        )),
        Ok(ModelEvent::ToolCallEnd {
            tool_call_id: "complete-call".parse().unwrap(),
        }),
        Ok(ModelEvent::ToolCallStart {
            tool_call_id: "unfinished".parse().unwrap(),
            tool_name: "echo".parse().unwrap(),
        }),
        Ok(ModelEvent::tool_call_arguments_delta(
            "unfinished".parse().unwrap(),
            "{\"secret\":",
        )),
        Ok(ModelEvent::text_delta("\nworld")),
        Ok(ModelEvent::Usage { usage }),
        Ok(ModelEvent::ProviderReplay {
            replay: ProviderReplay::new("test/v1", json!({"opaque":"retained"})).unwrap(),
        }),
        Err(interrupted()),
    ])]);
    let report = driver(&model)
        .call(
            request(),
            context(4, CancellationToken::new(), Duration::from_secs(120)),
            &mut |_| {},
        )
        .await;
    assert_eq!(report.attempts, 1);
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::StreamInterrupted);
    assert_eq!(failure.usage, Some(usage));
    let partial = failure.partial.unwrap();
    assert_eq!(partial.request_index, 4);
    assert!(
        matches!(&partial.content[1], AssistantPart::Text(text) if text.as_ref() == "Hello\nworld")
    );
    assert!(
        matches!(&partial.content[2], AssistantPart::ToolCall(call) if call.tool_call_id().as_str() == "complete-call")
    );
    assert_eq!(
        partial.incomplete_tool_calls[0].arguments_text,
        "{\"secret\":"
    );
    assert_eq!(partial.incomplete_tool_calls[0].part_index, 3);
    assert_eq!(partial.usage, Some(usage));
    assert!(partial.provider_replay.is_some());
    assert_eq!(
        serde_json::from_slice::<ModelPartial>(&serde_json::to_vec(partial.as_ref()).unwrap())
            .unwrap(),
        *partial
    );
    assert!(!format!("{partial:?}").contains("secret"));
}

#[tokio::test]
async fn cancellation_keeps_partial_without_an_event_subscriber() {
    let drained = Arc::new(Notify::new());
    let model = Arc::new(TestModel::new(vec![Behavior::Hold(
        (0..100).map(|_| ModelEvent::text_delta("delta ")).collect(),
        drained.clone(),
    )]));
    let cancellation = CancellationToken::new();
    let task_cancel = cancellation.clone();
    let task = tokio::spawn(async move {
        driver(model.as_ref())
            .call(
                request(),
                context(0, task_cancel, Duration::from_secs(120)),
                &mut |_| {},
            )
            .await
    });
    drained.notified().await;
    cancellation.cancel();
    let failure = task.await.unwrap().outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Cancelled);
    assert!(
        matches!(&failure.partial.unwrap().content[0], AssistantPart::Text(text) if text.as_ref() == "delta ".repeat(100))
    );
}

#[tokio::test(start_paused = true)]
async fn deadline_keeps_partial_from_actual_assembly() {
    let model = TestModel::new(vec![Behavior::Hold(
        vec![ModelEvent::text_delta("partial")],
        Arc::new(Notify::new()),
    )]);
    let report = driver(&model)
        .call(
            request(),
            context(0, CancellationToken::new(), Duration::from_secs(2)),
            &mut |_| {},
        )
        .await;
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Turn));
    assert_eq!(failure.error().kind(), ModelErrorKind::Timeout);
    assert!(failure.partial.is_some());
}

#[tokio::test]
async fn cancelled_empty_model_has_no_synthetic_partial() {
    let model = TestModel::new(vec![]);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let report = driver(&model)
        .call(
            request(),
            context(0, cancellation, Duration::from_secs(120)),
            &mut |_| {},
        )
        .await;
    assert_eq!(report.attempts, 0);
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().delivery(), DeliveryState::NotStarted);
    assert!(failure.partial.is_none());
    assert!(failure.usage.is_none());
}

#[tokio::test]
async fn incomplete_second_request_has_original_index_without_reclassifying_first_response() {
    let model = TestModel::new(vec![
        text_success(),
        Behavior::Events(vec![Ok(ModelEvent::text_delta("unfinished"))]),
    ]);
    let driver = driver(&model);
    let first = driver
        .call(
            request(),
            context(0, CancellationToken::new(), Duration::from_secs(120)),
            &mut |_| {},
        )
        .await
        .outcome
        .unwrap();
    let second = driver
        .call(
            request(),
            context(1, CancellationToken::new(), Duration::from_secs(120)),
            &mut |_| {},
        )
        .await
        .outcome
        .unwrap_err();
    assert_eq!(first.finish_reason(), ModelFinishReason::Stop);
    assert_eq!(first.usage(), None);
    assert_eq!(second.partial.unwrap().request_index, 1);
}

#[tokio::test]
async fn usage_without_content_survives_failure_and_prevents_retry() {
    let usage = Usage::new(8, 0, 0);
    let model = TestModel::new(vec![
        Behavior::Events(vec![
            Ok(ModelEvent::Usage { usage }),
            Err(ModelError::not_started(
                ModelErrorKind::RateLimited,
                None,
                diagnostic(),
            )),
        ]),
        text_success(),
    ]);
    let report = driver(&model)
        .call(
            request(),
            context(0, CancellationToken::new(), Duration::from_secs(120)),
            &mut |_| {},
        )
        .await;
    assert_eq!(report.attempts, 1);
    let failure = report.outcome.unwrap_err();
    let partial = failure
        .partial
        .as_ref()
        .expect("observed usage is a real partial response fact");
    assert!(partial.content.is_empty());
    assert!(partial.incomplete_tool_calls.is_empty());
    assert_eq!(partial.usage, Some(usage));
    assert_eq!(failure.usage, Some(usage));
    assert_eq!(failure.error().delivery(), DeliveryState::Started);
    assert_eq!(model.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn duplicate_usage_is_ambiguous_and_not_billed_from_the_last_event() {
    let model = TestModel::new(vec![Behavior::Events(vec![
        Ok(ModelEvent::text_delta("partial")),
        Ok(ModelEvent::Usage {
            usage: Usage::new(1, 2, 3),
        }),
        Ok(ModelEvent::Usage {
            usage: Usage::new(4, 5, 6),
        }),
    ])]);
    let failure = driver(&model)
        .call(
            request(),
            context(0, CancellationToken::new(), Duration::from_secs(120)),
            &mut |_| {},
        )
        .await
        .outcome
        .unwrap_err();
    assert_eq!(
        failure.error().kind(),
        ModelErrorKind::InvalidProviderResponse
    );
    assert_eq!(failure.usage, None);
    assert_eq!(failure.partial.unwrap().usage, None);
}

#[tokio::test(start_paused = true)]
async fn report_retains_attempts_and_retry_wait_when_observer_panics() {
    let model = TestModel::new(vec![
        Behavior::StartError(ModelError::not_started(
            ModelErrorKind::RateLimited,
            Some(Duration::from_secs(2)),
            diagnostic(),
        )),
        text_success(),
    ]);
    let mut events = Vec::new();
    let report = driver(&model)
        .call(
            request(),
            context(0, CancellationToken::new(), Duration::from_secs(120)),
            &mut |event| {
                events.push(event.clone());
                if matches!(event, ModelCallEvent::TextDelta(_)) {
                    panic!("test observer panic");
                }
            },
        )
        .await;
    assert!(report.outcome.is_ok());
    assert_eq!(report.attempts, 2);
    assert_eq!(report.retry_wait_elapsed, Duration::from_secs(2));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ModelCallEvent::RetryWaiting(_)))
    );
}
