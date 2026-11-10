//! Real public in-process Host routing through Runtime and production reqwest/SSE
//! adapter to a local scripted HTTP fixture. This is not a live model/TUI test.
#[path = "../../host-provider/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;
use minicore_host_core::{BoundResources, Host, HostSettings, ResourceFuture, ResourceResolver};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{
    ExecutionConfig, LoopOptions, Text,
    model::{Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::json;
use std::{collections::BTreeSet, sync::Arc};
use support::{Fixture, Reply};
struct ConfiguredResources {
    model: Arc<OpenAiChatModel>,
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
        Box::pin(async move {
            self.validate(config)?;
            let prompt = Text::new(format!(
                "Host framework rules\n{}",
                config.role.instructions
            ));
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(Some(prompt))),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
    )
}

// Audit-only probe: real Runtime/provider over loopback HTTP, real persistence,
// controlled shutdown, same data-root reopen, no old execution reconstruction.
#[tokio::test]
async fn persisted_turn_result_remains_queryable_without_restoring_execution() {
    let directory = tempfile::tempdir().unwrap();
    let server = Fixture::start(vec![Reply::sse(answer("durable audit result 世界"))]).await;
    let descriptor = ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([RuntimeReasoning::Auto]),
        true,
    )
    .unwrap();
    let provider = Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(&server.endpoint, "fixture-model", descriptor),
            SecretApiKey::new("fixture-not-a-real-secret").unwrap(),
        )
        .unwrap(),
    );
    let resources: Arc<dyn ResourceResolver> = Arc::new(ConfiguredResources { model: provider });
    let host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "audit".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/chat".parse().unwrap();
    config.session_defaults.execution.compaction.enabled = false;
    let created = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "audit".into(),
                mode: StorageMode::Persistent,
                config,
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
    host.send(
        &caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "save this result".into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(turn) = host.turns(&caller, &target).unwrap().last() {
                break host
                    .wait_turn_snapshot(&caller, &target, &turn.id)
                    .await
                    .unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(terminal.data.info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(terminal.data.info.persistence, PersistenceState::Persisted);
    let turn = terminal.source.turn_id.clone();
    let old_info = host.info(&caller).unwrap();
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let history_path = directory
        .path()
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str())
        .join("history.jsonl");
    let before = std::fs::read(&history_path).unwrap();
    let records: Vec<minicore_host_storage::HistoryRecord> = String::from_utf8(before.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records.iter().any(|record| matches!(&record.entry,
        minicore_host_storage::HistoryEntry::TurnFinished { turn_id, outcome, .. }
        if turn_id == &turn && *outcome == TurnOutcome::Completed)));
    println!(
        "before restart: completed/persisted, valid saved terminal matches TurnId; actual JSONL records={}",
        records.len()
    );
    let requests = server.finish().await;
    assert_eq!(requests.len(), 1);
    connection.close();
    drop(caller);
    drop(connection);
    drop(host);
    let reopened = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "ignored".into(),
        },
        resources,
    )
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    let now = reopened.info(&caller).unwrap();
    assert_eq!(now.host_id, old_info.host_id);
    assert_ne!(now.host_incarnation, old_info.host_incarnation);
    let cold_history = reopened
        .history_read(
            &caller,
            &target,
            &HistoryReadRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&cold_history)
            .unwrap()
            .contains("durable audit result")
    );
    let cold = reopened.turn_result_snapshot(&caller, &target, &turn);
    println!(
        "cold history is readable; cold turn.result={:?}; cold turn.list count={}",
        cold.as_ref()
            .map(|_| "result")
            .map_err(|f| (f.type_name(), f.code())),
        reopened.turn_list(&caller, &target).unwrap().data.len()
    );
    reopened
        .session_load(
            &caller,
            &target,
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = reopened.session_state(&caller, &target).unwrap();
            if state.data.residency == Residency::Loaded {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let loaded = reopened.turn_result_snapshot(&caller, &target, &turn);
    println!(
        "after explicit load turn.result={:?}; turn.list count={}",
        loaded
            .as_ref()
            .map(|_| "result")
            .map_err(|f| (f.type_name(), f.code())),
        reopened.turn_list(&caller, &target).unwrap().data.len()
    );
    assert_eq!(std::fs::read(&history_path).unwrap(), before);
    assert!(
        reopened
            .session_state(&caller, &target)
            .unwrap()
            .data
            .active_turn_id
            .is_none()
    );
    assert!(
        loaded.is_ok(),
        "SPEC §8 turn.result requires the actual saved result; loading history must not restore execution, but must not discard durable result facts"
    );
}
