//! Real production Turns, with a held actual mutation guard as the test gate.
#[path = "../tests/task042/support.rs"]
mod support;
use super::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_workspace::{ChangeScope, ChangesListRequest, EditOutcome, FileSnapshot};
use serde_json::json;
use support::*;
use tokio::sync::Semaphore;

async fn running(subscription: &mut EventSubscription, call: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(ObservationNotification::Event(push)) = subscription.recv().await.unwrap()
                && let ObservationEvent::State(StateEvent::Toolcall { source, data }) = push.event
                && source.tool_call_id.0 == call
                && data.state == ToolCallState::Running
            {
                return;
            }
        }
    })
    .await
    .expect("actual Runtime tool start");
}
fn observe(host: &Host, caller: &CallerContext, target: &SessionRef) -> EventSubscription {
    host.events_subscribe(
        caller,
        SubscribeRequest {
            scope: SubscriptionScope::Session {
                agent_id: target.agent_id.clone(),
                session_id: target.session_id.clone(),
            },
            categories: [EventCategory::State].into_iter().collect(),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn old_and_new_publications_share_workspace_mutations_across_real_sessions() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("shared.txt"), b"alpha\nbeta\n").unwrap();
    let gate_a = Arc::new(Semaphore::new(0));
    let gate_b = Arc::new(Semaphore::new(0));
    let mut a = Server::start(vec![
        (
            tool_answer(
                "same-id",
                "edit",
                json!({"path":"shared.txt", "old_text":"alpha", "new_text":"ALPHA"}),
            ),
            Some(gate_a.clone()),
        ),
        (answer("old publication finished"), None),
    ])
    .await;
    let mut b = Server::start(vec![
        (
            tool_answer(
                "same-id",
                "edit",
                json!({"path":"shared.txt", "old_text":"beta", "new_text":"BETA"}),
            ),
            Some(gate_b.clone()),
        ),
        (answer("new publication finished"), None),
    ])
    .await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let root = workspace(directory.path()).unwrap();
    let shared = builder.workspaces.bind(&root).await.unwrap();
    let mutation = shared.begin_mutation().await;
    let host = Host::memory_configured(prepare(&builder, config(&a.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let first = create(&host, &caller, selection(&["edit"]), Some(root)).await;
    let mut first_events = observe(&host, &caller, &first);
    send(&host, &caller, &first, "first edit");
    assert_eq!(a.request().await["model"], "provider-A");
    // An active Turn really holds the cached object, rather than a freshly
    // bound Workspace with an unrelated mutation mutex.
    assert_eq!(Arc::strong_count(&shared), 2);

    let clone = builder.clone();
    let mut next = config(&b.endpoint);
    next.models[0].provider_model = "provider-B".into();
    assert_eq!(
        host.reload_configuration(&caller, 0, prepare(&clone, next).await)
            .await
            .unwrap(),
        HostReloadResult::Applied { config_revision: 1 }
    );
    let second = create(
        &host,
        &caller,
        selection(&["edit"]),
        Some(WorkspaceRef {
            path: format!("{}/.", directory.path().display()),
        }),
    )
    .await;
    let mut second_events = observe(&host, &caller, &second);
    send(&host, &caller, &second, "second edit");
    assert_eq!(b.request().await["model"], "provider-B");
    assert_eq!(
        Arc::strong_count(&shared),
        3,
        "both real production Turns retain one binding across reload"
    );
    gate_a.add_permits(1);
    gate_b.add_permits(1);
    tokio::join!(
        running(&mut first_events, "same-id"),
        running(&mut second_events, "same-id")
    );
    assert_eq!(
        std::fs::read(directory.path().join("shared.txt")).unwrap(),
        b"alpha\nbeta\n"
    );
    drop(mutation);
    let (old_request, new_request) = tokio::join!(a.request(), b.request());
    assert_eq!(old_request["model"], "provider-A");
    assert_eq!(new_request["model"], "provider-B");
    let (first_turns, second_turns) = tokio::join!(
        settled(&host, &caller, &first, 1),
        settled(&host, &caller, &second, 1)
    );
    assert_eq!(first_turns[0].outcome, Some(TurnOutcome::Completed));
    assert_eq!(second_turns[0].outcome, Some(TurnOutcome::Completed));
    assert_eq!(
        std::fs::read(directory.path().join("shared.txt")).unwrap(),
        b"ALPHA\nBETA\n"
    );
    let records: Vec<_> = [&first, &second]
        .iter()
        .map(|target| {
            let changes = host
                .session_changes(
                    &caller,
                    target,
                    ChangesListRequest {
                        scope: ChangeScope::Session,
                        cursor: None,
                        limit: 10,
                    },
                )
                .unwrap();
            assert_eq!(changes.records.len(), 1);
            host.session_change_record(&caller, target, &changes.records[0].change_ref)
                .unwrap()
        })
        .collect();
    assert!(
        records
            .iter()
            .all(|record| record.outcome == EditOutcome::Applied)
    );
    let original = FileSnapshot::Content(b"alpha\nbeta\n".to_vec());
    let (before, after) = if records[0].before == original {
        (&records[0], &records[1])
    } else {
        (&records[1], &records[0])
    };
    assert_eq!(before.before, original);
    assert_eq!(
        before.after, after.before,
        "both changes must preserve the actual serialized observation chain"
    );
    assert_eq!(
        after.after,
        FileSnapshot::Content(b"ALPHA\nBETA\n".to_vec())
    );
    assert_ne!(before.tool_call.turn_id, after.tool_call.turn_id);
    assert_eq!(before.tool_call.tool_call_id, after.tool_call.tool_call_id);
}
