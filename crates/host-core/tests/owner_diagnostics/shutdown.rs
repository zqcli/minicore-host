//! An already stopped Host does not start a second lifecycle transition.
use super::support::*;
use minicore_host_core::Host;
use minicore_host_protocol::*;
use tracing::instrument::WithSubscriber;

#[tokio::test]
async fn persistent_stopped_host_does_not_reclassify_or_restart_for_later_force() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let repeated = async {
        let host = Host::open_persistent(directory.path(), settings(), resources())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        shutdown(&host, &caller).await;
        let operation = OperationId::new().unwrap();
        let receipt = host
            .shutdown(
                &caller,
                HostShutdownRequest {
                    operation_id: operation.clone(),
                    strategy: HostShutdownStrategy::Force,
                },
            )
            .unwrap();
        assert_eq!(receipt.result.lifecycle, HostLifecycle::Stopped);
        assert_eq!(receipt.result.strategy, HostShutdownStrategy::Force);
        assert_eq!(
            host.receipt_get(&caller, &operation).unwrap(),
            Receipt::HostShutdown(receipt)
        );
        operation
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    assert!(
        !rows
            .iter()
            .any(|row| row["operation_id"] == repeated.as_str())
    );
    for event in ["host.shutdown_accepted", "host.stopped"] {
        let matching: Vec<_> = rows.iter().filter(|row| row["event"] == event).collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0]["attributes"]["mode"], "persistent");
    }
}
