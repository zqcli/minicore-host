//! Real owner acceptance and HTTP execution after an explicit fixture resource refusal.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_core::{BoundResources, Host, MessageBinding, ResourceFuture, ResourceResolver};
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use minicore_host_transport::HostClient;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
const PRIVATE_FAULT: &str = "PRIVATE_RESOURCE_PREPARATION_FAULT";
struct FixtureResources {
    model: Arc<dyn Model>,
    refuse: AtomicBool,
    fault: Fault,
}
impl ResourceResolver for FixtureResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            if self.refuse.load(Ordering::SeqCst) {
                return Err(self.fault.clone());
            }
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_refusal_preserves_original_queued_input_without_invented_turn_or_body() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-preparation");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let mut expected = Vec::new();
    for kind in [
        FaultKind::SessionResourceMissing,
        FaultKind::SessionInternalUnexpected,
    ] {
        let http = Http::start(vec![Reply::success()]).await;
        let model = Arc::new(
            OpenAiChatModel::new(
                OpenAiChatConfig::new(
                    &http.endpoint,
                    PROVIDER_MODEL,
                    ModelDescriptor::new(
                        MODEL.parse().unwrap(),
                        128_000,
                        BTreeSet::from([RuntimeReasoning::Auto]),
                        true,
                    )
                    .unwrap(),
                ),
                SecretApiKey::new(KEY).unwrap(),
            )
            .unwrap(),
        );
        let fault =
            Fault::new(kind, PRIVATE_FAULT).with_diagnostic_id("actual-resource-fixture-refusal");
        let resources = Arc::new(FixtureResources {
            model,
            refuse: AtomicBool::new(true),
            fault: fault.clone(),
        });
        let host = Host::memory(resources.clone()).unwrap();
        let client = HostClient::in_process(host.clone()).unwrap();
        let created = client.create_agent(create(None), options()).await.unwrap();
        let target = target(&created);
        let session = client.session(target.clone());
        let message = input();
        session.send(message.clone(), options()).await.unwrap();
        let blocked = tokio::time::timeout(WAIT, async {
            loop {
                let value = session.state(options()).await.unwrap();
                if value.data.block_reason.is_some() {
                    break value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(blocked.data.active_turn_id.is_none());
        assert_eq!(blocked.data.next_turn.len(), 1);
        assert_eq!(blocked.data.next_turn[0].message_id, message.message_id);
        assert!(matches!(
            session
                .message(message.message_id.clone(), options())
                .await
                .unwrap()
                .data
                .binding,
            MessageBinding::Queued
        ));
        assert_eq!(http.count(), 0);
        resources.refuse.store(false, Ordering::SeqCst);
        if kind == FaultKind::SessionResourceMissing {
            session
                .resume(
                    SessionControlRequest {
                        operation_id: OperationId::new().unwrap(),
                    },
                    options(),
                )
                .await
                .unwrap();
            let result = result(&session, &message.message_id).await;
            assert_eq!(
                result.data.info.as_ref().unwrap().outcome,
                Some(TurnOutcome::Completed)
            );
            assert_eq!(http.finish().await.len(), 1);
            stop(&host).await;
        } else {
            // Existing policy does not recover an unexpected preparation Fault.
            // This assertion must not be replaced with a new retry or fallback.
            assert!(
                session
                    .resume(
                        SessionControlRequest {
                            operation_id: OperationId::new().unwrap()
                        },
                        options()
                    )
                    .await
                    .is_err()
            );
            assert_eq!(http.count(), 0);
            host.process_control()
                .shutdown(HostShutdownRequest {
                    operation_id: OperationId::new().unwrap(),
                    strategy: HostShutdownStrategy::Force,
                })
                .unwrap();
            stop(&host).await;
            http.abort().await;
        }
        expected.push((target, fault));
    }
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (target, fault) in expected {
        let records: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "session.resource_refused"
                    && row["session_id"] == target.session_id.as_str()
            })
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["fault_code"], fault.code());
        assert_eq!(records[0]["diagnostic_id"], fault.diagnostic_id().unwrap());
        assert_eq!(
            records[0]["level"],
            if fault.kind() == Some(FaultKind::SessionResourceMissing) {
                "WARN"
            } else {
                "ERROR"
            }
        );
        assert_eq!(records[0]["attributes"]["phase"], "preparing");
        assert_eq!(records[0]["attributes"]["next_turn_len"], 1);
        assert!(records[0].get("turn_id").is_none());
        assert!(records[0].get("operation_id").is_none());
        let changes: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "session.scheduling_changed"
                    && row["session_id"] == target.session_id.as_str()
                    && row["attributes"]["reason"] == "resource_prepare_failed"
            })
            .collect();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["attributes"]["blocked"], true);
        assert_eq!(changes[0]["attributes"]["next_turn_len"], 1);
        assert_eq!(
            changes[0]["attributes"]["controller_revision"],
            records[0]["attributes"]["controller_revision"]
        );
    }
    assert_eq!(rows.iter().filter(|row| row["level"] == "ERROR").count(), 1);
    for private in [
        PRIVATE_FAULT,
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        ERROR_BODY,
        PROVIDER_MODEL,
        directory.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
}
