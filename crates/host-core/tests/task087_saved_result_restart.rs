//! Actual Host/Runtime, production provider, loopback HTTP/SSE and JSONL.
//! Scripted HTTP responses are not a live model/TUI test or a Runtime double.
use minicore_host_storage::testing::read_history_bytes;
#[path = "../../host-provider/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;
use minicore_host_core::{
    BoundResources, CallerContext, ConnectionHandle, Host, HostSettings, ResourceFuture,
    ResourceResolver, TurnResultSnapshot,
};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::{Fixture, Reply};
use tokio::sync::Notify;

struct CountedEcho {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
}
impl Tool for CountedEcho {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("actual echo result").unwrap(),
            ))
        })
    }
}
struct ConfiguredResources {
    model: Arc<OpenAiChatModel>,
    resolves: AtomicUsize,
    tools: Arc<AtomicUsize>,
}
impl ResourceResolver for ConfiguredResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if config.model.as_str() != self.model.descriptor().model_ref.as_str() {
            return Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "model reference is not configured",
            ));
        }
        Ok(())
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            self.validate(config)?;
            let mut tools = ToolSet::builder();
            tools.register(CountedEcho {
                spec: ToolSpec::new(
                    "echo".parse().unwrap(),
                    "test echo tool",
                    json!({"type":"object"}),
                )
                .unwrap(),
                calls: self.tools.clone(),
            });
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    tools.build().unwrap(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
fn delta(text: &str) -> String {
    format!(
        "data: {}\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]})
    )
}
fn end(reason: &str) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]})
    )
}
fn answer(text: &str) -> Reply {
    Reply::sse(delta(text) + &end("stop"))
}
fn tool_call() -> Reply {
    Reply::sse(format!(
        "data: {}\n\n{}",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"echo-0","type":"function","function":{"name":"echo","arguments":"{}"}}]},"finish_reason":null}]}),
        end("tool_calls")
    ))
}
fn value(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}
struct Rig {
    directory: tempfile::TempDir,
    server: Fixture,
    resources: Arc<ConfiguredResources>,
    host: Arc<Host>,
    connection: ConnectionHandle,
    target: SessionRef,
}
impl Rig {
    async fn new(replies: Vec<Reply>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let server = Fixture::start(replies).await;
        let descriptor = ModelDescriptor::new(
            "fixture/chat".parse().unwrap(),
            128_000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap();
        let resources = Arc::new(ConfiguredResources {
            model: Arc::new(
                OpenAiChatModel::new(
                    OpenAiChatConfig::new(&server.endpoint, "fixture-model", descriptor),
                    SecretApiKey::new("fixture-not-a-real-secret").unwrap(),
                )
                .unwrap(),
            ),
            resolves: AtomicUsize::new(0),
            tools: Arc::new(AtomicUsize::new(0)),
        });
        let host = Host::open_persistent(
            directory.path(),
            HostSettings {
                name: "saved result regression".into(),
            },
            resources.clone(),
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        let mut config = AgentConfig::default();
        config.session_defaults.model = "fixture/chat".parse().unwrap();
        config.session_defaults.execution.compaction.enabled = false;
        let created = host
            .create_agent_async(
                &connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "saved result regression".into(),
                    mode: StorageMode::Persistent,
                    config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .await
            .unwrap();
        Self {
            directory,
            server,
            resources,
            host,
            connection,
            target: SessionRef {
                agent_id: created.agent_id.unwrap(),
                session_id: created.session_id.unwrap(),
            },
        }
    }
    fn caller(&self) -> CallerContext {
        self.connection.caller()
    }
    fn path(&self) -> PathBuf {
        self.directory
            .path()
            .join("agents")
            .join(self.target.agent_id.as_str())
            .join("sessions")
            .join(self.target.session_id.as_str())
            .join("history.jsonl")
    }
    fn bytes(&self) -> Vec<u8> {
        read_history_bytes(self.path()).unwrap()
    }
    fn counts(&self) -> (usize, usize, usize) {
        (
            self.resources.resolves.load(Ordering::SeqCst),
            self.server.request_count(),
            self.resources.tools.load(Ordering::SeqCst),
        )
    }
    async fn send(&self, text: &str) -> TurnId {
        let previous = self.host.turns(&self.caller(), &self.target).unwrap().len();
        self.host
            .send(
                &self.caller(),
                SendMessage::new(
                    MessageInput {
                        message_id: MessageId::new().unwrap(),
                        mode: MessageMode::FollowUp,
                        text: text.into(),
                        in_reply_to: None,
                    },
                    self.target.clone(),
                ),
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let turns = self.host.turns(&self.caller(), &self.target).unwrap();
                if turns.len() > previous {
                    break turns.last().unwrap().id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    async fn wait(&self, id: &TurnId) -> TurnResultSnapshot {
        tokio::time::timeout(
            Duration::from_secs(5),
            self.host
                .wait_turn_snapshot(&self.caller(), &self.target, id),
        )
        .await
        .unwrap()
        .unwrap()
    }
    async fn result(&self, id: &TurnId) -> Result<TurnResultSnapshot, Fault> {
        tokio::time::timeout(
            Duration::from_secs(5),
            self.host
                .turn_result_snapshot(&self.caller(), &self.target, id),
        )
        .await
        .unwrap()
    }
    async fn reopen(&mut self) {
        self.host
            .shutdown(
                &self.caller(),
                HostShutdownRequest {
                    operation_id: OperationId::new().unwrap(),
                    strategy: HostShutdownStrategy::Graceful,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.host.info(&self.caller()).unwrap().lifecycle != HostLifecycle::Stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        self.connection.close();
        self.host = Host::open_persistent(
            self.directory.path(),
            HostSettings {
                name: "ignored on reopen".into(),
            },
            self.resources.clone(),
        )
        .await
        .unwrap();
        self.connection = self.host.connect().unwrap();
    }
    async fn load(&self) {
        self.host
            .session_load(
                &self.caller(),
                &self.target,
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while self
                .host
                .session_state(&self.caller(), &self.target)
                .unwrap()
                .data
                .residency
                != Residency::Loaded
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    fn assert_no_execution(&self) {
        assert!(
            self.host
                .turn_list(&self.caller(), &self.target)
                .unwrap()
                .data
                .is_empty()
        );
        assert!(
            self.host
                .session_state(&self.caller(), &self.target)
                .unwrap()
                .data
                .active_turn_id
                .is_none()
        );
        assert!(
            self.host
                .input_list(&self.caller(), &self.target)
                .unwrap()
                .next_step
                .is_empty()
        );
    }
}
fn assert_saved(saved: &TurnResultSnapshot, original: &TurnResultSnapshot, reason: &Value) {
    assert_eq!(saved.source.turn_id, original.source.turn_id);
    assert_eq!(
        saved.source.session.host_id,
        original.source.session.host_id
    );
    assert_ne!(
        saved.source.session.host_incarnation,
        original.source.session.host_incarnation
    );
    assert_eq!(
        saved.source.session.agent_id,
        original.source.session.agent_id
    );
    assert_eq!(
        saved.source.session.session_id,
        original.source.session.session_id
    );
    assert!(saved.source.load_id.is_none());
    assert!(saved.data.info.is_none() && saved.data.config.is_none());
    let terminal = saved.data.saved_terminal.as_ref().unwrap();
    assert_eq!(
        Some(terminal.outcome),
        original.data.info.as_ref().unwrap().outcome
    );
    assert_eq!(value(&terminal.reason), *reason);
    assert_eq!(value(&saved.data.history), value(&original.data.history));
    assert_eq!(value(&saved.data.partials), value(&original.data.partials));
    let wire = value(saved);
    assert!(wire.get("load_id").is_none());
    assert!(wire["data"].get("info").is_none());
    assert!(wire["data"].get("config").is_none());
    for key in ["\"loop_id\":", "\"request_index\":", "\"provider_replay\":"] {
        assert!(!wire.to_string().contains(key));
    }
}
fn saved_reason(bytes: &[u8], turn: &TurnId) -> Value {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .lines()
        .find_map(|line| {
            let record: Value = serde_json::from_str(line).unwrap();
            (record["entry"]["kind"] == "turn_finished"
                && record["entry"]["data"]["turn_id"] == value(turn))
            .then(|| record["entry"]["data"]["reason"].clone())
        })
        .unwrap()
}

#[tokio::test]
async fn persisted_turn_result_remains_queryable_without_restoring_execution() {
    let mut rig = Rig::new(vec![
        tool_call(),
        answer("durable audit result 世界"),
        answer("new run only"),
    ])
    .await;
    let turn = rig.send("save this result").await;
    let original = rig.wait(&turn).await;
    assert_eq!(
        original.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    assert_eq!(
        original.data.info.as_ref().unwrap().persistence,
        PersistenceState::Persisted
    );
    assert!(original.data.saved_terminal.is_none());
    assert!(value(&original)["data"].get("saved_terminal").is_none());
    assert_eq!(
        original
            .data
            .history
            .iter()
            .filter_map(|item| item.response_index)
            .collect::<Vec<_>>(),
        [0, 0, 1]
    );
    assert_eq!(rig.counts(), (1, 2, 1));
    let before = rig.bytes();
    let reason = saved_reason(&before, &turn);
    rig.reopen().await;
    let cold_state = value(&rig.host.session_state(&rig.caller(), &rig.target).unwrap());
    rig.assert_no_execution();
    for _ in 0..3 {
        assert_saved(&rig.result(&turn).await.unwrap(), &original, &reason);
        assert_saved(&rig.wait(&turn).await, &original, &reason);
    }
    assert_eq!(
        value(&rig.host.session_state(&rig.caller(), &rig.target).unwrap()),
        cold_state
    );
    assert_eq!(rig.bytes(), before);
    assert_eq!(rig.counts(), (1, 2, 1));
    rig.load().await;
    rig.assert_no_execution();
    assert_eq!(
        rig.host
            .turn_snapshot(&rig.caller(), &rig.target, &turn)
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionTurnNotFound)
    );
    for _ in 0..3 {
        assert_saved(&rig.result(&turn).await.unwrap(), &original, &reason);
    }
    assert_eq!(rig.bytes(), before);
    assert_eq!(rig.counts(), (1, 2, 1));
    let new_turn = rig.send("only this new input may execute").await;
    let new_result = rig.wait(&new_turn).await;
    assert_ne!(new_turn, turn);
    assert_ne!(new_result.source.load_id, original.source.load_id);
    assert_eq!(
        new_result.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    let after_new = rig.bytes();
    assert_saved(&rig.result(&turn).await.unwrap(), &original, &reason);
    assert_eq!(
        value(&rig.result(&new_turn).await.unwrap()),
        value(&new_result)
    );
    assert_eq!(rig.bytes(), after_new);
    assert_eq!(rig.counts(), (2, 3, 1));
    assert_eq!(
        rig.host
            .turn_list(&rig.caller(), &rig.target)
            .unwrap()
            .data
            .len(),
        1
    );
    rig.host
        .session_close(
            &rig.caller(),
            &rig.target,
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while rig
            .host
            .session_state(&rig.caller(), &rig.target)
            .unwrap()
            .data
            .lifecycle
            != SessionLifecycle::Closed
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_saved(&rig.result(&turn).await.unwrap(), &original, &reason);
    assert_eq!(rig.bytes(), after_new);
    assert_eq!(rig.server.finish().await.len(), 3);
}

async fn partial_restart(cancel: bool) {
    let partial = "unfinished original 世界";
    let written = Arc::new(Notify::new());
    let replies = if cancel {
        vec![Reply::Hold {
            prefix: delta(partial),
            written: written.clone(),
            disconnected: Arc::new(Notify::new()),
        }]
    } else {
        vec![Reply::sse(delta(partial))]
    };
    let mut rig = Rig::new(replies).await;
    let turn = rig.send("partial input").await;
    if cancel {
        written.notified().await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if rig
                    .host
                    .turn_output_read(
                        &rig.caller(),
                        &rig.target,
                        &turn,
                        0,
                        &TextReadRequest {
                            channel: "text".into(),
                            start: 0,
                            end: partial.len() as u64,
                            max_bytes: 1000,
                        },
                    )
                    .is_ok_and(|read| read.data.text == partial)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        rig.host
            .turn_cancel(
                &rig.caller(),
                &rig.target,
                TurnCancelRequest {
                    operation_id: OperationId::new().unwrap(),
                    turn_id: turn.clone(),
                    keep_inbox: false,
                },
            )
            .unwrap();
    }
    let original = rig.wait(&turn).await;
    assert_eq!(
        original.data.info.as_ref().unwrap().outcome,
        Some(if cancel {
            TurnOutcome::Cancelled
        } else {
            TurnOutcome::Failed
        })
    );
    assert_eq!(
        original.data.info.as_ref().unwrap().persistence,
        PersistenceState::Persisted
    );
    assert_eq!(original.data.partials.len(), 1);
    assert_eq!(original.data.partials[0].response_index, 0);
    assert!(value(&original.data.partials).to_string().contains(partial));
    assert_eq!(original.data.history.len(), 1);
    let before = rig.bytes();
    let reason = saved_reason(&before, &turn);
    assert!(reason.is_string());
    rig.reopen().await;
    for loaded in [false, true] {
        if loaded {
            rig.load().await;
        }
        for _ in 0..2 {
            assert_saved(&rig.result(&turn).await.unwrap(), &original, &reason);
            assert_saved(&rig.wait(&turn).await, &original, &reason);
        }
        rig.assert_no_execution();
        assert_eq!(rig.bytes(), before);
        assert_eq!(rig.counts(), (1, 1, 0));
    }
    assert_eq!(rig.server.finish().await.len(), 1);
}
#[tokio::test]
async fn failed_partial_result_preserves_exact_saved_facts_after_restart() {
    partial_restart(false).await;
}
#[tokio::test]
async fn cancelled_partial_result_preserves_exact_saved_facts_after_restart() {
    partial_restart(true).await;
}

#[tokio::test]
async fn missing_unfinished_corrupt_and_absent_history_remain_distinct() {
    let mut rig = Rig::new(vec![answer("terminal prefix must not hide corruption")]).await;
    let turn = rig.send("real original input").await;
    rig.wait(&turn).await;
    let original = rig.bytes();
    rig.reopen().await;
    assert_eq!(
        rig.result(&TurnId::new().unwrap())
            .await
            .unwrap_err()
            .kind(),
        Some(FaultKind::SessionTurnNotFound)
    );
    // Explicit test file faults, never a production rewrite/recovery operation.
    let unfinished = String::from_utf8(original.clone())
        .unwrap()
        .lines()
        .filter(|line| !line.contains("\"kind\":\"turn_finished\""))
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    std::fs::write(rig.path(), &unfinished).unwrap();
    assert_eq!(
        rig.result(&turn).await.unwrap_err().kind(),
        Some(FaultKind::SessionTurnNotFound)
    );
    assert_eq!(rig.bytes(), unfinished.as_bytes());
    let mut corrupt = original.clone();
    corrupt.extend_from_slice(b"{incomplete trailing record");
    std::fs::write(rig.path(), &corrupt).unwrap();
    let fault = rig.result(&turn).await.unwrap_err();
    assert_eq!(fault.kind(), Some(FaultKind::SessionHistoryLoadFailed));
    assert!(value(&fault).to_string().contains("byte_offset"));
    assert_eq!(rig.bytes(), corrupt);
    std::fs::remove_file(rig.path()).unwrap();
    assert_eq!(
        rig.result(&turn).await.unwrap_err().kind(),
        Some(FaultKind::SessionHistoryLoadFailed)
    );
    assert!(!rig.path().exists());
    assert_eq!(rig.counts(), (1, 1, 0));
    rig.assert_no_execution();
    assert_eq!(rig.server.finish().await.len(), 1);
}
