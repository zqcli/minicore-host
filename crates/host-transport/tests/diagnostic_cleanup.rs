//! Real Host/Runtime/HTTP with an explicitly injected cleanup-owner result.
//! These faults are fixture cleanup failures, not disk or process-wait failures.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_core::{
    BoundResources, Host, OwnerFuture, PreparedTurnBinding, ResourceFuture, ResourceResolver,
    TurnToolCleanup,
};
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_transport::HostClient;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use tracing::{
    Dispatch, Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    subscriber::Interest,
};

// Forward every span/event to the real diagnostics subscriber. This test-only
// barrier observes cleanup emission after the production subscriber has handled
// it, without reading or unlocking a log file that its writer still owns.
struct CleanupEmission {
    inner: Dispatch,
    emitted: Arc<Semaphore>,
    scheduling_emitted: Arc<Semaphore>,
}

#[derive(Default)]
struct CleanupRecord {
    failed: bool,
    blocked_scheduling: bool,
}

impl Visit for CleanupRecord {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "record" {
            let Ok(record) = serde_json::from_str::<Value>(value) else {
                return;
            };
            self.failed = record["event"] == "turn.cleanup_failed";
            self.blocked_scheduling = record["event"] == "session.scheduling_changed"
                && record["attributes"]["reason"] == "cleanup_finished"
                && record["attributes"]["blocked"] == true;
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

impl Subscriber for CleanupEmission {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        self.inner.register_callsite(metadata)
    }
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }
    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        self.inner.new_span(attributes)
    }
    fn record(&self, span: &Id, values: &Record<'_>) {
        self.inner.record(span, values);
    }
    fn record_follows_from(&self, span: &Id, follows: &Id) {
        self.inner.record_follows_from(span, follows);
    }
    fn event(&self, event: &Event<'_>) {
        self.inner.event(event);
        let mut record = CleanupRecord::default();
        event.record(&mut record);
        if record.failed {
            self.emitted.add_permits(1);
        }
        if record.blocked_scheduling {
            self.scheduling_emitted.add_permits(1);
        }
    }
    fn enter(&self, span: &Id) {
        self.inner.enter(span);
    }
    fn exit(&self, span: &Id) {
        self.inner.exit(span);
    }
    fn clone_span(&self, span: &Id) -> Id {
        self.inner.clone_span(span)
    }
    fn try_close(&self, span: Id) -> bool {
        self.inner.try_close(span)
    }
    fn current_span(&self) -> tracing_core::span::Current {
        self.inner.current_span()
    }
}

const PRIVATE_CLEANUP: &str = "PRIVATE_INJECTED_CLEANUP_FAULT_BODY";
struct FixtureCleanup {
    calls: AtomicUsize,
    confirmed: bool,
}
impl TurnToolCleanup for FixtureCleanup {
    fn finish(&self) -> OwnerFuture<'_, Vec<Fault>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let fault = Fault::new(FaultKind::TurnInternalUnexpected, PRIVATE_CLEANUP)
                .with_diagnostic_id("original-fixture-cleanup-fault");
            if self.confirmed {
                Ok(vec![fault])
            } else {
                Err(fault)
            }
        })
    }
}
struct FixtureResources {
    model: Arc<dyn Model>,
    cleanup: Arc<FixtureCleanup>,
}
impl ResourceResolver for FixtureResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "fixture requires the actual Turn binding",
            ))
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        _: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            binding.register_tool_cleanup(self.cleanup.clone())?;
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::builder().build().unwrap(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_cleanup_failures_do_not_replace_primary_or_falsely_finish_or_retry() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-cleanup");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let (dispatch, guard) = minicore_host_diagnostics::prepare(logging).unwrap();
    let emitted = Arc::new(Semaphore::new(0));
    let scheduling_emitted = Arc::new(Semaphore::new(0));
    tracing::dispatcher::set_global_default(Dispatch::new(CleanupEmission {
        inner: dispatch,
        emitted: emitted.clone(),
        scheduling_emitted: scheduling_emitted.clone(),
    }))
    .unwrap();
    let mut expected = Vec::new();
    for confirmed in [true, false] {
        let http = Http::start(vec![Reply::auth_failure()]).await;
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
        let cleanup = Arc::new(FixtureCleanup {
            calls: AtomicUsize::new(0),
            confirmed,
        });
        let host = Host::memory(Arc::new(FixtureResources {
            model,
            cleanup: cleanup.clone(),
        }))
        .unwrap();
        let client = HostClient::in_process(host.clone()).unwrap();
        let created = client.create_agent(create(None), options()).await.unwrap();
        let target = target(&created);
        let session = client.session(target.clone());
        let message = input();
        session.send(message.clone(), options()).await.unwrap();
        let turn = bound(&session, &message.message_id).await;
        let details = tokio::time::timeout(WAIT, async {
            loop {
                let value = session.turn(turn.clone(), options()).await.unwrap();
                if !value.data.state.cleanup_faults.is_empty() {
                    break value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(details.data.state.outcome, Some(TurnOutcome::Failed));
        assert_eq!(details.data.state.fault.as_ref().unwrap().code(), 4204);
        assert_eq!(details.data.state.cleanup_faults.len(), 1);
        assert_eq!(
            details.data.state.cleanup_faults[0].diagnostic_id(),
            Some("original-fixture-cleanup-fault")
        );
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), 1);
        if confirmed {
            let result = session.wait_turn(turn.clone(), options()).await.unwrap();
            assert_eq!(
                result
                    .data
                    .info
                    .as_ref()
                    .unwrap()
                    .fault
                    .as_ref()
                    .unwrap()
                    .code(),
                4204
            );
            assert_eq!(result.data.info.as_ref().unwrap().cleanup_faults.len(), 1);
            stop(&host).await;
        } else {
            assert_eq!(details.data.state.state, TurnState::Finalizing);
            assert!(
                session
                    .state(options())
                    .await
                    .unwrap()
                    .data
                    .owned_work
                    .tool_cleanup
            );
            assert!(session.turn_result(turn.clone(), options()).await.is_err());
            // The deliberately unconfirmed owner stays retained. No clean
            // Host shutdown or cleanup retry is asserted for this fixture.
        }
        assert_eq!(http.count(), 1);
        http.finish().await;
        expected.push((target, turn, confirmed, cleanup));
    }
    // Publication alone does not prove emission. The unconfirmed owner emits
    // its blocked scheduling record after cleanup_failed. Wait for both actual
    // errors and that tail event before closing and draining the real writer.
    let (errors, scheduling) = tokio::time::timeout(WAIT, async {
        tokio::try_join!(emitted.acquire_many(2), scheduling_emitted.acquire())
    })
    .await
    .expect("cleanup diagnostics did not reach the production subscriber")
    .unwrap();
    errors.forget();
    scheduling.forget();
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (target, turn, confirmed, cleanup) in expected {
        let errors: Vec<_> = rows
            .iter()
            .filter(|row| row["turn_id"] == turn.as_str() && row["level"] == "ERROR")
            .collect();
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors
                .iter()
                .filter(|row| row["event"] == "turn.failed" && row["fault_code"] == 4204)
                .count(),
            1
        );
        let records: Vec<_> = errors
            .iter()
            .filter(|row| row["event"] == "turn.cleanup_failed")
            .collect();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["fault_code"], 4999);
        assert_eq!(
            records[0]["diagnostic_id"],
            "original-fixture-cleanup-fault"
        );
        assert_eq!(records[0]["session_id"], target.session_id.as_str());
        assert_eq!(records[0]["attributes"]["cleanup_confirmed"], confirmed);
        assert_eq!(records[0]["attributes"]["outcome"], "failed");
        assert_eq!(records[0]["attributes"]["phase"], "finalizing");
        assert_eq!(
            rows.iter()
                .filter(|row| row["event"] == "turn.finished" && row["turn_id"] == turn.as_str())
                .count(),
            usize::from(confirmed)
        );
        assert_eq!(cleanup.calls.load(Ordering::SeqCst), 1);
        let changed: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == "session.scheduling_changed"
                    && row["turn_id"] == turn.as_str()
                    && row["attributes"]["reason"] == "cleanup_finished"
            })
            .collect();
        assert_eq!(changed.len(), usize::from(!confirmed));
        if !confirmed {
            assert_eq!(changed[0]["attributes"]["blocked"], true);
            assert_eq!(changed[0]["level"], "DEBUG");
        }
    }
    for private in [
        PRIVATE_CLEANUP,
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
