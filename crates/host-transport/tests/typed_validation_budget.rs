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

async fn bounded_validation_wait(at: usize) {
    let (host, resources) = fixture();
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((at, gate.clone()));
    let client = HostClient::in_process(host.clone()).unwrap();
    let released = Arc::new(AtomicBool::new(false));
    let (finished, completed) = mpsc::channel();
    let release_flag = released.clone();
    // Independent fallback release keeps the regression bounded even if a
    // synchronous callback blocks the sole async runtime thread.
    let releaser = std::thread::spawn(move || {
        let _ = completed.recv_timeout(Duration::from_millis(400));
        release_flag.store(true, Ordering::SeqCst);
        gate.release();
    });
    let request = create_request();
    let operation = request.operation_id.clone();
    let result = client.create_agent(request, options(25)).await;
    let returned_before_release = !released.load(Ordering::SeqCst);
    let _ = finished.send(());
    releaser.join().unwrap();
    assert!(
        returned_before_release,
        "the caller must stop waiting before synchronous validation finishes"
    );
    assert_eq!(
        result.unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 25
        }
    );
    assert_eq!(client.info(options(500)).await.unwrap().control.revision, 1);
    if at == 1 {
        // Preparation was not acceptance. Dropping this typed waiter does not
        // continue a request that has not yet crossed the owner boundary.
        assert!(client.agents(options(500)).await.unwrap().is_empty());
    } else {
        // The Host already owns this combination. Its Session creation keeps
        // progressing even though the client no longer waits for its receipt.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let Receipt::AgentCreate(receipt) = client
                    .receipt(operation.clone(), options(500))
                    .await
                    .unwrap()
                else {
                    panic!("wrong receipt");
                };
                if receipt.result.steps.session == CreateStep::Succeeded {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(client.agents(options(500)).await.unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn typed_deadline_returns_during_preacceptance_validation() {
    bounded_validation_wait(1).await;
}

#[tokio::test]
async fn single_thread_deadline_returns_during_preacceptance_validation() {
    bounded_validation_wait(1).await;
}

#[tokio::test]
async fn single_thread_deadline_does_not_cancel_accepted_child_validation() {
    bounded_validation_wait(2).await;
}
