//! Canonical methods use real owners and HTTP utility work on every transport.
mod compaction_common;
mod wss_common;

use compaction_common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[tokio::test]
async fn all_transports_compact_real_history_without_turns_and_preserve_paused_queue_order() {
    for transport in 0..3 {
        let gate = Arc::new(Semaphore::new(0));
        let mut replies = seed_replies();
        replies.extend([
            gated(SUMMARY, &gate),
            reply("Independent session result"),
            reply("First resumed result"),
            reply("Second resumed result"),
        ]);
        let mut http = HttpFixture::start(replies).await;
        let peer = Connected::new(host(&http).await, transport).await;
        let session = seed(&peer.client, &mut http).await;
        session.pause(control(), options()).await.unwrap();
        let before = history(&session).await;
        let accepted = session.compact(compact(), options()).await.unwrap();
        assert_eq!(accepted.result.state, CompactionState::Preparing {});
        assert!(!accepted.result.cancellation_requested);
        assert_eq!(accepted.host_id, peer.client.initial_info().host_id);
        assert_eq!(
            accepted.host_incarnation,
            peer.client.initial_info().host_incarnation
        );
        assert_eq!(accepted.agent_id, session.target().agent_id);
        assert_eq!(accepted.session_id, session.target().session_id);
        let utility = http.request().await;
        assert!(is_utility(&utility));
        assert!(utility.get("tools").is_none());
        retained(&peer.client, &accepted).await;
        let busy = compact();
        let rejected = busy.operation_id.clone();
        assert_eq!(
            fault(session.compact(busy, options()).await.unwrap_err()).kind(),
            Some(FaultKind::SessionBusy)
        );
        assert_eq!(
            fault(peer.client.receipt(rejected, options()).await.unwrap_err()).kind(),
            Some(FaultKind::HostReceiptNotFound)
        );
        assert_eq!(
            fault(session.unload(control(), options()).await.unwrap_err()).kind(),
            Some(FaultKind::SessionBusy)
        );

        let mut next_step = Vec::new();
        let mut next_turn = Vec::new();
        // Interleaved arrival does not impose a global FIFO across the queues.
        for (text, mode) in [
            ("follow up first", MessageMode::FollowUp),
            ("steer first", MessageMode::Steer),
            ("follow up second", MessageMode::FollowUp),
            ("steer second", MessageMode::Steer),
        ] {
            let receipt = session.send(input(text, mode), options()).await.unwrap();
            if mode == MessageMode::Steer {
                next_step.push(receipt.message_id);
            } else {
                next_turn.push(receipt.message_id);
            }
        }
        let other = create(&peer.client).await;
        other
            .send(
                input("Independent session", MessageMode::FollowUp),
                options(),
            )
            .await
            .unwrap();
        assert!(!is_utility(&http.request().await));
        settled(&other, 1).await;
        assert_eq!(session.turns(options()).await.unwrap().data.len(), 2);
        gate.add_permits(1);
        let state = finished(&session, &accepted.operation_id).await;
        let activity = state.data.compaction.as_ref().unwrap();
        assert!(matches!(
            activity.state,
            CompactionState::Finished {
                result: CompactionOutcome::Compacted {
                    persistence: PersistenceState::NotApplicable,
                    ..
                }
            }
        ));
        assert_eq!(activity.utility_usage.len(), 1);
        assert!(activity.utility_usage[0].completed);
        assert_eq!(state.data.scheduling, Scheduling::Paused);
        assert_eq!(
            state
                .data
                .next_step
                .iter()
                .map(|i| &i.message_id)
                .collect::<Vec<_>>(),
            next_step.iter().collect::<Vec<_>>()
        );
        assert_eq!(
            state
                .data
                .next_turn
                .iter()
                .map(|i| &i.message_id)
                .collect::<Vec<_>>(),
            next_turn.iter().collect::<Vec<_>>()
        );
        assert!(state.data.active_turn_id.is_none());
        assert_eq!(session.turns(options()).await.unwrap().data.len(), 2);
        let after = history(&session).await;
        let original_items = before["items"].as_array().unwrap();
        let actual_items = after["items"].as_array().unwrap();
        assert_eq!(&actual_items[..original_items.len()], original_items);
        assert_eq!(actual_items.len(), original_items.len() + 1);
        assert_eq!(actual_items.last().unwrap()["kind"], "summary");
        assert_eq!(actual_items.last().unwrap()["text"], SUMMARY);
        retained(&peer.client, &accepted).await;
        let already_done = session
            .cancel_compact(cancel(accepted.operation_id.clone()), options())
            .await
            .unwrap();
        assert!(!already_done.result.cancel_claimed);
        assert_eq!(already_done.result.target.state, activity.state);

        session.resume(control(), options()).await.unwrap();
        let first = http.request().await;
        let second = http.request().await;
        settled(&session, 4).await;
        let messages = first["messages"].as_array().unwrap();
        assert!(messages.iter().any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|text| text.starts_with("Conversation summary:"))
        }));
        let text = serde_json::to_string(messages).unwrap();
        assert!(text.find("steer first").unwrap() < text.find("steer second").unwrap());
        assert!(text.find("steer second").unwrap() < text.find("follow up first").unwrap());
        assert!(!text.contains("follow up second"));
        assert!(
            serde_json::to_string(&second["messages"])
                .unwrap()
                .contains("follow up second")
        );
        let state = session.state(options()).await.unwrap();
        assert!(state.data.next_step.is_empty() && state.data.next_turn.is_empty());
        peer.finish().await;
    }
}

#[tokio::test]
async fn all_transports_cancel_exact_original_without_clearing_queues_or_later_work() {
    for transport in 0..3 {
        let first_gate = Arc::new(Semaphore::new(0));
        let second_gate = Arc::new(Semaphore::new(0));
        let turn_gate = Arc::new(Semaphore::new(0));
        let mut replies = seed_replies();
        replies.extend([
            gated(SUMMARY, &first_gate),
            gated(SUMMARY, &second_gate),
            gated("Later turn completed", &turn_gate),
        ]);
        let mut http = HttpFixture::start(replies).await;
        let peer = Connected::new(host(&http).await, transport).await;
        let session = seed(&peer.client, &mut http).await;
        session.pause(control(), options()).await.unwrap();
        let raw = history(&session).await;
        let original = session.compact(compact(), options()).await.unwrap();
        assert!(is_utility(&http.request().await));
        let follow_up = session
            .send(input("queued follow up", MessageMode::FollowUp), options())
            .await
            .unwrap();
        let steer = session
            .send(input("queued steer", MessageMode::Steer), options())
            .await
            .unwrap();
        let request = cancel(original.operation_id.clone());
        let cancellation = session
            .cancel_compact(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(cancellation.operation_id, request.operation_id);
        assert_eq!(cancellation.target_operation_id, original.operation_id);
        assert_ne!(cancellation.operation_id, cancellation.target_operation_id);
        assert!(cancellation.result.cancel_claimed);
        assert!(cancellation.result.target.cancellation_requested);
        assert!(matches!(
            cancellation.result.target.state,
            CompactionState::Summarizing {} | CompactionState::Merging {}
        ));
        let finished_first = finished(&session, &original.operation_id).await;
        assert!(matches!(
            finished_first.data.compaction.unwrap().state,
            CompactionState::Finished {
                result: CompactionOutcome::Cancelled
            }
        ));
        first_gate.add_permits(1);
        assert_eq!(history(&session).await, raw);
        retained(&peer.client, &original).await;
        assert_eq!(
            peer.client
                .receipt(cancellation.operation_id.clone(), options())
                .await
                .unwrap(),
            Receipt::CompactCancel(cancellation.clone())
        );
        assert!(
            !session
                .cancel_compact(cancel(original.operation_id.clone()), options())
                .await
                .unwrap()
                .result
                .cancel_claimed
        );

        let second = session.compact(compact(), options()).await.unwrap();
        assert!(is_utility(&http.request().await));
        let stale = session
            .cancel_compact(cancel(original.operation_id.clone()), options())
            .await
            .unwrap();
        assert!(!stale.result.cancel_claimed);
        assert!(matches!(
            stale.result.target.state,
            CompactionState::Finished {
                result: CompactionOutcome::Cancelled
            }
        ));
        let state = session.state(options()).await.unwrap();
        let activity = state.data.compaction.unwrap();
        assert!(
            matches!(activity.source, CompactionSource::Manual { operation_id } if operation_id == second.operation_id)
        );
        assert!(!activity.cancellation_requested);
        assert_eq!(state.data.next_step[0].message_id, steer.message_id);
        assert_eq!(state.data.next_turn[0].message_id, follow_up.message_id);
        let other = create(&peer.client).await;
        assert_eq!(
            fault(
                other
                    .cancel_compact(cancel(original.operation_id.clone()), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionCompactionNotFound)
        );
        assert_eq!(
            fault(
                session
                    .cancel_compact(cancel(OperationId::new().unwrap()), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionCompactionNotFound)
        );
        second_gate.add_permits(1);
        let complete = finished(&session, &second.operation_id).await;
        assert_eq!(complete.data.scheduling, Scheduling::Paused);
        assert_eq!(complete.data.next_step.len(), 1);
        assert_eq!(complete.data.next_turn.len(), 1);
        retained(&peer.client, &second).await;

        session.resume(control(), options()).await.unwrap();
        assert!(!is_utility(&http.request().await));
        let running = session
            .state(options())
            .await
            .unwrap()
            .data
            .active_turn_id
            .unwrap();
        assert_eq!(
            fault(session.compact(compact(), options()).await.unwrap_err()).kind(),
            Some(FaultKind::SessionBusy)
        );
        assert!(
            !session
                .cancel_compact(cancel(original.operation_id), options())
                .await
                .unwrap()
                .result
                .cancel_claimed
        );
        assert_eq!(
            session.state(options()).await.unwrap().data.active_turn_id,
            Some(running)
        );
        turn_gate.add_permits(1);
        settled(&session, 3).await;
        peer.finish().await;
    }
}

#[tokio::test]
async fn all_transports_use_current_connection_rights_and_allow_new_operator_to_cancel() {
    for transport in 0..3 {
        let gate = Arc::new(Semaphore::new(0));
        let mut replies = seed_replies();
        replies.push(gated(SUMMARY, &gate));
        let mut http = HttpFixture::start(replies).await;
        let host = host(&http).await;
        let operator = Connected::new(host.clone(), transport).await;
        let session = seed(&operator.client, &mut http).await;
        let receipt = session.compact(compact(), options()).await.unwrap();
        assert!(is_utility(&http.request().await));
        let reader = Connected::new(host.clone(), transport).await;
        assert!(!reader.client.initial_info().can_operate);
        let readonly = reader.client.session(session.target().clone());
        for close in [false, true] {
            if close {
                operator.client.close();
                tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    while reader
                        .client
                        .info(options())
                        .await
                        .unwrap()
                        .control
                        .occupied
                    {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
            }
            assert_eq!(
                fault(readonly.compact(compact(), options()).await.unwrap_err()).kind(),
                Some(FaultKind::HostAccessReadOnly)
            );
            assert_eq!(
                fault(
                    readonly
                        .cancel_compact(cancel(receipt.operation_id.clone()), options())
                        .await
                        .unwrap_err()
                )
                .kind(),
                Some(FaultKind::HostAccessReadOnly)
            );
            let snapshot = readonly.state(options()).await.unwrap();
            assert!(!snapshot.data.compaction.unwrap().cancellation_requested);
        }
        let replacement = Connected::new(host, transport).await;
        assert!(replacement.client.initial_info().can_operate);
        let adopted = replacement.client.session(session.target().clone());
        assert!(
            adopted
                .cancel_compact(cancel(receipt.operation_id.clone()), options())
                .await
                .unwrap()
                .result
                .cancel_claimed
        );
        let done = finished(&readonly, &receipt.operation_id).await;
        assert!(matches!(
            done.data.compaction.unwrap().state,
            CompactionState::Finished {
                result: CompactionOutcome::Cancelled
            }
        ));
        retained(&reader.client, &receipt).await;
        gate.add_permits(1);
        reader.finish().await;
        replacement.finish().await;
        operator.finish().await;
    }
}
