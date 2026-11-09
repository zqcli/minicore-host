use minicore_host_protocol::*;
use serde_json::json;

#[test]
fn subscriptions_require_categories_and_deduplicate_them() {
    let scope = SubscriptionScope::Host {
        host_id: HostId::new().unwrap(),
    };
    assert!(
        serde_json::from_value::<SubscribeRequest>(json!({"scope":scope,"categories":[]})).is_err()
    );
    let value: SubscribeRequest =
        serde_json::from_value(json!({"scope":scope,"categories":["state","state","output"]}))
            .unwrap();
    assert_eq!(value.categories.len(), 2);
    assert!(
        serde_json::from_value::<SubscribeRequest>(
            json!({"scope":scope,"categories":["arbitrary"]})
        )
        .is_err()
    );
}
#[test]
fn range_contract_validates_utf8_bytes_and_nonempty_progress() {
    let delta = OutputDelta {
        channel: "text".into(),
        start: 4,
        end: 7,
        text: "中".into(),
    };
    let value = serde_json::to_value(&delta).unwrap();
    assert_eq!(serde_json::from_value::<OutputDelta>(value).unwrap(), delta);
    assert!(
        serde_json::from_value::<OutputDelta>(
            json!({"channel":"text","start":0,"end":1,"text":"中"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<TextRangeData>(
            json!({"channel":"text","start":1,"end":1,"text":"","has_more":true})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<TextReadRequest>(
            json!({"channel":"text","start":2,"end":1,"max_bytes":1})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<TextReadRequest>(
            json!({"channel":"text","start":0,"end":1,"max_bytes":0})
        )
        .is_err()
    );
}
#[test]
fn output_source_has_one_identity_and_no_fabricated_load() {
    let source = OutputSource::Model {
        source: ModelOutputIdentity {
            turn: TurnIdentity {
                session: SessionIdentity {
                    host_id: HostId::new().unwrap(),
                    host_incarnation: HostIncarnation::new().unwrap(),
                    agent_id: AgentId::new().unwrap(),
                    session_id: SessionId::new().unwrap(),
                },
                turn_id: TurnId::new().unwrap(),
                load_id: None,
            },
            response_index: 2,
        },
    };
    let position = OutputPosition {
        source,
        channel: "text".into(),
        end: 8,
    };
    let value = serde_json::to_value(&position).unwrap();
    assert!(value.get("load_id").is_none());
    assert_eq!(value["kind"], "model");
    assert!(value.get("session_id").is_some());
    assert_eq!(
        serde_json::from_value::<OutputPosition>(value).unwrap(),
        position
    );
}

#[test]
fn retry_state_validates_attempts_and_source_rfc3339_time() {
    let progress = ModelRetryProgress {
        response_index: 0,
        attempt: 2,
        max_attempts: 3,
        fault: Fault::new(FaultKind::TurnModelUpstreamOverloaded, "retry fixture"),
        phase: ModelRetryPhase::Waiting {
            retry_at: time::OffsetDateTime::UNIX_EPOCH,
        },
    };
    let value = serde_json::to_value(&progress).unwrap();
    assert_eq!(value["retry_at"], "1970-01-01T00:00:00Z");
    assert_eq!(
        serde_json::from_value::<ModelRetryProgress>(value.clone()).unwrap(),
        progress
    );
    let mut invalid = value.clone();
    invalid["attempt"] = json!(1);
    assert!(serde_json::from_value::<ModelRetryProgress>(invalid).is_err());
    let mut invalid = value;
    invalid["retry_at"] = json!("yesterday");
    assert!(serde_json::from_value::<ModelRetryProgress>(invalid).is_err());
}
