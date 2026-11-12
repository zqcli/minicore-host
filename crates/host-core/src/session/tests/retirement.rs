use super::*;

#[test]
fn idle_closed_memory_reclaims_bodies_and_root_without_a_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(dir.path().join("root")).unwrap();
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let session = SessionController::new(
        SessionId::new().unwrap(),
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        SessionConfig::default(),
        access,
        Arc::new(Resources {
            model: model(Vec::new()),
            tools: ToolSet::default(),
        }),
        Arc::new(MemoryHistory),
        vec![HistoryItem::Summary(
            minicore_host_runtime::history::SummaryHistory {
                content: Text::new("provided memory fixture context"),
            },
        )],
        Arc::new(AtomicBool::new(true)),
    )
    .hold_root(Some(Arc::new(root)));
    let observer = session.subscribe_state(&connection.caller()).unwrap();
    let retained = session.clone();
    assert_eq!(session.history(&connection.caller()).unwrap().len(), 1);
    assert!(DataRoot::acquire(dir.path().join("root")).is_err());
    let (_, records) = session
        .begin_close_owned(OperationId::new().unwrap())
        .unwrap();
    diagnostic::emit(records);
    assert!(session.retire_closed_immediately_owned().unwrap());
    assert_eq!(
        observer.snapshot().unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert_eq!(
        retained.history(&connection.caller()).unwrap_err().kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
    assert!(retained.state.lock().unwrap().history.is_empty());
    let reopened = DataRoot::acquire(dir.path().join("root")).unwrap();
    assert!(
        !dir.path().join("root/agents").exists(),
        "memory creates no Session JSONL"
    );
    drop(reopened);
}

#[tokio::test]
async fn save_acknowledgement_does_not_replace_actual_worker_retirement() {
    let dir = tempfile::tempdir().unwrap();
    let root_path = dir.path().join("root");
    let root = DataRoot::acquire(&root_path).unwrap();
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let session = SessionController::new(
        SessionId::new().unwrap(),
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        SessionConfig::default(),
        access,
        Arc::new(Resources {
            model: model(Vec::new()),
            tools: ToolSet::default(),
        }),
        Arc::new(MemoryHistory),
        Vec::new(),
        Arc::new(AtomicBool::new(true)),
    )
    .hold_root(Some(Arc::new(root)));
    let gate = Arc::new(Semaphore::new(0));
    let worker_gate = gate.clone();
    // Explicit worker-retirement fixture, not a fabricated Runtime execution:
    // all public save/business flags are settled while the actual task is alive.
    session.state.lock().unwrap().driver_task = Some(tokio::spawn(async move {
        worker_gate.acquire().await.unwrap().forget();
    }));
    session.begin_host_shutdown_owned(true);
    assert!(session.shutdown_status_owned().quiescent);
    let owner = session.clone();
    let release = tokio::spawn(async move { owner.release_host_resources_owned().await });
    until_releasing(&session).await;
    assert!(!release.is_finished());
    assert!(DataRoot::acquire(&root_path).is_err());
    assert_eq!(
        session.pause(&connection.caller()).unwrap_err().kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
    gate.add_permits(1);
    release.await.unwrap().unwrap();
    assert!(DataRoot::acquire(&root_path).is_ok());
}
