use super::*;
use minicore_host_core::{BoundResources, Host, ResourceFuture, ResourceResolver};

struct UnusedResources;
impl ResourceResolver for UnusedResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err::<BoundResources, _>(Fault::new(
                FaultKind::SessionResourceMissing,
                "no execution requested in correlation test",
            ))
        })
    }
}
fn pending() -> (Arc<Pending>, mpsc::UnboundedReceiver<Vec<u8>>) {
    let (outgoing, frames) = mpsc::unbounded_channel();
    let (closed, _) = watch::channel(false);
    (
        Arc::new(Pending {
            state: Mutex::new(PendingState {
                next_id: 3,
                calls: BTreeMap::new(),
                streams: BTreeMap::new(),
                early: BTreeMap::new(),
                completed_setups: BTreeMap::new(),
                failure: None,
            }),
            outgoing,
            closed,
        }),
        frames,
    )
}
type TestReply = oneshot::Receiver<Result<Value, CallError>>;
type TestEvents = mpsc::UnboundedReceiver<Result<ObservationNotification, CallError>>;
fn setup(pending: &Pending, id: i64) -> (TestReply, TestEvents) {
    let (reply, result) = oneshot::channel();
    let (observation, events) = mpsc::unbounded_channel();
    pending.state.lock().unwrap().calls.insert(
        id,
        PendingRequest {
            reply: Some(reply),
            observation: Some(observation),
        },
    );
    (result, events)
}
#[tokio::test]
async fn real_host_early_resync_stays_ended_and_old_control_cannot_end_a_new_subscription() {
    let host = Host::memory(Arc::new(UnusedResources)).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let request = SubscribeRequest {
        scope: SubscriptionScope::Host {
            host_id: connection.initial_info().host_id.clone(),
        },
        categories: [EventCategory::State].into_iter().collect(),
    };
    let mut old = host.events_subscribe(&caller, request.clone()).unwrap();
    let (pending, _frames) = pending();
    let (reply, mut events) = setup(&pending, 1);
    // This control frame is produced by the real Host observation owner.
    old.end_after_delivery_failure(Fault::new(
        FaultKind::HostObservationResyncRequired,
        "test adapter confirmed a missed dispatch",
    ));
    let control = old.recv().await.unwrap().unwrap();
    pending.notification(control.clone());
    pending.receive(RpcResponse::success(
        RpcId::Integer(1),
        serde_json::to_value(old.result()).unwrap(),
    ));
    reply.await.unwrap().unwrap();
    assert!(matches!(
        events.recv().await.unwrap().unwrap(),
        ObservationNotification::ResyncRequired(_)
    ));
    assert!(events.recv().await.is_none());
    assert!(pending.state.lock().unwrap().streams.is_empty());
    let fresh = host.events_subscribe(&caller, request).unwrap();
    let (reply, mut events) = setup(&pending, 2);
    pending.receive(RpcResponse::success(
        RpcId::Integer(2),
        serde_json::to_value(fresh.result()).unwrap(),
    ));
    reply.await.unwrap().unwrap();
    pending.notification(control);
    pending.unsubscribe(&old.result().subscription_id);
    assert!(
        pending
            .state
            .lock()
            .unwrap()
            .streams
            .contains_key(&fresh.result().subscription_id)
    );
    assert!(matches!(
        events.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn cancelling_setup_after_reader_registers_it_still_releases_only_that_observation() {
    let host = Host::memory(Arc::new(UnusedResources)).unwrap();
    let connection = host.connect().unwrap();
    let subscription = host
        .events_subscribe(
            &connection.caller(),
            SubscribeRequest {
                scope: SubscriptionScope::Host {
                    host_id: connection.initial_info().host_id.clone(),
                },
                categories: [EventCategory::State].into_iter().collect(),
            },
        )
        .unwrap();
    let (pending, mut frames) = pending();
    let (_reply, _events) = setup(&pending, 1);
    let waiter = PendingCall {
        pending: pending.clone(),
        id: 1,
        sent: true,
    };
    pending.receive(RpcResponse::success(
        RpcId::Integer(1),
        serde_json::to_value(subscription.result()).unwrap(),
    ));
    assert!(
        pending
            .state
            .lock()
            .unwrap()
            .streams
            .contains_key(&subscription.result().subscription_id)
    );
    drop(waiter);
    assert!(pending.state.lock().unwrap().streams.is_empty());
    let cleanup: Value = serde_json::from_slice(&frames.recv().await.unwrap()).unwrap();
    assert_eq!(cleanup["method"], "events.unsubscribe");
    assert_eq!(
        cleanup["params"]["subscription_id"],
        subscription.result().subscription_id.to_string()
    );
    assert!(!*pending.closed.borrow());
}
