//! Real Host, Runtime, HTTP provider and append-only Store. Only the loopback
//! responses and response gates are scripted; file replacement exercises the
//! production Journal's original-inode check, not a simulated disk failure.
use minicore_host_storage::testing::read_history_bytes;
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_core::{CallerContext, CompactionSource, Host, SessionSnapshot};
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, SessionClient};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, Weak, mpsc},
    time::Duration,
};
use tokio::sync::Semaphore;
use tracing::{
    Dispatch, Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

const SUMMARY: &str = "PRIVATE_COMPACTION_SUMMARY";
fn response(text: &str, gate: Option<Arc<Semaphore>>) -> Reply {
    Reply {
        status: 200,
        gate,
        body: format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
        ),
    }
}
fn replies(last: impl IntoIterator<Item = Reply>) -> Vec<Reply> {
    [
        response(&ANSWER.repeat(90), None),
        response(&ANSWER.repeat(95), None),
    ]
    .into_iter()
    .chain(last)
    .collect()
}
fn control() -> SessionControlRequest {
    SessionControlRequest {
        operation_id: OperationId::new().unwrap(),
    }
}
async fn seed(
    http: &Http,
    root: Option<&std::path::Path>,
) -> (Arc<Host>, HostClient, SessionClient) {
    let mut config = configuration(&http.endpoint);
    config.models[0].context_window = 4096;
    let host = match root {
        Some(root) => {
            Host::open_configured(root, config, |config| async { Ok(prepare(config).await) })
                .await
                .unwrap()
        }
        None => Host::memory_configured(prepare(config).await).unwrap(),
    };
    let client = HostClient::in_process(host.clone()).unwrap();
    let mut creation = create(None);
    creation.mode = if root.is_some() {
        StorageMode::Persistent
    } else {
        StorageMode::Memory
    };
    creation.config.session_defaults.execution.compaction = CompactionSettings {
        enabled: false,
        trigger_percent: 50,
        target_percent: 20,
    };
    let created = client.create_agent(creation, options()).await.unwrap();
    let session = client.session(target(&created));
    for _ in 0..2 {
        let mut message = input();
        message.text = BODY.repeat(65);
        session.send(message.clone(), options()).await.unwrap();
        assert_eq!(
            result(&session, &message.message_id)
                .await
                .data
                .info
                .as_ref()
                .unwrap()
                .outcome,
            Some(TurnOutcome::Completed)
        );
    }
    (host, client, session)
}
async fn compact(session: &SessionClient) -> CompactReceipt {
    session
        .compact(
            CompactRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap()
}
async fn state_where(
    session: &SessionClient,
    predicate: impl Fn(&SessionSnapshot) -> bool,
) -> SessionSnapshot {
    tokio::time::timeout(WAIT, async {
        loop {
            let state = session.state(options()).await.unwrap();
            if predicate(&state) {
                return state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
async fn finished(session: &SessionClient) -> SessionSnapshot {
    state_where(session, |state| {
        state
            .data
            .compaction
            .as_ref()
            .is_some_and(|activity| activity.state.finished())
    })
    .await
}
async fn utility_started(http: &Http) {
    // Consume the two seed notifications and the actual utility request. No
    // sleep is used to infer the boundary at which the journal may be swapped.
    tokio::time::timeout(WAIT, http.started.acquire_many(3))
        .await
        .unwrap()
        .unwrap()
        .forget();
}

async fn emitted_for_session(
    emitted: &EmittedDiagnostics,
    event: &str,
    session: &SessionId,
) -> Vec<Value> {
    tokio::time::timeout(
        WAIT,
        emitted.wait_for(|records| {
            records.iter().any(|record| {
                record["event"] == event && record["context"]["session_id"] == session.as_str()
            })
        }),
    )
    .await
    .expect("compaction diagnostic did not reach the production subscriber")
}

async fn enable_automatic(session: &SessionClient) {
    // Seed Turn completion confirms its history; its accepted activity save
    // must also finish before this separate configuration candidate is admitted.
    let settled = state_where(session, |state| !state.data.metadata_saving).await;
    assert!(settled.data.metadata_fault.is_none());
    let mut execution = session.info(options()).await.unwrap().config.execution;
    execution.compaction.enabled = true;
    session
        .update(
            SessionUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: SessionUpdateChanges {
                    config: SessionConfigPatch {
                        execution: Some(execution),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            },
            options(),
        )
        .await
        .unwrap();
    tokio::time::timeout(WAIT, async {
        while !session
            .info(options())
            .await
            .unwrap()
            .config
            .execution
            .compaction
            .enabled
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[derive(Clone)]
struct QueryTarget {
    host: Weak<Host>,
    caller: CallerContext,
    session: SessionRef,
}
#[derive(Default)]
struct Probe {
    target: Mutex<Option<QueryTarget>>,
    calls: Mutex<Vec<&'static str>>,
    blocked: Mutex<Vec<&'static str>>,
}
impl Probe {
    fn arm(&self, host: &Arc<Host>, caller: CallerContext, session: SessionRef) {
        *self.target.lock().unwrap() = Some(QueryTarget {
            host: Arc::downgrade(host),
            caller,
            session,
        });
    }
    fn check(&self, hook: &'static str) {
        let Some(target) = self.target.lock().unwrap().clone() else {
            return;
        };
        let Some(host) = target.host.upgrade() else {
            return;
        };
        self.calls.lock().unwrap().push(hook);
        let (send, receive) = mpsc::channel();
        // Same public reentry as a synchronous callback, bounded so a lock
        // regression fails instead of hanging the integration executable.
        std::thread::spawn(move || {
            host.info(&target.caller).unwrap();
            host.session_state(&target.caller, &target.session).unwrap();
            let _ = send.send(());
        });
        if receive.recv_timeout(Duration::from_millis(500)).is_err() {
            self.blocked.lock().unwrap().push(hook);
        }
    }
    fn disarm(&self) {
        self.target.lock().unwrap().take();
    }
}
struct CompactionEvent(bool);
impl Visit for CompactionEvent {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "record" {
            let value: Value = serde_json::from_str(value).unwrap();
            self.0 = value["component"] == "session.compaction"
                || (value["event"] == "session.history_write_failed"
                    && value["attributes"]["phase"] == "compaction");
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}
struct Reentry {
    inner: Dispatch,
    probe: Arc<Probe>,
}
impl Subscriber for Reentry {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.inner.enabled(metadata)
    }
    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        self.probe.check("new_span");
        self.inner.new_span(attributes)
    }
    fn record(&self, span: &Id, values: &Record<'_>) {
        self.inner.record(span, values);
    }
    fn record_follows_from(&self, span: &Id, follows: &Id) {
        self.inner.record_follows_from(span, follows);
    }
    fn event(&self, event: &Event<'_>) {
        let mut compaction = CompactionEvent(false);
        event.record(&mut compaction);
        if compaction.0 {
            self.probe.check("event");
        }
        self.inner.event(event);
    }
    fn enter(&self, span: &Id) {
        self.probe.check("enter");
        self.inner.enter(span);
    }
    fn exit(&self, span: &Id) {
        self.probe.check("exit");
        self.inner.exit(span);
    }
    fn clone_span(&self, span: &Id) -> Id {
        self.inner.clone_span(span)
    }
    fn try_close(&self, span: Id) -> bool {
        self.probe.check("try_close");
        self.inner.try_close(span)
    }
}
fn operation_rows<'a>(rows: &'a [Value], operation: &OperationId, event: &str) -> Vec<&'a Value> {
    rows.iter()
        .filter(|row| row["event"] == event && row["operation_id"] == operation.as_str())
        .collect()
}
fn one<'a>(rows: &'a [Value], operation: &OperationId, event: &str) -> &'a Value {
    let found = operation_rows(rows, operation, event);
    assert_eq!(found.len(), 1, "expected one {event} for {operation}");
    found[0]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_compaction_boundaries_preserve_original_saves_identities_and_lock_free_diagnostics() {
    // The same accepted utility path succeeds without a subscriber.
    let http = Http::start(replies([response(SUMMARY, None)])).await;
    let (host, _client, session) = seed(&http, None).await;
    compact(&session).await;
    assert!(matches!(
        finished(&session).await.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted {
                persistence: PersistenceState::NotApplicable,
                ..
            }
        }
    ));
    stop(&host).await;
    http.finish().await;

    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-compaction");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let (inner, guard) = minicore_host_diagnostics::prepare(logging).unwrap();
    let (inner, emitted) = observe_diagnostics(inner);
    let probe = Arc::new(Probe::default());
    tracing::subscriber::set_global_default(Reentry {
        inner,
        probe: probe.clone(),
    })
    .unwrap();

    // An actual append refusal must not finish or throw away the original
    // candidate. Resume confirms exactly that save, with no second utility.
    let gate = Arc::new(Semaphore::new(0));
    let http = Http::start(replies([response(SUMMARY, Some(gate.clone()))])).await;
    let root = directory.path().join("persistent");
    let (host, client, session) = seed(&http, Some(&root)).await;
    let connection = host.connect().unwrap();
    probe.arm(&host, connection.caller(), session.target().clone());
    let accepted = compact(&session).await;
    utility_started(&http).await;
    let rejected = session
        .compact(
            CompactRequest {
                operation_id: OperationId::new().unwrap(),
            },
            options(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, CallError::Business(fault) if fault.kind() == Some(FaultKind::SessionBusy))
    );
    let history = root
        .join("agents")
        .join(session.target().agent_id.as_str())
        .join("sessions")
        .join(session.target().session_id.as_str())
        .join("history.jsonl");
    let original = history.with_extension("original");
    let bytes = read_history_bytes(&history).unwrap();
    std::fs::rename(&history, &original).unwrap();
    std::fs::write(&history, b"").unwrap();
    gate.add_permits(1);
    let failed = state_where(&session, |state| {
        matches!(
            state.data.compaction.as_ref().map(|a| &a.state),
            Some(CompactionState::Committing {
                persistence: PersistenceState::Failed,
                ..
            })
        )
    })
    .await;
    let CompactionState::Committing {
        change,
        fault: Some(save_fault),
        ..
    } = &failed.data.compaction.as_ref().unwrap().state
    else {
        unreachable!()
    };
    let change = change.clone();
    assert_eq!(
        save_fault.kind(),
        Some(FaultKind::SessionHistoryWriteFailed)
    );
    assert!(failed.data.active_turn_id.is_none());
    let pending_events = emitted_for_session(
        &emitted,
        "session.history_write_failed",
        &session.target().session_id,
    )
    .await;
    assert!(!pending_events.iter().any(|record| {
        record["event"] == "session.compaction_finished"
            && record["context"]["operation_id"] == accepted.operation_id.as_str()
    }));
    std::fs::remove_file(&history).unwrap();
    std::fs::rename(&original, &history).unwrap();
    session.resume(control(), options()).await.unwrap();
    let saved = finished(&session).await;
    assert_eq!(
        saved.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted {
                change: change.clone(),
                persistence: PersistenceState::Persisted
            }
        }
    );
    assert_eq!(
        client
            .receipt(accepted.operation_id.clone(), options())
            .await
            .unwrap(),
        Receipt::Compact(accepted.clone())
    );
    assert_eq!(http.count(), 3);
    let after = read_history_bytes(&history).unwrap();
    assert!(after.starts_with(&bytes));
    assert_eq!(
        std::str::from_utf8(&after[bytes.len()..])
            .unwrap()
            .lines()
            .count(),
        1
    );
    probe.disarm();
    stop(&host).await;
    http.finish().await;

    // Cancel an actual running utility, and do not reinterpret the cancellation
    // Fault used internally to unblock its waiter as an ERROR outcome.
    let gate = Arc::new(Semaphore::new(0));
    let http = Http::start(replies([response(SUMMARY, Some(gate))])).await;
    let (host, _client, session) = seed(&http, None).await;
    let connection = host.connect().unwrap();
    probe.arm(&host, connection.caller(), session.target().clone());
    let cancelled = compact(&session).await;
    utility_started(&http).await;
    let cancel_operation = OperationId::new().unwrap();
    let claim = session
        .cancel_compact(
            CompactCancelRequest {
                operation_id: cancel_operation.clone(),
                target_operation_id: cancelled.operation_id.clone(),
            },
            options(),
        )
        .await
        .unwrap();
    assert!(claim.result.cancel_claimed);
    assert_eq!(
        finished(&session).await.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Cancelled
        }
    );
    let repeated = session
        .cancel_compact(
            CompactCancelRequest {
                operation_id: OperationId::new().unwrap(),
                target_operation_id: cancelled.operation_id.clone(),
            },
            options(),
        )
        .await
        .unwrap();
    assert!(!repeated.result.cancel_claimed);
    probe.disarm();
    stop(&host).await;
    http.abort().await;

    // Real HTTP 401 utility failure belongs to the compaction owner once.
    let http = Http::start(replies([Reply::auth_failure()])).await;
    let (host, _client, session) = seed(&http, None).await;
    let failed_utility = compact(&session).await;
    let CompactionState::Finished {
        result: CompactionOutcome::Failed { fault },
    } = finished(&session).await.data.compaction.unwrap().state
    else {
        panic!("utility must fail")
    };
    assert_eq!(fault.kind(), Some(FaultKind::SessionCompactionFailed));
    assert_eq!(http.count(), 3);
    stop(&host).await;
    http.finish().await;

    // Automatic work is bound to its real running Turn, never a manual
    // operation or an extra public Turn for the utility.
    let http = Http::start(replies([response(SUMMARY, None), Reply::success()])).await;
    let (host, _client, session) = seed(&http, None).await;
    enable_automatic(&session).await;
    let connection = host.connect().unwrap();
    probe.arm(&host, connection.caller(), session.target().clone());
    let message = input();
    session.send(message.clone(), options()).await.unwrap();
    let turn = result(&session, &message.message_id).await;
    assert_eq!(
        turn.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    let automatic = finished(&session).await.data.compaction.unwrap();
    assert!(
        matches!(&automatic.source, CompactionSource::Automatic { turn_id, load_id, .. }
        if turn_id == &turn.source.turn_id && load_id == turn.source.load_id.as_ref().unwrap())
    );
    assert_eq!(http.count(), 4);
    probe.disarm();
    stop(&host).await;
    http.finish().await;

    // A failed automatic utility must not become a second primary ERROR at
    // the forwarding prompt/Turn boundary. The original business Fault and
    // Runtime result remain unchanged by diagnostics.
    let http = Http::start(replies([Reply::auth_failure()])).await;
    let (host, _client, session) = seed(&http, None).await;
    enable_automatic(&session).await;
    let message = input();
    session.send(message.clone(), options()).await.unwrap();
    let automatic_failure = result(&session, &message.message_id).await;
    assert_eq!(
        automatic_failure.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert_eq!(
        automatic_failure
            .data
            .info
            .as_ref()
            .unwrap()
            .fault
            .as_ref()
            .unwrap()
            .kind(),
        Some(FaultKind::SessionCompactionFailed)
    );
    let CompactionState::Finished {
        result: CompactionOutcome::Failed {
            fault: original_fault,
        },
    } = finished(&session).await.data.compaction.unwrap().state
    else {
        panic!("automatic utility must fail")
    };
    assert_eq!(
        automatic_failure.data.info.as_ref().unwrap().fault.as_ref(),
        Some(&original_fault)
    );
    assert_eq!(
        automatic_failure
            .data
            .info
            .as_ref()
            .unwrap()
            .fault
            .as_ref()
            .unwrap()
            .diagnostic_id(),
        original_fault.diagnostic_id()
    );
    assert_eq!(http.count(), 3);
    stop(&host).await;
    http.finish().await;

    // Automatic utility's save failure is a separate original-save fact. Its
    // Turn remains failed even after the same candidate is durably confirmed.
    let gate = Arc::new(Semaphore::new(0));
    let http = Http::start(replies([response(SUMMARY, Some(gate.clone()))])).await;
    let root = directory.path().join("automatic-save");
    let (host, _client, session) = seed(&http, Some(&root)).await;
    enable_automatic(&session).await;
    let message = input();
    session.send(message.clone(), options()).await.unwrap();
    utility_started(&http).await;
    let automatic_save_turn = bound(&session, &message.message_id).await;
    let automatic_history = root
        .join("agents")
        .join(session.target().agent_id.as_str())
        .join("sessions")
        .join(session.target().session_id.as_str())
        .join("history.jsonl");
    let original = automatic_history.with_extension("original");
    std::fs::rename(&automatic_history, &original).unwrap();
    std::fs::write(&automatic_history, b"").unwrap();
    gate.add_permits(1);
    let failed_automatic_save = state_where(&session, |state| {
        matches!(
            state.data.compaction.as_ref().map(|a| &a.state),
            Some(CompactionState::Committing {
                persistence: PersistenceState::Failed,
                ..
            })
        )
    })
    .await;
    tokio::time::timeout(WAIT, async {
        loop {
            let turn = session
                .turn(automatic_save_turn.clone(), options())
                .await
                .unwrap();
            if turn.data.state.state == TurnState::Finalizing {
                assert_eq!(turn.data.state.outcome, Some(TurnOutcome::Failed));
                assert_eq!(turn.data.state.persistence, PersistenceState::Failed);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let pending_events = emitted_for_session(
        &emitted,
        "session.history_write_failed",
        &session.target().session_id,
    )
    .await;
    assert!(
        !pending_events
            .iter()
            .any(|record| record["event"] == "session.compaction_finished"
                && record["context"]["turn_id"] == automatic_save_turn.as_str())
    );
    std::fs::remove_file(&automatic_history).unwrap();
    std::fs::rename(&original, &automatic_history).unwrap();
    // Finalizing records the failure before the original driver has returned.
    // Wait only for that explicit refusal; a successful admission is never retried.
    let resume = control();
    tokio::time::timeout(WAIT, async {
        loop {
            match session.resume(resume.clone(), options()).await {
                Ok(_) => break,
                Err(CallError::Business(fault))
                    if fault.kind() == Some(FaultKind::SessionBusy)
                        && fault.message() == "original execution or save is still returning" =>
                {
                    let state = session.state(options()).await.unwrap();
                    assert_eq!(
                        state.data.active_turn_id.as_ref(),
                        Some(&automatic_save_turn)
                    );
                    assert!(state.data.owned_work.history_save);
                    assert!(!state.data.saving);
                    let compaction = state.data.compaction.as_ref().unwrap();
                    assert_eq!(
                        compaction.state,
                        failed_automatic_save
                            .data
                            .compaction
                            .as_ref()
                            .unwrap()
                            .state
                    );
                    assert!(matches!(
                        &compaction.source,
                        CompactionSource::Automatic { turn_id, .. }
                        if turn_id == &automatic_save_turn
                    ));
                    let turn = session
                        .turn(automatic_save_turn.clone(), options())
                        .await
                        .unwrap();
                    assert_eq!(turn.data.state.state, TurnState::Finalizing);
                    assert_eq!(turn.data.state.outcome, Some(TurnOutcome::Failed));
                    assert_eq!(turn.data.state.persistence, PersistenceState::Failed);
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("unexpected original-save resume failure: {error:?}"),
            }
        }
    })
    .await
    .expect("original execution must return before same-save continuation is admitted");
    let automatic_saved = session
        .wait_turn(automatic_save_turn, options())
        .await
        .unwrap();
    assert_eq!(
        automatic_saved.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Failed)
    );
    assert_eq!(
        automatic_saved.data.info.as_ref().unwrap().persistence,
        PersistenceState::Persisted
    );
    assert!(matches!(
        finished(&session).await.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted {
                persistence: PersistenceState::Persisted,
                ..
            }
        }
    ));
    assert_eq!(http.count(), 3);
    stop(&host).await;
    http.finish().await;

    assert!(
        probe.blocked.lock().unwrap().is_empty(),
        "callbacks held owner locks: {:?}",
        probe.blocked.lock().unwrap()
    );
    for hook in ["new_span", "enter", "exit", "try_close", "event"] {
        assert!(
            probe.calls.lock().unwrap().contains(&hook),
            "missing reentry hook {hook}"
        );
    }
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let automatic_errors: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["level"] == "ERROR" && row["turn_id"] == automatic_failure.source.turn_id.as_str()
        })
        .collect();
    assert_eq!(
        automatic_errors.len(),
        1,
        "forwarded automatic utility failure was logged twice: {automatic_errors:?}"
    );
    assert_eq!(automatic_errors[0]["event"], "session.compaction_finished");
    let save_errors: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["level"] == "ERROR" && row["turn_id"] == automatic_saved.source.turn_id.as_str()
        })
        .collect();
    assert_eq!(
        save_errors.len(),
        1,
        "forwarded original-save failure was logged twice: {save_errors:?}"
    );
    assert_eq!(save_errors[0]["event"], "session.history_write_failed");
    assert_eq!(save_errors[0]["fault_code"], save_fault.code());
    let saved_compaction = rows
        .iter()
        .find(|row| {
            row["event"] == "session.compaction_finished"
                && row["turn_id"] == automatic_saved.source.turn_id.as_str()
        })
        .unwrap();
    assert_eq!(saved_compaction["attributes"]["outcome"], "compacted");
    assert_eq!(saved_compaction["attributes"]["persistence"], "persisted");
    let saved_turn = rows
        .iter()
        .find(|row| {
            row["event"] == "turn.finished"
                && row["turn_id"] == automatic_saved.source.turn_id.as_str()
        })
        .unwrap();
    assert_eq!(saved_turn["attributes"]["outcome"], "failed");
    assert_eq!(saved_turn["attributes"]["persistence"], "persisted");
    let start = one(&rows, &accepted.operation_id, "session.compaction_started");
    assert_eq!(start["attributes"]["receipt_index"], 0);
    assert_eq!(
        start["attributes"]["controller_revision"],
        accepted.result.controller_revision
    );
    for row in rows.iter().filter(|row| {
        row["operation_id"] == accepted.operation_id.as_str()
            && (row["component"] == "session.compaction"
                || row["event"] == "session.history_write_failed")
    }) {
        assert_eq!(row["host_id"], accepted.host_id.as_str());
        assert_eq!(row["host_incarnation"], accepted.host_incarnation.as_str());
        assert_eq!(row["agent_id"], accepted.agent_id.as_str());
        assert_eq!(row["session_id"], accepted.session_id.as_str());
        assert!(row.get("turn_id").is_none());
    }
    let committing = one(
        &rows,
        &accepted.operation_id,
        "session.compaction_committing",
    );
    assert_eq!(committing["attributes"]["persistence"], "pending");
    let save_error = one(
        &rows,
        &accepted.operation_id,
        "session.history_write_failed",
    );
    assert_eq!(save_error["fault_code"], save_fault.code());
    assert_eq!(save_error["level"], "ERROR");
    let done = one(&rows, &accepted.operation_id, "session.compaction_finished");
    assert_eq!(done["level"], "INFO");
    assert_eq!(done["attributes"]["outcome"], "compacted");
    assert_eq!(done["attributes"]["persistence"], "persisted");
    assert_eq!(done["attributes"]["covered_items"], change.covered_items);
    assert!(done["attributes"]["duration_ms"].is_u64());
    let claim = one(
        &rows,
        &cancelled.operation_id,
        "session.compaction_cancel_claimed",
    );
    assert_eq!(
        claim["attributes"]["cancel_operation_id"],
        cancel_operation.as_str()
    );
    let cancelled_done = one(
        &rows,
        &cancelled.operation_id,
        "session.compaction_finished",
    );
    assert_eq!(cancelled_done["level"], "INFO");
    assert_eq!(cancelled_done["attributes"]["outcome"], "cancelled");
    assert!(cancelled_done.get("fault_code").is_none());
    let failed = one(
        &rows,
        &failed_utility.operation_id,
        "session.compaction_finished",
    );
    assert_eq!(failed["level"], "ERROR");
    assert_eq!(failed["fault_code"], fault.code());
    assert_eq!(
        rows.iter()
            .filter(|row| row["level"] == "ERROR"
                && row["operation_id"] == failed_utility.operation_id.as_str())
            .count(),
        1
    );
    let auto_rows: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["component"] == "session.compaction"
                && row["turn_id"] == turn.source.turn_id.as_str()
        })
        .collect();
    for event in [
        "session.compaction_started",
        "session.compaction_committing",
        "session.compaction_finished",
    ] {
        assert_eq!(
            auto_rows.iter().filter(|row| row["event"] == event).count(),
            1
        );
    }
    for row in auto_rows {
        assert_eq!(
            row["load_id"],
            turn.source.load_id.as_ref().unwrap().as_str()
        );
        assert!(row["attributes"]["loop_id"].is_string());
        assert_eq!(row["attributes"]["request_index"], 0);
        assert_eq!(row["attributes"]["response_index"], 0);
        assert!(row.get("operation_id").is_none());
    }
    for row in rows
        .iter()
        .filter(|row| row["component"] == "session.compaction")
    {
        assert!(
            row["attributes"].get("input_tokens").is_none(),
            "unknown usage must stay absent"
        );
    }
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        ERROR_BODY,
        PROVIDER_MODEL,
        SUMMARY,
        history.to_str().unwrap(),
        automatic_history.to_str().unwrap(),
    ] {
        assert!(!text.contains(private), "private payload leaked: {private}");
    }

    // A closed writer cannot prevent another real utility from completing or
    // change the original business error into success.
    let http = Http::start(replies([Reply::auth_failure()])).await;
    let (host, _client, session) = seed(&http, None).await;
    compact(&session).await;
    assert!(matches!(
        finished(&session).await.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Failed { .. }
        }
    ));
    stop(&host).await;
    http.finish().await;
}
