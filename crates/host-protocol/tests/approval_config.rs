use minicore_host_protocol::*;
use serde_json::json;

#[test]
fn approval_defaults_and_old_execution_configs_are_ask() {
    assert_eq!(ApprovalMode::default(), ApprovalMode::Ask);
    assert_eq!(ExecutionSettings::default().approval, ApprovalMode::Ask);
    let mut old = json!(ExecutionSettings::default());
    old.as_object_mut().unwrap().remove("approval");
    let restored: ExecutionSettings = serde_json::from_value(old).unwrap();
    assert_eq!(restored.approval, ApprovalMode::Ask);
    assert_eq!(json!(restored)["approval"], "ask");
}

#[test]
fn approval_has_only_three_explicit_values_and_cannot_be_null() {
    for (mode, text) in [
        (ApprovalMode::Auto, "auto"),
        (ApprovalMode::Ask, "ask"),
        (ApprovalMode::ReadOnly, "read_only"),
    ] {
        let mut value = json!(ExecutionSettings::default());
        value["approval"] = json!(text);
        let actual: ExecutionSettings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(actual.approval, mode);
        assert_eq!(json!(actual), value);
    }
    for invalid in [json!(null), json!(true), json!("allow"), json!("read-only")] {
        let mut value = json!(ExecutionSettings::default());
        value["approval"] = invalid;
        assert!(serde_json::from_value::<ExecutionSettings>(value).is_err());
    }
    let mut role = json!(Role::default());
    role["approval"] = json!("auto");
    assert!(serde_json::from_value::<Role>(role).is_err());
}

#[test]
fn approval_follows_whole_execution_replacement_and_session_copy() {
    let mut agent = AgentConfig::default();
    agent.session_defaults.execution.approval = ApprovalMode::ReadOnly;
    let original = SessionConfig::from_agent(&agent, &SessionOverrides::default()).unwrap();
    assert_eq!(original.execution.approval, ApprovalMode::ReadOnly);
    let mut replacement = json!(ExecutionSettings::default());
    replacement.as_object_mut().unwrap().remove("approval");
    let patch: SessionConfigPatch =
        serde_json::from_value(json!({"execution":replacement})).unwrap();
    let updated = original.apply_patch(&patch).unwrap();
    assert_eq!(updated.execution.approval, ApprovalMode::Ask);
    assert_eq!(original.execution.approval, ApprovalMode::ReadOnly);
    assert!(
        serde_json::from_value::<SessionConfigPatch>(json!({"execution":{"approval":"auto"}}))
            .is_err()
    );
    assert!(serde_json::from_value::<SessionConfigPatch>(json!({"execution":null})).is_err());
}
