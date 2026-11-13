//! Actual Host/Runtime owners over typed calls, OS pipes and loopback TLS.
mod common;
mod observation_common;
mod os_pipe;
mod wss_common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::model::ModelEvent;
use minicore_host_transport::*;
use observation_common::*;
use std::{sync::Arc, time::Duration};

enum ServerGuard {
    Local,
    Pipes(tokio::task::JoinHandle<std::io::Result<()>>),
    Wss { _server: wss_common::Server },
}
impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Self::Pipes(task) = self {
            task.abort();
        }
    }
}
async fn dial(host: Arc<Host>, kind: u8) -> (HostClient, ServerGuard) {
    match kind {
        0 => (HostClient::in_process(host).unwrap(), ServerGuard::Local),
        1 => {
            let (client, task) = stdio_pipes(host).await;
            (client, ServerGuard::Pipes(task))
        }
        _ => {
            let server = wss_common::Server::start(host).await;
            let client = server.client(wss_common::FIRST).await.unwrap();
            (client, ServerGuard::Wss { _server: server })
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn typed_pipes_and_tls_share_real_scoped_push_snapshots_and_history() {
    for kind in 0..3 {
        let (host, events, mut started) = setup(2, None);
        let (client, guard) = dial(host.clone(), kind).await;
        let mut all = client
            .subscribe(
                request(
                    SubscriptionScope::Host {
                        host_id: client.initial_info().host_id.clone(),
                    },
                    &[
                        EventCategory::State,
                        EventCategory::Output,
                        EventCategory::Completion,
                    ],
                ),
                options(2000),
            )
            .await
            .unwrap();
        let session = create(&client).await;
        let mut selected = client
            .subscribe(
                request(
                    scope(session.target()),
                    &[EventCategory::Output, EventCategory::Completion],
                ),
                options(2000),
            )
            .await
            .unwrap();
        assert!(selected.result().initial_states.is_empty());
        session
            .send(input("真实输入"), options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        events[0].send(ModelEvent::text_delta("你a🙂tail")).unwrap();
        finish(&events[0]);
        let source = match model_output(&mut selected).await {
            ObservationEvent::Activity(ActivityEvent::ModelOutput { source, data }) => {
                assert_eq!(data.text, "你a🙂tail");
                source
            }
            _ => unreachable!(),
        };
        let end = turn_finished(&mut selected).await;
        assert_eq!(source.turn, end);
        let snapshot = session
            .turn(end.turn_id.clone(), options(2000))
            .await
            .unwrap();
        assert_eq!(snapshot.source, end);
        assert_eq!(snapshot.data.state.state, TurnState::Finished);
        let baseline = client
            .subscribe(
                request(
                    SubscriptionScope::Turn {
                        session_id: session.target().session_id.clone(),
                        turn_id: end.turn_id.clone(),
                    },
                    &[EventCategory::State, EventCategory::Output],
                ),
                options(2000),
            )
            .await
            .unwrap();
        let state = baseline
            .result()
            .initial_states
            .iter()
            .find_map(|s| {
                if let StateEvent::Turn { source, data } = s {
                    Some((source, data))
                } else {
                    None
                }
            })
            .unwrap();
        assert!(state.1.runner_revision >= snapshot.data.state.runner_revision);
        let mut expected = snapshot.data.state.clone();
        expected.runner_revision = state.1.runner_revision;
        assert_eq!(
            serde_json::to_value(state.1).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(state.0, &snapshot.source);
        let mut buffer = OutputBuffer::new(OutputSource::Model { source }, "text".into());
        assert!(
            buffer.position(
                baseline
                    .result()
                    .output_positions
                    .iter()
                    .find(|position| position.channel == "text")
                    .unwrap()
            )
        );
        while let Some(read) = buffer.next_read(2) {
            let text = session
                .turn_output(end.turn_id.clone(), 0, read, options(2000))
                .await
                .unwrap();
            buffer.model_read(&text).unwrap();
        }
        assert_eq!(buffer.text(), "你a🙂tail");
        let page = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor: None,
                    limit: 1,
                },
                options(2000),
            )
            .await
            .unwrap();
        assert!(
            matches!(&page.data.items[0].item, HistoryItemData::User { text, .. } if text == "真实输入")
        );
        let page = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor: page.data.next_cursor,
                    limit: 1,
                },
                options(2000),
            )
            .await
            .unwrap();
        assert!(matches!(
            page.data.items[0].item,
            HistoryItemData::Assistant { .. }
        ));
        assert!(page.data.next_cursor.is_none());
        let old_id = selected.result().subscription_id.clone();
        drop(selected);
        let _ = client
            .unsubscribe(
                UnsubscribeRequest {
                    subscription_id: old_id,
                },
                options(2000),
            )
            .await
            .unwrap();
        // The existing Host stream includes a Session created after setup.
        let new_session = client
            .create_session(
                SessionCreate {
                    operation_id: OperationId::new().unwrap(),
                    target: AgentRef {
                        host_id: client.initial_info().host_id.clone(),
                        agent_id: session.target().agent_id.clone(),
                    },
                    mode: None,
                    overrides: None,
                    initial_message: None,
                },
                options(2000),
            )
            .await
            .unwrap();
        let newer = client.session(SessionRef {
            agent_id: session.target().agent_id.clone(),
            session_id: new_session.session_id.unwrap(),
        });
        newer.send(input("next turn"), options(2000)).await.unwrap();
        started.recv().await.unwrap();
        events[1].send(ModelEvent::text_delta("second")).unwrap();
        finish(&events[1]);
        let first_global = model_output(&mut all).await;
        assert!(
            matches!(first_global, ObservationEvent::Activity(ActivityEvent::ModelOutput {source, ..}) if source.turn == end)
        );
        let second_global = model_output(&mut all).await;
        assert!(
            matches!(second_global, ObservationEvent::Activity(ActivityEvent::ModelOutput {source, data}) if source.turn.turn_id != end.turn_id && data.text == "second")
        );
        turn_finished(&mut all).await;
        assert!(client.info(options(2000)).await.is_ok());
        client.close();
        drop(guard);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readonly_observers_only_cancel_their_own_stream_and_source_can_be_reclaimed() {
    for kind in 0..3 {
        let (host, events, mut started) = setup(1, None);
        let owner = HostClient::in_process(host.clone()).unwrap();
        let session = create(&owner).await;
        let (reader, _guard) = dial(host.clone(), kind).await;
        assert!(!reader.initial_info().can_operate);
        let observer = reader.session(session.target().clone());
        let mut first = reader
            .subscribe(
                request(scope(session.target()), &[EventCategory::Completion]),
                options(2000),
            )
            .await
            .unwrap();
        let id = first.result().subscription_id.clone();
        let denied = owner
            .unsubscribe(
                UnsubscribeRequest {
                    subscription_id: id.clone(),
                },
                options(2000),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(denied, CallError::Business(f) if f.kind() == Some(FaultKind::HostAccessInvalid))
        );
        assert!(
            reader
                .unsubscribe(
                    UnsubscribeRequest {
                        subscription_id: id.clone()
                    },
                    options(2000)
                )
                .await
                .unwrap()
                .ended
        );
        assert!(first.recv().await.unwrap().is_none());
        let mut second = reader
            .subscribe(
                request(scope(session.target()), &[EventCategory::Completion]),
                options(2000),
            )
            .await
            .unwrap();
        assert!(
            !reader
                .unsubscribe(
                    UnsubscribeRequest {
                        subscription_id: id
                    },
                    options(2000)
                )
                .await
                .unwrap()
                .ended
        );
        drop(first);
        session
            .send(input("survive observer"), options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        events[0].send(ModelEvent::text_delta("kept")).unwrap();
        finish(&events[0]);
        let turn = turn_finished(&mut second).await;
        let read = TextReadRequest {
            channel: "text".into(),
            start: 0,
            end: 4,
            max_bytes: 4,
        };
        assert_eq!(
            observer
                .turn_output(turn.turn_id.clone(), 0, read.clone(), options(2000))
                .await
                .unwrap()
                .data
                .text,
            "kept"
        );
        let keep_observing = reader
            .subscribe(
                request(scope(session.target()), &[EventCategory::Output]),
                options(2000),
            )
            .await
            .unwrap();
        session
            .close(
                SessionCloseRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(2000),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if observer.info(options(2000)).await.is_err() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            observer
                .turn_output(turn.turn_id, 0, read, options(2000))
                .await
                .is_err()
        );
        drop(keep_observing);
        assert!(reader.info(options(2000)).await.is_ok());
        reader.close();
    }
}

#[tokio::test]
async fn established_stream_has_no_thirty_second_deadline() {
    let (host, events, mut started) = setup(1, None);
    let (client, _server) = stdio_pipes(host).await;
    let session = create(&client).await;
    let mut sub = client
        .subscribe(
            request(scope(session.target()), &[EventCategory::Output]),
            CallOptions::default(),
        )
        .await
        .unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    session
        .send(input("after thirty seconds"), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    events[0]
        .send(ModelEvent::text_delta("still live"))
        .unwrap();
    assert!(
        matches!(model_output(&mut sub).await, ObservationEvent::Activity(ActivityEvent::ModelOutput { data, .. }) if data.text == "still live")
    );
    finish(&events[0]);
    client.close();
}
