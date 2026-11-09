use minicore_host_protocol::*;
use serde_json::{Value, json};

fn unknown() -> Value {
    json!({
        "type":"SESSION_FUTURE_FIXTURE_LEAF", "code":3599,
        "message":"future fixture failure, not a protocol error",
        "diagnostic_id":"fixture-diagnostic", "retry_after_ms":123,
        "details":{"future_field":["preserve", 42, {"nested":true}]}
    })
}

#[test]
fn unknown_fault_roundtrips_raw_facts_without_inventing_known_classification() {
    let wire = unknown();
    let fault: Fault = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(fault.kind(), None);
    assert_eq!(fault.code(), 3599);
    assert_eq!(fault.type_name(), "SESSION_FUTURE_FIXTURE_LEAF");
    assert_eq!(
        fault.message(),
        "future fixture failure, not a protocol error"
    );
    assert_eq!(fault.diagnostic_id(), Some("fixture-diagnostic"));
    assert_eq!(fault.retry_after_ms(), Some(123));
    assert!(
        matches!(fault.details(), Some(FaultDetails::Unrecognized(value)) if value == &wire["details"])
    );
    assert!(!fault.is_a(FaultClass::Session));
    assert!(!fault.is_a(FaultClass::Host));
    assert_eq!(serde_json::to_value(&fault).unwrap(), wire);
    assert!(fault.to_string().contains("3599"));
    fault.validate().unwrap();
}

#[test]
fn forward_compatibility_does_not_relax_registered_mapping_or_known_details() {
    for wire in [
        json!({"type":"HOST_ACCESS_INVALID","code":1499,"message":"wrong registered code"}),
        json!({"type":"HOST_UNKNOWN_FIXTURE","code":1001,"message":"known code alias"}),
        json!({"type":"HOST_ACCESS_INVALID","code":1001,"message":"wrong known details","details":{"future":true}}),
        json!({"type":"SESSION_HISTORY_WRITE_FAILED","code":3301,"message":"invalid schema","details":{"stage":"future"}}),
    ] {
        assert!(serde_json::from_value::<Fault>(wire).is_err());
    }
    assert!(
        Fault::new(FaultKind::HostAccessInvalid, "known")
            .with_details(FaultDetails::Unrecognized(json!({})))
            .is_err()
    );
    assert!(
        serde_json::from_value::<Fault>(unknown())
            .unwrap()
            .with_details(FaultDetails::HistoryWrite(HistoryWriteDetails {
                stage: HistoryWriteStage::Write
            }))
            .is_err()
    );
}

#[test]
fn unknown_faults_survive_nested_partial_creation_receipts() {
    // Only the unfamiliar fault is synthetic; construct the receipt with the
    // public typed shape so its partial-creation facts are checked normally.
    let fault: Fault = serde_json::from_value(unknown()).unwrap();
    let receipt = AgentCreateReceipt {
        kind: AgentCreateKind::AgentCreate,
        host_id: HostId::new().unwrap(),
        host_incarnation: HostIncarnation::new().unwrap(),
        operation_id: OperationId::new().unwrap(),
        agent_id: Some(AgentId::new().unwrap()),
        session_id: None,
        message_id: None,
        result: AgentCreateResult::without_delivery(AgentCreateSteps {
            agent: CreateStep::Succeeded,
            session: CreateStep::Failed {
                error: fault.clone(),
            },
            message: None,
        }),
    };
    let wire = serde_json::to_value(&receipt).unwrap();
    let decoded: AgentCreateReceipt = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, receipt);
    assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
}
