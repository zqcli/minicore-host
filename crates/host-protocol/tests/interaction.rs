use minicore_host_protocol::*;
use serde_json::json;

#[test]
fn answer_request_requires_original_execution_identity_and_rejects_unknown_fields() {
    let request = InteractionAnswerRequest {
        operation_id: OperationId::new().unwrap(),
        turn_id: TurnId::new().unwrap(),
        load_id: LoadId::new().unwrap(),
        interaction_id: InteractionId::new().unwrap(),
        answer: true,
    };
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<InteractionAnswerRequest<bool>>(value.clone()).unwrap(),
        request
    );
    for field in [
        "operation_id",
        "turn_id",
        "load_id",
        "interaction_id",
        "answer",
    ] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<InteractionAnswerRequest<bool>>(missing).is_err());
        let mut null = value.clone();
        null[field] = json!(null);
        assert!(serde_json::from_value::<InteractionAnswerRequest<bool>>(null).is_err());
    }
    let mut invalid = value;
    invalid["internal"] = json!(true);
    assert!(serde_json::from_value::<InteractionAnswerRequest<bool>>(invalid).is_err());
}
#[test]
fn answered_receipt_roundtrips_through_common_receipt_without_claiming_completion() {
    let receipt = InteractionAnswerReceipt {
        kind: InteractionAnswerKind::InteractionAnswer,
        host_id: HostId::new().unwrap(),
        host_incarnation: HostIncarnation::new().unwrap(),
        agent_id: AgentId::new().unwrap(),
        session_id: SessionId::new().unwrap(),
        turn_id: TurnId::new().unwrap(),
        load_id: LoadId::new().unwrap(),
        interaction_id: InteractionId::new().unwrap(),
        operation_id: OperationId::new().unwrap(),
        result: InteractionAnswerResult {
            status: InteractionAnswerStatus::Answered,
        },
    };
    let value = serde_json::to_value(&receipt).unwrap();
    let common: Receipt = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(common.operation_id(), &receipt.operation_id);
    assert!(matches!(common, Receipt::InteractionAnswer(actual) if actual == receipt));
    for status in ["completed", "cancelled", "pending"] {
        let mut invalid = value.clone();
        invalid["result"]["status"] = json!(status);
        assert!(serde_json::from_value::<Receipt>(invalid).is_err());
    }
    let mut invalid = value;
    invalid["result"]["retry"] = json!(true);
    assert!(serde_json::from_value::<Receipt>(invalid).is_err());
}
