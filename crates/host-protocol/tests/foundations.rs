use minicore_host_protocol::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

const ID: &str = "9k3m7x2p5v8c4r6t0w1z";

#[test]
fn ids_are_strict_random_and_opaque() {
    assert_eq!(ID.len(), ID_LENGTH);
    let parsed: HostId = ID.parse().unwrap();
    assert_eq!(parsed.as_str(), ID);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), json!(ID));
    let mut seen = BTreeSet::new();
    for _ in 0..10_000 {
        let id = MessageId::new().unwrap();
        assert_eq!(id.as_str().len(), 20);
        assert!(id.as_str().bytes().all(|b| ID_ALPHABET.contains(&b)));
        assert!(seen.insert(id));
    }
}

#[test]
fn invalid_ids_are_not_normalized() {
    for invalid in [
        "",
        "a",
        "9k3m7x2p5v8c4r6t0w1",
        "9k3m7x2p5v8c4r6t0w1zz",
        "9K3m7x2p5v8c4r6t0w1z",
        "9i3m7x2p5v8c4r6t0w1z",
        "9l3m7x2p5v8c4r6t0w1z",
        "9o3m7x2p5v8c4r6t0w1z",
        "9u3m7x2p5v8c4r6t0w1z",
        "9-3m7x2p5v8c4r6t0w1z",
    ] {
        assert!(invalid.parse::<HostId>().is_err(), "{invalid}");
        assert!(serde_json::from_value::<HostId>(json!(invalid)).is_err());
    }
    assert!(serde_json::from_value::<HostId>(json!(123)).is_err());
}

#[test]
fn provider_tool_call_id_is_preserved_and_turn_scoped() {
    let upstream = ToolCallId("CALL_provider-I/O_123".into());
    let reference = ToolCallRef {
        turn_id: ID.parse().unwrap(),
        tool_call_id: upstream.clone(),
    };
    let wire = serde_json::to_value(&reference).unwrap();
    assert_eq!(wire["tool_call_id"], json!(upstream.as_str()));
    assert_eq!(
        serde_json::from_value::<ToolCallRef>(wire).unwrap(),
        reference
    );
}

#[test]
fn references_contain_only_immediate_owner() {
    let session: SessionRef =
        serde_json::from_value(json!({"agent_id":ID,"session_id":ID})).unwrap();
    assert_eq!(session.agent_id.as_str(), ID);
    for wire in [
        json!({"host_id":ID,"agent_id":ID,"session_id":ID}),
        json!({"session_id":ID}),
        json!({"agent":{"id":ID},"session_id":ID}),
    ] {
        assert!(serde_json::from_value::<SessionRef>(wire).is_err());
    }
}

#[test]
fn mode_matrix_never_promotes_a_memory_owner() {
    use StorageMode::*;
    assert_eq!(Memory.session_mode(None).unwrap(), Memory);
    assert_eq!(Persistent.session_mode(None).unwrap(), Persistent);
    assert_eq!(Persistent.session_mode(Some(Memory)).unwrap(), Memory);
    assert!(Memory.session_mode(Some(Persistent)).is_err());
}

#[test]
fn configuration_roundtrips_without_runtime_state() {
    let config = AgentConfig::default();
    let wire = serde_json::to_value(&config).unwrap();
    assert_eq!(
        serde_json::from_value::<AgentConfig>(wire.clone()).unwrap(),
        config
    );
    for key in [
        "id",
        "mode",
        "host_id",
        "current_turn",
        "access_key",
        "system_prompt",
    ] {
        let mut extra = wire.clone();
        extra[key] = json!("injected");
        assert!(
            serde_json::from_value::<AgentConfig>(extra).is_err(),
            "{key}"
        );
    }
}

#[test]
fn patches_distinguish_omission_clear_and_replacement() {
    let original = SessionConfig {
        workspace: Some(WorkspaceRef {
            path: "/old".into(),
        }),
        tools: vec![ToolBinding {
            name: "read".into(),
        }],
        ..SessionConfig::default()
    };
    let omitted: SessionConfigPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(original.apply_patch(&omitted).unwrap(), original);
    let clear: SessionConfigPatch =
        serde_json::from_value(json!({"workspace":null,"tools":[]})).unwrap();
    let candidate = original.apply_patch(&clear).unwrap();
    assert!(candidate.workspace.is_none());
    assert!(candidate.tools.is_empty());
    assert!(original.workspace.is_some());
    assert_eq!(original.tools.len(), 1);
    assert_eq!(
        serde_json::to_value(&clear).unwrap(),
        json!({"workspace":null,"tools":[]})
    );
    for key in ["role", "model", "reasoning", "tools", "execution"] {
        let mut wire = json!({});
        wire[key] = Value::Null;
        assert!(
            serde_json::from_value::<SessionConfigPatch>(wire).is_err(),
            "{key}"
        );
    }
    assert!(serde_json::from_value::<SessionConfigPatch>(json!({"role":{"name":"new"}})).is_err());
    assert!(serde_json::from_value::<SessionConfigPatch>(json!({"mode":"memory"})).is_err());
}

#[test]
fn candidate_validation_preserves_original_and_independent_snapshot() {
    let mut agent = AgentConfig::default();
    let session = SessionConfig::from_agent(&agent, &SessionOverrides::default()).unwrap();
    agent.role.instructions = "new defaults".into();
    assert_eq!(session.role.instructions, "");
    let patch = SessionConfigPatch {
        tools: Some(vec![
            ToolBinding {
                name: "read".into(),
            },
            ToolBinding {
                name: "read".into(),
            },
        ]),
        ..Default::default()
    };
    assert!(session.apply_patch(&patch).is_err());
    assert!(session.tools.is_empty());
}

#[test]
fn supported_execution_settings_exclude_application_quotas() {
    let wire = serde_json::to_value(ExecutionSettings::default()).unwrap();
    for key in [
        "event_capacity",
        "max_tool_rounds",
        "max_sessions",
        "max_input_bytes",
        "deadline",
    ] {
        let mut extra = wire.clone();
        extra[key] = json!(5);
        assert!(
            serde_json::from_value::<ExecutionSettings>(extra).is_err(),
            "{key}"
        );
    }
    for (key, value) in [
        ("model_retry_attempts", 0),
        ("model_retry_attempts", 5),
        ("model_timeout_ms", 0),
        ("model_timeout_ms", 86_400_001),
        ("model_retry_base_delay_ms", 30_001),
    ] {
        let mut bad = wire.clone();
        bad[key] = json!(value);
        assert!(serde_json::from_value::<ExecutionSettings>(bad).is_err());
    }
}

#[test]
fn invalid_config_values_fail_during_deserialization() {
    assert!(serde_json::from_value::<ModelRef>(json!("model with spaces")).is_err());
    assert!(serde_json::from_value::<ToolBinding>(json!({"name":""})).is_err());
    assert!(serde_json::from_value::<WorkspaceRef>(json!({"path":""})).is_err());
    assert!(
        serde_json::from_value::<Role>(json!({"name":"", "description":"", "instructions":""}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<CompactionSettings>(
            json!({"enabled":true,"trigger_percent":95,"target_percent":95})
        )
        .is_err()
    );
}

#[test]
fn every_fault_type_and_code_has_one_registry_mapping() {
    let mut codes = BTreeSet::new();
    let mut names = BTreeSet::new();
    for kind in FAULT_KINDS {
        assert!(codes.insert(kind.code()));
        assert!(names.insert(kind.type_name()));
        let fault = Fault::new(*kind, "safe summary");
        let value = serde_json::to_value(&fault).unwrap();
        assert_eq!(value["type"], json!(kind.type_name()));
        assert_eq!(value["code"], json!(kind.code()));
        assert_eq!(serde_json::from_value::<Fault>(value).unwrap(), fault);
    }
    assert_eq!(FaultKind::SessionLifecycleClosed.code(), 3101);
    assert_eq!(FaultKind::SessionHistoryWriteFailed.code(), 3301);
    assert!(FaultKind::TurnModelUpstreamRateLimited.is_a(FaultClass::TurnModelUpstream));
}

#[test]
fn faults_reject_mismatched_codes_and_arbitrary_details() {
    for wire in [
        json!({"type":"SESSION_LIFECYCLE_CLOSED","code":3301,"message":"x"}),
        json!({"type":"SESSION_LIFECYCLE_CLOSED","code":3101,"message":"x","kind":"extra"}),
        json!({"type":"SESSION_LIFECYCLE_CLOSED","code":3101,"message":"x","details":{}}),
        json!({"type":"SESSION_HISTORY_WRITE_FAILED","code":3301,"message":"x","details":{"stage":"rename"}}),
        json!({"type":"SESSION_HISTORY_WRITE_FAILED","code":3301,"message":"x","details":{"stage":"sync","raw_error":"secret"}}),
        json!({"type":"TURN_MODEL_UPSTREAM_QUOTA_EXHAUSTED","code":4203,"message":"x","retry_after_ms":5}),
    ] {
        assert!(serde_json::from_value::<Fault>(wire).is_err());
    }
}

#[test]
fn typed_fault_details_and_wait_hints_are_not_retry_authorization() {
    let write = Fault::new(FaultKind::SessionHistoryWriteFailed, "sync failed")
        .with_details(FaultDetails::HistoryWrite(HistoryWriteDetails {
            stage: HistoryWriteStage::Sync,
        }))
        .unwrap();
    assert_eq!(
        serde_json::from_value::<Fault>(serde_json::to_value(&write).unwrap()).unwrap(),
        write
    );
    assert!(write.with_retry_after_ms(100).is_err());
    let upstream = Fault::new(FaultKind::TurnModelUpstreamRateLimited, "rate limited")
        .with_retry_after_ms(10)
        .unwrap();
    assert_eq!(upstream.retry_after_ms(), Some(10));
    assert!(
        Fault::new(FaultKind::SessionHistoryLoadFailed, "corrupt")
            .with_details(FaultDetails::HistoryLoad(HistoryLoadDetails {
                line: Some(0),
                byte_offset: None
            }))
            .is_err()
    );
}
