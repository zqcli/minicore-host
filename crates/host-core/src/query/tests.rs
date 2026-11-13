//! Wire-contract fixtures only; these values do not claim actual execution.
use super::*;
use serde_json::{Value, json};

fn retained() -> TurnResultSnapshot {
    TurnResultSnapshot {
        source: TurnIdentity {
            session: SessionIdentity {
                host_id: HostId::new().unwrap(),
                host_incarnation: HostIncarnation::new().unwrap(),
                agent_id: AgentId::new().unwrap(),
                session_id: SessionId::new().unwrap(),
            },
            turn_id: TurnId::new().unwrap(),
            load_id: Some(LoadId::new().unwrap()),
        },
        data: TurnResultData {
            info: Some(TurnStateData {
                history_position: None,
                recorded_at: None,
                finished_at: None,
                state: TurnState::Finished,
                runner_revision: 2,
                config_revision: 1,
                outcome: Some(TurnOutcome::Completed),
                persistence: PersistenceState::Persisted,
                requests: 1,
                tool_rounds: 0,
                phase: None,
                model_retry: None,
                model_attempts: vec![1],
                fault: None,
                persistence_fault: None,
                cleanup_faults: Vec::new(),
            }),
            config: Some(SessionConfig::default()),
            saved_terminal: None,
            history: Vec::new(),
            partials: Vec::new(),
        },
    }
}

fn saved() -> TurnResultSnapshot {
    let mut value = retained();
    value.source.load_id = None;
    value.data.info = None;
    value.data.config = None;
    value.data.saved_terminal = Some(SavedTurnTerminal {
        position: 0,
        recorded_at: Timestamp::now(),
        terminal: crate::TurnTerminalData {
            finished_at: Timestamp::now(),
            requests: 0,
            tool_rounds: 0,
            model_attempts: Vec::new(),
            outcome: TurnOutcome::Completed,
            reason: None,
        },
    });
    value
}

#[test]
fn retained_and_saved_results_keep_their_distinct_wire_shapes() {
    for fixture in [retained(), saved()] {
        let value = serde_json::to_value(&fixture).unwrap();
        let read: TurnResultSnapshot = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(read).unwrap(), value);
        let retained = fixture.data.info.is_some();
        assert_eq!(value.get("load_id").is_some(), retained);
        assert_eq!(value["data"].get("info").is_some(), retained);
        assert_eq!(value["data"].get("config").is_some(), retained);
        assert_eq!(value["data"].get("saved_terminal").is_some(), !retained);
    }
}

#[test]
fn result_metadata_rejects_every_missing_or_mixed_branch() {
    let live = serde_json::to_value(retained().data).unwrap();
    let saved = serde_json::to_value(saved().data).unwrap();
    for mask in 0..8 {
        let mut value = json!({"history": [], "partials": []});
        for (bit, key, source) in [
            (1, "info", &live),
            (2, "config", &live),
            (4, "saved_terminal", &saved),
        ] {
            if mask & bit != 0 {
                value[key] = source[key].clone();
            }
        }
        assert_eq!(
            serde_json::from_value::<TurnResultData>(value).is_ok(),
            matches!(mask, 3 | 4),
            "metadata combination {mask}"
        );
    }
}

#[test]
fn result_identity_rejects_a_missing_or_invented_load() {
    let mut live = serde_json::to_value(retained()).unwrap();
    let load = live.as_object_mut().unwrap().remove("load_id").unwrap();
    assert!(serde_json::from_value::<TurnResultSnapshot>(live).is_err());
    let mut saved: Value = serde_json::to_value(saved()).unwrap();
    saved["load_id"] = load;
    assert!(serde_json::from_value::<TurnResultSnapshot>(saved).is_err());
}

#[test]
fn invalid_public_result_values_cannot_be_serialized_as_success() {
    let mut fixture = retained();
    fixture.data.info = None;
    assert!(serde_json::to_value(&fixture.data).is_err());
    assert!(serde_json::to_value(&fixture).is_err());
    fixture.data.config = None;
    assert!(serde_json::to_value(&fixture.data).is_err());
    let mut fixture = saved();
    fixture.source.load_id = Some(LoadId::new().unwrap());
    assert!(serde_json::to_value(&fixture).is_err());
    let mut fixture = retained();
    fixture.source.load_id = None;
    assert!(serde_json::to_value(&fixture).is_err());
}
