//! Scope semantics are tested with explicit narrow owner fixtures.
mod support;
use minicore_host_protocol::*;
use minicore_host_tools::*;
use std::sync::{Arc, atomic::Ordering};
use support::*;

#[tokio::test]
async fn instant_completion_is_owned_before_start_call_returns_and_delivered_once() {
    let port = FixtureTurn::new();
    port.state.finish_inline.store(true, Ordering::Release);
    let scopes = ScopeManager::new();
    let child = session();
    let accepted = scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            true,
        )
        .await
        .unwrap();
    released(&port.state, 1).await;
    let notifications = port.state.notifications.lock().unwrap();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].0, port.source.session);
    assert_ne!(notifications[0].1.message_id, accepted.message_id);
    assert_eq!(
        notifications[0].1.in_reply_to.as_ref(),
        Some(&accepted.message_id)
    );
    let text: serde_json::Value = serde_json::from_str(&notifications[0].1.text).unwrap();
    assert_eq!(text["results"][0]["outcome"], "completed");
    assert!(text["results"][0].get("history").is_none());
    drop(notifications);
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    assert!(
        relation
            .sink
            .handoff(completion(&relation.registration, TurnOutcome::Completed))
            .is_err()
    );
}
#[tokio::test]
async fn old_turn_without_task_input_cannot_deliver_a_result() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    let mut old = completion(&relation.registration, TurnOutcome::Completed);
    old.inputs[0].message_ids = vec![MessageId::new().unwrap()];
    assert!(relation.sink.handoff(old).is_err());
    assert!(port.state.notifications.lock().unwrap().is_empty());
    relation
        .sink
        .handoff(completion(&relation.registration, TurnOutcome::Failed))
        .unwrap();
    released(&port.state, 1).await;
    let notifications = port.state.notifications.lock().unwrap();
    let text: serde_json::Value = serde_json::from_str(&notifications[0].1.text).unwrap();
    assert_eq!(text["results"][0]["outcome"], "failed");
}
#[tokio::test]
async fn append_keeps_original_parent_task_and_single_receiver() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    let first = input();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            first.clone(),
            true,
            false,
        )
        .await
        .unwrap();
    scopes
        .send(
            port.port(),
            ToolCallRef {
                turn_id: port.source.turn.turn_id.clone(),
                tool_call_id: ToolCallId("second".into()),
            },
            child.clone(),
            input(),
            false,
            false,
        )
        .await
        .unwrap();
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    assert_eq!(relation.accepted.lock().unwrap().len(), 2);
    assert_eq!(relation.registration.task_message_id, first.message_id);
    assert_eq!(
        relation.registration.origin.tool_call_id.as_str(),
        "fixture-call"
    );
    assert_eq!(
        port.state
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "register_sink")
            .count(),
        1
    );
}
#[tokio::test]
async fn another_scope_cannot_replace_a_receiver_but_plain_send_does_not_reassign() {
    let root = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            root.port(),
            origin(&root),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    let outsider = root.for_session(session());
    assert!(
        scopes
            .send(
                outsider.port(),
                origin(&outsider),
                child.clone(),
                input(),
                true,
                false
            )
            .await
            .is_err()
    );
    scopes
        .send(
            outsider.port(),
            origin(&outsider),
            child.clone(),
            input(),
            false,
            false,
        )
        .await
        .unwrap();
    let relation = root
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    assert_eq!(relation.registration.parent, root.source.session);
}
#[tokio::test]
async fn parent_turn_ending_does_not_revoke_child_notification() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    port.active.store(false, Ordering::Release);
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    relation
        .sink
        .handoff(completion(&relation.registration, TurnOutcome::Cancelled))
        .unwrap();
    released(&port.state, 1).await;
    assert_eq!(port.state.notifications.lock().unwrap().len(), 1);
    assert!(
        scopes
            .send(port.port(), origin(&port), child, input(), false, false)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn failed_notification_is_recorded_once_without_retry_or_retarget() {
    let port = FixtureTurn::new();
    port.state.notify_fails.store(true, Ordering::Release);
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            true,
        )
        .await
        .unwrap();
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    relation
        .sink
        .handoff(completion(&relation.registration, TurnOutcome::Completed))
        .unwrap();
    released(&port.state, 1).await;
    assert!(matches!(
        &port.state.releases.lock().unwrap()[0],
        DelegationDisposition::NotificationFailed(_)
    ));
    assert_eq!(
        port.state
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "notify")
            .count(),
        1
    );
}
#[tokio::test]
async fn close_scope_returns_before_root_join_and_seals_new_work() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *port.state.root_close_gate.lock().unwrap() = Some(Arc::clone(&gate));
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    let accepted = scopes.close_scope(port.as_ref()).unwrap();
    assert!(accepted.accepted);
    assert!(
        scopes
            .send(port.port(), origin(&port), session(), input(), true, false)
            .await
            .is_err()
    );
    released(&port.state, 1).await;
    assert!(
        !port
            .state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event == "close_root")
    );
    gate.add_permits(1);
}
#[tokio::test]
async fn member_cannot_close_root_scope_and_intermediate_completion_does_not_close_grandchild() {
    let root = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    let grandchild = session();
    scopes
        .send(
            root.port(),
            origin(&root),
            child.clone(),
            input(),
            true,
            true,
        )
        .await
        .unwrap();
    let intermediate = root.for_session(child.clone());
    scopes
        .send(
            intermediate.port(),
            origin(&intermediate),
            grandchild.clone(),
            input(),
            true,
            true,
        )
        .await
        .unwrap();
    assert!(scopes.close_scope(intermediate.as_ref()).is_err());
    let relation = root
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    relation
        .sink
        .handoff(completion(&relation.registration, TurnOutcome::Completed))
        .unwrap();
    released(&root.state, 1).await;
    assert!(
        !root
            .state
            .events
            .lock()
            .unwrap()
            .contains(&format!("close:{}", grandchild.session_id))
    );
    let grand = root
        .state
        .relations
        .lock()
        .unwrap()
        .get(&grandchild)
        .cloned()
        .unwrap();
    assert_eq!(grand.registration.parent, child);
    assert_eq!(grand.registration.root, root.source.session);
}
#[tokio::test]
async fn draining_rejects_new_delegation_to_an_existing_session() {
    let port = FixtureTurn::new();
    port.state.draining.store(true, Ordering::Release);
    let scopes = ScopeManager::new();
    assert!(
        scopes
            .send(port.port(), origin(&port), session(), input(), true, false)
            .await
            .is_err()
    );
    assert!(
        !port
            .state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event == "accept_task")
    );
}

#[tokio::test]
async fn self_and_root_delegation_are_rejected_without_a_cycle() {
    let root = FixtureTurn::new();
    let scopes = ScopeManager::new();
    assert!(
        scopes
            .send(
                root.port(),
                origin(&root),
                root.source.session.clone(),
                input(),
                true,
                false
            )
            .await
            .is_err()
    );
    let child = session();
    scopes
        .send(
            root.port(),
            origin(&root),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    let member = root.for_session(child.clone());
    assert!(
        scopes
            .send(member.port(), origin(&member), child, input(), true, false)
            .await
            .is_err()
    );
    assert!(
        scopes
            .send(
                member.port(),
                origin(&member),
                root.source.session.clone(),
                input(),
                true,
                false
            )
            .await
            .is_err()
    );
    assert_eq!(root.state.relations.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn explicitly_discarded_unconsumed_task_never_fabricates_a_turn_result() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            true,
        )
        .await
        .unwrap();
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    relation
        .sink
        .handoff(DelegationCompletion {
            results: vec![],
            inputs: vec![],
            discarded_inputs: vec![relation.registration.task_message_id.clone()],
        })
        .unwrap();
    released(&port.state, 1).await;
    let messages = port.state.notifications.lock().unwrap();
    let body: serde_json::Value = serde_json::from_str(&messages[0].1.text).unwrap();
    assert_eq!(body["status"], "not_executed");
    assert_eq!(body["results"], serde_json::json!([]));
    assert_eq!(
        body["discarded_inputs"][0],
        relation.registration.task_message_id.as_str()
    );
}

#[tokio::test]
async fn cleanup_failure_is_not_retried_until_an_explicit_later_close() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    port.state.close_fails.store(true, Ordering::Release);
    scopes.close_scope(port.as_ref()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let changed = port.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if port
                .state
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == &format!("close:{}", child.session_id))
            {
                break;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
    tokio::task::yield_now().await;
    assert_eq!(
        port.state
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| *event == &format!("close:{}", child.session_id))
            .count(),
        1
    );
    port.state.close_fails.store(false, Ordering::Release);
    scopes.close_scope(port.as_ref()).unwrap();
    released(&port.state, 1).await;
    assert_eq!(
        port.state
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| *event == &format!("close:{}", child.session_id))
            .count(),
        2
    );
}

#[tokio::test]
async fn cancellation_after_real_input_binding_does_not_report_not_executed() {
    let port = FixtureTurn::new();
    let scopes = ScopeManager::new();
    let child = session();
    scopes
        .send(
            port.port(),
            origin(&port),
            child.clone(),
            input(),
            true,
            false,
        )
        .await
        .unwrap();
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .get(&child)
        .cloned()
        .unwrap();
    let mut actual = completion(&relation.registration, TurnOutcome::Cancelled);
    actual.results[0].info.requests = 0;
    actual.results[0].info.model_attempts.clear();
    relation.sink.handoff(actual).unwrap();
    released(&port.state, 1).await;
    let notifications = port.state.notifications.lock().unwrap();
    let text: serde_json::Value = serde_json::from_str(&notifications[0].1.text).unwrap();
    assert_eq!(text["status"], "ended");
    assert_eq!(text["results"][0]["outcome"], "cancelled");
    assert_eq!(text["discarded_inputs"], serde_json::json!([]));
}
