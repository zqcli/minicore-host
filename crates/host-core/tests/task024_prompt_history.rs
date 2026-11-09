//! Task 024: real Host storage/Runtime and production HTTP/SSE against loopback.
//! Resource and tool fixtures are explicit; no live provider or TUI is claimed.
#[path = "../../host-provider/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_storage::*;
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopId, LoopOptions, Text, ToolCallId, UserInput,
    history::{
        AssistantHistory, HistoryView, SummaryHistory, ToolResultHistory, UserHistory,
        UserMessageKind,
    },
    model::{
        AssistantPart, IncompleteToolCall, ModelDescriptor, ModelFinishReason, ModelLimits,
        ModelMessage, ModelPartial, ModelRequest, ModelValueError, ProviderReplay,
        ReasoningContent, ReasoningPreference as Reasoning, ToolCall, Usage,
    },
    prompt::{DefaultPromptProvider, PromptError, PromptFuture, PromptProvider, PromptRequest},
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput,
        ToolResultOutcome, ToolSet, ToolSpec,
    },
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use support::{Fixture, Reply};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn descriptor() -> ModelDescriptor {
    ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([Reasoning::Auto]),
        true,
    )
    .unwrap()
}
fn prompt_request<'a>(
    base: &'a [HistoryItem],
    appended: &'a [HistoryItem],
    model: &'a ModelDescriptor,
) -> PromptRequest<'a> {
    PromptRequest {
        loop_id: LoopId::new().unwrap(),
        request_index: 0,
        history: HistoryView::new(base, appended),
        model,
        reasoning: Reasoning::Auto,
        tools: &[],
        cancellation: CancellationToken::new(),
        deadline: tokio::time::Instant::now() + Duration::from_secs(10),
    }
}
fn model_request(messages: Vec<ModelMessage>) -> Result<ModelRequest, ModelValueError> {
    ModelRequest::new(messages, vec![], ModelLimits::default(), Reasoning::Auto)
}
fn call(id: &str, index: u32) -> AssistantPart {
    AssistantPart::ToolCall(
        ToolCall::new(
            ToolCallId::new(id).unwrap(),
            "record".parse().unwrap(),
            json!({"input": format!("original {id} 世界"), "nested": [1, {"exact": true}]}),
            index,
        )
        .unwrap(),
    )
}
fn user(loop_id: LoopId, text: &str) -> HistoryItem {
    HistoryItem::User(UserHistory {
        loop_id,
        kind: UserMessageKind::Prompt,
        input: UserInput::text(text).unwrap(),
    })
}
fn assistant(loop_id: LoopId, content: Vec<AssistantPart>) -> HistoryItem {
    HistoryItem::Assistant(AssistantHistory {
        loop_id,
        request_index: 0,
        model: "fixture/chat".parse().unwrap(),
        reasoning: Reasoning::Auto,
        content,
        provider_replay: Some(
            ProviderReplay::new(
                "minicore-host.openai-chat.v1",
                json!({"finish_reason":"tool_calls", "usage":{"total_tokens": 42}}),
            )
            .unwrap(),
        ),
        finish_reason: ModelFinishReason::ToolCalls,
        usage: Usage::default(),
    })
}
fn result(loop_id: LoopId, id: &str) -> HistoryItem {
    HistoryItem::ToolResult(ToolResultHistory {
        loop_id,
        request_index: 0,
        call_id: ToolCallId::new(id).unwrap(),
        tool_name: "record".parse().unwrap(),
        outcome: ToolResultOutcome::Success,
        output: ToolOutput::new(format!("actual recorded result for {id} 世界")).unwrap(),
    })
}
fn parts(message: &ModelMessage) -> &[AssistantPart] {
    match message {
        ModelMessage::Assistant(parts) | ModelMessage::AssistantWithReplay { parts, .. } => parts,
        _ => panic!("expected assistant"),
    }
}
fn as_call(part: &AssistantPart) -> &ToolCall {
    part.as_tool_call().unwrap()
}
fn text(part: &AssistantPart) -> &str {
    match part {
        AssistantPart::Text(text) => text,
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn default_runtime_projection_rejects_the_legal_missing_result() {
    let id = LoopId::new().unwrap();
    let history = vec![
        user(id, "old input"),
        assistant(id, vec![call("old-missing", 0)]),
    ];
    let model = descriptor();
    let default = DefaultPromptProvider::new(None)
        .prepare(prompt_request(&history, &[], &model))
        .await
        .unwrap();
    assert_eq!(
        model_request(default.messages).unwrap_err(),
        ModelValueError::IncompleteToolExchange
    );
}

#[tokio::test]
async fn complete_history_is_identical_to_default_including_replay_and_result_order() {
    let id = LoopId::new().unwrap();
    let history = vec![
        HistoryItem::Summary(SummaryHistory {
            content: Text::new("summary exact 世界"),
        }),
        user(id, "original input"),
        assistant(
            id,
            vec![
                AssistantPart::Text("before".into()),
                call("first", 0),
                AssistantPart::Reasoning(
                    ReasoningContent::new(Some("reasoning exact".into()), None, None, None)
                        .unwrap(),
                ),
                call("second", 1),
                AssistantPart::Text("after".into()),
            ],
        ),
        result(id, "second"),
        result(id, "first"),
    ];
    let model = descriptor();
    let before = history.clone();
    let default = DefaultPromptProvider::new(None)
        .prepare(prompt_request(&history, &[], &model))
        .await
        .unwrap();
    let projected = prepare_prompt_history(prompt_request(&history, &[], &model))
        .await
        .unwrap();
    assert_eq!(projected.messages, default.messages);
    model_request(projected.messages).unwrap();
    assert_eq!(history, before);
}

#[tokio::test]
async fn mixed_response_keeps_content_ids_results_and_only_reindexes_retained_calls() {
    let id = LoopId::new().unwrap();
    let large = "original content 世界\n".repeat(20_000);
    let history = vec![
        user(id, "old mixed input"),
        assistant(
            id,
            vec![
                AssistantPart::Text(large.clone().into()),
                call("missing-first", 0),
                AssistantPart::Text("between".into()),
                call("complete-first", 1),
                call("missing-middle", 2),
                call("complete-last", 3),
                AssistantPart::Text("after".into()),
            ],
        ),
        result(id, "complete-last"),
        result(id, "complete-first"),
    ];
    let model = descriptor();
    let before = history.clone();
    let projected = prepare_prompt_history(prompt_request(&history, &[], &model))
        .await
        .unwrap();
    let projected_parts = parts(&projected.messages[1]);
    assert_eq!(text(&projected_parts[0]), large);
    assert_eq!(text(&projected_parts[2]), "between");
    assert_eq!(text(&projected_parts[6]), "after");
    for (position, missing_id, old_index) in [(1, "missing-first", 0), (4, "missing-middle", 2)] {
        let note = text(&projected_parts[position]);
        assert!(note.contains("no corresponding result record"));
        assert!(note.contains(missing_id));
        assert!(note.contains(id.to_string().as_str()));
        assert!(note.contains(&format!("\"call_index\":{old_index}")));
        assert!(note.contains("original"));
        assert!(note.contains("世界"));
    }
    let HistoryItem::Assistant(original) = &history[1] else {
        unreachable!()
    };
    for (position, new_index) in [(3, 0), (5, 1)] {
        let actual = as_call(&projected_parts[position]);
        let original = as_call(&original.content[position]);
        assert_eq!(actual.tool_call_id(), original.tool_call_id());
        assert_eq!(actual.name(), original.name());
        assert_eq!(actual.arguments(), original.arguments());
        assert_eq!(actual.call_index(), new_index);
    }
    assert!(
        matches!(&projected.messages[1], ModelMessage::AssistantWithReplay { provider_replay, .. } if Some(provider_replay) == original.provider_replay.as_ref())
    );
    let default = DefaultPromptProvider::new(None)
        .prepare(prompt_request(&history, &[], &model))
        .await
        .unwrap();
    assert_eq!(projected.messages[2..], default.messages[2..]);
    model_request(projected.messages).unwrap();
    assert_eq!(history, before);
}

#[tokio::test]
async fn repeated_upstream_id_in_another_loop_does_not_close_the_old_call() {
    let first = LoopId::new().unwrap();
    let second = LoopId::new().unwrap();
    let history = vec![
        user(first, "first"),
        assistant(first, vec![call("reused-id", 0)]),
        user(second, "second"),
        assistant(second, vec![call("reused-id", 0)]),
        result(second, "reused-id"),
    ];
    let model = descriptor();
    let projected = prepare_prompt_history(prompt_request(&history, &[], &model))
        .await
        .unwrap();
    assert!(matches!(
        parts(&projected.messages[1])[0],
        AssistantPart::Text(_)
    ));
    assert_eq!(
        as_call(&parts(&projected.messages[3])[0])
            .tool_call_id()
            .as_str(),
        "reused-id"
    );
    model_request(projected.messages).unwrap();
}

#[tokio::test]
async fn current_loop_incomplete_exchange_is_not_reclassified_as_old_history() {
    let id = LoopId::new().unwrap();
    let appended = vec![user(id, "new"), assistant(id, vec![call("new-pending", 0)])];
    let model = descriptor();
    let projected = prepare_prompt_history(prompt_request(&[], &appended, &model))
        .await
        .unwrap();
    assert_eq!(
        model_request(projected.messages).unwrap_err(),
        ModelValueError::IncompleteToolExchange
    );
}

#[tokio::test]
async fn cancelled_expired_and_empty_preparations_keep_the_runtime_contract() {
    let model = descriptor();
    let history = vec![user(LoopId::new().unwrap(), "input")];
    let request = prompt_request(&history, &[], &model);
    request.cancellation.cancel();
    assert_eq!(
        prepare_prompt_history(request).await.unwrap_err(),
        PromptError::Cancelled
    );
    let mut request = prompt_request(&history, &[], &model);
    request.deadline = tokio::time::Instant::now();
    assert_eq!(
        prepare_prompt_history(request).await.unwrap_err(),
        PromptError::Cancelled
    );
    assert_eq!(
        prepare_prompt_history(prompt_request(&[], &[], &model))
            .await
            .unwrap_err(),
        PromptError::EmptyPrompt
    );
}

struct ProjectionPromptFixture;
impl PromptProvider for ProjectionPromptFixture {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(prepare_prompt_history(request))
    }
}
struct CountingTool {
    spec: ToolSpec,
    invocations: Arc<Mutex<Vec<ToolInvocation>>>,
}
impl Tool for CountingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.invocations.lock().unwrap().push(invocation);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("new tool actually executed once").unwrap(),
            ))
        })
    }
}
struct ResourcesFixture {
    model: Arc<OpenAiChatModel>,
    tool: Arc<CountingTool>,
}
impl ResourceResolver for ResourcesFixture {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            let mut tools = ToolSet::builder();
            tools.register_arc(self.tool.clone());
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    Reasoning::Auto,
                    tools.build().unwrap(),
                    None,
                    Arc::new(ProjectionPromptFixture),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
type Invocations = Arc<Mutex<Vec<ToolInvocation>>>;
fn resources(endpoint: &str) -> (Arc<dyn ResourceResolver>, Invocations) {
    let invocations = Arc::new(Mutex::new(Vec::new()));
    let model = Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(endpoint, "fixture-model", descriptor()),
            SecretApiKey::new("fixture-not-a-secret").unwrap(),
        )
        .unwrap(),
    );
    let tool = Arc::new(CountingTool {
        spec: ToolSpec::new(
            "record".parse().unwrap(),
            "Explicit counting test tool",
            json!({"type":"object"}),
        )
        .unwrap(),
        invocations: invocations.clone(),
    });
    (Arc::new(ResourcesFixture { model, tool }), invocations)
}
fn settings() -> HostSettings {
    HostSettings {
        name: "history projection fixture".into(),
    }
}
fn seed(path: &Path, entries: Vec<HistoryEntry>) -> (SessionRef, PathBuf, Vec<HistoryItem>) {
    let root = DataRoot::acquire(path).unwrap();
    let host_id = HostId::new().unwrap();
    let host = root.host(host_id.clone());
    host.save_host(&HostRecord::new(host_id.clone(), settings(), 0))
        .unwrap();
    let target = SessionRef {
        agent_id: AgentId::new().unwrap(),
        session_id: SessionId::new().unwrap(),
    };
    let agent = host.agent(target.agent_id.clone(), StorageMode::Persistent);
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/chat".parse().unwrap();
    agent
        .save(&AgentRecord::new(
            target.agent_id.clone(),
            host_id,
            AgentData {
                name: "saved fixture agent".into(),
                mode: StorageMode::Persistent,
                config: config.clone(),
                config_revision: 0,
                default_session_id: None,
            },
        ))
        .unwrap();
    let session = agent
        .session(target.session_id.clone(), StorageMode::Persistent)
        .unwrap();
    session
        .save(&SessionRecord::new(
            target.session_id.clone(),
            target.agent_id.clone(),
            SessionData {
                title: None,
                mode: StorageMode::Persistent,
                agent_config_revision: 0,
                config_revision: 0,
                effective_config: SessionConfig::from_agent(&config, &SessionOverrides::default())
                    .unwrap(),
                parent: None,
                closed: None,
            },
        ))
        .unwrap();
    let mut journal = session.open_journal().unwrap();
    journal.accept(entries).unwrap();
    journal.persist_pending().unwrap();
    drop(journal);
    let native = session.read_history().unwrap().runtime_history();
    let file = path
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
        .join("history.jsonl");
    (target, file, native)
}
fn saved_exchange(id: LoopId, content: Vec<AssistantPart>, results: &[&str]) -> Vec<HistoryEntry> {
    let turn_id = TurnId::new().unwrap();
    let HistoryItem::User(user) = user(id, "saved input") else {
        unreachable!()
    };
    let mut entries = vec![
        HistoryEntry::Input {
            turn_id: turn_id.clone(),
            message_id: MessageId::new().unwrap(),
            source: MessageSource::User,
            mode: MessageMode::FollowUp,
            in_reply_to: None,
            user,
        },
        HistoryEntry::Runtime {
            turn_id: turn_id.clone(),
            response_index: 0,
            item: assistant(id, content),
        },
    ];
    entries.extend(results.iter().map(|call_id| HistoryEntry::Runtime {
        turn_id: turn_id.clone(),
        response_index: 0,
        item: result(id, call_id),
    }));
    entries
}
fn partial_entries() -> Vec<HistoryEntry> {
    let turn_id = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    vec![
        HistoryEntry::Partial {
            turn_id: turn_id.clone(),
            loop_id,
            response_index: 0,
            partial: ModelPartial {
                request_index: 0,
                content: vec![AssistantPart::Text(
                    "true saved partial must not become complete".into(),
                )],
                incomplete_tool_calls: vec![IncompleteToolCall {
                    part_index: 1,
                    call_index: 0,
                    tool_call_id: ToolCallId::new("partial-only").unwrap(),
                    tool_name: "record".parse().unwrap(),
                    arguments_text: "{\"unfinished\":".into(),
                }],
                usage: None,
                provider_replay: None,
            },
        },
        HistoryEntry::TurnFinished {
            turn_id,
            outcome: TurnOutcome::Cancelled,
            reason: Some("saved cancellation".into()),
        },
    ]
}
fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
    )
}
fn new_call_reply() -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"new-call-only","type":"function","function":{"name":"record","arguments":"{\"new\":true}"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
    )
}
async fn until<T>(mut read: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(value) = read() {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("deterministic local condition")
}
fn send(host: &Host, caller: &CallerContext, target: &SessionRef) {
    host.send(
        caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "new request".into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
}
fn records(path: &Path) -> Vec<HistoryRecord> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn sha256(bytes: &[u8]) -> String {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

async fn persistent_case(missing: bool, complete: bool) {
    let server = Fixture::start(vec![
        Reply::sse(new_call_reply()),
        Reply::sse(answer("finished new request")),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let loop_id = LoopId::new().unwrap();
    let mut content = vec![AssistantPart::Text("before original calls\n".into())];
    if missing {
        content.push(call("old-missing", 0));
    }
    if complete {
        content.push(call("old-complete", u32::from(missing)));
    }
    content.push(AssistantPart::Text("\nafter original calls".into()));
    let mut entries = saved_exchange(
        loop_id,
        content,
        if complete { &["old-complete"] } else { &[] },
    );
    entries.extend(partial_entries());
    let (target, file, native) = seed(directory.path(), entries);
    let before = std::fs::read(&file).unwrap();
    let before = if missing && !complete {
        // A complete final JSON record needs no repair when its newline is absent.
        let bytes = before.strip_suffix(b"\n").unwrap().to_vec();
        std::fs::write(&file, &bytes).unwrap();
        bytes
    } else {
        before
    };
    let before_hash = sha256(&before);
    let (resources, calls) = resources(&server.endpoint);
    let host = Host::open_persistent(directory.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    host.resume(&caller, &target).unwrap();
    until(|| {
        (host.session_state(&caller, &target).unwrap().data.residency == Residency::Loaded)
            .then_some(())
    })
    .await;
    assert_eq!(
        host.history(&caller, &target).unwrap().as_slice(),
        native.as_slice()
    );
    assert_eq!(sha256(&std::fs::read(&file).unwrap()), before_hash);
    assert_eq!(server.request_count(), 0);
    assert!(calls.lock().unwrap().is_empty());
    // No output/state/event subscription is opened by this test.
    send(&host, &caller, &target);
    let turn = until(|| {
        host.turns(&caller, &target)
            .unwrap()
            .last()
            .filter(|turn| turn.state == TurnState::Finished)
            .cloned()
    })
    .await;
    assert_eq!(
        turn.outcome,
        Some(TurnOutcome::Completed),
        "real Runtime must accept loaded old history: {:?}",
        turn.fault
    );
    assert_eq!(turn.persistence, PersistenceState::Persisted);
    let history = host.history(&caller, &target).unwrap();
    assert_eq!(&history[..native.len()], native.as_slice());
    let after = std::fs::read(&file).unwrap();
    assert_eq!(&after[..before.len()], before);
    assert_eq!(sha256(&after[..before.len()]), before_hash);
    assert_eq!(
        records(&file)
            .iter()
            .filter(|record| matches!(record.entry, HistoryEntry::Partial { .. }))
            .count(),
        1
    );
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_call_id.as_str(), "new-call-only");
        assert_eq!(calls[0].arguments, json!({"new":true}));
    }
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert!(
            request
                .head
                .starts_with("POST /v1/chat/completions HTTP/1.1")
        );
        let messages = request.body["messages"].as_array().unwrap();
        let old = &messages[1];
        let old_text = old["content"].as_str().unwrap();
        assert!(old_text.starts_with("before original calls\n"));
        assert!(old_text.ends_with("\nafter original calls"));
        assert_eq!(old_text.contains("no corresponding result record"), missing);
        if complete {
            assert_eq!(old["tool_calls"].as_array().unwrap().len(), 1);
            assert_eq!(old["tool_calls"][0]["id"], "old-complete");
            assert_eq!(messages[2]["tool_call_id"], "old-complete");
            assert_eq!(
                messages[2]["content"],
                "actual recorded result for old-complete 世界"
            );
        } else {
            assert!(old.get("tool_calls").is_none());
        }
        assert!(!request.body.to_string().contains("true saved partial"));
        assert!(!request.body.to_string().contains("partial-only"));
        assert!(
            !messages.iter().any(
                |message| message["role"] == "tool" && message["tool_call_id"] == "old-missing"
            )
        );
    }
    let messages = requests[1].body["messages"].as_array().unwrap();
    assert!(messages.iter().any(|message| message["role"] == "tool"
        && message["tool_call_id"] == "new-call-only"
        && message["content"] == "new tool actually executed once"));
}
#[tokio::test]
async fn persistent_complete_history_runs_new_tool_once_over_real_http_without_observers() {
    persistent_case(false, true).await;
}
#[tokio::test]
async fn persistent_missing_result_runs_new_tool_once_over_real_http_without_observers() {
    persistent_case(true, false).await;
}
#[tokio::test]
async fn persistent_mixed_history_runs_new_tool_once_over_real_http_without_observers() {
    persistent_case(true, true).await;
}

#[tokio::test]
async fn cancellation_after_old_history_projection_keeps_real_partial_and_original_file_prefix() {
    let written = Arc::new(Notify::new());
    let disconnected = Arc::new(Notify::new());
    let server = Fixture::start(vec![Reply::Hold { prefix: format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":"new genuine partial"},"finish_reason":null}]})), written: written.clone(), disconnected: disconnected.clone() }]).await;
    let directory = tempfile::tempdir().unwrap();
    let entries = saved_exchange(LoopId::new().unwrap(), vec![call("old-missing", 0)], &[]);
    let (target, file, native) = seed(directory.path(), entries);
    let before = std::fs::read(&file).unwrap();
    let (resources, calls) = resources(&server.endpoint);
    let host = Host::open_persistent(directory.path(), settings(), resources)
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    send(&host, &caller, &target);
    tokio::time::timeout(Duration::from_secs(5), written.notified())
        .await
        .unwrap();
    // Wait for actual Runtime assembly, not merely server-side write completion.
    let turn = until(|| {
        let turn = host.turns(&caller, &target).unwrap().last()?.clone();
        let output = host
            .turn_output_read(
                &caller,
                &target,
                &turn.id,
                0,
                &TextReadRequest {
                    channel: "text".into(),
                    start: 0,
                    end: 19,
                    max_bytes: 19,
                },
            )
            .ok()?;
        (output.data.text == "new genuine partial").then_some(turn)
    })
    .await;
    host.cancel_turn(&caller, &target, &turn.id, false).unwrap();
    let result = host.wait_turn(&caller, &target, &turn.id).await.unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(
        &host.history(&caller, &target).unwrap()[..native.len()],
        native.as_slice()
    );
    let after = std::fs::read(&file).unwrap();
    assert_eq!(&after[..before.len()], before);
    assert_eq!(result.partials.len(), 1);
    assert_eq!(
        text(&result.partials[0].partial.content[0]),
        "new genuine partial"
    );
    assert_eq!(
        records(&file)
            .iter()
            .filter(|record| matches!(record.entry, HistoryEntry::Partial { .. }))
            .count(),
        1
    );
    tokio::time::timeout(Duration::from_secs(5), disconnected.notified())
        .await
        .unwrap();
    assert_eq!(server.finish().await.len(), 1);
}

#[tokio::test]
async fn unsupported_replay_and_signed_reasoning_are_rejected_before_http_not_flattened() {
    for opaque_replay in [true, false] {
        let server = Fixture::start(vec![]).await;
        let directory = tempfile::tempdir().unwrap();
        let mut entries = saved_exchange(LoopId::new().unwrap(), vec![call("old-missing", 0)], &[]);
        let HistoryEntry::Runtime {
            item: HistoryItem::Assistant(assistant),
            ..
        } = &mut entries[1]
        else {
            unreachable!()
        };
        if opaque_replay {
            assistant.provider_replay = Some(ProviderReplay::new("foreign.provider.v1", json!({"tool_calls":[{"id":"old-missing", "signature":"opaque-not-displayable"}]})).unwrap());
        } else {
            assistant.content.insert(
                0,
                AssistantPart::Reasoning(
                    ReasoningContent::new(
                        None,
                        None,
                        Some("opaque-encrypted-value".into()),
                        Some("signed-original-response".into()),
                    )
                    .unwrap(),
                ),
            );
        }
        let (target, file, native) = seed(directory.path(), entries);
        let before = std::fs::read(&file).unwrap();
        let model = descriptor();
        let projected = prepare_prompt_history(prompt_request(&native, &[], &model))
            .await
            .unwrap();
        let HistoryItem::Assistant(original) = &native[1] else {
            unreachable!()
        };
        assert!(
            matches!(&projected.messages[1], ModelMessage::AssistantWithReplay { provider_replay, .. } if Some(provider_replay) == original.provider_replay.as_ref())
        );
        let display: String = parts(&projected.messages[1])
            .iter()
            .filter_map(|part| match part {
                AssistantPart::Text(value) => Some(value.as_ref()),
                _ => None,
            })
            .collect();
        assert!(!display.contains("opaque-not-displayable"));
        assert!(!display.contains("opaque-encrypted-value"));
        assert!(!display.contains("signed-original-response"));
        if !opaque_replay {
            assert_eq!(parts(&projected.messages[1])[0], original.content[0]);
        }
        let (resources, calls) = resources(&server.endpoint);
        let host = Host::open_persistent(directory.path(), settings(), resources)
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        send(&host, &caller, &target);
        let turn = until(|| {
            host.turns(&caller, &target)
                .unwrap()
                .last()
                .filter(|turn| turn.state == TurnState::Finished)
                .cloned()
        })
        .await;
        assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
        assert!(turn.fault.is_some());
        assert_eq!(turn.persistence, PersistenceState::Persisted);
        assert_eq!(
            &host.history(&caller, &target).unwrap()[..native.len()],
            native.as_slice()
        );
        assert_eq!(&std::fs::read(&file).unwrap()[..before.len()], before);
        assert!(calls.lock().unwrap().is_empty());
        assert!(
            server.finish().await.is_empty(),
            "incompatible opaque data must not be sent or flattened"
        );
    }
}
