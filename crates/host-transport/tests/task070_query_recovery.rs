//! New recovery regressions, not a rerun of the lost task054 probe.
//! Scripted model streams drive real Host/Runtime owners and real OS pipes.
#![cfg(unix)]
mod common;
#[path = "task081_query_support/pipe.rs"]
mod query_pipes;
use common::*;
use minicore_host_core::{
    ActivityEvent, Host, HostSettings, ObservationEvent, ObservationNotification, StateEvent,
    TurnSnapshot,
};
use minicore_host_protocol::*;
use minicore_host_transport::*;
use query_pipes::stdio_pipes;
use std::time::Duration;

fn subscription(session: &SessionClient, turn: &TurnId) -> SubscribeRequest {
    SubscribeRequest {
        scope: SubscriptionScope::Turn {
            session_id: session.target().session_id.clone(),
            turn_id: turn.clone(),
        },
        categories: [EventCategory::State, EventCategory::Completion]
            .into_iter()
            .collect(),
    }
}

async fn original_terminal(
    session: &SessionClient,
    client: &HostClient,
    resources: &FixtureResources,
) -> (TurnSnapshot, u64) {
    session
        .send(input("first actual Runtime Turn"), options(5000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let turn = running(session).await;
    let mut events = client
        .subscribe(subscription(session, &turn), options(5000))
        .await
        .unwrap();
    resources.model.finish.add_permits(1);
    let (finished, previous_revision) = tokio::time::timeout(Duration::from_secs(5), async {
        let mut previous: Option<(TurnState, u64)> = None;
        let mut finished = None;
        loop {
            let ObservationNotification::Event(push) = events.recv().await.unwrap().unwrap() else {
                panic!("original owner was unexpectedly lost")
            };
            match push.event {
                ObservationEvent::State(StateEvent::Turn { source, data }) => {
                    assert_eq!(source.turn_id, turn);
                    if data.state == TurnState::Finished {
                        assert_eq!(previous.as_ref().unwrap().0, TurnState::Finalizing);
                        finished = Some(TurnSnapshot { source, data });
                    } else {
                        previous = Some((data.state, data.runner_revision));
                    }
                }
                ObservationEvent::Activity(ActivityEvent::TurnFinished { source, data }) => {
                    let finished = finished.unwrap();
                    assert_eq!(source, finished.source);
                    assert_eq!(value(data), value(&finished.data));
                    break (finished, previous.unwrap().1);
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    client
        .unsubscribe(
            UnsubscribeRequest {
                subscription_id: events.result().subscription_id.clone(),
            },
            options(5000),
        )
        .await
        .unwrap();
    drop(events);
    (finished, previous_revision)
}

async fn assert_retained(
    client: &HostClient,
    session: &SessionClient,
    expected: &TurnSnapshot,
    original_config: &SessionConfig,
) {
    let turn = expected.source.turn_id.clone();
    let detail = session.turn(turn.clone(), options(5000)).await.unwrap();
    assert_eq!(detail.source, expected.source);
    assert_eq!(value(&detail.data.state), value(&expected.data));
    assert_eq!(&detail.data.config, original_config);
    let list = session.turns(options(5000)).await.unwrap();
    let listed = list
        .data
        .iter()
        .find(|entry| entry.turn_id == turn)
        .unwrap();
    assert_eq!(Some(&listed.load_id), expected.source.load_id.as_ref());
    assert_eq!(value(&listed.data.state), value(&expected.data));
    assert_eq!(&listed.data.config, original_config);
    let result = session
        .turn_result(turn.clone(), options(5000))
        .await
        .unwrap();
    assert_eq!(result.source, expected.source);
    // JSON comparison also catches a separate result type silently omitting the
    // original terminal runner revision. State and results share one contract.
    assert_eq!(
        value(result.data.info.as_ref().unwrap()),
        value(&expected.data)
    );
    assert_eq!(result.data.config.as_ref().unwrap(), original_config);
    assert_eq!(
        value(
            session
                .wait_turn(turn.clone(), options(5000))
                .await
                .unwrap()
        ),
        value(result)
    );
    let baseline = client
        .subscribe(subscription(session, &turn), options(5000))
        .await
        .unwrap();
    let states: Vec<_> = baseline
        .result()
        .initial_states
        .iter()
        .filter_map(|state| match state {
            StateEvent::Turn { source, data } => Some((source, data)),
            _ => None,
        })
        .collect();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].0, &expected.source);
    assert_eq!(value(states[0].1), value(&expected.data));
    assert!(value(states[0].1).get("config").is_none());
    client
        .unsubscribe(
            UnsubscribeRequest {
                subscription_id: baseline.result().subscription_id.clone(),
            },
            options(5000),
        )
        .await
        .unwrap();
    drop(baseline);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_transition_owns_a_revision_and_all_queries_keep_it_during_later_turn() {
    let (host, resources) = fixture();
    let owner = HostClient::in_process(host.clone()).unwrap();
    let session = create(&owner).await;
    let original = session.info(options(5000)).await.unwrap().config;
    let (reader, server) = stdio_pipes(host).await;
    let observed = reader.session(session.target().clone());
    let (finished, previous) = original_terminal(&session, &reader, &resources).await;
    assert!(
        finished.data.runner_revision > previous,
        "Finished must own a newer revision than its Finalizing predecessor: {} <= {previous}",
        finished.data.runner_revision
    );
    assert_retained(&owner, &session, &finished, &original).await;
    assert_retained(&reader, &observed, &finished, &original).await;
    session
        .update(
            SessionUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: SessionUpdateChanges {
                    title: NullablePatch::Unchanged,
                    config: SessionConfigPatch {
                        role: Some(Role {
                            name: "next Turn only".into(),
                            ..Role::default()
                        }),
                        ..Default::default()
                    },
                },
            },
            options(5000),
        )
        .await
        .unwrap();
    session
        .send(input("second Turn"), options(5000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let second = running(&session).await;
    let active = session.turn(second.clone(), options(5000)).await.unwrap();
    assert_ne!(active.source.turn_id, finished.source.turn_id);
    assert_eq!(active.source.load_id, finished.source.load_id);
    assert_eq!(active.data.config.role.name, "next Turn only");
    assert_retained(&owner, &session, &finished, &original).await;
    assert_retained(&reader, &observed, &finished, &original).await;
    resources.model.finish.add_permits(1);
    session.wait_turn(second, options(5000)).await.unwrap();
    assert_retained(&reader, &observed, &finished, &original).await;
    server.close(&reader, Some(Duration::from_secs(5))).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_unload_reload_preserves_original_terminal_revision_and_load() {
    let directory = tempfile::tempdir().unwrap();
    let (_, resources) = fixture();
    let host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "query recovery".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let owner = HostClient::in_process(host.clone()).unwrap();
    let mut request = create_request();
    request.mode = StorageMode::Persistent;
    let created = owner.create_agent(request, options(5000)).await.unwrap();
    let session = owner.session(SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    });
    let original = session.info(options(5000)).await.unwrap().config;
    let (reader, server) = stdio_pipes(host).await;
    let observed = reader.session(session.target().clone());
    let (finished, _) = original_terminal(&session, &reader, &resources).await;
    assert_eq!(finished.data.persistence, PersistenceState::Persisted);
    // Avoid an earlier comparison so the unfixed source specifically exposes
    // its unload/reload query drift, independently of the transition test.
    session.unload(control(), options(5000)).await.unwrap();
    let unloaded = session.state(options(5000)).await.unwrap();
    assert_eq!(unloaded.data.residency, Residency::Unloaded);
    assert!(unloaded.data.load_id.is_none());
    session.load(control(), options(5000)).await.unwrap();
    let loaded = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = session.state(options(5000)).await.unwrap();
            if state.data.residency == Residency::Loaded && !state.data.preparing {
                break state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_ne!(loaded.data.load_id, finished.source.load_id);
    assert_retained(&owner, &session, &finished, &original).await;
    assert_retained(&reader, &observed, &finished, &original).await;
    session
        .send(input("new residency Turn"), options(5000))
        .await
        .unwrap();
    resources.model.started.acquire().await.unwrap().forget();
    let next = running(&session).await;
    let active = session.turn(next.clone(), options(5000)).await.unwrap();
    assert_eq!(active.source.load_id, loaded.data.load_id);
    assert_retained(&reader, &observed, &finished, &original).await;
    resources.model.finish.add_permits(1);
    session.wait_turn(next, options(5000)).await.unwrap();
    assert_retained(&reader, &observed, &finished, &original).await;
    server.close(&reader, Some(Duration::from_secs(5))).await;
}
