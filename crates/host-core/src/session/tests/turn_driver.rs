//! Deterministic races at the Session's direct request boundary.
use super::*;
use minicore_host_runtime::{
    model::ModelMessage,
    prompt::{PromptFuture, PromptProvider, PromptRequest},
};
use std::time::Duration;

struct GatedPrompt {
    calls: AtomicUsize,
    entered: Semaphore,
    release: Semaphore,
    boundaries: Mutex<Vec<(u32, tokio::time::Instant)>>,
}
impl PromptProvider for GatedPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.boundaries
                .lock()
                .unwrap()
                .push((request.request_index, request.deadline));
            if call == 0 {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            DefaultPromptProvider::new(None).prepare(request).await
        })
    }
}
struct CapturingModel {
    inner: Arc<FixtureModel>,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Model for CapturingModel {
    fn descriptor(&self) -> &ModelDescriptor {
        self.inner.descriptor()
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        self.requests.lock().unwrap().push(request.clone());
        self.inner.start(request, context)
    }
}
struct PreparingResources {
    model: Arc<CapturingModel>,
    prompt: Arc<dyn PromptProvider>,
}
impl ResourceResolver for PreparingResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    ToolSet::default(),
                    None,
                    self.prompt.clone(),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
fn fixture() -> (Fixture, Arc<CapturingModel>, Arc<GatedPrompt>) {
    let model = Arc::new(CapturingModel {
        inner: model(vec![final_events("done")]),
        requests: Mutex::new(Vec::new()),
    });
    let prompt = Arc::new(GatedPrompt {
        calls: AtomicUsize::new(0),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
        boundaries: Mutex::new(Vec::new()),
    });
    let resources = Arc::new(PreparingResources {
        model: model.clone(),
        prompt: prompt.clone(),
    });
    (setup(resources, |inner| inner), model, prompt)
}
async fn entered(prompt: &GatedPrompt) {
    tokio::time::timeout(Duration::from_secs(5), prompt.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}
fn input(fixture: &Fixture, mode: MessageMode) -> MessageId {
    let message_id = MessageId::new().unwrap();
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            MessageInput {
                message_id: message_id.clone(),
                mode,
                text: "same text".into(),
                in_reply_to: None,
            },
        )
        .unwrap();
    message_id
}

#[tokio::test]
async fn steering_during_preparation_rebuilds_request_with_distinct_identity_and_same_deadline() {
    let (fixture, model, prompt) = fixture();
    let first = input(&fixture, MessageMode::FollowUp);
    entered(&prompt).await;
    let second = input(&fixture, MessageMode::Steer);
    prompt.release.add_permits(1);
    until(&fixture, |state| state.completed.len() == 1).await;
    let boundaries = prompt.boundaries.lock().unwrap();
    assert_eq!(boundaries.len(), 2);
    assert_eq!(boundaries[0], boundaries[1]);
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let messages = requests[0]
        .messages()
        .iter()
        .filter_map(|message| match message {
            ModelMessage::User(text) => Some(text.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].contains(first.as_str()));
    assert!(messages[1].contains(second.as_str()));
    assert!(
        messages
            .iter()
            .all(|message| message.ends_with("same text"))
    );
    let saved = records(&fixture);
    let ids = saved
        .iter()
        .filter_map(|record| match &record.entry {
            HistoryEntry::Input { message_id, .. } => Some(message_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![&first, &second]);
}

#[tokio::test]
async fn cancellation_after_binding_before_model_keeps_each_input_once_without_model_call() {
    let (fixture, model, prompt) = fixture();
    let message_id = input(&fixture, MessageMode::FollowUp);
    entered(&prompt).await;
    let turn_id = fixture
        .session
        .state
        .lock()
        .unwrap()
        .active
        .as_ref()
        .unwrap()
        .info
        .id
        .clone();
    let (cancel, _) = fixture
        .session
        .cancel(&fixture.connection.caller(), Some(&turn_id), true, None)
        .unwrap();
    assert_eq!(cancel.status, CancelStatus::Claimed);
    until(&fixture, |state| state.completed.len() == 1).await;
    assert!(model.requests.lock().unwrap().is_empty());
    let result = fixture
        .session
        .turn_result(&fixture.connection.caller(), &turn_id)
        .unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(result.info.requests, 0);
    assert!(matches!(&result.history[..], [HistoryItem::User(_)]));
    let saved = records(&fixture);
    assert!(
        matches!(&saved[..], [HistoryRecord { entry: HistoryEntry::Input { message_id: id, .. }, .. },
        HistoryRecord { entry: HistoryEntry::TurnFinished { outcome: TurnOutcome::Cancelled, .. }, .. }] if id == &message_id)
    );
}

#[tokio::test]
async fn invalid_no_tool_finish_preserves_upstream_fault_and_actual_output() {
    for reason in [ModelFinishReason::Unknown, ModelFinishReason::ToolCalls] {
        let fixture = setup(
            Arc::new(Resources {
                model: model(vec![vec![
                    ModelEvent::text_delta("actual complete content"),
                    ModelEvent::Finish { reason },
                ]]),
                tools: ToolSet::default(),
            }),
            |inner| inner,
        );
        send(&fixture, "run malformed response");
        until(&fixture, |state| state.completed.len() == 1).await;
        let state = fixture.session.state.lock().unwrap();
        let result = state.completed.values().next().unwrap();
        assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
        assert_eq!(
            result.info.fault.as_ref().unwrap().kind(),
            Some(FaultKind::TurnModelUpstreamUnclassified)
        );
        if reason == ModelFinishReason::Unknown {
            assert!(
                matches!(&result.history[..], [HistoryItem::User(_), HistoryItem::Assistant(value)] if value.finish_reason == reason)
            );
            assert!(result.partials.is_empty());
        } else {
            // The shared assembler rejects ToolCalls without any calls before
            // producing a complete response; preserve its genuine partial.
            assert!(matches!(&result.history[..], [HistoryItem::User(_)]));
            assert_eq!(result.partials.len(), 1);
        }
        assert_eq!(result.info.persistence, PersistenceState::Persisted);
    }
}

struct PanickingPrompt {
    on_poll: bool,
    children: Mutex<Vec<CancellationToken>>,
}
impl PromptProvider for PanickingPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        self.children
            .lock()
            .unwrap()
            .push(request.cancellation.clone());
        assert!(self.on_poll, "declared prompt construction panic");
        Box::pin(async { panic!("declared prompt polling panic") })
    }
}
#[tokio::test]
async fn prompt_panics_cancel_retained_child_and_do_not_start_model() {
    for on_poll in [false, true] {
        let model = Arc::new(CapturingModel {
            inner: model(vec![]),
            requests: Mutex::new(Vec::new()),
        });
        let prompt = Arc::new(PanickingPrompt {
            on_poll,
            children: Mutex::new(Vec::new()),
        });
        let fixture = setup(
            Arc::new(PreparingResources {
                model: model.clone(),
                prompt: prompt.clone(),
            }),
            |inner| inner,
        );
        send(&fixture, "panic boundary fixture");
        until(&fixture, |state| state.completed.len() == 1).await;
        assert!(model.requests.lock().unwrap().is_empty());
        let children = prompt.children.lock().unwrap();
        assert_eq!(children.len(), 1);
        assert!(children[0].is_cancelled());
        let state = fixture.session.state.lock().unwrap();
        let result = state.completed.values().next().unwrap();
        assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
        assert_eq!(
            result.info.fault.as_ref().unwrap().kind(),
            Some(FaultKind::TurnInternalUnexpected)
        );
        assert_eq!(result.info.persistence, PersistenceState::Persisted);
    }
}
