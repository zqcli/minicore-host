//! Public Host reads with real owners/Runtime/storage. Model replies and gates
//! are explicitly scripted fixtures, not live-provider or TUI validation.
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    AssistantHistory, ExecutionConfig, ExecutionOptions, HistoryItem, LoopId, Text, UserHistory,
    UserInput, UserMessageKind,
    model::{
        AssistantPart, Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason,
        ModelMessage, ModelRequest, ModelStartFuture, ModelStream,
        ReasoningPreference as RuntimeReasoning, Usage,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use minicore_host_storage::testing::read_history_bytes;
use minicore_host_storage::*;
use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

struct Script {
    text: &'static str,
    usage: Option<Usage>,
    gate: Option<Arc<Semaphore>>,
    complete: bool,
}
impl Script {
    fn answer(usage: Option<Usage>) -> Self {
        Self {
            text: "scripted complete answer",
            usage,
            gate: None,
            complete: true,
        }
    }
    fn gated(gate: Arc<Semaphore>) -> Self {
        Self {
            gate: Some(gate),
            ..Self::answer(Some(Usage::new(23, 7, 0)))
        }
    }
}
struct ScriptedModel {
    descriptor: ModelDescriptor,
    scripts: Mutex<VecDeque<Script>>,
    calls: AtomicUsize,
    utility_calls: AtomicUsize,
    entered: Semaphore,
    utility_entered: Semaphore,
    utility_gate: Option<Arc<Semaphore>>,
}
impl ScriptedModel {
    fn new(window: u64, scripts: Vec<Script>, utility_gate: Option<Arc<Semaphore>>) -> Arc<Self> {
        Arc::new(Self {
            descriptor: ModelDescriptor::new(
                "test/model".parse().unwrap(),
                window,
                BTreeSet::from([RuntimeReasoning::Auto]),
                true,
            )
            .unwrap(),
            scripts: Mutex::new(scripts.into()),
            calls: AtomicUsize::new(0),
            utility_calls: AtomicUsize::new(0),
            entered: Semaphore::new(0),
            utility_entered: Semaphore::new(0),
            utility_gate,
        })
    }
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let utility = matches!(request.messages().first(), Some(ModelMessage::System(text))
            if text.starts_with("Summarize historical conversation data"));
        let script = if utility {
            self.utility_calls.fetch_add(1, Ordering::SeqCst);
            self.utility_entered.add_permits(1);
            Script {
                text: "Actual scripted utility summary: retain the task and recorded facts.",
                usage: Some(Usage::from_optional(Some(123), Some(7), None)),
                gate: self.utility_gate.clone(),
                complete: true,
            }
        } else {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            self.scripts
                .lock()
                .unwrap()
                .pop_front()
                .expect("declared scripted response")
        };
        Box::pin(async move {
            if let Some(gate) = script.gate {
                gate.acquire().await.unwrap().forget();
            }
            let mut events = vec![ModelEvent::text_delta(script.text)];
            if let Some(usage) = script.usage {
                events.push(ModelEvent::Usage { usage });
            }
            if script.complete {
                events.push(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                });
            }
            Ok(Box::pin(futures_util::stream::iter(events.into_iter().map(Ok))) as ModelStream)
        })
    }
}
struct Resources {
    model: Arc<ScriptedModel>,
    resolves: AtomicUsize,
}
impl Resources {
    fn new(model: Arc<ScriptedModel>) -> Arc<Self> {
        Arc::new(Self {
            model,
            resolves: AtomicUsize::new(0),
        })
    }
}
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|e| Fault::new(FaultKind::SessionConfigInvalid, e.to_string()))
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.resolves.fetch_add(1, Ordering::SeqCst);
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(Some(Text::new(
                        config.role.instructions.clone(),
                    )))),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
fn config(automatic: bool) -> AgentConfig {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "test/model".parse().unwrap();
    config.session_defaults.execution.compaction = CompactionSettings {
        enabled: automatic,
        trigger_percent: 50,
        target_percent: 20,
    };
    config.role.instructions = "Do the requested work".into();
    config
}
fn input(text: &str, mode: MessageMode) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode,
        text: text.into(),
        in_reply_to: None,
    }
}
async fn memory(resources: Arc<Resources>) -> (Arc<Host>, ConnectionHandle, SessionRef) {
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let created = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "context fixture".into(),
                mode: StorageMode::Memory,
                config: config(false),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    (host, connection, target)
}
async fn entered(semaphore: &Semaphore) {
    tokio::time::timeout(Duration::from_secs(5), semaphore.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}
async fn turn_finished(host: &Host, caller: &CallerContext, target: &SessionRef, count: usize) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let turns = host.turns(caller, target).unwrap();
            if turns.len() == count && turns.iter().all(|turn| turn.state == TurnState::Finished) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn loaded(host: &Host, caller: &CallerContext, target: &SessionRef) {
    host.load_session(caller, target).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let context = host.session_context(caller, target).await.unwrap();
            assert!(context.data.block_reason.is_none(), "{context:?}");
            if context.data.history_loaded {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        result.is_ok(),
        "load did not finish: {:?}",
        host.session_context(caller, target).await
    );
}
async fn compacted(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
) -> SessionContextSnapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let context = host.session_context(caller, target).await.unwrap();
            if context
                .data
                .compaction
                .as_ref()
                .is_some_and(|work| work.state.finished())
            {
                return context;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn empty_and_queued_context_is_meaningful_read_only_and_does_not_prepare_resources() {
    let model = ScriptedModel::new(4096, vec![], None);
    let resources = Resources::new(model.clone());
    let (host, operator, target) = memory(resources.clone()).await;
    let observer = host.connect().unwrap();
    assert!(!observer.initial_info().can_operate);
    let caller = operator.caller();
    host.pause(&caller, &target).unwrap();
    for mode in [MessageMode::Steer, MessageMode::FollowUp] {
        host.send(
            &caller,
            SendMessage::new(input("queued, not consumed", mode), target.clone()),
        )
        .unwrap();
    }
    let before = serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap();
    let context = host
        .session_context(&observer.caller(), &target)
        .await
        .unwrap();
    assert_eq!(context.source.session_id, target.session_id);
    assert_eq!(context.source.agent_id, target.agent_id);
    assert_eq!(
        context.source.host_incarnation,
        host.info(&caller).unwrap().host_incarnation
    );
    assert_eq!(context.data.history.as_ref().unwrap().raw_items, 0);
    assert_eq!(context.data.history.as_ref().unwrap().effective_items, 0);
    assert!(
        context
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .is_none()
    );
    assert!(context.data.history.as_ref().unwrap().projection.is_none());
    assert!(context.data.active_turn.is_none());
    assert_eq!(
        context.data.queued_inputs.next_step + context.data.queued_inputs.next_turn,
        2
    );
    assert!(!context.data.history_save.pending);
    assert_eq!(
        serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap(),
        before
    );
    assert_eq!(resources.resolves.load(Ordering::SeqCst), 0);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    assert_eq!(model.utility_calls.load(Ordering::SeqCst), 0);
    let wire = serde_json::to_value(&context).unwrap();
    assert!(
        wire["data"].get("config").is_none(),
        "Role/body belongs in session.get"
    );
    let _: SessionContextSnapshot = serde_json::from_value(wire).unwrap();
    observer.close();
    assert_eq!(
        host.session_context(&observer.caller(), &target)
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessInvalid)
    );
}

#[tokio::test]
async fn usage_is_latest_complete_history_response_and_preserves_unknown_fields_and_real_zero() {
    let reported = Usage::from_optional(Some(71), Some(9), None).with_cache_read_tokens(Some(12));
    let model = ScriptedModel::new(
        4096,
        vec![
            Script::answer(Some(reported)),
            Script::answer(None),
            Script::answer(Some(Usage::new(0, 0, 0))),
        ],
        None,
    );
    let (host, connection, target) = memory(Resources::new(model)).await;
    let caller = connection.caller();
    for index in 0..3 {
        host.send(
            &caller,
            SendMessage::new(
                input("ordinary input", MessageMode::FollowUp),
                target.clone(),
            ),
        )
        .unwrap();
        turn_finished(&host, &caller, &target, index + 1).await;
        let context = host.session_context(&caller, &target).await.unwrap();
        assert_eq!(
            context.data.history.as_ref().unwrap().raw_items,
            2 * (index as u64 + 1)
        );
        let response = context
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .as_ref()
            .unwrap();
        assert_eq!(response.raw_item_index, 2 * index as u64 + 1);
        assert!(!response.covered_by_compaction);
        assert_eq!(response.model.as_str(), "test/model");
        let wire = serde_json::to_value(response).unwrap();
        assert!(wire.get("loop_id").is_none());
        assert!(wire.get("request_index").is_none());
        match index {
            0 => {
                assert_eq!(response.usage, reported);
                assert!(wire["usage"].get("reasoning_tokens").is_none());
            }
            1 => assert_eq!(
                wire["usage"],
                serde_json::json!({}),
                "latest unknown does not use earlier counts"
            ),
            2 => assert_eq!(response.usage.input_tokens(), Some(0)),
            _ => unreachable!(),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gated_active_and_continuation_bindings_remain_frozen_across_config_and_host_reload() {
    let first = Arc::new(Semaphore::new(0));
    let second = Arc::new(Semaphore::new(0));
    let next = Arc::new(Semaphore::new(0));
    let old_model = ScriptedModel::new(
        4096,
        vec![Script::gated(first.clone()), Script::gated(second.clone())],
        None,
    );
    let old_resources = Resources::new(old_model.clone());
    let (host, connection, target) = memory(old_resources.clone()).await;
    let caller = connection.caller();
    host.send(
        &caller,
        SendMessage::new(input("first", MessageMode::FollowUp), target.clone()),
    )
    .unwrap();
    entered(&old_model.entered).await;
    let before = host.session_context(&caller, &target).await.unwrap();
    let original = before.data.active_turn.as_ref().unwrap();
    let turn_id = original.turn_id.clone();
    assert_eq!(
        before.data.history.as_ref().unwrap().raw_items,
        1,
        "Session-authored input is readable before the model response"
    );
    assert_eq!(original.config_revision, 0);
    assert_eq!(original.model.context_window, 4096);
    let mut execution = config(false).session_defaults.execution;
    execution.compaction.target_percent = 30;
    host.update_session(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            role: Some(Role {
                instructions: "Do not duplicate this long Role in context".repeat(100),
                ..config(false).role
            }),
            execution: Some(execution.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let new_model = ScriptedModel::new(8192, vec![Script::gated(next.clone())], None);
    let new_resources = Resources::new(new_model.clone());
    host.reload_configuration(
        &caller,
        0,
        PreparedHostConfiguration::new(
            HostConfig {
                name: "new publication".into(),
                ..Default::default()
            },
            new_resources.clone(),
            vec![],
            None,
        )
        .unwrap(),
    )
    .await
    .unwrap();
    host.send(
        &caller,
        SendMessage::new(
            input("same Turn continuation", MessageMode::Steer),
            target.clone(),
        ),
    )
    .unwrap();
    for _ in 0..10 {
        let context = tokio::time::timeout(
            Duration::from_secs(1),
            host.session_context(&caller, &target),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(context.data.config_revision, 1);
        assert_eq!(context.data.compaction_settings.target_percent, 30);
        let binding = context.data.active_turn.unwrap();
        assert_eq!(binding.turn_id, turn_id);
        assert_eq!(binding.config_revision, 0);
        assert_eq!(binding.model.context_window, 4096);
        assert_eq!(binding.compaction_settings.target_percent, 20);
        assert_eq!(context.data.history.as_ref().unwrap().raw_items, 1);
    }
    assert_eq!(new_resources.resolves.load(Ordering::SeqCst), 0);
    first.add_permits(1);
    entered(&old_model.entered).await;
    let continuation = host.session_context(&caller, &target).await.unwrap();
    assert_eq!(continuation.data.history.as_ref().unwrap().raw_items, 3);
    assert_eq!(continuation.data.active_turn.unwrap().turn_id, turn_id);
    second.add_permits(1);
    turn_finished(&host, &caller, &target, 1).await;
    host.send(
        &caller,
        SendMessage::new(input("next Turn", MessageMode::FollowUp), target.clone()),
    )
    .unwrap();
    entered(&new_model.entered).await;
    let context = host.session_context(&caller, &target).await.unwrap();
    let binding = context.data.active_turn.unwrap();
    assert_ne!(binding.turn_id, turn_id);
    assert_eq!(binding.config_revision, 1);
    assert_eq!(binding.model.context_window, 8192);
    assert_eq!(binding.compaction_settings.target_percent, 30);
    assert_eq!(old_resources.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(new_resources.resolves.load(Ordering::SeqCst), 1);
    next.add_permits(1);
    turn_finished(&host, &caller, &target, 2).await;
}

fn seed(path: &Path, pairs: usize, automatic: bool) -> (SessionRef, PathBuf) {
    let root = DataRoot::acquire(path).unwrap();
    let host_id = HostId::new().unwrap();
    let host = root.host(host_id.clone());
    host.save_host(&HostRecord::new(host_id.clone(), HostConfig::default(), 0))
        .unwrap();
    let agent_id = AgentId::new().unwrap();
    let agent = host.agent(agent_id.clone(), StorageMode::Persistent);
    let session_id = SessionId::new().unwrap();
    agent
        .save(&AgentRecord::new(
            agent_id.clone(),
            host_id,
            AgentData {
                name: "stored fixture".into(),
                mode: StorageMode::Persistent,
                config: config(automatic),
                config_revision: 3,
                default_session_id: None,
            },
        ))
        .unwrap();
    let store = agent
        .session(session_id.clone(), StorageMode::Persistent)
        .unwrap();
    store
        .save(&SessionRecord::new(
            session_id.clone(),
            agent_id.clone(),
            SessionData {
                created_at: minicore_host_protocol::Timestamp::now(),
                last_activity_at: minicore_host_protocol::Timestamp::now(),
                title: None,
                mode: StorageMode::Persistent,
                agent_config_revision: 3,
                config_revision: 7,
                effective_config: SessionConfig::from_agent(
                    &config(automatic),
                    &SessionOverrides::default(),
                )
                .unwrap(),
                parent: None,
                closed: None,
            },
        ))
        .unwrap();
    let mut entries = Vec::new();
    for index in 0..pairs {
        let turn_id = TurnId::new().unwrap();
        let loop_id = LoopId::new().unwrap();
        entries.push(HistoryEntry::Input {
            received_at: minicore_host_protocol::Timestamp::now(),
            turn_id: turn_id.clone(),
            message_id: MessageId::new().unwrap(),
            source: MessageSource::User,
            mode: MessageMode::FollowUp,
            in_reply_to: None,
            user: UserHistory {
                loop_id,
                kind: UserMessageKind::Prompt,
                input: UserInput::text(format!(
                    "saved task {index}: {}",
                    "historical detail ".repeat(110)
                ))
                .unwrap(),
            },
        });
        entries.push(HistoryEntry::Runtime {
            turn_id: turn_id.clone(),
            response_index: 0,
            item: HistoryItem::Assistant(AssistantHistory {
                loop_id,
                request_index: 0,
                model: "test/model".parse().unwrap(),
                reasoning: RuntimeReasoning::Auto,
                content: vec![AssistantPart::Text(
                    format!("saved response {index}: {}", "recorded fact ".repeat(110)).into(),
                )],
                provider_replay: None,
                finish_reason: ModelFinishReason::Stop,
                usage: Usage::from_optional(Some(200 + index as u64), None, None),
            }),
        });
        entries.push(HistoryEntry::TurnFinished {
            finished_at: minicore_host_protocol::Timestamp::now(),
            requests: 1,
            tool_rounds: 0,
            model_attempts: vec![1],
            turn_id,
            outcome: TurnOutcome::Completed,
            reason: None,
        });
    }
    let mut journal = store.open_journal().unwrap();
    if !entries.is_empty() {
        journal.accept(entries).unwrap();
        journal.persist_pending().unwrap();
    }
    (
        SessionRef {
            agent_id,
            session_id,
        },
        store.canonical_path().join("history.jsonl"),
    )
}
async fn persistent(path: &Path, resources: Arc<Resources>) -> (Arc<Host>, ConnectionHandle) {
    let host = Host::open_persistent(
        path,
        HostSettings {
            name: "ignored".into(),
        },
        resources,
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    (host, connection)
}
async fn metadata_saved(host: &Host, caller: &CallerContext, target: &SessionRef) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = host.session_state(caller, target).unwrap().data;
            if !state.metadata_saving {
                assert!(state.metadata_fault.is_none());
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn shutdown(host: &Arc<Host>, caller: &CallerContext) {
    host.shutdown(
        caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.info(caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cold_history_is_unknown_until_explicit_load_and_retained_across_unload() {
    let dir = tempfile::tempdir().unwrap();
    let (target, path) = seed(dir.path(), 2, false);
    let resources = Resources::new(ScriptedModel::new(4096, vec![], None));
    let (host, connection) = persistent(dir.path(), resources.clone()).await;
    let caller = connection.caller();
    let bytes = read_history_bytes(&path).unwrap();
    let state = serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap();
    let cold = host.session_context(&caller, &target).await.unwrap();
    assert!(cold.data.history.is_none());
    assert!(!cold.data.history_loaded);
    assert!(cold.data.load_id.is_none());
    assert!(cold.data.active_turn.is_none());
    assert_eq!(cold.data.config_revision, 7);
    assert_eq!(
        serde_json::to_value(&cold).unwrap()["data"]["history"],
        serde_json::Value::Null
    );
    assert_eq!(
        serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap(),
        state
    );
    assert_eq!(resources.resolves.load(Ordering::SeqCst), 0);
    // Query repeatedly while the real load runs. No competing file lock is taken.
    loaded(&host, &caller, &target).await;
    let warm = host.session_context(&caller, &target).await.unwrap();
    assert!(warm.data.history_loaded);
    let history = warm.data.history.as_ref().unwrap();
    assert_eq!(history.raw_items, 4);
    assert_eq!(history.position, 6);
    assert_eq!(
        history
            .latest_response
            .as_ref()
            .unwrap()
            .usage
            .input_tokens(),
        Some(201)
    );
    let old_load = warm.data.load_id.unwrap();
    metadata_saved(&host, &caller, &target).await;
    host.unload(&caller, &target).unwrap();
    let unloaded = host.session_context(&caller, &target).await.unwrap();
    assert!(!unloaded.data.history_loaded);
    assert!(unloaded.data.load_id.is_none());
    assert_eq!(unloaded.data.history.as_ref().unwrap().raw_items, 4);
    loaded(&host, &caller, &target).await;
    assert_ne!(
        host.session_context(&caller, &target)
            .await
            .unwrap()
            .data
            .load_id
            .unwrap(),
        old_load
    );
    assert_eq!(read_history_bytes(path).unwrap(), bytes);
    assert!(
        host.turns(&caller, &target).unwrap().is_empty(),
        "old Turns are not resurrected"
    );
    shutdown(&host, &caller).await;
}

#[tokio::test]
async fn empty_persistent_context_is_unknown_before_load_and_known_empty_afterward() {
    let dir = tempfile::tempdir().unwrap();
    let (target, _) = seed(dir.path(), 0, false);
    let resources = Resources::new(ScriptedModel::new(4096, vec![], None));
    let (host, connection) = persistent(dir.path(), resources.clone()).await;
    let caller = connection.caller();
    assert!(
        host.session_context(&caller, &target)
            .await
            .unwrap()
            .data
            .history
            .is_none()
    );
    loaded(&host, &caller, &target).await;
    let loaded = host.session_context(&caller, &target).await.unwrap();
    let history = loaded.data.history.as_ref().unwrap();
    assert_eq!(history.raw_items, 0);
    assert_eq!(history.effective_items, 0);
    assert!(history.latest_response.is_none());
    assert!(loaded.data.history_loaded);
    assert_eq!(resources.resolves.load(Ordering::SeqCst), 0);
    shutdown(&host, &caller).await;
}

#[tokio::test]
async fn cold_corruption_remains_uninspected_until_load_and_then_returns_actual_owner_blocker() {
    let dir = tempfile::tempdir().unwrap();
    let (target, path) = seed(dir.path(), 0, false);
    let resources = Resources::new(ScriptedModel::new(4096, vec![], None));
    let (host, connection) = persistent(dir.path(), resources.clone()).await;
    let caller = connection.caller();
    std::fs::write(&path, b"{corrupt partial record").unwrap();
    let state = serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap();
    let cold = host.session_context(&caller, &target).await.unwrap();
    assert!(
        cold.data.history.is_none(),
        "unknown file is not validated or treated as empty"
    );
    assert!(cold.data.block_reason.is_none());
    assert_eq!(
        serde_json::to_value(host.session_state(&caller, &target).unwrap()).unwrap(),
        state
    );
    host.load_session(&caller, &target).unwrap();
    let failed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let context = host.session_context(&caller, &target).await.unwrap();
            if context.data.block_reason.is_some() {
                break context;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(failed.data.history.is_none());
    assert!(!failed.data.history_loaded);
    assert_eq!(
        failed.data.block_reason.unwrap().kind(),
        Some(FaultKind::SessionHistoryLoadFailed)
    );
    assert_eq!(std::fs::read(path).unwrap(), b"{corrupt partial record");
    assert_eq!(resources.resolves.load(Ordering::SeqCst), 0);
    shutdown(&host, &caller).await;
}

#[tokio::test]
async fn failed_reload_retains_old_context_but_never_claims_it_is_valid_for_execution() {
    let dir = tempfile::tempdir().unwrap();
    let (target, path) = seed(dir.path(), 1, false);
    let resources = Resources::new(ScriptedModel::new(4096, vec![], None));
    let (host, connection) = persistent(dir.path(), resources.clone()).await;
    let caller = connection.caller();
    loaded(&host, &caller, &target).await;
    metadata_saved(&host, &caller, &target).await;
    host.unload(&caller, &target).unwrap();
    // Unload retains the original journal owner. Replace the filename instead
    // of opening a second writer through its Windows no-write-sharing lease.
    std::fs::rename(&path, path.with_extension("original")).unwrap();
    std::fs::write(&path, b"{corrupt replacement").unwrap();
    host.load_session(&caller, &target).unwrap();
    let failed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let context = host.session_context(&caller, &target).await.unwrap();
            if context.data.block_reason.is_some() {
                break context;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!failed.data.history_loaded);
    assert!(failed.data.load_id.is_none());
    assert_eq!(failed.data.history.as_ref().unwrap().raw_items, 2);
    assert_eq!(
        failed.data.block_reason.unwrap().kind(),
        Some(FaultKind::SessionHistoryLoadFailed)
    );
    assert_eq!(std::fs::read(path).unwrap(), b"{corrupt replacement");
    assert_eq!(resources.resolves.load(Ordering::SeqCst), 0);
    shutdown(&host, &caller).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_queries_keep_original_history_until_commit_and_do_not_block_sends() {
    let dir = tempfile::tempdir().unwrap();
    let (target, _) = seed(dir.path(), 3, false);
    let gate = Arc::new(Semaphore::new(0));
    let model = ScriptedModel::new(4096, vec![], Some(gate.clone()));
    let resources = Resources::new(model.clone());
    let (host, connection) = persistent(dir.path(), resources.clone()).await;
    let caller = connection.caller();
    loaded(&host, &caller, &target).await;
    host.pause(&caller, &target).unwrap();
    let operation_id = OperationId::new().unwrap();
    host.session_compact(
        &caller,
        &target,
        CompactRequest {
            operation_id: operation_id.clone(),
        },
    )
    .unwrap();
    entered(&model.utility_entered).await;
    let resolves = resources.resolves.load(Ordering::SeqCst);
    for _ in 0..10 {
        let during = tokio::time::timeout(
            Duration::from_secs(1),
            host.session_context(&caller, &target),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(during.data.history.as_ref().unwrap().raw_items, 6);
        assert_eq!(during.data.history.as_ref().unwrap().effective_items, 6);
        assert!(during.data.history.as_ref().unwrap().projection.is_none());
        assert!(
            matches!(during.data.compaction.unwrap().source, CompactionSource::Manual { operation_id: id } if id == operation_id)
        );
    }
    let queued = input("queued during utility", MessageMode::FollowUp);
    host.send(&caller, SendMessage::new(queued.clone(), target.clone()))
        .unwrap();
    assert_eq!(
        host.session_context(&caller, &target)
            .await
            .unwrap()
            .data
            .queued_inputs
            .next_turn,
        1
    );
    assert_eq!(resources.resolves.load(Ordering::SeqCst), resolves);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    gate.add_permits(100);
    let result = compacted(&host, &caller, &target).await;
    let projection = result
        .data
        .history
        .as_ref()
        .unwrap()
        .projection
        .as_ref()
        .unwrap();
    assert!(
        projection
            .summary
            .content
            .as_str()
            .contains("Actual scripted utility summary")
    );
    assert!(projection.covered_items > 0);
    assert_eq!(
        result.data.history.as_ref().unwrap().effective_items,
        7 - projection.covered_items
    );
    let compaction = result.data.compaction.as_ref().unwrap();
    assert!(
        matches!(&compaction.state, CompactionState::Finished { result: CompactionOutcome::Compacted { persistence: PersistenceState::Persisted, change } } if change.covered_items == projection.covered_items && change.after_tokens < change.before_tokens)
    );
    assert!(
        compaction
            .utility_usage
            .iter()
            .all(|call| call.completed && call.usage.unwrap().input_tokens() == Some(123))
    );
    assert_eq!(
        result
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .as_ref()
            .unwrap()
            .usage
            .input_tokens(),
        Some(202),
        "utility usage does not overwrite ordinary history usage"
    );
    assert_eq!(result.data.history.as_ref().unwrap().raw_items, 6);
    assert!(!result.data.history_save.pending);
    let summary = projection.summary.clone();
    let coverage = projection.covered_items;
    metadata_saved(&host, &caller, &target).await;
    host.unload(&caller, &target).unwrap();
    assert_eq!(
        host.session_context(&caller, &target)
            .await
            .unwrap()
            .data
            .history
            .as_ref()
            .unwrap()
            .projection
            .as_ref()
            .unwrap()
            .summary,
        summary
    );
    loaded(&host, &caller, &target).await;
    assert_eq!(
        host.session_context(&caller, &target)
            .await
            .unwrap()
            .data
            .history
            .as_ref()
            .unwrap()
            .projection
            .as_ref()
            .unwrap()
            .covered_items,
        coverage
    );
    host.remove_input(&caller, &target, &queued.message_id)
        .unwrap();
    shutdown(&host, &caller).await;
    drop(connection);
    drop(host);
    let (reopened, connection) = persistent(dir.path(), resources.clone()).await;
    let cold = reopened
        .session_context(&connection.caller(), &target)
        .await
        .unwrap();
    assert!(cold.data.history.is_none());
    loaded(&reopened, &connection.caller(), &target).await;
    let restored = reopened
        .session_context(&connection.caller(), &target)
        .await
        .unwrap();
    assert_eq!(
        restored
            .data
            .history
            .as_ref()
            .unwrap()
            .projection
            .as_ref()
            .unwrap()
            .summary,
        summary
    );
    assert!(
        cold.data.compaction.is_none(),
        "runtime operation is not rebuilt from stored marker"
    );
    assert_eq!(cold.data.queued_inputs.next_turn, 0);
    assert_ne!(cold.source.host_incarnation, result.source.host_incarnation);
    shutdown(&reopened, &connection.caller()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_compaction_context_is_owned_by_active_turn_and_query_never_launches_utility() {
    let dir = tempfile::tempdir().unwrap();
    let (target, _) = seed(dir.path(), 3, true);
    let gate = Arc::new(Semaphore::new(0));
    let model = ScriptedModel::new(
        4096,
        vec![Script::answer(Some(Usage::new(88, 9, 1)))],
        Some(gate.clone()),
    );
    let resources = Resources::new(model.clone());
    let (host, connection) = persistent(dir.path(), resources.clone()).await;
    let caller = connection.caller();
    host.resume(&caller, &target).unwrap();
    host.send(
        &caller,
        SendMessage::new(
            input("new protected input", MessageMode::FollowUp),
            target.clone(),
        ),
    )
    .unwrap();
    entered(&model.utility_entered).await;
    let initial = host.session_context(&caller, &target).await.unwrap();
    let binding = initial.data.active_turn.as_ref().unwrap();
    assert!(
        matches!(&initial.data.compaction.as_ref().unwrap().source, CompactionSource::Automatic { turn_id, load_id, .. } if turn_id == &binding.turn_id && load_id == &binding.load_id)
    );
    for _ in 0..10 {
        let snapshot = host.session_context(&caller, &target).await.unwrap();
        assert_eq!(
            snapshot.data.history.as_ref().unwrap().raw_items,
            7,
            "new input is accepted once and protected from compaction until saved"
        );
        assert!(snapshot.data.history.as_ref().unwrap().projection.is_none());
    }
    assert_eq!(model.utility_calls.load(Ordering::SeqCst), 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    gate.add_permits(100);
    turn_finished(&host, &caller, &target, 1).await;
    let finished = host.session_context(&caller, &target).await.unwrap();
    assert_eq!(finished.data.history.as_ref().unwrap().raw_items, 8);
    assert!(finished.data.history.as_ref().unwrap().projection.is_some());
    assert_eq!(
        finished
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .as_ref()
            .unwrap()
            .usage
            .input_tokens(),
        Some(88)
    );
    assert_eq!(resources.resolves.load(Ordering::SeqCst), 1);
    shutdown(&host, &caller).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_compaction_on_continuation_uses_public_response_index_everywhere() {
    let first = Arc::new(Semaphore::new(0));
    let utility = Arc::new(Semaphore::new(0));
    let model = ScriptedModel::new(
        4096,
        vec![Script::gated(first.clone()), Script::answer(None)],
        Some(utility.clone()),
    );
    let (host, connection, target) = memory(Resources::new(model.clone())).await;
    let caller = connection.caller();
    host.update_session(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            execution: Some(config(true).session_defaults.execution),
            ..Default::default()
        },
    )
    .unwrap();
    // The first request has no accepted prefix to compact. Its complete handoff
    // becomes eligible at the next model request in this same public Turn.
    host.send(
        &caller,
        SendMessage::new(
            input(&"historical input ".repeat(550), MessageMode::FollowUp),
            target.clone(),
        ),
    )
    .unwrap();
    entered(&model.entered).await;
    let turn = host
        .session_state(&caller, &target)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    assert_eq!(model.utility_calls.load(Ordering::SeqCst), 0);
    host.send(
        &caller,
        SendMessage::new(
            input("continue original Turn", MessageMode::Steer),
            target.clone(),
        ),
    )
    .unwrap();
    first.add_permits(1);
    entered(&model.utility_entered).await;
    let context = host.session_context(&caller, &target).await.unwrap();
    let state = host.session_state(&caller, &target).unwrap();
    let source = serde_json::to_value(&context.data.compaction.as_ref().unwrap().source).unwrap();
    assert_eq!(source["turn_id"], serde_json::json!(turn));
    assert_eq!(
        source["load_id"],
        serde_json::json!(context.data.active_turn.as_ref().unwrap().load_id)
    );
    assert_eq!(source["response_index"], 1);
    assert!(source.get("loop_id").is_none());
    assert!(source.get("request_index").is_none());
    assert_eq!(
        serde_json::to_value(&state.data.compaction.as_ref().unwrap().source).unwrap(),
        source
    );
    let subscription = host
        .events_subscribe(
            &caller,
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: target.agent_id.clone(),
                    session_id: target.session_id.clone(),
                },
                categories: BTreeSet::from([EventCategory::State]),
            },
        )
        .unwrap();
    let observed = subscription
        .result()
        .initial_states
        .iter()
        .find_map(|state| match state {
            StateEvent::Session { data, .. } => data.compaction.as_ref().map(|work| &work.source),
            _ => None,
        })
        .unwrap();
    assert_eq!(serde_json::to_value(observed).unwrap(), source);
    utility.add_permits(100);
    turn_finished(&host, &caller, &target, 1).await;
    let finished = host.session_context(&caller, &target).await.unwrap();
    let work = finished.data.compaction.as_ref().unwrap();
    assert_eq!(serde_json::to_value(&work.source).unwrap(), source);
    assert_eq!(work.utility_usage.first().unwrap().request_index, 1);
    assert_eq!(host.turn_info(&caller, &target, &turn).unwrap().requests, 2);
    let history = host.history(&caller, &target).unwrap();
    let responses: Vec<_> = history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::Assistant(response) => Some(response),
            _ => None,
        })
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].loop_id, responses[1].loop_id);
    assert_eq!(responses[0].request_index, 0);
    assert_eq!(responses[1].request_index, 1);
}

async fn next_event(subscription: &mut EventSubscription) -> ObservationEvent {
    match tokio::time::timeout(Duration::from_secs(5), subscription.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
    {
        ObservationNotification::Event(push) => push.event,
        notification => panic!("unexpected observation notification: {notification:?}"),
    }
}
async fn accepted_history_and_save_are_published_together(mode: StorageMode) {
    let directory = tempfile::tempdir().unwrap();
    let gate = Arc::new(Semaphore::new(0));
    let model = ScriptedModel::new(
        4096,
        vec![Script::gated(gate.clone()), Script::gated(gate.clone())],
        None,
    );
    let resources = Resources::new(model.clone());
    let (host, connection, target) = if mode == StorageMode::Persistent {
        let (target, _) = seed(directory.path(), 0, false);
        let (host, connection) = persistent(directory.path(), resources).await;
        (host, connection, target)
    } else {
        memory(resources).await
    };
    let caller = connection.caller();
    let mut subscription = host
        .events_subscribe(
            &caller,
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: target.agent_id.clone(),
                    session_id: target.session_id.clone(),
                },
                categories: BTreeSet::from([EventCategory::State, EventCategory::Completion]),
            },
        )
        .unwrap();
    host.send(
        &caller,
        SendMessage::new(
            input("first request", MessageMode::FollowUp),
            target.clone(),
        ),
    )
    .unwrap();
    for response_index in 0..2 {
        entered(&model.entered).await;
        let context = host.session_context(&caller, &target).await.unwrap();
        let history = context.data.history.as_ref().unwrap();
        assert_eq!(history.raw_items, response_index * 2 + 1);
        assert_eq!(history.position, response_index * 2);
        assert!(!context.data.history_save.pending);
        let turn_id = &context.data.active_turn.as_ref().unwrap().turn_id;
        if response_index == 0 {
            host.send(
                &caller,
                SendMessage::new(
                    input("same Turn continuation", MessageMode::Steer),
                    target.clone(),
                ),
            )
            .unwrap();
        } else {
            assert_eq!(
                host.turn_info(&caller, &target, turn_id)
                    .unwrap()
                    .persistence,
                if mode == StorageMode::Persistent {
                    PersistenceState::Pending
                } else {
                    PersistenceState::NotApplicable
                }
            );
        }
        gate.add_permits(1);
        // ModelFinished and these two immediately following states are emitted
        // by the acceptance boundary itself. Their queued values expose the old
        // split-lock bug deterministically, without racing the query scheduler.
        loop {
            if let ObservationEvent::Activity(ActivityEvent::ModelFinished { source, .. }) =
                next_event(&mut subscription).await
            {
                assert_eq!(&source.turn.turn_id, turn_id);
                assert_eq!(source.response_index, response_index);
                break;
            }
        }
        let ObservationEvent::State(StateEvent::Session { data, .. }) =
            next_event(&mut subscription).await
        else {
            panic!("accepted response must publish its Session state");
        };
        assert!(
            data.saving,
            "accepted response must include its original save"
        );
        let ObservationEvent::State(StateEvent::Turn { source, data }) =
            next_event(&mut subscription).await
        else {
            panic!("accepted response must publish its Turn state");
        };
        assert_eq!(&source.turn_id, turn_id);
        assert_eq!(
            data.persistence,
            if mode == StorageMode::Persistent {
                PersistenceState::Pending
            } else {
                PersistenceState::NotApplicable
            }
        );
    }
    turn_finished(&host, &caller, &target, 1).await;
    let context = host.session_context(&caller, &target).await.unwrap();
    let history = context.data.history.unwrap();
    assert_eq!(history.raw_items, 4);
    assert_eq!(history.position, 5);
    assert_eq!(history.latest_response.unwrap().raw_item_index, 3);
    assert!(!context.data.history_save.pending && !context.data.history_save.running);
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    assert_eq!(model.utility_calls.load(Ordering::SeqCst), 0);
    shutdown(&host, &caller).await;
    if mode == StorageMode::Memory {
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_acceptance_publishes_pending_save_before_continuation_observers() {
    accepted_history_and_save_are_published_together(StorageMode::Persistent).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_acceptance_publishes_registered_save_without_applicable_persistence() {
    accepted_history_and_save_are_published_together(StorageMode::Memory).await;
}

#[tokio::test]
async fn incomplete_model_output_is_not_complete_context_history_and_reclaimed_is_not_empty() {
    let model = ScriptedModel::new(
        4096,
        vec![Script {
            complete: false,
            ..Script::answer(Some(Usage::new(44, 3, 0)))
        }],
        None,
    );
    let (host, connection, target) = memory(Resources::new(model)).await;
    let caller = connection.caller();
    host.send(
        &caller,
        SendMessage::new(input("partial task", MessageMode::FollowUp), target.clone()),
    )
    .unwrap();
    turn_finished(&host, &caller, &target, 1).await;
    let context = host.session_context(&caller, &target).await.unwrap();
    assert_eq!(context.data.history.as_ref().unwrap().raw_items, 1);
    assert!(
        context
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .is_none()
    );
    let history = host
        .history_read(
            &caller,
            &target,
            &HistoryReadRequest {
                direction: minicore_host_protocol::HistoryDirection::Forward,
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(
        history
            .data
            .items
            .iter()
            .any(|item| matches!(item.item, HistoryItemData::Partial { .. }))
    );
    host.close_session(&caller, &target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match host.session_context(&caller, &target).await {
                Err(fault) => {
                    assert!(matches!(
                        fault.kind(),
                        Some(FaultKind::AgentSessionNotFound | FaultKind::HostAgentNotFound)
                    ));
                    break;
                }
                Ok(snapshot) => assert_eq!(snapshot.data.history.as_ref().unwrap().raw_items, 1),
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
