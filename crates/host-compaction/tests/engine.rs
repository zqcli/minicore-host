//! Scripted Models implement the real Runtime Model port. They are not live
//! provider evidence and do not replace Runtime request/history validation.
use futures_util::StreamExt;
use minicore_host_compaction::*;
use minicore_host_protocol::CompactionSettings;
use minicore_runtime::{
    AssistantHistory, HistoryItem, HistoryView, LoopId, SummaryHistory, Text, ToolCallId,
    ToolResultHistory, UserHistory, UserInput, UserMessageKind,
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
    model::{
        AssistantPart, Model, ModelCallContext, ModelDescriptor, ModelError, ModelErrorKind,
        ModelEvent, ModelFinishReason, ModelLimits, ModelMessage, ModelRequest, ModelStartFuture,
        ProviderReplay, ReasoningPreference, ToolCall, Usage,
    },
    prompt::{DefaultPromptProvider, PromptProvider, PromptRequest},
    tools::{ToolName, ToolOutput, ToolResultOutcome},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
enum Script {
    Summary(String, Option<Usage>),
    Events(Vec<Result<ModelEvent, ModelError>>),
    HoldStart,
    HoldStream(Vec<ModelEvent>),
}
struct ScriptedModel {
    descriptor: ModelDescriptor,
    script: Script,
    requests: Mutex<Vec<ModelRequest>>,
    contexts: Mutex<Vec<ModelCallContext>>,
    reached: Arc<Notify>,
}
impl ScriptedModel {
    fn new(window: u64, script: Script) -> Self {
        Self {
            descriptor: descriptor(window),
            script,
            requests: Mutex::new(vec![]),
            contexts: Mutex::new(vec![]),
            reached: Arc::new(Notify::new()),
        }
    }
    fn summary(window: u64) -> Self {
        Self::new(
            window,
            Script::Summary("Keep the agreed task.".into(), None),
        )
    }
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        self.requests.lock().unwrap().push(request);
        let mut contexts = self.contexts.lock().unwrap();
        assert!(
            contexts
                .iter()
                .all(|prior| prior.cancellation.is_cancelled()),
            "each prior utility call must be cancelled before the next begins"
        );
        contexts.push(context);
        drop(contexts);
        let reached = self.reached.clone();
        let script = self.script.clone();
        Box::pin(async move {
            let events = match script {
                Script::Summary(text, usage) => {
                    let mut events = vec![Ok(ModelEvent::text_delta(text))];
                    if let Some(usage) = usage {
                        events.push(Ok(ModelEvent::Usage { usage }));
                    }
                    events.push(Ok(ModelEvent::Finish {
                        reason: ModelFinishReason::Stop,
                    }));
                    events
                }
                Script::Events(events) => events,
                Script::HoldStart => {
                    reached.notify_one();
                    return std::future::pending().await;
                }
                Script::HoldStream(events) => {
                    let stream = futures_util::stream::iter(events.into_iter().map(Ok)).chain(
                        futures_util::stream::once(async move {
                            // Notification occurs only after the consumer has
                            // processed every preceding event, including usage.
                            reached.notify_one();
                            std::future::pending().await
                        }),
                    );
                    return Ok(Box::pin(stream) as minicore_runtime::model::ModelStream);
                }
            };
            Ok(Box::pin(futures_util::stream::iter(events))
                as minicore_runtime::model::ModelStream)
        })
    }
}
fn descriptor(window: u64) -> ModelDescriptor {
    ModelDescriptor::new(
        "scripted/summary".parse().unwrap(),
        window,
        BTreeSet::from([ReasoningPreference::Auto]),
        true,
    )
    .unwrap()
}
fn context() -> ModelCallContext {
    ModelCallContext::new(
        LoopId::new().unwrap(),
        7,
        CancellationToken::new(),
        Instant::now() + Duration::from_secs(5),
    )
}
fn user(text: impl AsRef<str>) -> HistoryItem {
    HistoryItem::User(UserHistory {
        loop_id: LoopId::new().unwrap(),
        kind: UserMessageKind::Prompt,
        input: UserInput::text(text).unwrap(),
    })
}
fn assistant(loop_id: LoopId, content: Vec<AssistantPart>) -> HistoryItem {
    HistoryItem::Assistant(AssistantHistory {
        loop_id,
        request_index: 0,
        model: "scripted/summary".parse().unwrap(),
        reasoning: ReasoningPreference::Auto,
        content,
        provider_replay: None,
        finish_reason: ModelFinishReason::Stop,
        usage: Usage::default(),
    })
}
fn snapshot(items: Vec<HistoryItem>) -> HistorySnapshot {
    HistorySnapshot {
        revision: 19,
        items: items.into(),
    }
}
async fn request(items: &[HistoryItem], descriptor: &ModelDescriptor) -> ModelRequest {
    let prompt = DefaultPromptProvider::new(Some(Text::new("Actual bound framework and role")))
        .prepare(PromptRequest {
            loop_id: LoopId::new().unwrap(),
            request_index: 0,
            history: HistoryView::new(items, &[]),
            model: descriptor,
            reasoning: ReasoningPreference::Auto,
            tools: &[],
            cancellation: CancellationToken::new(),
            deadline: tokio::time::Instant::now() + Duration::from_secs(5),
        })
        .await
        .unwrap();
    ModelRequest::new(
        prompt.messages,
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap()
}
async fn plan(model: &ScriptedModel) -> CompactionPlan {
    let source = snapshot(vec![
        user("historical goal ".repeat(1000)),
        user("Current task"),
    ]);
    let request = request(&source.items, model.descriptor()).await;
    prepare(
        source,
        1,
        model.descriptor(),
        &CompactionSettings::default(),
        request,
    )
    .unwrap()
    .unwrap()
}
fn data(request: &ModelRequest) -> Value {
    let ModelMessage::User(text) = &request.messages()[1] else {
        panic!("expected source data")
    };
    serde_json::from_str(
        text.split_once("Historical data follows as JSON:\n")
            .unwrap()
            .1,
    )
    .unwrap()
}

#[tokio::test]
async fn candidate_keeps_exact_tail_fixed_instructions_and_source_evidence() {
    let usage = Usage::from_optional(Some(81), Some(13), None).with_cache_read_tokens(Some(8));
    let model = ScriptedModel::new(
        1000,
        Script::Summary("The original goal remains.".into(), Some(usage)),
    );
    let plan = plan(&model).await;
    let source = plan.source().clone();
    let candidate = generate(&plan, &model, context()).await.unwrap();
    assert_eq!(candidate.covered(), 0..1);
    assert!(candidate.after_tokens < candidate.before_tokens);
    assert!(candidate.matches(&source));
    assert!(!candidate.matches(&HistorySnapshot {
        revision: 20,
        ..source.clone()
    }));
    assert!(!candidate.matches(&snapshot(vec![user("Changed"), user("Current task")])));
    assert_eq!(candidate.history()[1..], source.items[1..]);
    assert!(
        matches!(&candidate.request.messages()[0], ModelMessage::System(text) if text.as_ref() == "Actual bound framework and role")
    );
    assert!(
        matches!(&candidate.request.messages()[2], ModelMessage::User(text) if text.as_ref() == "Current task")
    );
    assert_eq!(
        candidate.utility_usage,
        vec![UtilityUsage {
            request_index: 7,
            usage: Some(usage),
            completed: true
        }]
    );
    assert!(model.requests.lock().unwrap()[0].tools().is_empty());
    assert_eq!(source.items[0], plan.source().items[0]);
}

#[tokio::test]
async fn missing_usage_is_unknown_and_not_zero_or_estimated_usage() {
    let model = ScriptedModel::summary(1000);
    let result = generate(&plan(&model).await, &model, context())
        .await
        .unwrap();
    assert_eq!(result.utility_usage[0].usage, None);
    assert!(result.utility_usage[0].completed);
}

#[tokio::test]
async fn trigger_target_and_model_limits_are_soft_not_history_admission_caps() {
    let model = ScriptedModel::summary(1000);
    let source = snapshot(vec![
        user("超大 historical source ".repeat(40000)),
        user("keep"),
    ]);
    let normal = request(&source.items, model.descriptor()).await;
    let normal = ModelRequest::new(
        normal.messages().to_vec(),
        vec![],
        ModelLimits::new(Some(800), Some(200)).unwrap(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let settings = CompactionSettings {
        enabled: true,
        trigger_percent: 90,
        target_percent: 40,
    };
    let prepared = prepare(source, 1, model.descriptor(), &settings, normal)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.estimate().context_window, 800);
    assert_eq!(prepared.estimate().trigger_tokens, 720);
    assert_eq!(prepared.estimate().target_tokens, 320);
    assert!(prepared.estimate().automatic_due);
    generate(&prepared, &model, context()).await.unwrap();
    let requests = model.requests.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "indivisible huge user item is not split or dropped"
    );
    assert!(estimate_request_tokens(&requests[0]).unwrap() > 800);
    assert_eq!(requests[0].limits().max_output_tokens(), Some(200));
    assert!(
        data(&requests[0])
            .to_string()
            .contains(&"超大 historical source ".repeat(40000))
    );
}

#[tokio::test]
async fn manual_plan_does_not_depend_on_automatic_trigger_or_enabled_flag() {
    let model = ScriptedModel::summary(10000);
    let source = snapshot(vec![user("old ".repeat(4000)), user("keep")]);
    let normal = request(&source.items, model.descriptor()).await;
    for enabled in [false, true] {
        let settings = CompactionSettings {
            enabled,
            ..CompactionSettings::default()
        };
        let prepared = prepare(
            source.clone(),
            1,
            model.descriptor(),
            &settings,
            normal.clone(),
        )
        .unwrap()
        .unwrap();
        assert!(!prepared.estimate().automatic_due);
        assert!(generate(&prepared, &model, context()).await.is_ok());
    }
}

fn tool_exchange() -> Vec<HistoryItem> {
    let loop_id = LoopId::new().unwrap();
    let call = ToolCall::new(
        "call_1".parse().unwrap(),
        "read".parse().unwrap(),
        json!({"path":"原始文件.txt"}),
        0,
    )
    .unwrap();
    let response = assistant(
        loop_id,
        vec![
            AssistantPart::Text("read context".into()),
            AssistantPart::ToolCall(call),
        ],
    );
    let result = HistoryItem::ToolResult(ToolResultHistory {
        loop_id,
        request_index: 0,
        call_id: "call_1".parse().unwrap(),
        tool_name: "read".parse().unwrap(),
        outcome: ToolResultOutcome::Success,
        output: ToolOutput::new("recorded result").unwrap(),
    });
    vec![response, result]
}

#[tokio::test]
async fn protected_boundary_inside_tool_exchange_rounds_back_and_keeps_pair_exact() {
    let model = ScriptedModel::summary(1000);
    let mut items = vec![user("older ".repeat(2000))];
    items.extend(tool_exchange());
    items.push(user("current"));
    let source = snapshot(items);
    let normal = request(&source.items, model.descriptor()).await;
    let prepared = prepare(
        source.clone(),
        2,
        model.descriptor(),
        &CompactionSettings::default(),
        normal,
    )
    .unwrap()
    .unwrap();
    assert_eq!(prepared.covered(), 0..1);
    let candidate = generate(&prepared, &model, context()).await.unwrap();
    assert_eq!(candidate.history()[1..], source.items[1..]);
    assert!(matches!(
        candidate.request.messages()[2],
        ModelMessage::Assistant(_)
    ));
    assert!(matches!(
        candidate.request.messages()[3],
        ModelMessage::Tool { .. }
    ));
}

#[tokio::test]
async fn complete_missing_and_mixed_calls_are_full_historical_data_not_executions() {
    let model = ScriptedModel::summary(1000);
    let loop_id = LoopId::new().unwrap();
    let complete = ToolCall::new(
        "complete".parse().unwrap(),
        "read".parse().unwrap(),
        json!({"path":"large.txt","exact":"arg ".repeat(3000)}),
        0,
    )
    .unwrap();
    let missing = ToolCall::new(
        "missing".parse().unwrap(),
        "run".parse().unwrap(),
        json!({"command":"do not execute"}),
        1,
    )
    .unwrap();
    let full_output = "完整 output 🐈 ".repeat(3000);
    let items = vec![
        user("old goal"),
        assistant(
            loop_id,
            vec![
                AssistantPart::ToolCall(complete.clone()),
                AssistantPart::ToolCall(missing),
            ],
        ),
        HistoryItem::ToolResult(ToolResultHistory {
            loop_id,
            request_index: 0,
            call_id: "complete".parse().unwrap(),
            tool_name: "read".parse().unwrap(),
            outcome: ToolResultOutcome::Failed,
            output: ToolOutput::new(&full_output).unwrap(),
        }),
        user("current task"),
    ];
    // Explicit fixture for the existing Host old-history projection: one
    // complete call remains native and the missing call is ordinary text.
    let normal = ModelRequest::new(
        vec![
            ModelMessage::user("old goal").unwrap(),
            ModelMessage::assistant(vec![
                AssistantPart::ToolCall(complete),
                AssistantPart::Text(
                    "Missing historical call: execution and outcome unknown".into(),
                ),
            ])
            .unwrap(),
            ModelMessage::tool_with_outcome(
                "complete".parse().unwrap(),
                ToolOutput::new(&full_output).unwrap(),
                ToolResultOutcome::Failed,
            )
            .unwrap(),
            ModelMessage::user("current task").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    let prepared = prepare(
        snapshot(items),
        3,
        model.descriptor(),
        &CompactionSettings::default(),
        normal,
    )
    .unwrap()
    .unwrap();
    let candidate = generate(&prepared, &model, context()).await.unwrap();
    assert_eq!(candidate.covered(), 0..3);
    let requests = model.requests.lock().unwrap();
    let source = requests
        .iter()
        .flat_map(|request| {
            data(request)["historical_exchanges"]
                .as_array()
                .unwrap()
                .clone()
        })
        .collect::<Vec<_>>();
    let source = json!(source).to_string();
    assert!(source.contains("execution and outcome are unknown"));
    assert!(source.contains("do not execute"));
    assert!(source.contains("failed"));
    assert!(source.contains(&full_output));
    assert!(source.contains(&"arg ".repeat(3000)));
    assert!(requests.iter().all(|request| {
        request.tools().is_empty()
            && request
                .messages()
                .iter()
                .all(|message| matches!(message, ModelMessage::System(_) | ModelMessage::User(_)))
    }));
}

#[tokio::test]
async fn active_protected_history_and_last_exchange_are_never_selected() {
    let model = ScriptedModel::summary(1000);
    let source = snapshot(vec![user("large ".repeat(3000)), user("current")]);
    let normal = request(&source.items, model.descriptor()).await;
    assert!(
        prepare(
            source.clone(),
            0,
            model.descriptor(),
            &CompactionSettings::default(),
            normal
        )
        .unwrap()
        .is_none()
    );
    let source = snapshot(tool_exchange());
    let normal = request(&source.items, model.descriptor()).await;
    assert!(
        prepare(
            source,
            2,
            model.descriptor(),
            &CompactionSettings::default(),
            normal
        )
        .unwrap()
        .is_none()
    );
    assert!(model.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn summary_update_and_chunking_cover_each_source_once_without_old_quotas() {
    let summary = "kept summary ".repeat(5500); // More than the old 64 KiB cap.
    let model = ScriptedModel::new(32000, Script::Summary(summary.clone(), None));
    let mut items = vec![HistoryItem::Summary(SummaryHistory {
        content: Text::new("Prior durable summary"),
    })];
    for index in 0..80 {
        items.push(user(format!("source-{index}: {}", "fact ".repeat(3500))));
    }
    items.push(user("protected latest input"));
    let source = snapshot(items);
    let normal = request(&source.items, model.descriptor()).await;
    let prepared = prepare(
        source.clone(),
        81,
        model.descriptor(),
        &CompactionSettings::default(),
        normal,
    )
    .unwrap()
    .unwrap();
    let candidate = generate(&prepared, &model, context()).await.unwrap();
    assert!(candidate.summary.content.byte_len() > 65536);
    assert!(candidate.covered().end > 64);
    let calls = model.requests.lock().unwrap();
    assert!(
        calls.len() > 1,
        "real context target requires several utility calls"
    );
    let mut covered = Vec::new();
    for (index, call) in calls.iter().enumerate() {
        let data = data(call);
        if index == 0 {
            assert!(data["previous_summary"].is_null());
        } else {
            assert_eq!(data["previous_summary"], summary);
        }
        covered.extend(
            data["historical_exchanges"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
    }
    assert_eq!(covered.len(), prepared.covered().end);
    assert!(covered[0].to_string().contains("Prior durable summary"));
    for index in 0..prepared.covered().end - 1 {
        assert!(
            covered[index + 1]
                .to_string()
                .contains(&format!("source-{index}:"))
        );
    }
    assert_eq!(
        candidate.history()[1..],
        source.items[prepared.covered().end..]
    );
    assert_eq!(candidate.utility_usage.len(), calls.len());
    for (index, entry) in candidate.utility_usage.iter().enumerate() {
        assert_eq!(entry.request_index, 7 + index as u32);
        assert_eq!(entry.usage, None);
    }
}

#[tokio::test]
async fn only_complete_valid_nonempty_stop_text_becomes_a_summary() {
    let stop = || {
        Ok(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
    };
    let replay = ProviderReplay::new("scripted.test.v1", json!({"marker":true})).unwrap();
    let cases = vec![
        vec![Ok(ModelEvent::text_delta("partial"))],
        vec![Ok(ModelEvent::text_delta(" \n\t")), stop()],
        vec![Ok(ModelEvent::reasoning_delta("reasoning only")), stop()],
        vec![Ok(ModelEvent::text_delta("\0\x01")), stop()],
        vec![Ok(ModelEvent::text_delta("text")), stop(), stop()],
        vec![
            Ok(ModelEvent::text_delta("text")),
            stop(),
            Ok(ModelEvent::text_delta("trailer")),
        ],
        vec![
            Ok(ModelEvent::text_delta("text")),
            Ok(ModelEvent::Usage {
                usage: Usage::new(1, 2, 0),
            }),
            Ok(ModelEvent::Usage {
                usage: Usage::new(1, 2, 0),
            }),
            stop(),
        ],
        vec![
            Ok(ModelEvent::ProviderReplay {
                replay: replay.clone(),
            }),
            Ok(ModelEvent::ProviderReplay { replay }),
            Ok(ModelEvent::text_delta("text")),
            stop(),
        ],
    ];
    for events in cases {
        let model = ScriptedModel::new(1000, Script::Events(events));
        let error = generate(&plan(&model).await, &model, context())
            .await
            .unwrap_err();
        assert_eq!(error.error, CompactionError::InvalidResponse);
        assert!(!error.utility_usage[0].completed);
    }
    for reason in [
        ModelFinishReason::Length,
        ModelFinishReason::ContentFiltered,
        ModelFinishReason::Refused,
        ModelFinishReason::ToolCalls,
        ModelFinishReason::Unknown,
    ] {
        let model = ScriptedModel::new(
            1000,
            Script::Events(vec![
                Ok(ModelEvent::text_delta("not a completed summary")),
                Ok(ModelEvent::Finish { reason }),
            ]),
        );
        assert_eq!(
            generate(&plan(&model).await, &model, context())
                .await
                .unwrap_err()
                .error,
            CompactionError::InvalidResponse
        );
    }
}

#[tokio::test]
async fn unexpected_tool_output_is_rejected_and_never_executed() {
    let model = ScriptedModel::new(
        1000,
        Script::Events(vec![Ok(ModelEvent::ToolCallStart {
            tool_call_id: ToolCallId::new("unexpected-call").unwrap(),
            tool_name: "run".parse::<ToolName>().unwrap(),
        })]),
    );
    let error = generate(&plan(&model).await, &model, context())
        .await
        .unwrap_err();
    assert_eq!(error.error, CompactionError::UnexpectedToolCall);
    assert!(
        model.contexts.lock().unwrap()[0]
            .cancellation
            .is_cancelled()
    );
}

#[tokio::test]
async fn cancelled_before_generation_never_starts_model() {
    let model = ScriptedModel::summary(1000);
    let context = context();
    context.cancellation.cancel();
    let error = generate(&plan(&model).await, &model, context)
        .await
        .unwrap_err();
    assert_eq!(error.error, CompactionError::Cancelled);
    assert!(error.utility_usage.is_empty());
    assert!(model.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_reaches_pending_actual_model_start() {
    let model = Arc::new(ScriptedModel::new(1000, Script::HoldStart));
    let plan = plan(&model).await;
    let context = context();
    let cancellation = context.cancellation.clone();
    let worker_model = model.clone();
    let worker = tokio::spawn(async move { generate(&plan, worker_model.as_ref(), context).await });
    model.reached.notified().await;
    cancellation.cancel();
    let error = worker.await.unwrap().unwrap_err();
    assert_eq!(error.error, CompactionError::Cancelled);
    assert!(
        model.contexts.lock().unwrap()[0]
            .cancellation
            .is_cancelled()
    );
    assert!(!error.utility_usage[0].completed);
}

#[tokio::test]
async fn deadline_cancels_actual_stream_and_retains_already_reported_usage() {
    let usage = Usage::from_optional(Some(500), None, None);
    let model = Arc::new(ScriptedModel::new(
        1000,
        Script::HoldStream(vec![
            ModelEvent::Usage { usage },
            ModelEvent::text_delta("unfinished"),
        ]),
    ));
    let plan = plan(&model).await;
    let mut context = context();
    context.deadline = Instant::now() + Duration::from_millis(100);
    let parent = context.cancellation.clone();
    let worker_model = model.clone();
    let worker = tokio::spawn(async move { generate(&plan, worker_model.as_ref(), context).await });
    model.reached.notified().await;
    let error = worker.await.unwrap().unwrap_err();
    assert_eq!(error.error, CompactionError::Timeout);
    assert_eq!(error.utility_usage[0].usage, Some(usage));
    assert!(!error.utility_usage[0].completed);
    assert!(
        model.contexts.lock().unwrap()[0]
            .cancellation
            .is_cancelled()
    );
    assert!(!parent.is_cancelled());
}

#[tokio::test]
async fn cancellation_retains_stream_usage_and_no_partial_summary_is_committable() {
    let usage = Usage::new(99, 11, 3);
    let model = Arc::new(ScriptedModel::new(
        1000,
        Script::HoldStream(vec![
            ModelEvent::Usage { usage },
            ModelEvent::text_delta("unfinished"),
        ]),
    ));
    let plan = plan(&model).await;
    let context = context();
    let cancellation = context.cancellation.clone();
    let worker_model = model.clone();
    let worker = tokio::spawn(async move { generate(&plan, worker_model.as_ref(), context).await });
    model.reached.notified().await;
    cancellation.cancel();
    let error = worker.await.unwrap().unwrap_err();
    assert_eq!(error.error, CompactionError::Cancelled);
    assert_eq!(error.utility_usage[0].usage, Some(usage));
    assert!(!error.utility_usage[0].completed);
}

#[tokio::test]
async fn provider_failure_preserves_real_error_usage_and_never_retries() {
    let actual = ModelError::not_started(
        ModelErrorKind::RateLimited,
        Some(Duration::from_secs(1)),
        DiagnosticSummary::new(
            DiagnosticCode::ModelUnavailable,
            DiagnosticCategory::Model,
            Text::new("scripted rejected request"),
            true,
        )
        .unwrap(),
    );
    let usage = Usage::new(12, 8, 0);
    let model = ScriptedModel::new(
        1000,
        Script::Events(vec![Ok(ModelEvent::Usage { usage }), Err(actual.clone())]),
    );
    let error = generate(&plan(&model).await, &model, context())
        .await
        .unwrap_err();
    assert_eq!(error.error, CompactionError::Model(actual));
    assert_eq!(error.utility_usage[0].usage, Some(usage));
    assert_eq!(model.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn expanding_summary_is_no_progress_not_a_successful_compaction() {
    let model = ScriptedModel::new(1000, Script::Summary("expanded ".repeat(5000), None));
    let error = generate(&plan(&model).await, &model, context())
        .await
        .unwrap_err();
    assert_eq!(error.error, CompactionError::NoProgress);
    assert!(
        error.utility_usage[0].completed,
        "generation completed but did not compact"
    );
}

#[tokio::test]
async fn changed_model_capabilities_fail_without_starting_utility() {
    let original = ScriptedModel::summary(1000);
    let plan = plan(&original).await;
    let changed = ScriptedModel::summary(2000);
    let error = generate(&plan, &changed, context()).await.unwrap_err();
    assert_eq!(error.error, CompactionError::ModelChanged);
    assert!(changed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn source_exchange_identity_is_checked_beyond_wire_call_ids() {
    let model = ScriptedModel::summary(1000);
    for mismatch in 0..4 {
        let mut items = vec![user("older ".repeat(1000))];
        items.extend(tool_exchange());
        let HistoryItem::ToolResult(result) = &mut items[2] else {
            unreachable!()
        };
        match mismatch {
            0 => result.loop_id = LoopId::new().unwrap(),
            1 => result.request_index = 1,
            2 => result.tool_name = "different_tool".parse().unwrap(),
            _ => {}
        }
        if mismatch == 3 {
            items.insert(3, items[2].clone());
        }
        items.push(user("current"));
        let source = snapshot(items);
        // The model request cannot express Runtime loop/request/name identity
        // on a tool-result message. Planning must check the native source too.
        let normal_items = [
            user("older"),
            tool_exchange()[0].clone(),
            tool_exchange()[1].clone(),
            user("current"),
        ];
        let mut normal = request(&normal_items, model.descriptor()).await;
        if mismatch == 3 {
            let mut messages = vec![ModelMessage::system("fixed").unwrap()];
            messages.extend_from_slice(normal.messages());
            normal = ModelRequest::new(
                messages,
                vec![],
                ModelLimits::default(),
                ReasoningPreference::Auto,
            )
            .unwrap();
        }
        assert_eq!(
            prepare(
                source,
                3,
                model.descriptor(),
                &CompactionSettings::default(),
                normal
            )
            .unwrap_err(),
            CompactionError::InvalidHistory
        );
    }
}

#[tokio::test]
async fn estimate_is_available_when_the_history_has_no_compactable_prefix() {
    let model = ScriptedModel::summary(1000);
    let source = snapshot(vec![user("single current input")]);
    let normal = request(&source.items, model.descriptor()).await;
    let estimate =
        estimate_context(model.descriptor(), &CompactionSettings::default(), &normal).unwrap();
    assert!(estimate.input_tokens > 0);
    assert_eq!(estimate.context_window, 1000);
    assert!(
        prepare(
            source,
            0,
            model.descriptor(),
            &CompactionSettings::default(),
            normal
        )
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn raw_model_text_uses_runtime_normalization_across_delta_boundaries() {
    let model = ScriptedModel::new(
        1000,
        Script::Events(vec![
            Ok(ModelEvent::text_delta("first\r")),
            Ok(ModelEvent::reasoning_delta("reasoning\0not summary")),
            Ok(ModelEvent::text_delta("\nsecond\0line\rthird")),
            Ok(ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
            }),
        ]),
    );
    let candidate = generate(&plan(&model).await, &model, context())
        .await
        .unwrap();
    assert_eq!(
        candidate.summary.content.as_str(),
        "first\nsecond line\nthird"
    );
    assert!(candidate.utility_usage[0].completed);
}
