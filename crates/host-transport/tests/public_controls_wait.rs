//! A slow validation callback is preparation, not accepted metadata work.
mod common;
use common::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

async fn update_wait_budget(agent: bool) {
    let (host, resources) = fixture();
    let client = HostClient::in_process(host).unwrap();
    let session = create(&client).await;
    let target = AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: session.target().agent_id.clone(),
    };
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((
        resources.validate_calls.load(Ordering::SeqCst) + 1,
        gate.clone(),
    ));
    let released = Arc::new(AtomicBool::new(false));
    let release_flag = released.clone();
    let (returned, waiting) = mpsc::channel();
    let release = std::thread::spawn(move || {
        // The fallback bounds a regression on a single async runtime thread.
        let _ = waiting.recv_timeout(Duration::from_millis(500));
        release_flag.store(true, Ordering::SeqCst);
        gate.release();
    });
    let operation = OperationId::new().unwrap();
    let result = if agent {
        client
            .update_agent(
                target.clone(),
                AgentUpdateRequest {
                    operation_id: operation.clone(),
                    expected_config_revision: 0,
                    changes: AgentUpdateChanges {
                        name: Some("must not commit".into()),
                        config: Default::default(),
                    },
                },
                options(25),
            )
            .await
            .map(|_| ())
    } else {
        session
            .update(
                SessionUpdateRequest {
                    operation_id: operation.clone(),
                    expected_config_revision: 0,
                    changes: SessionUpdateChanges {
                        title: NullablePatch::Set("must not commit".into()),
                        config: Default::default(),
                    },
                },
                options(25),
            )
            .await
            .map(|_| ())
    };
    let prompt = !released.load(Ordering::SeqCst);
    let _ = returned.send(());
    release.join().unwrap();
    assert!(
        prompt,
        "call deadline must return while pure validation remains gated"
    );
    assert_eq!(
        result.unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 25
        }
    );
    // Read-only calls observe no owner acceptance or candidate publication.
    let original_revision = session.info(options(2000)).await.unwrap().config_revision;
    assert_eq!(original_revision, 0);
    assert_eq!(
        client
            .agent(target, options(2000))
            .await
            .unwrap()
            .config_revision,
        0
    );
    assert!(
        matches!(client.receipt(operation, options(2000)).await.unwrap_err(), CallError::Business(f) if f.kind() == Some(FaultKind::HostReceiptNotFound))
    );
    client.close();
}
#[tokio::test]
async fn single_thread_agent_update_wait_ends_before_unaccepted_validation() {
    update_wait_budget(true).await;
}
#[tokio::test]
async fn single_thread_session_update_wait_ends_before_unaccepted_validation() {
    update_wait_budget(false).await;
}
