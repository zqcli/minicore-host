//! Relays delay only genuine Host frames. No compaction state or event is
//! manufactured, and HTTP/SSE utility work remains with the Session owner.
mod compaction_common;
mod os_pipe;
mod wss_common;

use compaction_common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::{Value, json};
use std::{io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Semaphore, mpsc},
    task::JoinHandle,
};

struct DelayedStdio {
    client: HostClient,
    held: mpsc::UnboundedReceiver<Value>,
    release: Arc<Semaphore>,
    tasks: Vec<JoinHandle<io::Result<()>>>,
}
impl DelayedStdio {
    async fn new(host: Arc<Host>, partial: bool) -> Self {
        let (client, front) = tokio::io::duplex(4096);
        let (reader, writer) = tokio::io::split(client);
        let (mut requests, mut responses) = tokio::io::split(front);
        let (back, server) = tokio::io::duplex(4096);
        let (replies, mut outgoing) = tokio::io::split(back);
        let (input, output) = tokio::io::split(server);
        let release = Arc::new(Semaphore::new(0));
        let gate = release.clone();
        let (captured, held) = mpsc::unbounded_channel();
        let tasks = vec![
            tokio::spawn(serve_stdio(host, input, output)),
            tokio::spawn(async move {
                tokio::io::copy(&mut requests, &mut outgoing).await?;
                outgoing.shutdown().await
            }),
            tokio::spawn(async move {
                let mut replies = BufReader::new(replies);
                let mut delayed = false;
                let mut held: Option<Vec<u8>> = None;
                let mut bytes = Vec::new();
                loop {
                    // A partial line cannot be interleaved with other frames.
                    if partial && held.is_some() {
                        gate.acquire().await.unwrap().forget();
                        responses.write_all(&held.take().unwrap()).await?;
                    }
                    tokio::select! {
                        permit = gate.acquire(), if held.is_some() => {
                            permit.unwrap().forget();
                            responses.write_all(&held.take().unwrap()).await?;
                        }
                        count = replies.read_until(b'\n', &mut bytes) => {
                            if count? == 0 { return Ok(()); }
                            let response: Value = serde_json::from_slice(&bytes).unwrap();
                            if !delayed && response["result"]["kind"] == "session.compact" {
                                delayed = true;
                                if partial {
                                    assert_eq!(bytes.pop(), Some(b'\n'));
                                    responses.write_all(&bytes).await?;
                                    bytes.clear();
                                    bytes.push(b'\n');
                                }
                                held = Some(std::mem::take(&mut bytes));
                                captured.send(response).unwrap();
                            } else {
                                responses.write_all(&bytes).await?;
                                bytes.clear();
                            }
                        }
                    }
                }
            }),
        ];
        Self {
            client: HostClient::stdio(reader, writer, options()).await.unwrap(),
            held,
            release,
            tasks,
        }
    }
    async fn accepted(&mut self) -> CompactReceipt {
        let response = tokio::time::timeout(Duration::from_secs(5), self.held.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(response["id"].is_number());
        assert_ne!(response["id"], response["result"]["operation_id"]);
        serde_json::from_value(response["result"].clone()).unwrap()
    }
}
impl Drop for DelayedStdio {
    fn drop(&mut self) {
        self.client.close();
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn fixture() -> (HttpFixture, Arc<Semaphore>, Arc<Host>) {
    let gate = Arc::new(Semaphore::new(0));
    let mut replies = seed_replies();
    replies.push(gated(SUMMARY, &gate));
    let http = HttpFixture::start(replies).await;
    let host = host(&http).await;
    (http, gate, host)
}
fn start(
    session: SessionClient,
    request: CompactRequest,
    options: CallOptions,
) -> JoinHandle<Result<CompactReceipt, CallError>> {
    tokio::spawn(async move { session.compact(request, options).await })
}

#[tokio::test]
async fn actual_compaction_completion_notification_can_precede_its_unchanged_acceptance_receipt() {
    let (mut http, gate, host) = fixture().await;
    let mut relay = DelayedStdio::new(host, false).await;
    let session = seed(&relay.client, &mut http).await;
    let mut subscription = relay
        .client
        .subscribe(subscription(session.target()), options())
        .await
        .unwrap();
    let pending = start(session.clone(), compact(), options());
    let formed = relay.accepted().await;
    assert!(is_utility(&http.request().await));
    assert_eq!(formed.result.state, CompactionState::Preparing {});
    gate.add_permits(1);
    let completed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(ObservationNotification::Event(push)) = subscription.recv().await.unwrap()
                && let ObservationEvent::State(StateEvent::Session { source, data }) = push.event
                && let Some(activity) = &data.compaction
                && matches!(&activity.source, CompactionSource::Manual { operation_id } if operation_id == &formed.operation_id)
                && activity.state.finished() {
                assert_eq!(source.session_id, session.target().session_id);
                assert!(matches!(activity.state, CompactionState::Finished { result: CompactionOutcome::Compacted { .. } }));
                break data;
            }
        }
    }).await.unwrap();
    assert!(completed.controller_revision > formed.result.controller_revision);
    assert!(!pending.is_finished());
    relay.release.add_permits(1);
    assert_eq!(pending.await.unwrap().unwrap(), formed);
    retained(&relay.client, &formed).await;
    assert_eq!(session.turns(options()).await.unwrap().data.len(), 2);
    http.no_request();
}

#[tokio::test]
async fn partial_compact_response_timeout_does_not_cancel_replay_or_release_operation_right() {
    let (mut http, gate, host) = fixture().await;
    let mut relay = DelayedStdio::new(host.clone(), true).await;
    let session = seed(&relay.client, &mut http).await;
    let observer = HostClient::in_process(host).unwrap();
    let watching = observer.session(session.target().clone());
    let pending = start(
        session.clone(),
        compact(),
        CallOptions {
            wait_timeout_ms: Some(200),
        },
    );
    let formed = relay.accepted().await;
    assert!(is_utility(&http.request().await));
    assert_eq!(
        pending.await.unwrap().unwrap_err(),
        CallError::WaitTimeout {
            wait_timeout_ms: 200
        }
    );
    gate.add_permits(1);
    finished(&watching, &formed.operation_id).await;
    retained(&observer, &formed).await;
    assert_eq!(
        fault(
            watching
                .cancel_compact(cancel(formed.operation_id.clone()), options())
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    relay.release.add_permits(1);
    retained(&relay.client, &formed).await;
    assert!(relay.client.info(options()).await.unwrap().control.occupied);
    http.no_request();
}

#[tokio::test]
async fn aborted_compact_caller_leaves_owned_http_work_and_shared_connection_alive() {
    let (mut http, gate, host) = fixture().await;
    let mut relay = DelayedStdio::new(host, false).await;
    let session = seed(&relay.client, &mut http).await;
    let pending = start(session.clone(), compact(), options());
    let formed = relay.accepted().await;
    assert!(is_utility(&http.request().await));
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    gate.add_permits(1);
    let done = finished(&session, &formed.operation_id).await;
    assert!(matches!(
        done.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted { .. }
        }
    ));
    retained(&relay.client, &formed).await;
    relay.release.add_permits(1);
    assert!(relay.client.info(options()).await.is_ok());
    assert_eq!(session.turns(options()).await.unwrap().data.len(), 2);
    http.no_request();
}

#[tokio::test]
async fn stdio_disconnect_before_receipt_does_not_cancel_accepted_compaction() {
    let (mut http, gate, host) = fixture().await;
    let mut relay = DelayedStdio::new(host.clone(), false).await;
    let session = seed(&relay.client, &mut http).await;
    let observer = HostClient::in_process(host).unwrap();
    let watching = observer.session(session.target().clone());
    let pending = start(session, compact(), options());
    let formed = relay.accepted().await;
    assert!(is_utility(&http.request().await));
    relay.client.close();
    assert!(matches!(
        pending.await.unwrap(),
        Err(CallError::Connection { .. })
    ));
    gate.add_permits(1);
    finished(&watching, &formed.operation_id).await;
    retained(&observer, &formed).await;
    assert_eq!(watching.turns(options()).await.unwrap().data.len(), 2);
    http.no_request();
}

#[tokio::test]
async fn wss_disconnect_before_reading_receipt_does_not_cancel_or_replay_compaction() {
    let (mut http, gate, host) = fixture().await;
    let operator = HostClient::in_process(host.clone()).unwrap();
    let session = seed(&operator, &mut http).await;
    let target = session.target().clone();
    operator.close();
    let server = tls_server(host.clone()).await;
    let mut original = server.raw().await;
    assert_eq!(
        original.initialize(KEY).await["result"]["can_operate"],
        true
    );
    let observer = HostClient::in_process(host).unwrap();
    let watching = observer.session(target.clone());
    let request = compact();
    original.write(json!({"jsonrpc":"2.0","id":811,"method":"session.compact","params":{"target":target,"operation_id":request.operation_id}})).await;
    assert!(is_utility(&http.request().await)); // Actual owner admission, not a guessed send outcome.
    original.socket.close(None).await.unwrap();
    drop(original);
    gate.add_permits(1);
    let done = finished(&watching, &request.operation_id).await;
    assert!(matches!(
        done.data.compaction.unwrap().state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted { .. }
        }
    ));
    let Receipt::Compact(formed) = observer
        .receipt(request.operation_id, options())
        .await
        .unwrap()
    else {
        panic!("expected original receipt");
    };
    assert_eq!(formed.result.state, CompactionState::Preparing {});
    tokio::time::timeout(Duration::from_secs(5), async {
        while observer.info(options()).await.unwrap().control.occupied {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let replacement = server.client(KEY).await.unwrap();
    assert!(replacement.initial_info().can_operate);
    retained(&replacement, &formed).await;
    let cancellation = replacement
        .session(target)
        .cancel_compact(cancel(formed.operation_id), options())
        .await
        .unwrap();
    assert!(!cancellation.result.cancel_claimed);
    assert!(cancellation.result.target.state.finished());
    http.no_request();
}
