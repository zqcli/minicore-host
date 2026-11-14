use minicore_host_protocol::*;
use serde_json::{Value, json};

#[test]
fn instructions_default_when_omitted_preserve_explicit_empty_values_and_reject_null() {
    let role: Role =
        serde_json::from_value(json!({"name":"Role","description":"description"})).unwrap();
    assert!(role.instructions.is_empty());
    assert_eq!(serde_json::to_value(&role).unwrap()["instructions"], "");
    assert!(
        serde_json::from_value::<Role>(json!({"name":"Role","description":"","instructions":null}))
            .is_err()
    );
    let agent = serde_json::to_value(AgentConfig::default()).unwrap();
    let session = serde_json::to_value(SessionConfig::default()).unwrap();
    for (is_agent, mut wire) in [(true, agent), (false, session)] {
        assert_eq!(wire["instructions"], "");
        assert_eq!(wire["role"]["instructions"], "");
        wire.as_object_mut().unwrap().remove("instructions");
        if is_agent {
            let decoded = serde_json::from_value::<AgentConfig>(wire.clone()).unwrap();
            assert_eq!(decoded.instructions, "");
            assert_eq!(serde_json::to_value(decoded).unwrap()["instructions"], "");
        } else {
            let decoded = serde_json::from_value::<SessionConfig>(wire.clone()).unwrap();
            assert_eq!(decoded.instructions, "");
            assert_eq!(serde_json::to_value(decoded).unwrap()["instructions"], "");
        }
        wire["instructions"] = Value::Null;
        if is_agent {
            assert!(serde_json::from_value::<AgentConfig>(wire).is_err());
        } else {
            assert!(serde_json::from_value::<SessionConfig>(wire).is_err());
        }
    }
}

#[test]
fn session_copies_one_agent_version_then_retains_its_independent_instructions() {
    let mut agent = AgentConfig {
        instructions: "Agent A".into(),
        role: Role {
            instructions: "Role A".into(),
            ..Role::default()
        },
        ..AgentConfig::default()
    };
    let session = SessionConfig::from_agent(&agent, &SessionOverrides::default()).unwrap();
    agent.instructions = "Agent B".into();
    agent.role.instructions = "Role B".into();
    assert_eq!(session.instructions, "Agent A");
    assert_eq!(session.role.instructions, "Role A");
    let next = SessionConfig::from_agent(&agent, &SessionOverrides::default()).unwrap();
    assert_eq!(next.instructions, "Agent B");
    let overrides = SessionOverrides(SessionConfigPatch {
        instructions: Some("Session-specific".into()),
        ..SessionConfigPatch::default()
    });
    assert_eq!(
        SessionConfig::from_agent(&agent, &overrides)
            .unwrap()
            .instructions,
        "Session-specific"
    );
    assert_eq!(agent.instructions, "Agent B");
}

#[test]
fn instructions_patches_replace_clear_and_preserve_without_inheritance() {
    let agent = AgentConfig {
        instructions: "Agent".into(),
        ..AgentConfig::default()
    };
    let session = SessionConfig::from_agent(&agent, &SessionOverrides::default()).unwrap();
    let agent_patch: AgentConfigPatch = serde_json::from_value(json!({"instructions":""})).unwrap();
    let session_patch: SessionConfigPatch =
        serde_json::from_value(json!({"instructions":""})).unwrap();
    assert!(!agent_patch.is_empty());
    assert!(!session_patch.is_empty());
    assert!(
        agent
            .apply_patch(&agent_patch)
            .unwrap()
            .instructions
            .is_empty()
    );
    let cleared = session.apply_patch(&session_patch).unwrap();
    assert!(cleared.instructions.is_empty());
    assert_eq!(
        cleared.apply_patch(&SessionConfigPatch::default()).unwrap(),
        cleared
    );
    assert_eq!(
        session.apply_patch(&SessionConfigPatch::default()).unwrap(),
        session
    );
    assert_eq!(
        agent.apply_patch(&AgentConfigPatch::default()).unwrap(),
        agent
    );
    assert_eq!(
        serde_json::to_value(agent_patch).unwrap(),
        json!({"instructions":""})
    );
    assert_eq!(
        serde_json::to_value(session_patch).unwrap(),
        json!({"instructions":""})
    );
    assert!(serde_json::from_value::<AgentConfigPatch>(json!({"instructions":null})).is_err());
    assert!(serde_json::from_value::<SessionConfigPatch>(json!({"instructions":null})).is_err());
}
