use super::*;

#[tokio::test]
async fn review_memory_default_precheck_cannot_hold_connection_acceptance() {
    let host_id = HostId::new().unwrap();
    let incarnation = HostIncarnation::new().unwrap();
    let access = ConnectionRegistry::new(host_id.clone(), incarnation.clone());
    let connection = access.connect().unwrap();
    let caller = connection.caller();
    let resources: Arc<dyn ResourceResolver> = Arc::new(Resources {
        model: model(Vec::new()),
        tools: ToolSet::default(),
    });
    let agent = crate::agent::AgentOwner::new(
        AgentId::new().unwrap(),
        host_id,
        incarnation,
        "acceptance fixture".into(),
        AgentConfig::default(),
        Arc::clone(&access),
        resources,
    );
    let create = |make_default| {
        agent.create_session_async_owned(
            OperationId::new().unwrap(),
            SessionOptions::default(),
            None,
            make_default,
        )
    };
    let first = create(true).await.unwrap().session_id.unwrap();
    let child_id = create(false).await.unwrap().session_id.unwrap();
    let child = agent.session(&child_id).unwrap();
    let baseline = Arc::strong_count(&child);
    let held = child.state.lock().unwrap();
    let setter_agent = Arc::clone(&agent);
    let setter_caller = caller.clone();
    let child_id = child.id.clone();
    let setter =
        std::thread::spawn(move || setter_agent.set_default(&setter_caller, Some(child_id)));
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while Arc::strong_count(&child) == baseline && std::time::Instant::now() < limit {
        std::thread::yield_now();
    }
    let reached_child = Arc::strong_count(&child) > baseline;
    // This holds only the child state as a test fixture. The real set_default
    // operation must not own the connection-acceptance mutex while waiting here.
    let (revoked, observed) = std::sync::mpsc::channel();
    let closer = std::thread::spawn(move || {
        connection.close();
        let _ = revoked.send(());
    });
    let revoked_during_precheck = observed
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_ok();
    drop(held);
    closer.join().unwrap();
    let result = setter.join().unwrap();
    assert!(
        reached_child,
        "setter must retain the actual target while checking its state"
    );
    assert!(
        revoked_during_precheck,
        "child precheck must not block connection revocation"
    );
    assert_eq!(
        result.unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    let replacement = access.connect().unwrap();
    assert_eq!(
        agent
            .info(&replacement.caller())
            .unwrap()
            .default_session_id,
        Some(first)
    );
}

// This fixture injects write interruptions into the real production backend.
// It does not mock storage acknowledgement, Runtime, or the Session owner.
