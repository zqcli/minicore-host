use minicore_host_protocol::*;
use minicore_host_tools::*;
use serde_json::{Value, json};

fn id() -> &'static str {
    "00000000000000000000"
}
fn session() -> Value {
    json!({"agent_id":id(),"session_id":id()})
}
fn agent() -> Value {
    json!({"host_id":id(),"agent_id":id()})
}
fn valid_calls() -> Vec<Value> {
    vec![
        json!({"action":"create","resource":"agent","target":{"host_id":id()},"name":"reviewer","mode":"memory"}),
        json!({"action":"create","resource":"session","target":agent()}),
        json!({"action":"send","target":{"kind":"session","agent_id":id(),"session_id":id()},"text":"work"}),
        json!({"action":"inspect","target":{"kind":"agent","host_id":id(),"agent_id":id()},"include_config":true}),
        json!({"action":"inspect","target":{"kind":"session","agent_id":id(),"session_id":id()},"include_output":true}),
        json!({"action":"inspect","target":{"kind":"turn","agent_id":id(),"session_id":id(),"turn_id":id()}}),
        json!({"action":"inspect","target":{"kind":"toolcall","agent_id":id(),"session_id":id(),"turn_id":id(),"tool_call_id":"upstream-call"}}),
        json!({"action":"inspect","target":{"kind":"message","agent_id":id(),"session_id":id(),"message_id":id()}}),
        json!({"action":"inspect","target":{"kind":"receipt","host_id":id(),"operation_id":id()}}),
        json!({"action":"wait"}),
        json!({"action":"control","operation":"cancel_turn","target":session(),"turn_id":id()}),
        json!({"action":"control","operation":"cancel_session","target":session()}),
        json!({"action":"control","operation":"close_session","target":session()}),
        json!({"action":"control","operation":"close_scope"}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{"name":"name","model":"new"}}),
        json!({"action":"update","resource":"session","target":session(),"changes":{"title":null,"workspace":null}}),
    ]
}
#[test]
fn all_six_actions_and_their_branches_round_trip() {
    for input in valid_calls() {
        let action =
            AgentAction::parse(input.clone()).unwrap_or_else(|error| panic!("{input}: {error}"));
        AgentAction::parse(serde_json::to_value(action).unwrap()).unwrap();
    }
}
#[test]
fn every_branch_rejects_authority_and_forwarding_fields() {
    for input in valid_calls() {
        for field in [
            "source",
            "caller",
            "internal",
            "operation_id",
            "message_id",
            "config_revision",
            "task_id",
            "method",
            "params",
        ] {
            let mut forged = input.clone();
            forged[field] = json!("forged");
            assert!(
                AgentAction::parse(forged).is_err(),
                "{field} accepted by {input}"
            );
        }
    }
}
#[test]
fn branch_specific_fields_and_nullability_are_strict() {
    let invalid = vec![
        json!({"action":"wait","target":session()}),
        json!({"action":"wait","timeout_ms":null}),
        json!({"action":"control","operation":"close_scope","target":session()}),
        json!({"action":"control","operation":"close_session","target":session(),"keep_inbox":true}),
        json!({"action":"create","resource":"session","target":agent(),"from_agent_id":id()}),
        json!({"action":"create","resource":"agent","target":{"host_id":id()},"name":"reviewer"}),
        json!({"action":"send","target":{"kind":"agent_default","host_id":id(),"agent_id":id()},"text":"work","delegate":true}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{}}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{"mode":"memory"}}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{"name":null}}),
        json!({"action":"update","resource":"session","target":session(),"changes":{"model":null}}),
        json!({"action":"update","resource":"session","target":session(),"changes":{"execution":{"tool_timeout_ms":1000}}}),
        json!({"action":"inspect","target":{"kind":"turn","agent_id":id(),"session_id":id(),"turn_id":id()},"include_config":true}),
        json!({"action":"inspect","target":{"kind":"receipt","host_id":id(),"operation_id":id()},"include_output":true}),
    ];
    for input in invalid {
        assert!(
            AgentAction::parse(input.clone()).is_err(),
            "accepted {input}"
        );
    }
}
#[test]
fn defaults_are_business_defaults_and_wait_has_no_implicit_timeout() {
    let AgentAction::Send(send) = AgentAction::parse(valid_calls()[2].clone()).unwrap() else {
        panic!()
    };
    assert_eq!(send.mode, MessageMode::FollowUp);
    assert!(!send.delegate);
    let AgentAction::Wait(wait) = AgentAction::parse(json!({"action":"wait"})).unwrap() else {
        panic!()
    };
    assert_eq!(wait.timeout_ms, None);
}
#[test]
fn source_values_are_copied_and_overrides_replace_without_aliasing() {
    let mut source = AgentConfig::default();
    source.role.instructions = "source role".into();
    source.session_defaults.tools = vec![ToolBinding {
        name: "read".into(),
    }];
    source.session_defaults.workspace = Some(WorkspaceRef {
        path: "/work".into(),
    });
    let settings: SessionConfigPatch =
        serde_json::from_value(json!({"model":"review-model","tools":[],"workspace":null}))
            .unwrap();
    let copied = agent_config_from_settings(&source, &settings).unwrap();
    assert_eq!(copied.role, source.role);
    assert_eq!(copied.session_defaults.model.as_str(), "review-model");
    assert!(copied.session_defaults.tools.is_empty());
    assert!(copied.session_defaults.workspace.is_none());
    assert_eq!(source.session_defaults.tools.len(), 1);
    source.role.instructions.clear();
    assert_eq!(copied.role.instructions, "source role");
}
#[test]
fn candidate_keeps_snapshot_revision_and_omissions() {
    let snapshot = AgentConfigSnapshot {
        name: "fixture name".into(),
        config: AgentConfig::default(),
        config_revision: 17,
    };
    let changes: AgentChanges =
        serde_json::from_value(json!({"name":"new","reasoning":"high"})).unwrap();
    let candidate = snapshot.apply(&changes).unwrap();
    assert_eq!(candidate.expected_revision, 17);
    assert_eq!(candidate.config.role, snapshot.config.role);
    assert_eq!(
        candidate.config.session_defaults.reasoning,
        ReasoningPreference::High
    );
    assert_eq!(candidate.name.as_str(), "new");
    let session_snapshot = SessionConfigSnapshot {
        title: Some("fixture title".into()),
        mode: StorageMode::Memory,
        config: SessionConfig::default(),
        config_revision: 23,
    };
    let changes: SessionChanges = serde_json::from_value(json!({"title":null})).unwrap();
    let candidate = session_snapshot.apply(&changes).unwrap();
    assert_eq!(candidate.expected_revision, 23);
    assert_eq!(candidate.config, session_snapshot.config);
    assert_eq!(candidate.title, None);
}
#[test]
fn actual_runtime_spec_has_only_six_actions_and_strict_objects() {
    let spec = agent_tool_spec();
    assert_eq!(spec.name().as_str(), "agent");
    let schema = spec.input_schema();
    let actions: std::collections::BTreeSet<_> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| branch["properties"]["action"]["const"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions.into_iter().collect::<Vec<_>>(),
        vec!["control", "create", "inspect", "send", "update", "wait"]
    );
    for branch in schema["oneOf"].as_array().unwrap() {
        assert_eq!(branch["additionalProperties"], false);
    }
}
