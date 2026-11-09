use minicore_host_diagnostics::{
    Context, EventData, EventSpec, InitError, LogScalar, LogValue, LoggingState, Options, Output,
    Scope, ShutdownOutcome, install, prepare,
};
use minicore_host_protocol::{
    AgentId, Fault, FaultKind, HostId, HostIncarnation, LoadId, MessageId, OperationId, SessionId,
    ToolCallId, TurnId,
};
use serde_json::{Value, json};
use std::{
    process::Command,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::{Dispatch, Level, instrument::WithSubscriber};

const EVENT: EventSpec = EventSpec {
    level: Level::INFO,
    scope: Scope::Session,
    component: "session.test",
    event: "session.test_summary",
    message: "Controlled metadata summary",
    attributes: &[
        "count",
        "outcome",
        "values",
        "available",
        "unknown",
        "origin_turn_id",
    ],
};

fn output(
    level: Level,
) -> (
    tempfile::NamedTempFile,
    Dispatch,
    minicore_host_diagnostics::Guard,
) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut options = Options::new("diagnostics-test");
    options.service_version = Some("0.1.0");
    options.level = level;
    options.output = Output::File(file.path().to_owned());
    let (dispatch, guard) = prepare(options).unwrap();
    (file, dispatch, guard)
}

fn records(file: &tempfile::NamedTempFile) -> Vec<Value> {
    let text = std::fs::read_to_string(file.path()).unwrap();
    assert!(text.is_empty() || text.ends_with('\n'));
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn jsonl_has_actual_utc_and_only_scalar_contract_fields() {
    let before = OffsetDateTime::now_utc();
    let (file, dispatch, guard) = output(Level::INFO);
    tracing::dispatcher::with_default(&dispatch, || {
        EVENT.emit(|| {
            EventData::default()
                .attribute("count", 7_u64)
                .attribute("available", true)
                .attribute("unknown", LogScalar::Null)
                .attribute(
                    "values",
                    vec![
                        LogScalar::String("line\n你好".into()),
                        LogScalar::Number((-3).into()),
                        LogScalar::Bool(false),
                        LogScalar::Null,
                    ],
                )
        });
    });
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let after = OffsetDateTime::now_utc();
    let lines = records(&file);
    assert_eq!(lines.len(), 1);
    let line = &lines[0];
    assert_eq!(line["schema_version"], 1);
    assert_eq!(line["level"], "INFO");
    assert_eq!(line["scope"], "SESSION");
    assert_eq!(line["service"], "diagnostics-test");
    assert_eq!(line["service_version"], "0.1.0");
    assert_eq!(line["component"], EVENT.component);
    assert_eq!(line["event"], EVENT.event);
    assert_eq!(line["message"], EVENT.message);
    let timestamp = line["timestamp"].as_str().unwrap();
    assert!(timestamp.ends_with('Z'));
    let parsed = OffsetDateTime::parse(timestamp, &Rfc3339).unwrap();
    assert!((before..=after).contains(&parsed));
    assert_eq!(
        line["attributes"],
        json!({"count":7,"available":true,"unknown":null,"values":["line\n你好",-3,false,null]})
    );
    for key in [
        "host_id",
        "turn_id",
        "fault_type",
        "fault_code",
        "spans",
        "span",
    ] {
        assert!(line.get(key).is_none(), "{key}");
    }
    assert!(serde_json::from_str::<LogValue>(r#"{"body":"secret"}"#).is_err());
    assert!(serde_json::from_str::<LogValue>(r#"[["secret"]]"#).is_err());
}

#[test]
fn default_filter_runs_before_data_building_and_raw_diagnostics_are_not_adapted() {
    let (file, dispatch, guard) = output(Options::new("test").level);
    let built = AtomicUsize::new(0);
    struct SecretDebug;
    impl std::fmt::Debug for SecretDebug {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("unknown Debug fields must not be evaluated")
        }
    }
    tracing::dispatcher::with_default(&dispatch, || {
        EventSpec {
            level: Level::DEBUG,
            ..EVENT
        }
        .emit(|| {
            built.fetch_add(1, Ordering::Relaxed);
            EventData::default()
        });
        tracing::error!(target: "provider", configuration = ?SecretDebug, "SECRET raw response");
        tracing::error!(target: "minicore_diagnostics", record = "SECRET forged adapter", request = ?SecretDebug);
        EVENT.emit(|| {
            EventData::default()
                .attribute("prompt", "SECRET prompt")
                .attribute("outcome", "accepted")
        });
    });
    assert_eq!(built.load(Ordering::Relaxed), 0);
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["attributes"], json!({"outcome":"accepted"}));
    assert!(
        !std::fs::read_to_string(file.path())
            .unwrap()
            .contains("SECRET")
    );
}

#[test]
fn fault_pair_and_diagnostic_id_are_copied_without_fault_message_or_details() {
    let (file, dispatch, guard) = output(Level::TRACE);
    let fault = Fault::new(FaultKind::SessionHistoryWriteFailed, "SECRET body")
        .with_diagnostic_id("diagnostic-real");
    tracing::dispatcher::with_default(&dispatch, || {
        EventSpec {
            scope: Scope::Turn,
            level: Level::ERROR,
            ..EVENT
        }
        .emit(|| EventData::default().fault(&fault));
    });
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines[0]["fault_type"], fault.type_name());
    assert_eq!(lines[0]["fault_code"], fault.code());
    assert_eq!(lines[0]["diagnostic_id"], "diagnostic-real");
    assert!(lines[0].get("attributes").is_none());
    assert!(
        !std::fs::read_to_string(file.path())
            .unwrap()
            .contains("SECRET")
    );
    assert_eq!(fault.message(), "SECRET body");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_owned_contexts_do_not_inherit_an_ended_parents_turn() {
    let (file, dispatch, guard) = output(Level::INFO);
    let mut tasks = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..4 {
        let context = Context {
            host_id: Some(HostId::new().unwrap()),
            host_incarnation: Some(HostIncarnation::new().unwrap()),
            session_id: Some(SessionId::new().unwrap()),
            turn_id: Some(TurnId::new().unwrap()),
            ..Context::default()
        };
        expected.push(context.clone());
        let dispatch = dispatch.clone();
        tasks.push(tokio::spawn(
            async move {
                let future = context.instrument(async {
                    for _ in 0..5 {
                        tokio::task::yield_now().await;
                        EVENT.emit(EventData::default);
                    }
                    // A notification with its own actual session does not inherit
                    // the parent Turn. The relationship is explicit metadata only.
                    let child = Context {
                        turn_id: None,
                        ..context.clone()
                    };
                    let origin = context.turn_id.as_ref().unwrap().as_str().to_owned();
                    child
                        .instrument(async {
                            tokio::task::yield_now().await;
                            EVENT.emit(|| EventData::default().attribute("origin_turn_id", origin));
                        })
                        .await;
                });
                future.await;
            }
            .with_subscriber(dispatch),
        ));
    }
    for task in tasks {
        task.await.unwrap();
    }
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines.len(), 24);
    for context in expected {
        let host = context.host_id.unwrap().to_string();
        let rows: Vec<_> = lines.iter().filter(|row| row["host_id"] == host).collect();
        assert_eq!(rows.len(), 6);
        for row in &rows {
            assert_eq!(
                row["host_incarnation"],
                context.host_incarnation.as_ref().unwrap().as_str()
            );
            assert_eq!(
                row["session_id"],
                context.session_id.as_ref().unwrap().as_str()
            );
            assert!(row.get("span").is_none());
            assert!(row.get("spans").is_none());
        }
        let executed: Vec<_> = rows
            .iter()
            .filter(|row| row.get("turn_id").is_some())
            .collect();
        assert_eq!(executed.len(), 5);
        for row in executed {
            assert_eq!(row["turn_id"], context.turn_id.as_ref().unwrap().as_str());
        }
        let child = rows
            .iter()
            .find(|row| row.get("turn_id").is_none())
            .unwrap();
        assert_eq!(
            child["attributes"]["origin_turn_id"],
            context.turn_id.unwrap().as_str()
        );
    }
}

#[test]
fn warn_filter_preserves_owned_context_and_explicit_empty_context_clears_it() {
    let (file, dispatch, guard) = output(Level::WARN);
    let host = HostId::new().unwrap();
    let context = Context {
        host_id: Some(host.clone()),
        ..Context::default()
    };
    tracing::dispatcher::with_default(&dispatch, || {
        context.span().in_scope(|| {
            let filtered = tracing::debug_span!(target: "third_party", "filtered");
            filtered.in_scope(|| {
                EventSpec {
                    level: Level::WARN,
                    ..EVENT
                }
                .emit(EventData::default);
                EventSpec {
                    level: Level::WARN,
                    ..EVENT
                }
                .emit(|| EventData::default().context(Context::default()));
            });
        });
    });
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines[0]["host_id"], host.as_str());
    assert!(lines[1].get("host_id").is_none());
}

#[test]
fn file_initialization_failure_is_explicit_without_directory_or_output_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("not-created").join("secret-path.log");
    let mut options = Options::new("test");
    options.output = Output::File(missing.clone());
    let error = match prepare(options) {
        Ok(_) => panic!("must fail"),
        Err(error) => error,
    };
    assert_eq!(error, InitError::OpenOutput(std::io::ErrorKind::NotFound));
    assert!(!error.to_string().contains("secret-path"));
    assert!(!missing.parent().unwrap().exists());
    assert!(matches!(
        prepare(Options::new("  ")),
        Err(InitError::EmptyService)
    ));
}

#[test]
fn global_installation_is_once_and_shared_hosts_are_not_reconfigured() {
    const CHILD: &str = "MINICORE_DIAGNOSTICS_INSTALL_PROBE";
    if std::env::var_os(CHILD).is_some() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut options = Options::new("app");
        options.output = Output::File(file.path().to_owned());
        let guard = install(options).unwrap();
        let host_a = guard.health();
        let host_b = guard.health();
        let second = install(Options::new("other-app"));
        assert!(matches!(second, Err(InitError::SubscriberAlreadyInstalled)));
        drop(host_a);
        assert_eq!(host_b.snapshot().status, LoggingState::Ok);
        EVENT.emit(EventData::default);
        assert_eq!(
            guard.shutdown(Duration::from_secs(2)),
            ShutdownOutcome::Flushed
        );
        assert_eq!(records(&file)[0]["service"], "app");
        return;
    }
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "global_installation_is_once_and_shared_hosts_are_not_reconfigured",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(child.status.success(), "{:?}", child);
    assert!(child.stderr.is_empty());
}

#[test]
fn multiple_os_threads_use_the_same_dispatch_with_distinct_actual_identities() {
    let (file, dispatch, guard) = output(Level::INFO);
    let identities: Vec<_> = (0..3).map(|_| HostId::new().unwrap()).collect();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let threads: Vec<_> = identities
        .iter()
        .cloned()
        .map(|host| {
            let dispatch = dispatch.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    barrier.wait();
                    Context {
                        host_id: Some(host),
                        ..Context::default()
                    }
                    .span()
                    .in_scope(|| EVENT.emit(EventData::default));
                })
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines.len(), 3);
    for host in identities {
        assert_eq!(
            lines
                .iter()
                .filter(|line| line["host_id"] == host.as_str())
                .count(),
            1
        );
    }
}

#[test]
fn every_actual_identity_is_preserved_without_duplicate_or_synthetic_fields() {
    let (file, dispatch, guard) = output(Level::INFO);
    let context = Context {
        host_id: Some(HostId::new().unwrap()),
        host_incarnation: Some(HostIncarnation::new().unwrap()),
        agent_id: Some(AgentId::new().unwrap()),
        session_id: Some(SessionId::new().unwrap()),
        turn_id: Some(TurnId::new().unwrap()),
        tool_call_id: Some(ToolCallId("provider-call-exact".into())),
        load_id: Some(LoadId::new().unwrap()),
        operation_id: Some(OperationId::new().unwrap()),
        message_id: Some(MessageId::new().unwrap()),
        in_reply_to: Some(MessageId::new().unwrap()),
    };
    tracing::dispatcher::with_default(&dispatch, || {
        EVENT.emit(|| EventData::default().context(context.clone()));
    });
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    let expected = serde_json::to_value(context).unwrap();
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&lines[0][key], value);
    }
    assert_eq!(lines[0].as_object().unwrap().len(), 19);
    let text = std::fs::read_to_string(file.path()).unwrap();
    assert_eq!(text.matches("\"host_id\":").count(), 1);
}

#[test]
fn client_fault_keeps_unknown_pair_but_never_recursively_serializes_remote_details() {
    let (file, dispatch, guard) = output(Level::TRACE);
    let fault: Fault = serde_json::from_value(json!({
        "type":"TURN_FUTURE_FAILURE", "code":4901, "message":"SECRET raw upstream body",
        "details":{"authorization":"SECRET bearer token", "causes":[{"model":"SECRET text"}]}
    }))
    .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        EventSpec {
            scope: Scope::Client,
            level: Level::ERROR,
            ..EVENT
        }
        .emit(|| EventData::default().fault(&fault));
    });
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines[0]["scope"], "CLIENT");
    assert_eq!(lines[0]["fault_type"], "TURN_FUTURE_FAILURE");
    assert_eq!(lines[0]["fault_code"], 4901);
    assert!(lines[0].get("diagnostic_id").is_none());
    assert!(lines[0].get("host_id").is_none());
    assert!(
        !std::fs::read_to_string(file.path())
            .unwrap()
            .contains("SECRET")
    );
}

#[test]
fn all_levels_reject_unapproved_payload_keys_and_file_output_only_appends() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "{\"prior\":true}\n").unwrap();
    let mut options = Options::new("app");
    options.level = Level::TRACE;
    options.output = Output::File(file.path().to_owned());
    let (dispatch, guard) = prepare(options).unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        for level in [
            Level::TRACE,
            Level::DEBUG,
            Level::INFO,
            Level::WARN,
            Level::ERROR,
        ] {
            EventSpec { level, ..EVENT }.emit(|| {
                EventData::default()
                    .attribute("config", "SECRET config")
                    .attribute("prompt", "SECRET prompt")
                    .attribute("request", "SECRET request")
                    .attribute("model_text", "SECRET model")
                    .attribute("tool_script", "SECRET script")
                    .attribute("stdout", "SECRET output")
                    .attribute("authorization", "SECRET bearer")
                    .attribute("url", "https://SECRET@private.invalid")
                    .attribute("outcome", "observed")
            });
        }
    });
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let lines = records(&file);
    assert_eq!(lines.len(), 6);
    assert_eq!(lines[0], json!({"prior":true}));
    for (line, level) in lines[1..]
        .iter()
        .zip(["TRACE", "DEBUG", "INFO", "WARN", "ERROR"])
    {
        assert_eq!(line["level"], level);
        assert_eq!(line["attributes"], json!({"outcome":"observed"}));
    }
    assert!(
        !std::fs::read_to_string(file.path())
            .unwrap()
            .contains("SECRET")
    );
}
