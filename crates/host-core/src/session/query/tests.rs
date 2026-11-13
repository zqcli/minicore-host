//! Real Session owners, Runtime and (for save failure) the actual journal.
//! Only model events and the journal acknowledgement failure are test fixtures.
use super::*;
use crate::backend::{JournalHistory, TestAppendFault};
use minicore_host_runtime::{
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use minicore_host_storage::testing::read_history_bytes;
use minicore_host_storage::{AgentData, AgentRecord, DataRoot, HostRecord, SessionData};
use std::{collections::BTreeSet, time::Duration};
use tokio::sync::mpsc;

struct ScriptedModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<mpsc::UnboundedReceiver<ModelEvent>>>,
    started: mpsc::UnboundedSender<()>,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let receiver = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("declared model round");
        self.started.send(()).unwrap();
        Box::pin(async move {
            Ok(Box::pin(futures_util::stream::unfold(
                (receiver, false),
                |(mut receiver, done)| async move {
                    if done {
                        return None;
                    }
                    receiver.recv().await.map(|event| {
                        let done = matches!(event, ModelEvent::Finish { .. });
                        (Ok(event), (receiver, done))
                    })
                },
            )) as ModelStream)
        })
    }
}
struct Echo(ToolSpec);
impl Tool for Echo {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("actual fixture result").unwrap(),
            ))
        })
    }
}
struct Resources(Arc<ScriptedModel>);
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            let mut tools = ToolSet::builder();
            tools.register(Echo(
                ToolSpec::new(
                    "echo".parse().unwrap(),
                    "fixture echo",
                    serde_json::json!({"type":"object"}),
                )
                .unwrap(),
            ));
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.0.clone(),
                    RuntimeReasoning::Auto,
                    tools.build().unwrap(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    connection: crate::ConnectionHandle,
    session: Arc<SessionController>,
    events: Vec<mpsc::UnboundedSender<ModelEvent>>,
    started: mpsc::UnboundedReceiver<()>,
    journal: Option<Arc<JournalHistory>>,
    store: Option<SessionStore>,
}
fn fixture(rounds: usize, persistent: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let agent_id = AgentId::new().unwrap();
    let session_id = SessionId::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let config = SessionConfig {
        model: "fixture/model".parse().unwrap(),
        ..Default::default()
    };
    let mut events = Vec::new();
    let mut streams = VecDeque::new();
    for _ in 0..rounds {
        let (sender, receiver) = mpsc::unbounded_channel();
        events.push(sender);
        streams.push_back(receiver);
    }
    let (started, starts) = mpsc::unbounded_channel();
    let model = Arc::new(ScriptedModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        rounds: Mutex::new(streams),
        started,
    });
    let store = persistent.then(|| {
        let root = DataRoot::acquire(directory.path()).unwrap();
        let host = root.host(host_id.clone());
        host.save_host(&HostRecord::new(
            host_id.clone(),
            crate::HostConfig::default(),
            0,
        ))
        .unwrap();
        let agent = host.agent(agent_id.clone(), StorageMode::Persistent);
        agent
            .save(&AgentRecord::new(
                agent_id.clone(),
                host_id.clone(),
                AgentData {
                    name: "query fixture".into(),
                    mode: StorageMode::Persistent,
                    config: AgentConfig::default(),
                    config_revision: 0,
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
                    agent_config_revision: 0,
                    config_revision: 0,
                    effective_config: config.clone(),
                    parent: None,
                    closed: None,
                },
            ))
            .unwrap();
        store
    });
    let journal = store
        .as_ref()
        .map(|store| Arc::new(JournalHistory::new(store.clone())));
    let backend: Arc<dyn HistoryBackend> = match &journal {
        Some(journal) => journal.clone(),
        None => Arc::new(MemoryHistory),
    };
    let session = SessionController::new(
        session_id,
        agent_id,
        host_id,
        incarnation,
        config,
        access,
        Arc::new(Resources(model)),
        backend,
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    );
    Fixture {
        _directory: directory,
        connection,
        session,
        events,
        started: starts,
        journal,
        store,
    }
}
fn send(fixture: &Fixture, mode: MessageMode) -> MessageId {
    let message_id = MessageId::new().unwrap();
    fixture
        .session
        .send(
            &fixture.connection.caller(),
            MessageInput {
                message_id: message_id.clone(),
                text: "real query task".into(),
                mode,
                in_reply_to: None,
            },
        )
        .unwrap();
    message_id
}
async fn started(fixture: &mut Fixture) {
    tokio::time::timeout(Duration::from_secs(3), fixture.started.recv())
        .await
        .unwrap()
        .unwrap();
}
fn finish(events: &mpsc::UnboundedSender<ModelEvent>) {
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
        .unwrap();
}
async fn until(fixture: &Fixture, condition: impl Fn(&SessionState) -> bool) {
    let mut snapshots = fixture.session.snapshots.subscribe();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if condition(&fixture.session.state.lock().unwrap()) {
                return;
            }
            snapshots.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}
fn assert_public(value: &impl serde::Serialize) {
    let wire = serde_json::to_string(value).unwrap();
    for key in ["\"loop_id\":", "\"request_index\":", "\"provider_replay\":"] {
        assert!(!wire.contains(key), "unexpected Runtime metadata: {wire}");
    }
}

#[tokio::test]
async fn original_config_and_actual_indexes_survive_tool_only_empty_response_and_next_request_partial()
 {
    let mut fixture = fixture(4, false);
    let caller = fixture.connection.caller();
    let original = fixture.session.info(&caller).unwrap().config;
    send(&fixture, MessageMode::FollowUp);
    started(&mut fixture).await;
    let first = fixture
        .session
        .turn_list(&caller)
        .unwrap()
        .data
        .remove(0)
        .turn_id;
    fixture
        .session
        .update(
            &caller,
            0,
            &SessionConfigPatch {
                role: Some(Role {
                    name: "next Turn binding".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let detail = fixture.session.turn_details(&caller, &first).unwrap();
    assert_eq!(detail.data.config, original);
    assert_eq!(detail.data.state.config_revision, 0);
    fixture.events[0]
        .send(ModelEvent::ToolCallStart {
            tool_call_id: "echo-0".parse().unwrap(),
            tool_name: "echo".parse().unwrap(),
        })
        .unwrap();
    fixture.events[0]
        .send(ModelEvent::tool_call_arguments_delta(
            "echo-0".parse().unwrap(),
            "{}",
        ))
        .unwrap();
    fixture.events[0]
        .send(ModelEvent::ToolCallEnd {
            tool_call_id: "echo-0".parse().unwrap(),
        })
        .unwrap();
    fixture.events[0]
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        })
        .unwrap();
    started(&mut fixture).await;
    // Inputs queued during the final response are taken by the next internal
    // Loop, after this complete empty response ends the first Runtime Loop.
    send(&fixture, MessageMode::Steer);
    fixture.events[1]
        .send(ModelEvent::ProviderReplay {
            replay: minicore_host_runtime::model::ProviderReplay::new(
                "fixture/v1",
                serde_json::json!({"opaque":"empty complete response"}),
            )
            .unwrap(),
        })
        .unwrap();
    finish(&fixture.events[1]);
    started(&mut fixture).await;
    let continued = fixture.session.turn_details(&caller, &first).unwrap();
    assert_eq!(continued.source, detail.source);
    assert_eq!(continued.data.config, original);
    assert_eq!(continued.data.state.config_revision, 0);
    fixture.events[2]
        .send(ModelEvent::text_delta("unfinished second Loop"))
        .unwrap();
    // A real stream termination creates the authoritative partial report.
    let sender = fixture.events.remove(2);
    drop(sender);
    until(&fixture, |state| state.completed.contains_key(&first)).await;
    let raw = fixture.session.turn_result(&caller, &first).unwrap();
    let users: Vec<_> = raw
        .history
        .iter()
        .filter_map(|item| {
            if let HistoryItem::User(user) = item {
                Some(user.loop_id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0], users[1]);
    let raw_assistants: Vec<_> = raw
        .history
        .iter()
        .filter_map(|item| {
            if let HistoryItem::Assistant(response) = item {
                Some(response.request_index)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(raw_assistants, [0, 1]);
    assert_eq!(raw.partials[0].partial.request_index, 2);
    let result = fixture
        .session
        .turn_result_snapshot(&caller, &first)
        .await
        .unwrap();
    assert_eq!(result.source, detail.source);
    assert_eq!(result.data.config.as_ref().unwrap(), &original);
    assert_eq!(result.data.info.as_ref().unwrap().config_revision, 0);
    assert_eq!(
        result.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert_eq!(
        result.data.info.as_ref().unwrap().persistence,
        PersistenceState::NotApplicable
    );
    assert_eq!(
        result
            .data
            .history
            .iter()
            .filter_map(|item| item.response_index)
            .collect::<Vec<_>>(),
        [0, 0, 1]
    );
    assert_eq!(result.data.partials.len(), 1);
    assert_eq!(result.data.partials[0].response_index, 2);
    assert_eq!(result.data.history.len(), raw.history.len());
    let rows = fixture
        .session
        .history_read(
            &caller,
            &HistoryReadRequest {
                cursor: None,
                limit: 100,
                direction: HistoryDirection::Forward,
            },
        )
        .await
        .unwrap()
        .data
        .items;
    assert!(result.data.saved_terminal.is_none());
    for item in &result.data.history {
        assert_eq!(rows[item.position as usize].turn_id.as_ref(), Some(&first));
        assert_eq!(rows[item.position as usize].recorded_at, item.recorded_at);
    }
    let partial = &result.data.partials[0];
    assert_eq!(
        rows[partial.position as usize].recorded_at,
        partial.recorded_at
    );
    assert_eq!(rows[partial.position as usize].response_index, Some(2));
    let terminal_row = rows.last().unwrap();
    let HistoryItemData::TurnFinished { terminal } = &terminal_row.item else {
        panic!("actual memory terminal")
    };
    let state = result.data.info.as_ref().unwrap();
    assert_eq!(state.history_position, Some(terminal_row.position));
    assert_eq!(state.recorded_at, Some(terminal_row.recorded_at));
    assert_eq!(state.finished_at, Some(terminal.finished_at));
    assert_eq!(terminal.requests, 3);
    assert_eq!(terminal.tool_rounds, 1);
    assert_eq!(terminal.model_attempts, [1, 1, 1]);
    // Empty complete projections remain complete, while the readable body of
    // the failed request remains a partial and never adds an Assistant item.
    for (response_index, expected) in [(0, ""), (1, ""), (2, "unfinished second Loop")] {
        let output = fixture
            .session
            .output_read(
                &caller,
                &first,
                Some(response_index),
                None,
                &TextReadRequest {
                    channel: "text".into(),
                    start: 0,
                    end: 1000,
                    max_bytes: 1000,
                },
            )
            .unwrap();
        assert_eq!(output.text, expected);
        assert_eq!(output.end, expected.len() as u64);
    }
    assert!(
        !result
            .data
            .history
            .iter()
            .any(|item| matches!(item.item, HistoryItemData::Partial { .. }))
    );
    assert_public(&result);
    let finished = fixture.session.turn_details(&caller, &first).unwrap();
    assert_eq!(finished.data.config, original);
    send(&fixture, MessageMode::FollowUp);
    fixture.session.resume(&caller).unwrap();
    started(&mut fixture).await;
    let second = fixture
        .session
        .turn_list(&caller)
        .unwrap()
        .data
        .pop()
        .unwrap()
        .turn_id;
    assert_ne!(second, first);
    fixture.events[2]
        .send(ModelEvent::text_delta("new Turn complete response"))
        .unwrap();
    finish(&fixture.events[2]);
    until(&fixture, |state| state.completed.contains_key(&second)).await;
    let second = fixture
        .session
        .turn_result_snapshot(&caller, &second)
        .await
        .unwrap();
    assert_eq!(second.data.info.as_ref().unwrap().config_revision, 1);
    assert_eq!(
        second.data.config.as_ref().unwrap().role.name,
        "next Turn binding"
    );
    assert_eq!(
        second
            .data
            .history
            .iter()
            .filter_map(|item| item.response_index)
            .collect::<Vec<_>>(),
        [0]
    );
    assert_eq!(
        fixture
            .session
            .turn_result_snapshot(&caller, &first)
            .await
            .unwrap()
            .data
            .config
            .unwrap(),
        original
    );
    let (states, _) = fixture
        .session
        .observation_baseline(None, true, false)
        .unwrap();
    for state in states {
        let wire = serde_json::to_value(state).unwrap();
        assert!(wire["data"].get("config").is_none());
    }
}

#[tokio::test]
async fn accepted_complete_history_stays_queryable_across_original_save_failure_without_rewriting()
{
    let mut fixture = fixture(1, true);
    let caller = fixture.connection.caller();
    let original = fixture.session.info(&caller).unwrap().config;
    fixture
        .journal
        .as_ref()
        .unwrap()
        .inject_next_append_fault(TestAppendFault::FullWrite);
    send(&fixture, MessageMode::FollowUp);
    started(&mut fixture).await;
    fixture.events[0]
        .send(ModelEvent::text_delta("complete but initially unconfirmed"))
        .unwrap();
    finish(&fixture.events[0]);
    until(&fixture, |state| {
        !state.history_work.as_ref().is_some_and(|work| work.running)
            && state.history_work.is_some()
            && state
                .active
                .as_ref()
                .is_some_and(|active| active.info.outcome.is_some())
    })
    .await;
    let turn = fixture
        .session
        .turn_list(&caller)
        .unwrap()
        .data
        .remove(0)
        .turn_id;
    let detail = fixture.session.turn_details(&caller, &turn).unwrap();
    assert_eq!(detail.data.config, original);
    assert_eq!(detail.data.state.persistence, PersistenceState::Failed);
    assert_eq!(detail.data.state.outcome, Some(TurnOutcome::Completed));
    assert_eq!(
        fixture
            .session
            .turn_result_snapshot(&caller, &turn)
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionTurnNotFound)
    );
    let body = fixture
        .session
        .output_read(
            &caller,
            &turn,
            Some(0),
            None,
            &TextReadRequest {
                channel: "text".into(),
                start: 0,
                end: "complete but initially unconfirmed".len() as u64,
                max_bytes: 1000,
            },
        )
        .unwrap();
    assert_eq!(body.text, "complete but initially unconfirmed");
    let path = fixture
        .store
        .as_ref()
        .unwrap()
        .canonical_path()
        .join("history.jsonl");
    let before = read_history_bytes(&path).unwrap();
    assert!(!before.is_empty());
    fixture.session.resume(&caller).unwrap();
    until(&fixture, |state| state.completed.contains_key(&turn)).await;
    let result = fixture
        .session
        .turn_result_snapshot(&caller, &turn)
        .await
        .unwrap();
    assert_eq!(result.data.config.as_ref().unwrap(), &original);
    assert_eq!(
        result.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        result.data.info.as_ref().unwrap().persistence,
        PersistenceState::Persisted
    );
    assert!(result.data.partials.is_empty());
    assert_eq!(result.data.history.len(), 2);
    assert_eq!(result.data.history[1].response_index, Some(0));
    assert_public(&result);
    let after = read_history_bytes(&path).unwrap();
    assert!(after.starts_with(&before));
    assert_eq!(String::from_utf8(after).unwrap().lines().count(), 3);
}

#[path = "saved_tests.rs"]
mod saved_tests;
