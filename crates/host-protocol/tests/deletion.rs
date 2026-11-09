use minicore_host_protocol::*;
use serde_json::json;

#[test]
fn omitted_scope_never_authorizes_cascade_or_close() {
    let operation = OperationId::new().unwrap();
    let agent: AgentDeleteRequest =
        serde_json::from_value(json!({"operation_id":operation})).unwrap();
    assert_eq!(agent.scope, AgentDeleteScope::Empty {});
    let session: SessionDeleteRequest =
        serde_json::from_value(json!({"operation_id":operation})).unwrap();
    assert!(!session.close);
    assert!(
        serde_json::from_value::<SessionDeleteRequest>(
            json!({"operation_id":operation,"recursive":true})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<AgentDeleteRequest>(
            json!({"operation_id":operation,"scope":{"kind":"empty","close":true}})
        )
        .is_err()
    );
}
#[test]
fn cascade_requires_explicit_distinct_session_ids() {
    let operation = OperationId::new().unwrap();
    let session = SessionId::new().unwrap();
    for ids in [json!([]), json!([session, session])] {
        assert!(
            serde_json::from_value::<AgentDeleteRequest>(
                json!({"operation_id":operation,"scope":{"kind":"sessions","session_ids":ids}})
            )
            .is_err()
        );
    }
    let request: AgentDeleteRequest = serde_json::from_value(json!({"operation_id":operation,"scope":{"kind":"sessions","session_ids":[session],"close":true}})).unwrap();
    assert_eq!(
        request.scope,
        AgentDeleteScope::Sessions {
            session_ids: vec![session],
            close: true
        }
    );
}
#[test]
fn failed_and_memory_deletion_are_distinct_from_physical_success() {
    assert!(serde_json::from_value::<DataDeletion>(json!({"status":"failed"})).is_err());
    assert!(
        serde_json::from_value::<DataDeletion>(
            json!({"status":"deleted","error":Fault::new(FaultKind::SessionDeleteFailed,"failure")})
        )
        .is_err()
    );
    assert_ne!(DataDeletion::Deleted {}, DataDeletion::Reclaimed {});
    assert!(!DataDeletion::NotAttempted {}.removed());
    let receipt = Receipt::SessionDelete(SessionDeleteReceipt {
        kind: SessionDeleteKind::SessionDelete,
        host_id: HostId::new().unwrap(),
        host_incarnation: HostIncarnation::new().unwrap(),
        operation_id: OperationId::new().unwrap(),
        agent_id: AgentId::new().unwrap(),
        session_id: SessionId::new().unwrap(),
        result: SessionDeleteAccepted {
            lifecycle: SessionCloseLifecycle::Closing,
            close: true,
        },
    });
    assert_eq!(
        serde_json::from_value::<Receipt>(serde_json::to_value(&receipt).unwrap()).unwrap(),
        receipt
    );
}

#[test]
fn deletion_acceptance_cannot_claim_open_or_unrequested_closing() {
    for value in [
        json!({"lifecycle":"open","close":true}),
        json!({"lifecycle":"closing","close":false}),
    ] {
        assert!(serde_json::from_value::<SessionDeleteAccepted>(value).is_err());
    }
    assert!(
        serde_json::to_value(SessionDeleteAccepted {
            lifecycle: SessionCloseLifecycle::Closing,
            close: false
        })
        .is_err()
    );
    let id = SessionId::new().unwrap();
    assert!(serde_json::from_value::<AgentDeleteAccepted>(json!({"lifecycle":"retiring","scope":{"kind":"sessions","session_ids":[id,id],"close":true}})).is_err());
}
