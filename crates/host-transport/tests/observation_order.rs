//! A relay only delays genuine Host frames; it never creates business facts.
mod common;
mod observation_common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use minicore_runtime::model::ModelEvent;
use observation_common::*;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Semaphore, mpsc},
    task::JoinHandle,
};

struct Relay {
    release: Arc<Semaphore>,
    setup: mpsc::UnboundedReceiver<Value>,
    notification: mpsc::UnboundedReceiver<Value>,
    cleanup: mpsc::UnboundedReceiver<Value>,
    task: JoinHandle<()>,
    server: JoinHandle<std::io::Result<()>>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
        self.server.abort();
    }
}
async fn delayed_setup(host: Arc<Host>) -> (HostClient, Relay) {
    let (to_server, input) = tokio::net::unix::pipe::pipe().unwrap();
    let (output, from_server) = tokio::net::unix::pipe::pipe().unwrap();
    let (mut to_client, from_relay) = tokio::net::unix::pipe::pipe().unwrap();
    let server = tokio::spawn(serve_stdio(host, input, output));
    let release = Arc::new(Semaphore::new(0));
    let released = release.clone();
    let (setup_tx, setup) = mpsc::unbounded_channel();
    let (event_tx, notification) = mpsc::unbounded_channel();
    let (cleanup_tx, cleanup) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        let mut reader = BufReader::new(from_server);
        let mut bytes = Vec::new();
        let mut held: Option<Vec<u8>> = None;
        let mut used = false;
        loop {
            tokio::select! {
                permit = released.acquire(), if held.is_some() => {
                    permit.unwrap().forget();
                    to_client.write_all(&held.take().unwrap()).await.unwrap();
                }
                count = reader.read_until(b'\n', &mut bytes) => {
                    if count.unwrap() == 0 { break; }
                    let value: Value = serde_json::from_slice(&bytes).unwrap();
                    if !used && value["result"].get("subscription_id").is_some() {
                        used = true;
                        held = Some(std::mem::take(&mut bytes));
                        setup_tx.send(value).unwrap();
                    } else {
                        to_client.write_all(&bytes).await.unwrap();
                        if value["method"] == "events.push" { let _ = event_tx.send(value.clone()); }
                        if value["result"].get("ended").is_some() { let _ = cleanup_tx.send(value); }
                        bytes.clear();
                    }
                }
            }
        }
    });
    let client = HostClient::stdio(from_relay, to_server, options(2000))
        .await
        .unwrap();
    (
        client,
        Relay {
            release,
            setup,
            notification,
            cleanup,
            task,
            server,
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_host_notification_before_subscribe_response_is_buffered_without_closing_rpc() {
    let (host, events, mut started) = setup(1, None);
    let owner = HostClient::in_process(host.clone()).unwrap();
    let session = create(&owner).await;
    let (client, mut relay) = delayed_setup(host).await;
    let pending = {
        let client = client.clone();
        let target = session.target().clone();
        tokio::spawn(async move {
            client
                .subscribe(
                    request(scope(&target), &[EventCategory::Output]),
                    options(2000),
                )
                .await
        })
    };
    let baseline = relay.setup.recv().await.unwrap();
    session
        .send(input("before response"), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    events[0].send(ModelEvent::text_delta("先到🙂")).unwrap();
    let event = relay.notification.recv().await.unwrap();
    assert_eq!(
        event["params"]["subscription_id"],
        baseline["result"]["subscription_id"]
    );
    assert!(!pending.is_finished());
    relay.release.add_permits(1);
    let mut subscription = pending.await.unwrap().unwrap();
    assert!(
        matches!(model_output(&mut subscription).await, ObservationEvent::Activity(ActivityEvent::ModelOutput { data, .. }) if data.text == "先到🙂")
    );
    assert!(client.info(options(2000)).await.is_ok());
    finish(&events[0]);
    client.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timed_out_setup_is_cleaned_by_its_late_result_without_closing_shared_connection() {
    let (host, events, mut started) = setup(1, None);
    let owner = HostClient::in_process(host.clone()).unwrap();
    let session = create(&owner).await;
    let (client, mut relay) = delayed_setup(host).await;
    let pending = {
        let client = client.clone();
        let target = session.target().clone();
        tokio::spawn(async move {
            client
                .subscribe(
                    request(scope(&target), &[EventCategory::Output]),
                    options(20),
                )
                .await
        })
    };
    relay.setup.recv().await.unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(CallError::WaitTimeout { .. })
    ));
    relay.release.add_permits(1);
    let cleanup = tokio::time::timeout(Duration::from_secs(3), relay.cleanup.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cleanup["result"]["ended"], true);
    let mut fresh = client
        .subscribe(
            request(scope(session.target()), &[EventCategory::Output]),
            options(2000),
        )
        .await
        .unwrap();
    session
        .send(input("new stream"), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    events[0].send(ModelEvent::text_delta("fresh")).unwrap();
    assert!(
        matches!(model_output(&mut fresh).await, ObservationEvent::Activity(ActivityEvent::ModelOutput {data, ..}) if data.text == "fresh")
    );
    assert!(client.info(options(2000)).await.is_ok());
    finish(&events[0]);
    client.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_utf8_gap_reads_merge_late_overlaps_and_completion_before_tail() {
    let (host, events, mut started) = setup(1, None);
    let (client, server) = stdio_pipes(host).await;
    let session = create(&client).await;
    let mut sub = client
        .subscribe(
            request(
                scope(session.target()),
                &[EventCategory::Output, EventCategory::Completion],
            ),
            options(2000),
        )
        .await
        .unwrap();
    session.send(input("gaps"), options(2000)).await.unwrap();
    started.recv().await.unwrap();
    let mut deltas = Vec::new();
    for text in ["你", "好🙂", "尾"] {
        events[0].send(ModelEvent::text_delta(text)).unwrap();
        deltas.push(model_output(&mut sub).await);
    }
    finish(&events[0]);
    let completion = loop {
        let event = next(&mut sub).await;
        if matches!(
            event,
            ObservationEvent::Activity(ActivityEvent::ModelFinished { .. })
        ) {
            break event;
        }
    };
    let source = match &deltas[0] {
        ObservationEvent::Activity(ActivityEvent::ModelOutput { source, .. }) => source.clone(),
        _ => unreachable!(),
    };
    let mut buffer = OutputBuffer::new(
        OutputSource::Model {
            source: source.clone(),
        },
        "text".into(),
    );
    buffer.event(&deltas[0]).unwrap();
    buffer.event(&completion).unwrap();
    assert!(!buffer.is_complete());
    buffer.event(&deltas[2]).unwrap();
    let gap = buffer.next_read(4).unwrap();
    assert_eq!((gap.start, gap.end), (3, 10));
    let read = session
        .turn_output(
            source.turn.turn_id.clone(),
            source.response_index,
            gap,
            options(2000),
        )
        .await
        .unwrap();
    assert_eq!(
        (read.data.start, read.data.end, read.data.text.as_str()),
        (3, 6, "好")
    );
    // A genuine late event fills the same range before its read is applied.
    buffer.event(&deltas[1]).unwrap();
    buffer.model_read(&read).unwrap();
    buffer.event(&deltas[2]).unwrap();
    assert_eq!(buffer.text(), "你好🙂尾");
    assert!(buffer.is_complete());
    assert!(buffer.next_read(4).is_none());
    let invalid = session
        .turn_output(
            source.turn.turn_id,
            source.response_index,
            TextReadRequest {
                channel: "text".into(),
                start: 1,
                end: 3,
                max_bytes: 2,
            },
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(invalid, CallError::Business(fault) if fault.kind() == Some(FaultKind::SessionOutputRangeInvalid))
    );
    assert_eq!(buffer.text(), "你好🙂尾");
    client.close();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopped_os_pipe_reader_does_not_block_real_owner_completion() {
    let (host, events, mut started) = setup(1, None);
    let owner = HostClient::in_process(host.clone()).unwrap();
    let session = create(&owner).await;
    let (mut to_server, input) = tokio::net::unix::pipe::pipe().unwrap();
    let (output, from_server) = tokio::net::unix::pipe::pipe().unwrap();
    let server = tokio::spawn(serve_stdio(host, input, output));
    let mut reader = BufReader::new(from_server);
    let mut response = String::new();
    to_server
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .await
        .unwrap();
    reader.read_line(&mut response).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&response).unwrap()["result"]["can_operate"],
        false
    );
    let mut wire = serde_json::to_vec(&serde_json::json!({"jsonrpc":"2.0", "id":2, "method":"events.subscribe", "params":request(scope(session.target()), &[EventCategory::Output, EventCategory::Completion])})).unwrap();
    wire.push(b'\n');
    to_server.write_all(&wire).await.unwrap();
    response.clear();
    reader.read_line(&mut response).await.unwrap();
    assert!(
        serde_json::from_str::<Value>(&response).unwrap()["result"]
            .get("subscription_id")
            .is_some()
    );
    session
        .send(common::input("reader stalls"), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    let turn = running(&session).await;
    let expected = "slow🙂".repeat(200_000);
    events[0]
        .send(ModelEvent::text_delta(expected.clone()))
        .unwrap();
    finish(&events[0]);
    let result = session
        .wait_turn(turn.clone(), options(5000))
        .await
        .unwrap();
    assert_eq!(result.data.info.outcome, Some(TurnOutcome::Completed));
    let text = session
        .turn_output(
            turn,
            0,
            TextReadRequest {
                channel: "text".into(),
                start: 0,
                end: expected.len() as u64,
                max_bytes: expected.len() as u64,
            },
            options(5000),
        )
        .await
        .unwrap();
    assert_eq!(text.data.text, expected);
    assert!(owner.info(options(2000)).await.is_ok());
    drop(reader);
    drop(to_server);
    let _ = server.await.unwrap();
}
