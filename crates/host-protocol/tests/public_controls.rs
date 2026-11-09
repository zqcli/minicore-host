use minicore_host_protocol::*;
use serde_json::{Value, json};

fn operation() -> Value {
    json!(OperationId::new().unwrap())
}
#[test]
fn metadata_changes_reject_empty_unknown_and_illegal_null_values() {
    for changes in [
        json!({}),
        json!({"id":"other"}),
        json!({"name":null}),
        json!({"name":" "}),
        json!({"config":{"role":null}}),
        json!({"config":{"mode":"memory"}}),
    ] {
        assert!(serde_json::from_value::<AgentUpdateChanges>(changes).is_err());
    }
    for changes in [
        json!({}),
        json!({"title":""}),
        json!({"title":"bad\u{0000}"}),
        json!({"config":{"model":null}}),
        json!({"config":{"default_session_id":null}}),
    ] {
        assert!(serde_json::from_value::<SessionUpdateChanges>(changes).is_err());
    }
    let clear: SessionUpdateChanges =
        serde_json::from_value(json!({"title":null,"config":{"tools":[],"workspace":null}}))
            .unwrap();
    assert_eq!(clear.title, NullablePatch::Clear);
    assert_eq!(clear.config.tools, Some(vec![]));
    assert_eq!(clear.config.workspace, NullablePatch::Clear);
    assert!(serde_json::to_value(AgentUpdateChanges::default()).is_err());
}
#[test]
fn default_target_must_be_explicit_and_cannot_claim_another_owner() {
    assert!(
        serde_json::from_value::<AgentSetDefaultSessionRequest>(
            json!({"operation_id":operation()})
        )
        .is_err()
    );
    let clear: AgentSetDefaultSessionRequest =
        serde_json::from_value(json!({"operation_id":operation(),"session_id":null})).unwrap();
    assert_eq!(clear.session_id, None);
    assert!(
        serde_json::from_value::<AgentSetDefaultSessionRequest>(
            json!({"operation_id":operation(),"session_id":null,"agent_id":AgentId::new().unwrap()})
        )
        .is_err()
    );
}
#[test]
fn saving_and_committed_receipts_have_distinct_strict_facts() {
    let receipt = AgentUpdateReceipt {
        kind: AgentUpdateKind::AgentUpdate,
        host_id: HostId::new().unwrap(),
        host_incarnation: HostIncarnation::new().unwrap(),
        operation_id: OperationId::new().unwrap(),
        agent_id: AgentId::new().unwrap(),
        result: MetadataUpdateResult::Saving { config_revision: 4 },
    };
    let value = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        serde_json::from_value::<Receipt>(value.clone()).unwrap(),
        Receipt::AgentUpdate(receipt)
    );
    let mut invalid = value;
    invalid["result"]["committed"] = json!(true);
    assert!(serde_json::from_value::<Receipt>(invalid).is_err());
    assert!(serde_json::from_value::<AgentStartResult>(json!({"lifecycle":"stopped"})).is_err());
    assert!(
        serde_json::from_value::<MetadataUpdateResult>(
            json!({"status":"saving","config_revision":0,"error":null})
        )
        .is_err()
    );
}
#[test]
fn queued_inputs_validate_each_fifo_without_inventing_global_order_or_turn() {
    let entry = |queue, order| QueuedInputRecord {
        input: MessageInput {
            message_id: MessageId::new().unwrap(),
            mode: MessageMode::Steer,
            text: "queued".into(),
            in_reply_to: None,
        },
        source: MessageSource::User,
        queue,
        order,
    };
    let list = InputList {
        host_id: HostId::new().unwrap(),
        host_incarnation: HostIncarnation::new().unwrap(),
        agent_id: AgentId::new().unwrap(),
        session_id: SessionId::new().unwrap(),
        controller_revision: 5,
        next_step: vec![
            entry(InputQueue::NextStep, 7),
            entry(InputQueue::NextStep, 9),
        ],
        next_turn: vec![entry(InputQueue::NextTurn, 2)],
    };
    let value = serde_json::to_value(&list).unwrap();
    assert_eq!(
        serde_json::from_value::<InputList>(value.clone()).unwrap(),
        list
    );
    let mut invalid = value.clone();
    invalid["next_step"][0]["bound_turn"] = json!(TurnId::new().unwrap());
    assert!(serde_json::from_value::<InputList>(invalid).is_err());
    let mut invalid = value.clone();
    invalid["next_step"][0]["queue"] = json!("next-turn");
    assert!(serde_json::from_value::<InputList>(invalid).is_err());
    let mut invalid = value;
    invalid["next_step"][1]["order"] = json!(6);
    assert!(serde_json::from_value::<InputList>(invalid).is_err());
}
#[test]
fn removal_has_required_identity_and_strict_optional_precondition() {
    let value = json!({"operation_id":operation(),"message_id":MessageId::new().unwrap()});
    assert!(serde_json::from_value::<InputRemoveRequest>(value.clone()).is_ok());
    let mut invalid = value.clone();
    invalid["expected_controller_revision"] = Value::Null;
    assert!(serde_json::from_value::<InputRemoveRequest>(invalid).is_err());
    let mut invalid = value;
    invalid["bound_turn"] = json!(TurnId::new().unwrap());
    assert!(serde_json::from_value::<InputRemoveRequest>(invalid).is_err());
}
