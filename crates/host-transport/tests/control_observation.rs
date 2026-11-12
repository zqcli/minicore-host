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
use std::time::Duration;

async fn control(subscription: &mut ClientSubscription, occupied: bool) -> ControlStateData {
    loop {
        if let ObservationEvent::State(StateEvent::Host { data, .. }) = next(subscription).await
            && data.control.occupied == occupied
        {
            return data.control;
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_disconnect_and_key_rotation_publish_control_without_promoting_readers_or_stopping_work()
 {
    let (host, events, mut started) = setup(1, None);
    let server = wss_common::Server::start(host.clone()).await;
    let operator = server.client(wss_common::FIRST).await.unwrap();
    let network_reader = server.client(wss_common::FIRST).await.unwrap();
    let local_reader = HostClient::in_process(host.clone()).unwrap();
    assert!(!network_reader.initial_info().can_operate && !local_reader.initial_info().can_operate);
    let scope = SubscriptionScope::Host {
        host_id: local_reader.initial_info().host_id.clone(),
    };
    let mut local = local_reader
        .subscribe(
            request(
                scope.clone(),
                &[EventCategory::State, EventCategory::Completion],
            ),
            options(2000),
        )
        .await
        .unwrap();
    let mut network = network_reader
        .subscribe(request(scope, &[EventCategory::State]), options(2000))
        .await
        .unwrap();
    let session = create(&operator).await;
    session
        .send(input("survive rotation"), options(2000))
        .await
        .unwrap();
    started.recv().await.unwrap();
    let turn = running(&session).await;
    let old_revision = operator.initial_info().control.revision;
    host.configure_network_access(Some(wss_common::SECOND.into()))
        .unwrap();
    let released = control(&mut local, false).await;
    assert!(released.revision > old_revision);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if network.recv().await.is_err() {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(operator.info(options(2000)).await.is_err());
    assert!(server.client(wss_common::FIRST).await.is_err());
    assert!(!local_reader.initial_info().can_operate);
    let denied = local_reader
        .create_agent(create_request(), options(2000))
        .await
        .unwrap_err();
    assert!(
        matches!(denied, CallError::Business(f) if f.kind() == Some(FaultKind::HostAccessReadOnly))
    );
    let replacement = server.client(wss_common::SECOND).await.unwrap();
    assert!(replacement.initial_info().can_operate);
    let occupied = control(&mut local, true).await;
    assert!(occupied.revision > released.revision);
    events[0]
        .send(ModelEvent::text_delta("accepted owner continues"))
        .unwrap();
    finish(&events[0]);
    let ended = turn_finished(&mut local).await;
    assert_eq!(ended.turn_id, turn);
    let result = local_reader
        .session(session.target().clone())
        .turn_result(turn, options(2000))
        .await
        .unwrap();
    assert_eq!(
        result.data.info.as_ref().unwrap().outcome,
        Some(TurnOutcome::Completed)
    );
    replacement.close();
    let closed = control(&mut local, false).await;
    assert!(closed.revision > occupied.revision);
    assert!(local_reader.info(options(2000)).await.is_ok());
    let newest = HostClient::in_process(host).unwrap();
    assert!(newest.initial_info().can_operate);
    assert!(control(&mut local, true).await.revision > closed.revision);
    local_reader.close();
    newest.close();
}
