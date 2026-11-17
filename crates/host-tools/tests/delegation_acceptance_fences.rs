//! Production Host, Runtime, agent tool, owner ports and sink; gated model only.
#[path = "support/gated_host.rs"]
mod gated_host;
#[path = "../../../tests/support/input_context.rs"]
mod input_context;
use gated_host::Fixture;
use minicore_host_core::MessageBinding;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    HistoryItem,
    model::{ModelEvent, ModelMessage},
};
use serde_json::{Value, json};

impl Fixture {
    async fn agent(&self) -> SessionRef {
        let mut config = AgentConfig::default();
        config.session_defaults.tools = vec![ToolBinding {
            name: "agent".into(),
        }];
        let receipt = self
            .host
            .create_agent_async(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "fence fixture".into(),
                    mode: StorageMode::Memory,
                    config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .await
            .unwrap();
        SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        }
    }
}
fn target(session: &SessionRef) -> Value {
    json!({"kind":"session", "agent_id":session.agent_id, "session_id":session.session_id})
}
#[derive(Clone, Copy)]
enum End {
    Complete,
    RemoveFirst,
    Cancel,
    Fail,
}
async fn shared_range_with_real_tools(end: End) {
    let mut f = Fixture::new();
    let parent = f.agent().await;
    let child = f.agent().await;
    f.send(&parent, "parent task");
    let root = f.next(&parent).await;
    let unrelated = f.send(&child, "older ordinary task");
    let old = f.next(&child).await;
    root.tool("delegate-first", json!({"action":"send", "target":target(&child), "text":"first follow-up", "mode":"follow_up", "delegate":true}));
    let after_first = f.next(&parent).await;
    let first_receipt = after_first.last_tool_result();
    assert_eq!(first_receipt["result"]["delivery"], "accepted");
    let first: MessageId = serde_json::from_value(first_receipt["message_id"].clone()).unwrap();
    let original_tool = ToolCallRef {
        turn_id: root.source.turn.turn_id.clone(),
        tool_call_id: ToolCallId("delegate-first".into()),
    };
    let startup = f
        .host
        .toolcall_get(&f.connection.caller(), &parent, &original_tool)
        .unwrap();
    let initial = &startup.data.state.delegations[0];
    assert_eq!(initial.parent, parent);
    assert_eq!(initial.origin, original_tool);
    assert_eq!(
        initial.accepted_message_ids.as_slice(),
        std::slice::from_ref(&first)
    );
    assert!(
        initial.inputs.is_empty(),
        "the busy child's unrelated Turn is not yet delegated"
    );
    assert!(initial.completion.is_none());
    after_first.tool("delegate-steer", json!({"action":"send", "target":target(&child), "text":"supplemental steer", "mode":"steer"}));
    let after_second = f.next(&parent).await;
    let supplemental_receipt = after_second.last_tool_result();
    assert_eq!(supplemental_receipt["result"]["delivery"], "accepted");
    let supplemental: MessageId =
        serde_json::from_value(supplemental_receipt["message_id"].clone()).unwrap();
    let appended = f
        .host
        .toolcall_get(
            &f.connection.caller(),
            &parent,
            &ToolCallRef {
                turn_id: root.source.turn.turn_id.clone(),
                tool_call_id: ToolCallId("delegate-steer".into()),
            },
        )
        .unwrap();
    assert_eq!(appended.data.state.delegations[0].origin, original_tool);
    assert_eq!(appended.data.state.delegations[0].task_message_id, first);
    after_second.complete("launches accepted");
    f.finished(&root.source).await;

    // The real agent tool runs in T0. Runtime consumes M2 at the next request,
    // while the earlier follow-up M1 must remain queued for a different Turn.
    old.tool(
        "inspect-shared-child",
        json!({"action":"inspect", "target":target(&child)}),
    );
    let continuation = f.next(&child).await;
    assert_eq!(continuation.source.turn, old.source.turn);
    assert_eq!(
        continuation.last_tool_result()["session_id"],
        json!(child.session_id)
    );
    assert!(
        matches!(f.host.message(&f.connection.caller(), &child, &supplemental).unwrap().binding,
        MessageBinding::Bound { turn_id } if turn_id == old.source.turn.turn_id)
    );
    assert!(matches!(
        f.host
            .message(&f.connection.caller(), &child, &first)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
    let child_state = f
        .host
        .session_state(&f.connection.caller(), &child)
        .unwrap();
    let binding = child_state.data.delegation.unwrap();
    assert_eq!(binding.inputs.len(), 1);
    assert_eq!(binding.inputs[0].turn, old.source.turn);
    assert_eq!(
        binding.inputs[0].message_ids,
        [unrelated.clone(), supplemental.clone()]
    );
    assert_eq!(
        binding.inputs[0].delegated_message_ids.as_slice(),
        std::slice::from_ref(&supplemental)
    );
    assert!(binding.completion.is_none());
    match end {
        End::RemoveFirst => {
            f.host
                .remove_input(&f.connection.caller(), &child, &first)
                .unwrap();
            continuation.complete("actual supplemental answer");
        }
        End::Complete => continuation.complete("actual supplemental answer"),
        End::Cancel | End::Fail => {
            f.host.pause(&f.connection.caller(), &child).unwrap();
            continuation
                .events
                .send(ModelEvent::text_delta(
                    "actual unfinished supplemental text",
                ))
                .unwrap();
            if matches!(end, End::Cancel) {
                f.host
                    .cancel_session(&f.connection.caller(), &child, true)
                    .unwrap();
            }
            // EOF without Finish is a real Runtime model failure, not a
            // manufactured owner result. Cancellation uses the actual handle.
            drop(continuation);
            f.finished(&old.source).await;
            let result = f
                .host
                .turn_result(&f.connection.caller(), &child, &old.source.turn.turn_id)
                .unwrap();
            assert_eq!(
                result.info.outcome,
                Some(if matches!(end, End::Cancel) {
                    TurnOutcome::Cancelled
                } else {
                    TurnOutcome::Failed
                })
            );
            assert!(f.calls.try_recv().is_err());
            assert!(
                f.host
                    .session_state(&f.connection.caller(), &child)
                    .unwrap()
                    .data
                    .delegation
                    .unwrap()
                    .completion
                    .is_none(),
                "a finished failed/cancelled Turn with preserved input is not delegation completion"
            );
            assert!(f.host.session_info(&f.connection.caller(), &child).is_ok());
            assert!(matches!(
                f.host
                    .message(&f.connection.caller(), &child, &first)
                    .unwrap()
                    .binding,
                MessageBinding::Queued
            ));
            assert!(
                f.host
                    .session_state(&f.connection.caller(), &parent)
                    .unwrap()
                    .data
                    .next_turn
                    .is_empty()
            );
            f.host.resume(&f.connection.caller(), &child).unwrap();
        }
    }
    if !matches!(end, End::RemoveFirst) {
        let follow_up = f.next(&child).await;
        assert_ne!(follow_up.source.turn, old.source.turn);
        assert!(
            f.host
                .session_state(&f.connection.caller(), &parent)
                .unwrap()
                .data
                .next_turn
                .is_empty()
        );
        follow_up.complete("actual initial follow-up answer");
    }
    let notified = f.next(&parent).await;
    assert_ne!(notified.source.turn, root.source.turn);
    let notice = notified
        .request
        .messages()
        .iter()
        .filter_map(|message| {
            if let ModelMessage::User(text) = message {
                let (context, original) = input_context::decode(text);
                let notice = serde_json::from_str::<Value>(original).ok()?;
                if notice.get("task_message_id").is_some() {
                    assert_eq!(
                        context.source,
                        MessageSource::Agent {
                            source: child.clone()
                        }
                    );
                }
                Some(notice)
            } else {
                None
            }
        })
        .find(|value| value.get("task_message_id").is_some())
        .expect("one actual final notification");
    assert_eq!(notice["source"], json!(child));
    assert_eq!(notice["task_message_id"], json!(first));
    assert_eq!(notice["origin"]["tool_call_id"], "delegate-first");
    assert_eq!(notice["status"], "ended");
    assert_eq!(notice["inputs"][0]["turn"], json!(old.source.turn));
    assert_eq!(
        notice["inputs"][0]["message_ids"],
        json!([unrelated, supplemental.clone()])
    );
    assert_eq!(
        notice["inputs"][0]["delegated_message_ids"],
        json!([supplemental])
    );
    let count = if matches!(end, End::RemoveFirst) {
        1
    } else {
        2
    };
    assert_eq!(notice["results"].as_array().unwrap().len(), count);
    let completed = f
        .host
        .toolcall_get(&f.connection.caller(), &parent, &original_tool)
        .unwrap();
    let settled = &completed.data.state.delegations[0];
    assert_eq!(settled.origin, original_tool);
    assert_eq!(settled.inputs.len(), count);
    assert_eq!(settled.completion.as_ref().unwrap().turns.len(), count);
    assert_eq!(
        serde_json::to_value(&settled.inputs).unwrap(),
        notice["inputs"]
    );
    assert_eq!(
        serde_json::to_value(&settled.completion.as_ref().unwrap().discarded_inputs).unwrap(),
        notice["discarded_inputs"]
    );
    assert!(completed.data.state.runner_revision > startup.data.state.runner_revision);
    assert_eq!(
        completed.data.state.state,
        minicore_host_core::ToolCallState::Finished
    );
    assert_eq!(
        notice["results"][0]["turn_id"],
        json!(old.source.turn.turn_id)
    );
    match end {
        End::Complete | End::RemoveFirst => assert_eq!(
            notice["results"][0]["output"],
            json!(["actual supplemental answer"])
        ),
        End::Cancel => assert_eq!(notice["results"][0]["outcome"], "cancelled"),
        End::Fail => assert_eq!(notice["results"][0]["outcome"], "failed"),
    }
    if matches!(end, End::RemoveFirst) {
        assert_eq!(notice["discarded_inputs"], json!([first]));
    } else {
        assert_eq!(notice["inputs"][1]["message_ids"], json!([first]));
        assert_eq!(
            notice["results"][1]["output"],
            json!(["actual initial follow-up answer"])
        );
        assert_eq!(notice["discarded_inputs"], json!([]));
    }
    let child_result = f
        .host
        .turn_result(&f.connection.caller(), &child, &old.source.turn.turn_id)
        .unwrap();
    assert!(child_result.history.iter().any(|item| matches!(item, HistoryItem::ToolResult(result) if result.call_id.as_str() == "inspect-shared-child")));
    let original = f
        .host
        .turn_result(&f.connection.caller(), &parent, &root.source.turn.turn_id)
        .unwrap();
    let saved_launch = original
        .history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(result) if result.call_id.as_str() == "delegate-first" => {
                Some(serde_json::from_str::<Value>(result.output.content().as_str()).unwrap())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(saved_launch, first_receipt);
    notified.complete("received one real notification");
    f.finished(&notified.source).await;
    let rows = f
        .host
        .history_read(
            &f.connection.caller(),
            &parent,
            &HistoryReadRequest {
                cursor: None,
                limit: 100,
                direction: HistoryDirection::Forward,
            },
        )
        .await
        .unwrap()
        .data
        .items;
    let saved_start = rows.iter().find(|row| matches!(&row.item,
        minicore_host_core::HistoryItemData::ToolResult { tool_call_id, .. } if tool_call_id == &original_tool.tool_call_id)).unwrap();
    assert_eq!(saved_start.delegations[0].origin, original_tool);
    assert!(
        saved_start.delegations[0].completion.is_none(),
        "startup history stays an acceptance fact"
    );
    let saved_notice = rows
        .iter()
        .find(|row| {
            matches!(&row.item,
        minicore_host_core::HistoryItemData::User { in_reply_to: Some(id), .. } if id == &first)
        })
        .unwrap();
    assert_eq!(saved_notice.delegations, completed.data.state.delegations);
    assert!(f.calls.try_recv().is_err());
    assert_eq!(
        f.host.turns(&f.connection.caller(), &parent).unwrap().len(),
        2
    );
    // Reused ordinary memory children remain available until explicitly closed.
    assert!(f.host.session_info(&f.connection.caller(), &child).is_ok());
    f.host
        .close_session(&f.connection.caller(), &child)
        .unwrap();
    assert!(f.host.session_info(&f.connection.caller(), &child).is_err());
}
#[tokio::test]
async fn real_agent_tool_notifies_both_shared_turns_in_actual_order() {
    shared_range_with_real_tools(End::Complete).await;
}
#[tokio::test]
async fn real_agent_tool_accepts_supplemental_result_after_first_input_removed() {
    shared_range_with_real_tools(End::RemoveFirst).await;
}
#[tokio::test]
async fn real_agent_tool_retains_failed_shared_turn_until_preserved_inbox_settles() {
    shared_range_with_real_tools(End::Fail).await;
}
#[tokio::test]
async fn real_agent_tool_retains_cancelled_shared_turn_until_preserved_inbox_settles() {
    shared_range_with_real_tools(End::Cancel).await;
}

#[tokio::test]
async fn reused_child_has_new_anchor_and_cannot_extend_old_delegation() {
    let mut f = Fixture::new();
    let parent = f.agent().await;
    let child = f.agent().await;
    let mut originals = Vec::new();
    for number in 0..2 {
        f.send(&parent, &format!("parent task {number}"));
        let root = f.next(&parent).await;
        f.send(&child, &format!("unrelated child task {number}"));
        let unrelated = f.next(&child).await;
        root.tool("start", json!({"action":"send", "target":target(&child), "text":format!("delegated task {number}"), "mode":"follow_up", "delegate":true}));
        let accepted = f.next(&parent).await;
        let origin = ToolCallRef {
            turn_id: root.source.turn.turn_id.clone(),
            tool_call_id: ToolCallId("start".into()),
        };
        let initial = f
            .host
            .toolcall_get(&f.connection.caller(), &parent, &origin)
            .unwrap()
            .data
            .state
            .delegations[0]
            .clone();
        assert!(initial.inputs.is_empty());
        accepted.complete("accepted while child runs");
        f.finished(&root.source).await;
        unrelated.complete("unrelated finished");
        let delegated = f.next(&child).await;
        assert_ne!(delegated.source.turn, unrelated.source.turn);
        delegated.complete("actual delegated result");
        let notified = f.next(&parent).await;
        notified.complete("notification consumed");
        f.finished(&notified.source).await;
        let mut states = f
            .host
            .session_states(&f.connection.caller(), &child)
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while states.snapshot().unwrap().data.delegation.is_some() {
                states.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let terminal = f
            .host
            .toolcall_get(&f.connection.caller(), &parent, &origin)
            .unwrap()
            .data
            .state
            .delegations[0]
            .clone();
        assert_eq!(terminal.inputs.len(), 1);
        assert_eq!(terminal.inputs[0].turn, delegated.source.turn);
        assert_eq!(
            terminal.completion.as_ref().unwrap().turns[0].outcome,
            TurnOutcome::Completed
        );
        originals.push((origin, terminal));
    }
    assert_ne!(
        originals[0].1.task_message_id,
        originals[1].1.task_message_id
    );
    assert_ne!(originals[0].0.turn_id, originals[1].0.turn_id);
    assert_eq!(
        originals[0].0.tool_call_id, originals[1].0.tool_call_id,
        "ToolCallId alone is not a card identity"
    );
    let first = f
        .host
        .toolcall_get(&f.connection.caller(), &parent, &originals[0].0)
        .unwrap();
    assert_eq!(first.data.state.delegations[0], originals[0].1);
    assert!(
        !first.data.state.delegations[0]
            .accepted_message_ids
            .contains(&originals[1].1.task_message_id)
    );
}

#[tokio::test]
async fn rejected_agent_send_is_failed_tool_without_failing_parent_turn() {
    let mut f = Fixture::new();
    let parent = f.agent().await;
    f.send(&parent, "attempt missing child");
    let root = f.next(&parent).await;
    let missing = SessionRef {
        agent_id: parent.agent_id.clone(),
        session_id: SessionId::new().unwrap(),
    };
    root.tool("rejected-send", json!({"action":"send", "target":target(&missing), "text":"cannot be accepted", "delegate":true}));
    let continued = f.next(&parent).await;
    let failed = f
        .host
        .toolcall_get(
            &f.connection.caller(),
            &parent,
            &ToolCallRef {
                turn_id: root.source.turn.turn_id.clone(),
                tool_call_id: ToolCallId("rejected-send".into()),
            },
        )
        .unwrap();
    assert_eq!(
        failed.data.result.as_ref().unwrap().outcome,
        minicore_host_runtime::tools::ToolResultOutcome::Failed
    );
    assert!(failed.data.state.delegations.is_empty());
    assert!(continued.last_tool_result().get("error").is_some());
    continued.complete("ordinary tool failure handled");
    f.finished(&root.source).await;
    assert_eq!(
        f.host
            .turn_result(&f.connection.caller(), &parent, &root.source.turn.turn_id)
            .unwrap()
            .info
            .outcome,
        Some(TurnOutcome::Completed)
    );
}
