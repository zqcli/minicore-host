//! Delay only genuine server response bytes. Acceptance and publication still
//! run through serve_stdio / serve_wss and the production resource assembler.
mod os_pipe;
mod reload_common;
mod wss_common;

use minicore_host_core::Host;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use reload_common::*;
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Semaphore,
    task::JoinHandle,
};

struct DelayedStdio {
    client: HostClient,
    partial_reply: Arc<Semaphore>,
    release: Arc<Semaphore>,
    tasks: Vec<JoinHandle<io::Result<()>>>,
}
impl DelayedStdio {
    async fn new(host: Arc<Host>) -> Self {
        let (client, front) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(client);
        let (mut requests, mut responses) = tokio::io::split(front);
        let (back, server) = tokio::io::duplex(4096);
        let (replies, mut outgoing) = tokio::io::split(back);
        let (input, output) = tokio::io::split(server);
        let partial_reply = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let partial = partial_reply.clone();
        let gate = release.clone();
        let tasks = vec![
            tokio::spawn(serve_stdio(host, input, output)),
            tokio::spawn(async move {
                tokio::io::copy(&mut requests, &mut outgoing).await?;
                outgoing.shutdown().await
            }),
            tokio::spawn(async move {
                let mut replies = BufReader::new(replies);
                let mut delayed = false;
                loop {
                    let mut line = Vec::new();
                    if replies.read_until(b'\n', &mut line).await? == 0 {
                        return Ok(());
                    }
                    let response: Value = serde_json::from_slice(&line).unwrap();
                    if !delayed && response["result"]["kind"] == "host.reload" {
                        delayed = true;
                        assert_eq!(line.pop(), Some(b'\n'));
                        responses.write_all(&line).await?;
                        partial.add_permits(1);
                        gate.acquire().await.unwrap().forget();
                        responses.write_all(b"\n").await?;
                    } else {
                        responses.write_all(&line).await?;
                    }
                }
            }),
        ];
        Self {
            client: HostClient::stdio(reader, writer, options()).await.unwrap(),
            partial_reply,
            release,
            tasks,
        }
    }
}
impl Drop for DelayedStdio {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

#[tokio::test]
async fn stdio_reload_deadline_includes_partial_real_response_without_replay_or_cancellation() {
    let (host, preparer) = CredentialFixture::host(config(), true).await;
    let relay = DelayedStdio::new(host.clone()).await;
    let observer = HostClient::in_process(host).unwrap();
    let reload = preparer.request(config(), 0);
    let operation = reload.operation_id.clone();
    tokio::time::pause();
    let calling = {
        let client = relay.client.clone();
        tokio::spawn(async move {
            client
                .reload(
                    reload,
                    CallOptions {
                        wait_timeout_ms: Some(40),
                    },
                )
                .await
        })
    };
    relay.partial_reply.acquire().await.unwrap().forget();
    preparer.started().await;
    tokio::time::advance(Duration::from_millis(39)).await;
    tokio::task::yield_now().await;
    assert!(!calling.is_finished());
    tokio::time::advance(Duration::from_millis(1)).await;
    assert_eq!(
        calling.await.unwrap().unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 40
        }
    );
    tokio::time::resume();
    assert_eq!(observer.config(options()).await.unwrap().config_revision, 0);
    preparer.release.add_permits(1);
    assert_eq!(
        terminal(&observer, operation.clone()).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    relay.release.add_permits(1); // Late genuine response is consumed once.
    assert_eq!(
        terminal(&relay.client, operation).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(preparer.count(), 1);
    assert_eq!(
        relay.client.info(options()).await.unwrap().control.revision,
        1
    );
    // A timeout did not release the original connection's operation right.
    assert_eq!(
        fault(
            observer
                .reload(preparer.request(config(), 1), options())
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    relay.client.close();
}

#[tokio::test]
async fn stdio_disconnect_before_complete_receipt_keeps_the_accepted_owner_running() {
    let (host, preparer) = CredentialFixture::host(config(), true).await;
    let relay = DelayedStdio::new(host.clone()).await;
    let observer = HostClient::in_process(host.clone()).unwrap();
    let reload = preparer.request(config(), 0);
    let operation = reload.operation_id.clone();
    let calling = {
        let client = relay.client.clone();
        tokio::spawn(async move { client.reload(reload, options()).await })
    };
    relay.partial_reply.acquire().await.unwrap().forget();
    preparer.started().await;
    relay.client.close();
    assert!(matches!(
        calling.await.unwrap(),
        Err(CallError::Connection { .. })
    ));
    preparer.release.add_permits(1);
    assert_eq!(
        terminal(&observer, operation.clone()).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while observer.info(options()).await.unwrap().control.occupied {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (replacement, server) = stdio(host).await;
    assert!(replacement.initial_info().can_operate);
    assert_eq!(
        terminal(&replacement, operation).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(preparer.count(), 1);
    replacement.close();
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn wss_disconnect_during_preparation_does_not_cancel_rotation_or_replay_reload() {
    let (host, preparer) = CredentialFixture::host(config(), true).await;
    let server = tls_server(host.clone()).await;
    let mut original = server.raw().await;
    assert_eq!(
        original.initialize(KEY_A).await["result"]["can_operate"],
        true
    );
    let observer = HostClient::in_process(host).unwrap();
    let mut candidate = config();
    candidate.network.access_key_env = Some(SOURCE_B.into());
    let reload = preparer.request(candidate.clone(), 0);
    let operation = reload.operation_id.clone();
    original
        .write(json!({"jsonrpc":"2.0","id":1,"method":"host.reload","params":reload}))
        .await;
    preparer.started().await; // Accepted by the real owner, without reading a reply.
    original.socket.close(None).await.unwrap();
    drop(original);
    preparer.release.add_permits(1);
    assert_eq!(
        terminal(&observer, operation.clone()).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(
        fault(server.client(KEY_A).await.err().unwrap()).kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    let replacement = server.client(KEY_B).await.unwrap();
    assert!(replacement.initial_info().can_operate);
    assert_eq!(
        terminal(&replacement, operation).await.result,
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    assert_eq!(
        replacement.config(options()).await.unwrap().config,
        candidate
    );
    assert_eq!(preparer.count(), 1);
}
