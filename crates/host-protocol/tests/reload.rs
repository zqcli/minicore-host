use minicore_host_protocol::*;
use serde_json::{Value, json};
const ID: &str = "9k3m7x2p5v8c4r6t0w1z";

#[test]
fn reload_requires_operation_revision_and_rejects_candidates_or_paths() {
    let valid = json!({"operation_id":ID,"expected_revision":3});
    let request: HostReloadRequest = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap(), valid);
    for field in ["operation_id", "expected_revision"] {
        let mut invalid = valid.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<HostReloadRequest>(invalid).is_err());
    }
    for (field, value) in [
        ("operation_id", json!("caller-made-short-id")),
        ("expected_revision", json!(-1)),
        ("expected_revision", json!(null)),
        ("candidate", json!({"name":"Host","access_key":"secret"})),
        ("config_dir", json!("/tmp/arbitrary")),
        ("caller", json!({"internal":true})),
        ("wait_timeout_ms", json!(100)),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(serde_json::from_value::<HostReloadRequest>(invalid).is_err());
    }
}
fn receipt(result: Value) -> Value {
    json!({"kind":"host.reload","host_id":ID,"host_incarnation":ID,"operation_id":ID,"result":result})
}

#[test]
fn reload_receipt_preserves_canonical_kind_and_each_real_owner_phase() {
    for result in [
        json!({"status":"preparing","config_revision":3}),
        json!({"status":"applying","config_revision":3}),
        json!({"status":"applied","config_revision":4}),
        json!({"status":"restart_required","config_revision":3,"fields":["network_listen","logging"]}),
        json!({"status":"failed","config_revision":3,"error":Fault::new(FaultKind::HostConfigInvalid,"invalid candidate")}),
    ] {
        let wire = receipt(result);
        let parsed: Receipt = serde_json::from_value(wire.clone()).unwrap();
        assert!(matches!(parsed, Receipt::HostReload(_)));
        assert_eq!(parsed.operation_id().as_str(), ID);
        assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
    }
}

#[test]
fn reload_rejects_aliases_unrelated_targets_and_inconsistent_phase_fields() {
    for result in [
        json!({"status":"accepted","config_revision":3}),
        json!({"status":"applying","config_revision":3,"error":Fault::new(FaultKind::HostConfigInvalid,"bad")}),
        json!({"status":"failed","config_revision":3}),
        json!({"status":"failed","config_revision":3,"error":null}),
        json!({"status":"restart_required","config_revision":3,"fields":[]}),
        json!({"status":"restart_required","config_revision":3,"fields":["logging","logging"]}),
        json!({"status":"applied","config_revision":4,"fields":["logging"]}),
    ] {
        assert!(serde_json::from_value::<HostReloadReceipt>(receipt(result)).is_err());
    }
    for (field, value) in [
        ("kind", json!("host.reload_config")),
        ("kind", json!("host.receipt.get")),
        ("agent_id", json!(ID)),
        ("session_id", json!(ID)),
        ("error", json!("not a second result")),
    ] {
        let mut wire = receipt(json!({"status":"preparing","config_revision":3}));
        wire[field] = value;
        assert!(serde_json::from_value::<Receipt>(wire).is_err());
    }
}

#[test]
fn reload_serialization_rejects_fabricated_empty_restart_facts() {
    let mut value: HostReloadReceipt =
        serde_json::from_value(receipt(json!({"status":"preparing","config_revision":3}))).unwrap();
    value.result = HostReloadResult::RestartRequired {
        config_revision: 3,
        fields: vec![],
    };
    assert!(serde_json::to_value(value).is_err());
}

#[test]
fn reload_partial_results_require_real_failed_owner_facts() {
    let failure = json!({"status":"failed","key":"writer","agent_id":ID,"error":Fault::new(FaultKind::HostStorageUnavailable,"save failed")});
    let wire = receipt(
        json!({"status":"partially_applied","config_revision":4,"agents":[failure.clone()]}),
    );
    let parsed: HostReloadReceipt = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
    assert!(
        serde_json::from_value::<HostReloadReceipt>(receipt(
            json!({"status":"applied","config_revision":4,"agents":[failure]})
        ))
        .is_err()
    );
    assert!(
        serde_json::from_value::<HostReloadReceipt>(receipt(
            json!({"status":"partially_applied","config_revision":4,"agents":[]})
        ))
        .is_err()
    );
}
