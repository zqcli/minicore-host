use super::*;
use minicore_host_protocol::{AgentId, HostId, OperationId, SessionId, TurnId};
use minicore_runtime::{
    ToolCallId,
    tools::{ToolInvocation, ToolSpec},
};
use serde_json::{Value, json};
use std::time::Duration;

fn request(name: &str, arguments: Value) -> ToolPolicyRequest {
    ToolPolicyRequest {
        invocation: ToolInvocation::new(
            ToolCallId::new("policy-fixture-call").unwrap(),
            name.parse().unwrap(),
            arguments,
        )
        .unwrap(),
        spec: ToolSpec::new(
            name.parse().unwrap(),
            "Explicit policy test declaration",
            json!({"type":"object"}),
        )
        .unwrap(),
        cancellation: Default::default(),
        deadline: Instant::now() + Duration::from_secs(5),
    }
}

fn assert_mode_decision(mode: ApprovalMode, decision: ToolDecision, read_only: bool) {
    decision.validate().unwrap();
    match (mode, read_only, decision) {
        (ApprovalMode::Auto, _, ToolDecision::Allow) | (_, true, ToolDecision::Allow) => {}
        (ApprovalMode::Ask, false, ToolDecision::RequireApproval { request }) => {
            assert_eq!(request.risk, ApprovalRisk::Medium);
        }
        (ApprovalMode::ReadOnly, false, ToolDecision::Deny { reason }) => {
            assert_eq!(reason.as_str(), READ_ONLY_DENIAL);
        }
        other => panic!("unexpected policy decision: {other:?}"),
    }
}

#[tokio::test]
async fn ordinary_and_external_tools_follow_the_three_mode_table() {
    for mode in [
        ApprovalMode::Ask,
        ApprovalMode::Auto,
        ApprovalMode::ReadOnly,
    ] {
        let policy = Policy::new(mode, true, true);
        for name in ["read", "write", "edit", "bash", "registered_external"] {
            assert_mode_decision(
                mode,
                policy.decide(request(name, json!({}))).await.unwrap(),
                name == "read",
            );
        }
    }
}

#[tokio::test]
async fn all_valid_agent_branches_are_classified_from_the_dispatch_parser() {
    let host = HostId::new().unwrap();
    let agent = AgentId::new().unwrap();
    let session = SessionId::new().unwrap();
    let turn = TurnId::new().unwrap();
    let agent_ref = json!({"host_id":host,"agent_id":agent});
    let session_ref = json!({"agent_id":agent,"session_id":session});
    let mut actions = vec![
        (
            json!({"action":"create","resource":"agent","target":{"host_id":host},"name":"Child","mode":"memory"}),
            false,
        ),
        (
            json!({"action":"create","resource":"agent","target":{"host_id":host},"name":"Child","mode":"memory","text":"delegate child"}),
            false,
        ),
        (
            json!({"action":"create","resource":"session","target":agent_ref}),
            false,
        ),
        (
            json!({"action":"create","resource":"session","target":agent_ref,"text":"delegate child"}),
            false,
        ),
        (
            json!({"action":"send","target":{"kind":"agent_default","host_id":host,"agent_id":agent},"text":"message"}),
            false,
        ),
        (
            json!({"action":"send","target":{"kind":"session","agent_id":agent,"session_id":session},"text":"message"}),
            false,
        ),
        (
            json!({"action":"send","target":{"kind":"session","agent_id":agent,"session_id":session},"text":"message","delegate":true}),
            false,
        ),
        (
            json!({"action":"update","resource":"agent","target":agent_ref,"changes":{"name":"Renamed"}}),
            false,
        ),
        (
            json!({"action":"update","resource":"session","target":session_ref,"changes":{"title":"Renamed"}}),
            false,
        ),
        (
            json!({"action":"control","operation":"cancel_turn","target":session_ref,"turn_id":turn}),
            false,
        ),
        (
            json!({"action":"control","operation":"cancel_session","target":session_ref}),
            false,
        ),
        (
            json!({"action":"control","operation":"close_session","target":session_ref}),
            false,
        ),
        (json!({"action":"control","operation":"close_scope"}), false),
        (json!({"action":"wait"}), true),
        (json!({"action":"wait","timeout_ms":0}), true),
    ];
    for target in [
        json!({"kind":"agent","host_id":host,"agent_id":agent}),
        json!({"kind":"session","agent_id":agent,"session_id":session}),
        json!({"kind":"turn","agent_id":agent,"session_id":session,"turn_id":turn}),
        json!({"kind":"toolcall","agent_id":agent,"session_id":session,"turn_id":turn,"tool_call_id":"call"}),
        json!({"kind":"message","agent_id":agent,"session_id":session,"message_id":minicore_host_protocol::MessageId::new().unwrap()}),
        json!({"kind":"receipt","host_id":host,"operation_id":OperationId::new().unwrap()}),
    ] {
        actions.push((json!({"action":"inspect","target":target}), true));
    }
    for mode in [
        ApprovalMode::Ask,
        ApprovalMode::Auto,
        ApprovalMode::ReadOnly,
    ] {
        let policy = Policy::new(mode, true, true);
        for (arguments, read_only) in &actions {
            AgentAction::parse(arguments.clone()).expect("valid production dispatch branch");
            assert_mode_decision(
                mode,
                policy
                    .decide(request("agent", arguments.clone()))
                    .await
                    .unwrap(),
                *read_only,
            );
        }
    }
}

#[tokio::test]
async fn malformed_agent_and_name_mismatch_never_bypass_policy() {
    for mode in [
        ApprovalMode::Ask,
        ApprovalMode::Auto,
        ApprovalMode::ReadOnly,
    ] {
        let policy = Policy::new(mode, true, true);
        for arguments in [
            json!({"action":"unknown"}),
            json!({"action":"inspect"}),
            json!({"action":"wait","command":"unrecognized"}),
            json!({"action":"wait","timeout_ms":-1}),
            json!({"action":"inspect","target":{"kind":"session","agent_id":"invalid","session_id":"invalid"}}),
        ] {
            assert!(matches!(
                policy.decide(request("agent", arguments)).await.unwrap(),
                ToolDecision::Deny { .. }
            ));
        }
        let mut mismatched = request("read", json!({}));
        mismatched.spec = request("write", json!({})).spec;
        assert!(matches!(
            policy.decide(mismatched).await.unwrap(),
            ToolDecision::Deny { .. }
        ));
    }
}

#[tokio::test]
async fn cancellation_and_deadline_are_checked_without_waiting_or_approving() {
    let policy = Policy::new(ApprovalMode::Auto, true, true);
    let mut cancelled = request("read", json!({}));
    cancelled.cancellation.cancel();
    cancelled.deadline = Instant::now() - Duration::from_millis(1);
    assert_eq!(
        policy.decide(cancelled).await,
        Err(ToolPolicyError::Cancelled)
    );
    let mut expired = request("read", json!({}));
    expired.deadline = Instant::now() - Duration::from_millis(1);
    assert_eq!(policy.decide(expired).await, Err(ToolPolicyError::Failed));
}

#[tokio::test]
async fn approvals_do_not_cache_or_copy_arguments_into_prompts() {
    for _ in 0..2 {
        let policy = Policy::new(ApprovalMode::Ask, true, true);
        for _ in 0..2 {
            let decision = policy
                .decide(request(
                    "write",
                    json!({"path":"private-path","content":"secret-fixture-content"}),
                ))
                .await
                .unwrap();
            let ToolDecision::RequireApproval { request } = decision else {
                panic!("each invocation needs its own approval")
            };
            assert_eq!(request.prompt.as_str(), "Allow tool `write` for this call?");
            assert_eq!(request.risk, ApprovalRisk::Medium);
            assert!(!format!("{request:?}").contains("secret-fixture-content"));
        }
    }
}

#[tokio::test]
async fn external_names_cannot_claim_either_builtin_read_only_exemption() {
    for mode in [
        ApprovalMode::Ask,
        ApprovalMode::Auto,
        ApprovalMode::ReadOnly,
    ] {
        let policy = Policy::new(mode, false, false);
        for (name, arguments) in [
            ("read", json!({})),
            ("agent", json!({"action":"wait","timeout_ms":0})),
            (
                "agent",
                json!({"action":"inspect","target":{"kind":"session","agent_id":AgentId::new().unwrap(),"session_id":SessionId::new().unwrap()}}),
            ),
        ] {
            assert_mode_decision(
                mode,
                policy.decide(request(name, arguments)).await.unwrap(),
                false,
            );
        }
    }
}
