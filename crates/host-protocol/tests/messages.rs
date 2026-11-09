use minicore_host_protocol::*;
use serde_json::{Value, json};

const ID: &str = "9k3m7x2p5v8c4r6t0w1z";
fn input() -> Value {
    json!({"message_id":ID,"mode":"steer","text":"Read /repo/file.png as needed"})
}
fn send() -> Value {
    let mut value = input();
    value["target"] = json!({"kind":"session","agent_id":ID,"session_id":ID});
    value
}
fn accepted() -> Value {
    json!({"kind":"message.send","host_id":ID,"host_incarnation":ID,"agent_id":ID,"session_id":ID,"message_id":ID,
    "result":{"delivery":"accepted","queue":"next-turn","controller_revision":12}})
}

#[test]
fn text_messages_roundtrip_without_reading_or_rewriting_paths() {
    let request: SendMessage = serde_json::from_value(send()).unwrap();
    assert_eq!(request.text, "Read /repo/file.png as needed");
    assert_eq!(serde_json::to_value(request).unwrap(), send());
}
#[test]
fn input_requires_text_and_rejects_attachments_source_and_control_claims() {
    for key in [
        "source",
        "internal",
        "can_operate",
        "attachments",
        "images",
        "files",
        "wait_timeout_ms",
        "turn_id",
        "target",
    ] {
        let mut value = input();
        value[key] = json!(true);
        assert!(
            serde_json::from_value::<MessageInput>(value).is_err(),
            "{key}"
        );
    }
    for text in [Value::Null, json!(["text"]), json!({"text":"x"}), json!(3)] {
        let mut value = input();
        value["text"] = text;
        assert!(serde_json::from_value::<MessageInput>(value).is_err());
    }
    let mut value = input();
    value.as_object_mut().unwrap().remove("text");
    assert!(serde_json::from_value::<MessageInput>(value).is_err());
    let mut value = input();
    value["in_reply_to"] = Value::Null;
    assert!(serde_json::from_value::<MessageInput>(value).is_err());
}
#[test]
fn message_target_requires_exact_kind_and_direct_owner() {
    for target in [
        json!({"kind":"session","host_id":ID,"agent_id":ID,"session_id":ID}),
        json!({"kind":"session","session_id":ID}),
        json!({"kind":"agent_default","agent_id":ID}),
        json!({"kind":"agent_default","host_id":ID,"agent_id":ID,"session_id":ID}),
        json!({"kind":"remote","url":"wss://host","agent_id":ID}),
    ] {
        let mut value = send();
        value["target"] = target;
        assert!(serde_json::from_value::<SendMessage>(value).is_err());
    }
    let target: MessageTarget =
        serde_json::from_value(json!({"kind":"agent_default","host_id":ID,"agent_id":ID})).unwrap();
    assert!(matches!(target, MessageTarget::AgentDefault { .. }));
}
#[test]
fn accepted_receipt_preserves_real_queue_and_known_start_failure() {
    let mut value = accepted();
    value["result"]["start_error"] = json!({"type":"SESSION_RESOURCE_MISSING","code":3403,"message":"selected model is unavailable"});
    let receipt: MessageAccepted = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(receipt.result.queue, InputQueue::NextTurn);
    assert!(receipt.result.load_id.is_none());
    assert!(receipt.result.start_error.is_some());
    assert_eq!(serde_json::to_value(receipt).unwrap(), value);
}
#[test]
fn receipt_does_not_accept_generic_success_or_receiver_wrappers() {
    for (key, value) in [
        ("kind", json!("session.create")),
        ("receiver", json!({"agent_id":ID,"session_id":ID})),
        ("turn_id", json!(ID)),
        ("operation_id", json!(ID)),
        ("success", json!(true)),
    ] {
        let mut wire = accepted();
        wire[key] = value;
        assert!(
            serde_json::from_value::<MessageAccepted>(wire).is_err(),
            "{key}"
        );
    }
    for key in ["queue", "controller_revision", "delivery"] {
        let mut wire = accepted();
        wire["result"].as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<MessageAccepted>(wire).is_err());
    }
    let mut wire = accepted();
    wire["result"]["delivery"] = json!("completed");
    assert!(serde_json::from_value::<MessageAccepted>(wire).is_err());
}

fn create_receipt() -> Value {
    json!({"kind":"agent.create","host_id":ID,"host_incarnation":ID,"operation_id":ID,"agent_id":ID,"session_id":ID,
    "result":{"steps":{"agent":{"status":"succeeded"},"session":{"status":"succeeded"}}}})
}
#[test]
fn successful_empty_creation_has_no_message_fields() {
    let receipt: AgentCreateReceipt = serde_json::from_value(create_receipt()).unwrap();
    receipt.validate_call_return().unwrap();
    assert_eq!(serde_json::to_value(receipt).unwrap(), create_receipt());
}
#[test]
fn failed_creation_retains_published_objects_and_requested_message() {
    let mut value = create_receipt();
    value.as_object_mut().unwrap().remove("session_id");
    value["message_id"] = json!(ID);
    value["result"]["steps"]["session"] = json!({"status":"failed","error":{"type":"HOST_STORAGE_UNAVAILABLE","code":1101,"message":"storage unavailable"}});
    value["result"]["steps"]["message"] = json!({"status":"not_attempted"});
    let receipt: AgentCreateReceipt = serde_json::from_value(value.clone()).unwrap();
    receipt.validate_call_return().unwrap();
    assert!(receipt.agent_id.is_some());
    assert!(receipt.session_id.is_none());
    assert!(receipt.message_id.is_some());
    assert_eq!(serde_json::to_value(receipt).unwrap(), value);
}
#[test]
fn accepted_initial_message_includes_all_acceptance_facts() {
    let mut value = create_receipt();
    value["message_id"] = json!(ID);
    value["result"]["steps"]["message"] = json!({"status":"succeeded"});
    value["result"]["delivery"] = json!("accepted");
    value["result"]["queue"] = json!("next-turn");
    value["result"]["controller_revision"] = json!(1);
    let receipt: AgentCreateReceipt = serde_json::from_value(value.clone()).unwrap();
    receipt.validate_call_return().unwrap();
    for key in ["delivery", "queue", "controller_revision"] {
        let mut bad = value.clone();
        bad["result"].as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<AgentCreateReceipt>(bad).is_err(),
            "{key}"
        );
    }
    let mut bad = value;
    bad.as_object_mut().unwrap().remove("message_id");
    assert!(serde_json::from_value::<AgentCreateReceipt>(bad).is_err());
}
#[test]
fn invalid_creation_combinations_fail_both_wire_directions() {
    for key in ["agent_id", "session_id"] {
        let mut wire = create_receipt();
        wire.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<AgentCreateReceipt>(wire).is_err(),
            "{key}"
        );
    }
    for key in [
        "delivery",
        "queue",
        "controller_revision",
        "load_id",
        "start_error",
    ] {
        let mut wire = create_receipt();
        wire["result"][key] = json!("unexpected");
        assert!(
            serde_json::from_value::<AgentCreateReceipt>(wire).is_err(),
            "{key}"
        );
    }
    let mut receipt: AgentCreateReceipt = serde_json::from_value(create_receipt()).unwrap();
    receipt.session_id = None;
    assert!(serde_json::to_value(receipt).is_err());
    assert!(serde_json::from_value::<CreateStep>(json!({"status":"failed"})).is_err());
    assert!(serde_json::from_value::<CreateStep>(json!({"status":"succeeded","error":{"type":"HOST_INTERNAL_UNEXPECTED","code":1999,"message":"x"}})).is_err());
}
#[test]
fn progress_queries_are_not_ordinary_creation_returns() {
    let mut value = create_receipt();
    value.as_object_mut().unwrap().remove("session_id");
    value["result"]["steps"]["session"] = json!({"status":"running"});
    let receipt: AgentCreateReceipt = serde_json::from_value(value.clone()).unwrap();
    assert!(receipt.validate_call_return().is_err());
    value["message_id"] = json!(ID);
    value["result"]["steps"]["message"] = json!({"status":"not_attempted"});
    assert!(serde_json::from_value::<AgentCreateReceipt>(value).is_err());
}
#[test]
fn session_creation_never_invents_an_agent_creation_step() {
    let mut value = create_receipt();
    value["kind"] = json!("session.create");
    assert!(serde_json::from_value::<SessionCreateReceipt>(value.clone()).is_err());
    value["result"]["steps"]
        .as_object_mut()
        .unwrap()
        .remove("agent");
    let receipt: SessionCreateReceipt = serde_json::from_value(value).unwrap();
    receipt.validate_call_return().unwrap();
}
#[test]
fn create_requests_validate_mode_config_and_target_semantics() {
    let mut value =
        json!({"operation_id":ID,"name":"Worker","mode":"memory","config":AgentConfig::default()});
    let request: AgentCreate = serde_json::from_value(value.clone()).unwrap();
    assert!(request.default_session.is_none());
    value["default_session"] = json!({"mode":"persistent"});
    assert!(serde_json::from_value::<AgentCreate>(value).is_err());
    let request: SessionCreate = serde_json::from_value(
        json!({"operation_id":ID,"target":{"host_id":ID,"agent_id":ID},"initial_message":input()}),
    )
    .unwrap();
    assert_eq!(
        request.resolved_mode(StorageMode::Memory).unwrap(),
        StorageMode::Memory
    );
    let mut initial = input();
    initial["target"] = json!({"kind":"session","agent_id":ID,"session_id":ID});
    assert!(serde_json::from_value::<SessionCreate>(json!({"operation_id":ID,"target":{"host_id":ID,"agent_id":ID},"initial_message":initial})).is_err());
}
