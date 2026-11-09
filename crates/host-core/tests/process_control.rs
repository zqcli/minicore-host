//! Trusted process supervision, using real Host owners and real root leases.
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_storage::DataRoot;
use std::{sync::Arc, time::Duration};

struct UnusedResources;
impl ResourceResolver for UnusedResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "unused test resources",
            ))
        })
    }
}
fn request(strategy: HostShutdownStrategy) -> HostShutdownRequest {
    HostShutdownRequest {
        operation_id: OperationId::new().unwrap(),
        strategy,
    }
}
fn creation() -> AgentCreate {
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "Process fixture".into(),
        mode: StorageMode::Persistent,
        config: AgentConfig::default(),
        default_session: None,
        initial_message: None,
    }
}
async fn host(path: &std::path::Path) -> Arc<Host> {
    Host::open_persistent(
        path,
        HostSettings {
            name: "Process fixture".into(),
        },
        Arc::new(UnusedResources),
    )
    .await
    .unwrap()
}
async fn stopped(control: &HostProcessControl) {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut state = control.snapshot().unwrap();
        while state.lifecycle != HostLifecycle::Stopped {
            state = control.changed(&state).await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn process_capability_never_claims_operator_and_external_shutdown_stays_gated() {
    let dir = tempfile::tempdir().unwrap();
    let host = host(dir.path()).await;
    let process = host.process_control();
    let initial = process.snapshot().unwrap();
    let first = host.connect().unwrap();
    assert!(first.initial_info().can_operate);
    assert_eq!(first.initial_info().control.revision, 1);
    assert_eq!(initial.host_id, first.initial_info().host_id);
    let observer = host.connect().unwrap();
    assert_eq!(
        host.shutdown(&observer.caller(), request(HostShutdownStrategy::Force))
            .unwrap_err()
            .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    let accepted = process
        .shutdown(request(HostShutdownStrategy::Graceful))
        .unwrap();
    assert_eq!(accepted.result.strategy, HostShutdownStrategy::Graceful);
    stopped(&process).await;
    assert_eq!(host.info(&first.caller()).unwrap().control.revision, 1);
    let root = DataRoot::acquire(dir.path()).unwrap();
    drop(root);
    drop(host);
    assert_eq!(
        process.snapshot().unwrap_err().kind(),
        Some(FaultKind::HostLifecycleUnavailable)
    );
}

#[tokio::test]
async fn process_handle_is_weak_and_cannot_keep_an_uncontrolled_idle_host_alive() {
    let dir = tempfile::tempdir().unwrap();
    let host = host(dir.path()).await;
    let control = host.process_control();
    assert!(DataRoot::acquire(dir.path()).is_err());
    drop(host);
    assert!(control.snapshot().is_err());
    assert!(
        control
            .shutdown(request(HostShutdownStrategy::Force))
            .is_err()
    );
    assert!(DataRoot::acquire(dir.path()).is_ok());
}

#[tokio::test]
async fn cancelled_wait_and_repeated_graceful_preserve_paused_backlog_until_explicit_force() {
    let dir = tempfile::tempdir().unwrap();
    let host = host(dir.path()).await;
    let process = host.process_control();
    let client = host.connect().unwrap();
    let receipt = host
        .create_agent_async(&client.caller(), creation())
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    host.pause(&client.caller(), &target).unwrap();
    host.send(
        &client.caller(),
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "queued fixture input".into(),
                in_reply_to: None,
            },
            target,
        ),
    )
    .unwrap();
    let initial = process.snapshot().unwrap();
    let mut waiting = Box::pin(process.changed(&initial));
    assert!(futures_util::poll!(&mut waiting).is_pending());
    drop(waiting);
    process
        .shutdown(request(HostShutdownStrategy::Graceful))
        .unwrap();
    let draining = process.changed(&initial).await.unwrap();
    assert_eq!(draining.lifecycle, HostLifecycle::Draining);
    assert!(DataRoot::acquire(dir.path()).is_err());
    for _ in 0..3 {
        let repeated = process
            .shutdown(request(HostShutdownStrategy::Graceful))
            .unwrap();
        assert_eq!(repeated.result.lifecycle, HostLifecycle::Draining);
        assert_eq!(repeated.result.strategy, HostShutdownStrategy::Graceful);
        tokio::task::yield_now().await;
    }
    process
        .shutdown(request(HostShutdownStrategy::Force))
        .unwrap();
    stopped(&process).await;
    let repeated = process
        .shutdown(request(HostShutdownStrategy::Graceful))
        .unwrap();
    assert_eq!(repeated.result.lifecycle, HostLifecycle::Stopped);
    assert_eq!(repeated.result.strategy, HostShutdownStrategy::Force);
    assert!(DataRoot::acquire(dir.path()).is_ok());
}

#[tokio::test]
async fn process_wait_reports_original_save_fault_without_claiming_stopped_or_releasing_root() {
    let dir = tempfile::tempdir().unwrap();
    let host = host(dir.path()).await;
    let process = host.process_control();
    let client = host.connect().unwrap();
    std::fs::write(dir.path().join("agents"), b"controlled storage obstruction").unwrap();
    let result = host
        .create_agent_async(&client.caller(), creation())
        .await
        .unwrap();
    assert!(!result.result.steps.agent.succeeded());
    process
        .shutdown(request(HostShutdownStrategy::Force))
        .unwrap();
    let blocked = tokio::time::timeout(Duration::from_secs(3), async {
        let mut current = process.snapshot().unwrap();
        while current.shutdown_fault.is_none() {
            current = process.changed(&current).await.unwrap();
        }
        current
    })
    .await
    .unwrap();
    assert_eq!(blocked.lifecycle, HostLifecycle::Stopping);
    std::fs::remove_file(dir.path().join("agents")).unwrap();
    let repeated = process
        .shutdown(request(HostShutdownStrategy::Force))
        .unwrap();
    assert_eq!(repeated.result.lifecycle, HostLifecycle::Stopping);
    assert!(DataRoot::acquire(dir.path()).is_err());
    assert_eq!(
        process.snapshot().unwrap().shutdown_fault,
        blocked.shutdown_fault
    );
}
