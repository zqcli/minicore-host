use super::*;
use serde_json::json;

const HOST: &str = "00000000000000000001";
const AGENT: &str = "00000000000000000002";
const SESSION: &str = "00000000000000000003";
const TURN: &str = "00000000000000000004";
const MESSAGE: &str = "00000000000000000005";
const REPLY: &str = "00000000000000000006";
const OPERATION: &str = "00000000000000000007";

#[test]
fn server_keeps_original_message_links_despite_an_invalid_target_leaf() {
    let params = json!({
        "operation_id": OPERATION,
        "message_id": MESSAGE,
        "in_reply_to": REPLY,
        "target": {"host_id": HOST, "agent_id": AGENT, "session_id": "invalid"},
        "text": "PRIVATE_BODY",
        "settings": {"role": "PRIVATE_ROLE", "api_key": "PRIVATE_KEY"}
    });
    let correlation = Correlation::from_params(&params);
    assert_eq!(correlation.operation_id.unwrap().as_str(), OPERATION);
    assert_eq!(correlation.message_id.unwrap().as_str(), MESSAGE);
    assert_eq!(correlation.in_reply_to.unwrap().as_str(), REPLY);
    let target = correlation.target.unwrap();
    assert_eq!(target.host_id.unwrap().as_str(), HOST);
    assert_eq!(target.agent_id.unwrap().as_str(), AGENT);
    assert!(target.session_id.is_none());
    assert!(target.turn_id.is_none());
    assert!(target.tool_call_id.is_none());
    // Extraction neither repairs nor rewrites the invalid business request.
    assert_eq!(params["target"]["session_id"], "invalid");
}

#[test]
fn creation_initial_message_keeps_links_without_inventing_unpublished_entities() {
    let correlation = Correlation::from_params(&json!({
        "operation_id": OPERATION,
        "initial_message": {
            "message_id": MESSAGE, "in_reply_to": REPLY,
            "text": "PRIVATE_INITIAL_MESSAGE"
        },
        "settings": {"role": "PRIVATE_ROLE"},
        "name": "PRIVATE_NAME"
    }));
    assert_eq!(correlation.operation_id.unwrap().as_str(), OPERATION);
    assert!(correlation.target.is_none());
    assert!(correlation.session_id.is_none());
    assert!(correlation.turn_id.is_none());
    assert!(correlation.message_id.is_none());
    let message = correlation.initial_message.unwrap();
    assert_eq!(message.message_id.unwrap().as_str(), MESSAGE);
    assert_eq!(message.in_reply_to.unwrap().as_str(), REPLY);
}

#[test]
fn server_extracts_each_target_and_request_identity_independently() {
    let correlation = Correlation::from_params(&json!({
        "target": {
            "host_id": HOST, "agent_id": AGENT, "session_id": SESSION,
            "turn_id": TURN, "tool_call_id": "provider-original-tool-id"
        },
        "session_id": SESSION,
        "turn_id": TURN,
        "tool": {"turn_id": TURN, "tool_call_id": "provider-tool-2"},
        "request": {"operation_id": OPERATION, "turn_id": TURN, "text": "PRIVATE"}
    }));
    let target = correlation.target.unwrap();
    assert_eq!(target.host_id.unwrap().as_str(), HOST);
    assert_eq!(target.agent_id.unwrap().as_str(), AGENT);
    assert_eq!(target.session_id.unwrap().as_str(), SESSION);
    assert_eq!(target.turn_id.unwrap().as_str(), TURN);
    assert_eq!(
        target.tool_call_id.unwrap().as_str(),
        "provider-original-tool-id"
    );
    assert_eq!(correlation.session_id.unwrap().as_str(), SESSION);
    assert_eq!(correlation.turn_id.unwrap().as_str(), TURN);
    let tool = correlation.tool.unwrap();
    assert_eq!(tool.turn_id.unwrap().as_str(), TURN);
    assert_eq!(tool.tool_call_id.unwrap().as_str(), "provider-tool-2");
    let request = correlation.request.unwrap();
    assert_eq!(request.operation_id.unwrap().as_str(), OPERATION);
    assert_eq!(request.turn_id.unwrap().as_str(), TURN);
}

#[test]
fn malformed_correlations_are_absent_not_replaced_by_new_ids() {
    let correlation = Correlation::from_params(&json!({
        "operation_id": [], "message_id": 7, "in_reply_to": null,
        "target": "PRIVATE_TARGET", "initial_message": true,
        "session_id": "bad", "turn_id": {}, "tool": false,
        "request": {"operation_id": {}, "turn_id": null}
    }));
    assert!(correlation.operation_id.is_none());
    assert!(correlation.message_id.is_none());
    assert!(correlation.in_reply_to.is_none());
    assert!(correlation.session_id.is_none());
    assert!(correlation.turn_id.is_none());
    assert!(correlation.target.unwrap().host_id.is_none());
    assert!(correlation.initial_message.unwrap().message_id.is_none());
    assert!(correlation.tool.unwrap().tool_call_id.is_none());
    let request = correlation.request.unwrap();
    assert!(request.operation_id.is_none());
    assert!(request.turn_id.is_none());
}

#[test]
fn typed_creation_passes_original_message_links_without_publishing_entities() {
    let message = MessageInput {
        message_id: MESSAGE.parse().unwrap(),
        mode: minicore_host_protocol::MessageMode::FollowUp,
        text: "PRIVATE_BODY".into(),
        in_reply_to: Some(REPLY.parse().unwrap()),
    };
    let correlation = Correlation::default()
        .operation(&OPERATION.parse().unwrap())
        .initial_message(Some(&message));
    assert_eq!(correlation.operation_id.unwrap().as_str(), OPERATION);
    assert_eq!(correlation.message_id.unwrap(), message.message_id);
    assert_eq!(correlation.in_reply_to, message.in_reply_to);
    assert!(correlation.target.is_none());
    assert!(correlation.session_id.is_none());
    assert!(correlation.turn_id.is_none());
    assert_eq!(message.text, "PRIVATE_BODY");
}

#[test]
fn typed_tools_pass_actual_target_leaves_and_unknown_metadata_stays_absent() {
    let target = SessionRef {
        agent_id: AGENT.parse().unwrap(),
        session_id: SESSION.parse().unwrap(),
    };
    let tool = ToolCallRef {
        turn_id: TURN.parse().unwrap(),
        tool_call_id: ToolCallId("provider-original-tool-id".into()),
    };
    let call = Call::client(
        None,
        "toolcall.get",
        Correlation::session(&target).tool(&tool),
    );
    assert_eq!(call.entities.agent_id, Some(target.agent_id));
    assert_eq!(call.entities.session_id, Some(target.session_id));
    assert_eq!(call.entities.turn_id, Some(tool.turn_id));
    assert_eq!(call.entities.tool_call_id, Some(tool.tool_call_id));
    assert!(call.peer.is_none());
    assert!(call.context.host_id.is_none());
    assert!(call.context.message_id.is_none());
}

#[test]
fn subscription_scope_keeps_requested_identity_leaves() {
    let correlation = Correlation::from_params(&json!({
        "scope": {"kind": "session", "agent_id": AGENT, "session_id": SESSION},
        "categories": ["state"]
    }));
    let scope = correlation.scope.unwrap();
    assert_eq!(scope.agent_id.unwrap().as_str(), AGENT);
    assert_eq!(scope.session_id.unwrap().as_str(), SESSION);
    assert!(scope.host_id.is_none());
}

#[test]
fn interaction_request_keeps_the_original_load_and_operation_without_answer_data() {
    let load = "00000000000000000008";
    let correlation = Correlation::from_params(&json!({
        "target": {"agent_id": AGENT, "session_id": SESSION},
        "request": {"operation_id": OPERATION, "turn_id": TURN, "load_id": load,
            "answer": {"private": "PRIVATE_TOOL_ANSWER"}}
    }));
    let request = correlation.request.unwrap();
    assert_eq!(request.operation_id.unwrap().as_str(), OPERATION);
    assert_eq!(request.turn_id.unwrap().as_str(), TURN);
    assert_eq!(request.load_id.unwrap().as_str(), load);
}
