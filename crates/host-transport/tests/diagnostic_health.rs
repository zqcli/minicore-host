//! Actual Hosts and production resources share one process health handle.
//! This does not substitute for business-failure or Runtime logging coverage.
use minicore_host_core::{Host, HostConfig};
use minicore_host_diagnostics::{InitError, LoggingState, Options, Output, ShutdownOutcome};
use minicore_host_protocol::{
    CallOptions, HostLifecycle, HostShutdownRequest, HostShutdownStrategy, OperationId,
};
use minicore_host_resources::HostResourceBuilder;
use minicore_host_transport::{HostClient, serve_stdio};
use serde_json::json;
use std::{sync::Arc, time::Duration};

async fn host() -> Arc<Host> {
    let candidate = HostResourceBuilder::builtins()
        .unwrap()
        .prepare(HostConfig::default())
        .await
        .unwrap();
    Host::memory_configured(candidate).unwrap()
}
async fn stop(host: &Arc<Host>) {
    let control = host.process_control();
    control
        .shutdown(HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut current = control.snapshot().unwrap();
        while current.lifecycle != HostLifecycle::Stopped {
            current = control.changed(&current).await.unwrap();
        }
    })
    .await
    .unwrap();
}
fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
fn file_options(path: &std::path::Path) -> Options {
    let mut options = Options::new("reconstructed-health");
    options.output = Output::File(path.to_owned());
    options
}

#[tokio::test]
async fn hosts_only_read_shared_process_health_and_never_own_the_writer() {
    let a = host().await;
    let b = host().await;
    let unbound = host().await;
    let a_client = HostClient::in_process(Arc::clone(&a)).unwrap();
    let b_client = HostClient::in_process(Arc::clone(&b)).unwrap();
    let unknown = HostClient::in_process(Arc::clone(&unbound)).unwrap();
    let disabled = unknown.info(options()).await.unwrap();
    assert_eq!(
        serde_json::to_value(disabled.logging).unwrap(),
        json!({"status":"disabled"})
    );

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagnostics.jsonl");
    let guard = minicore_host_diagnostics::install(file_options(&path)).unwrap();
    let health = guard.health();
    assert!(a.bind_logging_health(health.clone()).is_ok());
    assert!(b.bind_logging_health(health.clone()).is_ok());
    assert!(a.bind_logging_health(health.clone()).is_err());
    let first = a_client.info(options()).await.unwrap();
    let second = b_client.info(options()).await.unwrap();
    assert_ne!(first.host_id, second.host_id);
    assert_ne!(first.host_incarnation, second.host_incarnation);
    assert_eq!(first.logging, second.logging);
    assert_eq!(first.logging.status, LoggingState::Ok);
    assert_eq!(first.logging.dropped_records, Some(0));
    assert_eq!(first.logging.write_failures, Some(0));

    // A real read-only stdio connection can deserialize the same snapshot.
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (read, write) = tokio::io::split(server);
    let server = tokio::spawn(serve_stdio(Arc::clone(&b), read, write));
    let (read, write) = tokio::io::split(client);
    let observer = HostClient::stdio(read, write, options()).await.unwrap();
    assert!(!observer.initial_info().can_operate);
    let observed = observer.info(options()).await.unwrap();
    assert_eq!(observed.logging, first.logging);
    assert_eq!(observed.name, second.name);
    assert_eq!(observed.version, second.version);

    stop(&a).await;
    a_client.close();
    drop(a_client);
    drop(a);
    assert_eq!(health.snapshot().status, LoggingState::Ok);
    assert_eq!(
        b_client.info(options()).await.unwrap().logging.status,
        LoggingState::Ok
    );
    assert!(matches!(
        minicore_host_diagnostics::prepare(file_options(&path)),
        Err(InitError::LockOutput(_))
    ));

    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let stopped_output = observer.info(options()).await.unwrap().logging;
    assert_eq!(stopped_output, health.snapshot());
    assert_eq!(stopped_output.status, LoggingState::Disabled);
    assert_eq!(stopped_output.dropped_records, Some(0));
    assert_eq!(
        unknown
            .info(options())
            .await
            .unwrap()
            .logging
            .dropped_records,
        None
    );
    let (_other_dispatch, other_guard) =
        minicore_host_diagnostics::prepare(file_options(&path)).unwrap();
    assert!(b.bind_logging_health(other_guard.health()).is_err());
    assert_eq!(
        b_client.info(options()).await.unwrap().logging,
        stopped_output
    );
    assert_eq!(
        other_guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    observer.close();
    server.await.unwrap().unwrap();
    stop(&b).await;
    stop(&unbound).await;
}
