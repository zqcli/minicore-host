//! Production Host/Agent, storage and Runtime with named deterministic model fixtures.
//! Reconstructed after workspace replacement; not live-provider, TUI or a complete stage gate.
#[path = "task079/support.rs"]
mod support;
use minicore_host_core::*;
use minicore_host_protocol::*;
use serde_json::Value;
use std::os::unix::fs::PermissionsExt;
use support::*;
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn lifecycle_publication_and_runtime_keep_actual_identities_and_phases() {
    let (file, dispatch, guard) = logging();
    let (host_id, incarnation, agent, operation, stop, start, shutdown_id) = async {
        let host = Host::memory(resources()).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let mut create = request(StorageMode::Memory);
        create.initial_message = Some(input(false));
        let operation = create.operation_id.clone();
        let created = host.create_agent_async(&caller, create).await.unwrap();
        let agent = AgentRef {
            host_id: created.host_id.clone(),
            agent_id: created.agent_id.clone().unwrap(),
        };
        let session = SessionRef {
            agent_id: agent.agent_id.clone(),
            session_id: created.session_id.unwrap(),
        };
        let turn = until(|| host.turns(&caller, &session).unwrap().into_iter().next()).await;
        let finished = host.wait_turn(&caller, &session, &turn.id).await.unwrap();
        assert_eq!(finished.info.outcome, Some(TurnOutcome::Completed));
        let stop = OperationId::new().unwrap();
        host.agent_stop(
            &caller,
            &agent,
            AgentStopRequest {
                operation_id: stop.clone(),
            },
        )
        .unwrap();
        until(|| {
            (host.agent_info(&caller, &agent).unwrap().lifecycle == AgentLifecycle::Stopped)
                .then_some(())
        })
        .await;
        let start = OperationId::new().unwrap();
        host.agent_start(
            &caller,
            &agent,
            AgentStartRequest {
                operation_id: start.clone(),
            },
        )
        .unwrap();
        let shutdown_id = shutdown(&host, &caller).await;
        (
            created.host_id,
            created.host_incarnation,
            agent,
            operation,
            stop,
            start,
            shutdown_id,
        )
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    for (event, operation_id, phase) in [
        ("agent.created", &operation, "published"),
        ("session.created", &operation, "published"),
        ("agent.stop_accepted", &stop, "stopping"),
        ("agent.stopped", &stop, "stopped"),
        ("agent.started", &start, "active"),
        ("host.shutdown_accepted", &shutdown_id, "draining"),
        ("host.stopped", &shutdown_id, "stopped"),
    ] {
        let matching: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == event && row["operation_id"] == operation_id.as_str())
            .collect();
        assert_eq!(matching.len(), 1, "{event}: {rows:?}");
        let row = matching[0];
        assert_eq!(row["host_id"], host_id.as_str());
        assert_eq!(row["host_incarnation"], incarnation.as_str());
        assert_eq!(row["attributes"]["phase"], phase);
        assert_eq!(row["attributes"]["mode"], "memory");
        if event.starts_with("agent.") || event == "session.created" {
            assert_eq!(row["agent_id"], agent.agent_id.as_str());
        }
        assert!(row.get("turn_id").is_none());
    }
    assert_eq!(
        rows.iter()
            .filter(|row| row["event"] == "host.started")
            .count(),
        1
    );
}

#[tokio::test]
async fn actual_creation_filesystem_failures_have_one_owner_error_and_no_unpublished_ids() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let outcomes = async {
        let host = Host::open_persistent(directory.path(), settings(), resources())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let created = host
            .create_agent_async(&caller, request(StorageMode::Persistent))
            .await
            .unwrap();
        let agent = AgentRef {
            host_id: created.host_id.clone(),
            agent_id: created.agent_id.unwrap(),
        };
        let agents = directory.path().join("agents");
        let saved = directory.path().join("saved-agents");
        std::fs::rename(&agents, &saved).unwrap();
        std::fs::write(&agents, "private filesystem blocker").unwrap();
        assert!(std::fs::create_dir(agents.join("probe")).is_err());
        let mut create = request(StorageMode::Persistent);
        create.initial_message = Some(input(true));
        let message = create.initial_message.clone().unwrap();
        let failed_agent = host.create_agent_async(&caller, create).await.unwrap();
        let CreateStep::Failed { error: agent_fault } = &failed_agent.result.steps.agent else {
            panic!("actual Agent save must fail")
        };
        assert_eq!(agent_fault.kind(), Some(FaultKind::HostStorageUnavailable));
        assert!(failed_agent.agent_id.is_none());
        assert!(matches!(
            failed_agent.result.steps.session,
            CreateStep::NotAttempted
        ));
        std::fs::remove_file(&agents).unwrap();
        std::fs::rename(&saved, &agents).unwrap();
        let sessions = agents.join(agent.agent_id.as_str()).join("sessions");
        let saved = sessions.with_extension("saved");
        std::fs::rename(&sessions, &saved).unwrap();
        std::fs::write(&sessions, "private filesystem blocker").unwrap();
        assert!(std::fs::create_dir(sessions.join("probe")).is_err());
        let session_message = input(true);
        let failed_session = host
            .create_session_async(
                &caller,
                SessionCreate {
                    target: agent.clone(),
                    operation_id: OperationId::new().unwrap(),
                    mode: None,
                    overrides: None,
                    initial_message: Some(session_message.clone()),
                },
            )
            .await
            .unwrap();
        let CreateStep::Failed {
            error: session_fault,
        } = &failed_session.result.steps.session
        else {
            panic!("actual Session save must fail")
        };
        assert_eq!(
            session_fault.kind(),
            Some(FaultKind::SessionHistoryWriteFailed)
        );
        assert!(failed_session.session_id.is_none());
        std::fs::remove_file(&sessions).unwrap();
        std::fs::rename(&saved, &sessions).unwrap();
        // A failed creation save does not authorize a fabricated clean shutdown or retry.
        vec![
            (
                failed_agent.operation_id,
                agent_fault.clone(),
                message,
                None,
                "agent_metadata_save",
            ),
            (
                failed_session.operation_id,
                session_fault.clone(),
                session_message,
                Some(agent.agent_id),
                "session_metadata_save",
            ),
        ]
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    for (operation, fault, message, agent_id, phase) in outcomes {
        let primary: Vec<_> = rows
            .iter()
            .filter(|row| row["level"] == "ERROR" && row["operation_id"] == operation.as_str())
            .collect();
        assert_eq!(primary.len(), 1, "{rows:?}");
        let row = primary[0];
        assert_eq!(row["fault_type"], fault.type_name());
        assert_eq!(row["fault_code"], fault.code());
        assert_eq!(row["message_id"], message.message_id.as_str());
        assert_eq!(row["in_reply_to"], message.in_reply_to.unwrap().as_str());
        assert_eq!(
            row.get("agent_id"),
            agent_id
                .as_ref()
                .map(|id| Value::String(id.to_string()))
                .as_ref()
        );
        assert!(row.get("session_id").is_none());
        assert_eq!(row["attributes"]["phase"], phase);
        assert_eq!(row["attributes"]["mode"], "persistent");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_partial_creation_survives_dropped_wait_and_reports_actual_default_save_failure() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let (receipt, message) = async {
        let gate = ValidationGateFixture::new();
        let host = Host::open_persistent(directory.path(), settings(), gate.clone())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let mut create = request(StorageMode::Persistent);
        let message = input(true);
        create.initial_message = Some(message.clone());
        let operation = create.operation_id.clone();
        let owner = host.clone();
        let accepted_caller = caller.clone();
        let waiting = tokio::spawn(
            async move { owner.create_agent_async(&accepted_caller, create).await }
                .with_current_subscriber(),
        );
        gate.entered.notified().await;
        let progress = host.receipt(&caller, &operation).unwrap();
        assert!(progress.result.steps.agent.succeeded());
        assert!(progress.result.steps.session.in_progress());
        let path = directory
            .path()
            .join("agents")
            .join(progress.agent_id.as_ref().unwrap().as_str());
        // Real Session writes remain allowed, but Agent default publication is refused.
        std::fs::create_dir(path.join("sessions")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
        assert_eq!(
            std::fs::write(path.join("write-probe"), b"probe")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        waiting.abort();
        gate.release();
        let receipt = until(|| {
            let receipt = host.receipt(&caller, &operation).unwrap();
            receipt.validate_call_return().is_ok().then_some(receipt)
        })
        .await;
        assert!(receipt.result.steps.agent.succeeded());
        assert!(matches!(
            receipt.result.steps.session,
            CreateStep::Failed { .. }
        ));
        assert!(receipt.session_id.is_none());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            std::fs::read_dir(path.join("sessions"))
                .unwrap()
                .any(|entry| entry.unwrap().path().join("session.json").is_file())
        );
        (receipt, message)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    let primary: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["level"] == "ERROR" && row["operation_id"] == receipt.operation_id.as_str()
        })
        .collect();
    assert_eq!(primary.len(), 1, "{rows:?}");
    assert_eq!(primary[0]["scope"], "AGENT");
    assert_eq!(primary[0]["attributes"]["phase"], "default_metadata_save");
    assert_eq!(primary[0]["agent_id"], receipt.agent_id.unwrap().as_str());
    assert_eq!(primary[0]["message_id"], message.message_id.as_str());
    assert_eq!(
        primary[0]["in_reply_to"],
        message.in_reply_to.unwrap().as_str()
    );
    assert!(primary[0].get("session_id").is_none());
    assert!(!rows.iter().any(|row| row["event"] == "session.created"));
}

#[tokio::test]
async fn resource_refusal_preserves_fault_without_infrastructure_error() {
    let (file, dispatch, guard) = logging();
    let (operation, partial) = async {
        let host = Host::memory(resources()).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let mut rejected = request(StorageMode::Memory);
        rejected.config.session_defaults.model = "private/missing".parse().unwrap();
        let operation = rejected.operation_id.clone();
        assert_eq!(
            host.create_agent_async(&caller, rejected)
                .await
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionResourceMissing)
        );
        let mut request = request(StorageMode::Memory);
        request.default_session = Some(
            serde_json::from_value(serde_json::json!({"overrides":{"model":"private/missing"}}))
                .unwrap(),
        );
        let partial = host.create_agent_async(&caller, request).await.unwrap();
        assert!(partial.result.steps.agent.succeeded());
        assert!(matches!(
            partial.result.steps.session,
            CreateStep::Failed { .. }
        ));
        shutdown(&host, &caller).await;
        (operation, partial)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    for (op, event) in [
        (&operation, "host.resources_refused"),
        (&partial.operation_id, "agent.resources_refused"),
    ] {
        let matching: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == event && row["operation_id"] == op.as_str())
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0]["level"], "WARN");
        assert_eq!(matching[0]["fault_code"], 3403);
        assert_eq!(matching[0]["diagnostic_id"], "original-resource-diagnostic");
        assert!(matching[0].get("session_id").is_none());
    }
    assert!(!rows.iter().any(|row| row["level"] == "ERROR"));
}
