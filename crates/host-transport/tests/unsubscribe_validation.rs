//! Regression: a locally rejected call has not changed the real Host stream.
mod common;
mod observation_common;
mod os_pipe;
use common::*;
use minicore_host_protocol::*;
use minicore_host_runtime::model::ModelEvent;
use observation_common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_wait_option_does_not_locally_end_a_live_rpc_subscription() {
    let (host, events, mut starts) = setup(1, None);
    let (client, server) = stdio_pipes(host).await;
    let session = create(&client).await;
    let mut sub = client
        .subscribe(
            request(scope(session.target()), &[EventCategory::Output]),
            options(3000),
        )
        .await
        .unwrap();
    let id = sub.result().subscription_id.clone();
    let error = client
        .unsubscribe(
            UnsubscribeRequest {
                subscription_id: id.clone(),
            },
            options(0),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, CallError::Protocol { .. }));
    session
        .send(input("normal execution after invalid call"), options(3000))
        .await
        .unwrap();
    starts.recv().await.unwrap();
    events[0]
        .send(ModelEvent::text_delta("still subscribed"))
        .unwrap();
    let observed = tokio::time::timeout(std::time::Duration::from_secs(3), sub.recv())
        .await
        .unwrap()
        .unwrap();
    let host_result = client
        .unsubscribe(
            UnsubscribeRequest {
                subscription_id: id,
            },
            options(3000),
        )
        .await
        .unwrap();
    finish(&events[0]);
    client.close();
    server.await.unwrap().unwrap();
    assert!(
        host_result.ended,
        "the bad option must not have dispatched an unsubscribe"
    );
    assert!(
        observed.is_some(),
        "a rejected local option cannot close a still-live Host subscription"
    );
}
