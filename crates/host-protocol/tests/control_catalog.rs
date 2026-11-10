use minicore_host_protocol::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
const ID: &str = "9k3m7x2p5v8c4r6t0w1z";

#[test]
fn call_options_stay_local_and_require_positive_integer_time() {
    assert_eq!(CallOptions::default().timeout_ms(), 30_000);
    assert_eq!(
        serde_json::from_value::<CallOptions>(json!({"wait_timeout_ms":60_000}))
            .unwrap()
            .timeout_ms(),
        60_000
    );
    for wire in [
        json!({"wait_timeout_ms":0}),
        json!({"wait_timeout_ms":-1}),
        json!({"wait_timeout_ms":0.5}),
        json!({"wait_timeout_ms":null}),
        json!({"execution_timeout":30_000}),
    ] {
        assert!(serde_json::from_value::<CallOptions>(wire).is_err());
    }
    assert!(
        serde_json::to_value(CallOptions {
            wait_timeout_ms: Some(0)
        })
        .is_err()
    );
}
#[test]
fn connection_info_does_not_confuse_shared_occupied_with_own_rights() {
    for (operate, occupied) in [(true, true), (false, true), (false, false)] {
        let wire = json!({"host_id":ID,"host_incarnation":ID,"can_operate":operate,"control":{"occupied":occupied,"revision":4}});
        let info: ConnectionInfo = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(info).unwrap(), wire);
    }
    assert!(serde_json::from_value::<ConnectionInfo>(json!({"host_id":ID,"host_incarnation":ID,"can_operate":true,"control":{"occupied":false,"revision":4}})).is_err());
    assert!(
        serde_json::from_value::<ControlStateData>(
            json!({"occupied":true,"revision":4,"can_operate":true})
        )
        .is_err()
    );
}
#[test]
fn shutdown_rejects_force_draining_and_nonshutdown_lifecycles() {
    for (strategy, state) in [
        ("graceful", "draining"),
        ("graceful", "stopping"),
        ("graceful", "stopped"),
        ("force", "stopping"),
        ("force", "stopped"),
    ] {
        assert!(
            serde_json::from_value::<HostShutdownResult>(
                json!({"strategy":strategy,"lifecycle":state})
            )
            .is_ok()
        );
    }
    for (strategy, state) in [
        ("force", "draining"),
        ("force", "running"),
        ("graceful", "starting"),
        ("force", "forced"),
    ] {
        assert!(
            serde_json::from_value::<HostShutdownResult>(
                json!({"strategy":strategy,"lifecycle":state})
            )
            .is_err()
        );
    }
    assert!(
        serde_json::to_value(HostShutdownResult {
            strategy: HostShutdownStrategy::Force,
            lifecycle: HostLifecycle::Draining
        })
        .is_err()
    );
}
#[test]
fn shutdown_receipt_has_only_host_identity_and_operation() {
    let wire = json!({"kind":"host.shutdown","host_id":ID,"host_incarnation":ID,"operation_id":ID,"result":{"strategy":"force","lifecycle":"stopping"}});
    let receipt: HostShutdownReceipt = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap(), wire);
    let mut invalid = wire;
    invalid["agent_id"] = json!(ID);
    assert!(serde_json::from_value::<HostShutdownReceipt>(invalid).is_err());
}
#[test]
fn cancel_targets_exact_turn_and_defaults_to_clearing_only_on_claim() {
    let request: TurnCancelRequest =
        serde_json::from_value(json!({"operation_id":ID,"turn_id":ID})).unwrap();
    assert!(!request.keep_inbox);
    let kept: TurnCancelRequest =
        serde_json::from_value(json!({"operation_id":ID,"turn_id":ID,"keep_inbox":true})).unwrap();
    assert!(kept.keep_inbox);
    assert!(serde_json::from_value::<TurnCancelRequest>(json!({"operation_id":ID})).is_err());
    for state in ["cancelling", "finalizing", "finished"] {
        for claim in [true, false] {
            assert!(
                serde_json::from_value::<TurnCancelResult>(
                    json!({"cancel_claimed":claim,"turn_state":state,"controller_revision":4})
                )
                .is_ok()
            );
        }
    }
    for state in ["starting", "running"] {
        assert!(
            serde_json::from_value::<TurnCancelResult>(
                json!({"cancel_claimed":true,"turn_state":state,"controller_revision":4})
            )
            .is_err()
        );
    }
}
fn control_facts() -> Value {
    json!({"scheduling":"paused","pause_reasons":["explicit"],"block_reason":null,"residency":"unloaded","controller_revision":5})
}
#[test]
fn session_control_facts_preserve_pause_loading_and_blocking_separately() {
    let mut wire = control_facts();
    wire["residency"] = json!("loading");
    wire["block_reason"] =
        json!({"message":"history must be verified","resolution":"complete history validation"});
    let facts: SessionControlFacts = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(facts.scheduling, Scheduling::Paused);
    assert_eq!(facts.residency, Residency::Loading);
    assert!(facts.load_id.is_none());
    assert_eq!(serde_json::to_value(facts).unwrap(), wire);
}
#[test]
fn invalid_control_combinations_are_rejected() {
    let mut wire = control_facts();
    wire["scheduling"] = json!("auto");
    assert!(serde_json::from_value::<SessionControlFacts>(wire).is_err());
    let mut wire = control_facts();
    wire["pause_reasons"] = json!([]);
    assert!(serde_json::from_value::<SessionControlFacts>(wire).is_err());
    let mut wire = control_facts();
    wire["load_id"] = json!(ID);
    assert!(serde_json::from_value::<SessionControlFacts>(wire).is_err());
    let mut wire = control_facts();
    wire["residency"] = json!("loaded");
    assert!(serde_json::from_value::<SessionControlFacts>(wire).is_err());
    assert!(
        serde_json::from_value::<BlockReason>(json!({"message":"blocked","resolution":""}))
            .is_err()
    );
}
#[test]
fn pause_receipt_is_already_effective_but_resume_may_still_be_blocked() {
    let mut wire = json!({"kind":"session.pause","host_id":ID,"host_incarnation":ID,"operation_id":ID,"agent_id":ID,"session_id":ID,"result":control_facts()});
    assert!(serde_json::from_value::<SessionControlReceipt>(wire.clone()).is_ok());
    wire["result"]["scheduling"] = json!("auto");
    wire["result"]["pause_reasons"] = json!([]);
    assert!(serde_json::from_value::<SessionControlReceipt>(wire.clone()).is_err());
    wire["kind"] = json!("session.resume");
    wire["result"] = control_facts();
    assert!(serde_json::from_value::<SessionControlReceipt>(wire).is_ok());
}
#[test]
fn close_and_stop_do_not_claim_running_or_open() {
    assert!(
        serde_json::from_value::<SessionCloseResult>(
            json!({"lifecycle":"open","controller_revision":2})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<AgentStopResult>(json!({"lifecycle":"active"})).is_err());
    assert!(serde_json::from_value::<CompactCancelRequest>(json!({"operation_id":ID})).is_err());
}
#[test]
fn catalog_exactly_matches_all_52_spec_methods_in_order() {
    let spec = include_str!("../../../SPEC.md");
    let section = spec
        .split("# 8. 首版公共接口目录")
        .nth(1)
        .unwrap()
        .split("# 9. 输出观察与读取")
        .next()
        .unwrap();
    let from_spec: Vec<_> = section
        .lines()
        .filter_map(|line| {
            if !line.starts_with("| ") {
                return None;
            }
            let candidate = line.split('|').nth(1)?.trim();
            (candidate.contains('.')
                && !candidate.contains(' ')
                && candidate
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.'))
            .then_some(candidate)
        })
        .collect();
    assert_eq!(from_spec.len(), 52);
    assert_eq!(
        PUBLIC_METHODS
            .iter()
            .map(|method| method.as_str())
            .collect::<Vec<_>>(),
        from_spec
    );
    assert_eq!(PUBLIC_METHODS.iter().collect::<BTreeSet<_>>().len(), 52);
    for method in PUBLIC_METHODS {
        assert_eq!(method.as_str().parse::<Method>().unwrap(), *method);
        assert_eq!(
            serde_json::from_value::<Method>(json!(method.as_str())).unwrap(),
            *method
        );
    }
    for unknown in [
        "host.state",
        "turn.start",
        "remote.agent.create",
        "role.create",
        "message.retry",
    ] {
        assert!(unknown.parse::<Method>().is_err());
    }
}
#[test]
fn catalog_access_distinguishes_observation_from_business_operations() {
    assert_eq!(
        Method::EventsUnsubscribe.descriptor().access,
        AccessRequirement::OwnSubscription
    );
    assert_eq!(
        Method::InteractionAnswer.descriptor().access,
        AccessRequirement::Operate
    );
}
#[test]
fn persisted_provenance_is_strict_and_does_not_grant_authority() {
    assert!(
        serde_json::from_value::<MessageSource>(
            json!({"kind":"user","source":{"agent_id":ID,"session_id":ID}})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<MessageSource>(json!({"kind":"user","internal":true})).is_err()
    );
    assert!(serde_json::from_value::<MessageSource>(json!({"kind":"agent"})).is_err());
    let source: MessageSource =
        serde_json::from_value(json!({"kind":"agent","source":{"agent_id":ID,"session_id":ID}}))
            .unwrap();
    assert!(matches!(source, MessageSource::Agent { .. }));
}
#[test]
fn invalid_in_process_config_is_not_serialized_as_valid() {
    let mut config = AgentConfig::default();
    config.role.name = "".into();
    assert!(serde_json::to_value(config).is_err());
}

#[test]
fn snapshot_absent_block_is_explicit_null_not_a_partial_update() {
    let mut wire = control_facts();
    wire.as_object_mut().unwrap().remove("block_reason");
    assert!(serde_json::from_value::<SessionControlFacts>(wire).is_err());
    assert!(serde_json::from_value::<SessionControlFacts>(control_facts()).is_ok());
}

#[test]
fn session_cancel_receipt_correlates_only_an_actually_observed_turn() {
    let idle = json!({"kind":"session.cancel","host_id":ID,"host_incarnation":ID,"operation_id":ID,"agent_id":ID,"session_id":ID,"result":{"cancel_claimed":false,"controller_revision":3}});
    let receipt: SessionCancelReceipt = serde_json::from_value(idle.clone()).unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap(), idle);
    let mut active = idle.clone();
    active["turn_id"] = json!(ID);
    active["result"]["turn_state"] = json!("cancelling");
    active["result"]["cancel_claimed"] = json!(true);
    let receipt: Receipt = serde_json::from_value(active.clone()).unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap(), active);
    let mut unknown = active.clone();
    unknown["kind"] = json!("arbitrary.control");
    assert!(serde_json::from_value::<Receipt>(unknown).is_err());
    let mut invalid = idle.clone();
    invalid["result"]["cancel_claimed"] = json!(true);
    assert!(serde_json::from_value::<SessionCancelReceipt>(invalid).is_err());
    let mut invalid = idle.clone();
    invalid["turn_id"] = json!(ID);
    assert!(serde_json::from_value::<SessionCancelReceipt>(invalid).is_err());
    let mut invalid = idle;
    invalid["result"]["turn_state"] = json!("finished");
    assert!(serde_json::from_value::<SessionCancelReceipt>(invalid).is_err());
    let mut invalid = active.clone();
    invalid["result"]["turn_state"] = json!("running");
    assert!(serde_json::from_value::<SessionCancelReceipt>(invalid).is_err());
    let mut invalid = active;
    invalid["turn_id"] = Value::Null;
    assert!(serde_json::from_value::<SessionCancelReceipt>(invalid).is_err());
}
