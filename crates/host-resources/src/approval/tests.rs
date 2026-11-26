use super::*;
use minicore_host_protocol::{AgentId, HostId, OperationId, SessionId, TurnId};
use minicore_host_runtime::{
    ToolCallId,
    tools::{ToolInvocation, ToolSpec},
};
use serde_json::{Value, json};
use std::time::Duration;

struct PolicyFixture {
    invocation: ToolInvocation,
    spec: ToolSpec,
}

impl PolicyFixture {
    fn request(&self) -> ToolPolicyRequest<'_> {
        ToolPolicyRequest {
            invocation: &self.invocation,
            spec: &self.spec,
            cancellation: Default::default(),
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }
}

fn request(name: &str, arguments: Value) -> PolicyFixture {
    PolicyFixture {
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
    }
}

fn assert_mode_decision(mode: ApprovalMode, decision: ToolDecision, read_only: bool) {
    decision.validate().unwrap();
    match (mode, read_only, decision) {
        (ApprovalMode::Auto, _, ToolDecision::Allow) | (_, true, ToolDecision::Allow) => {}
        (ApprovalMode::Ask, false, ToolDecision::RequireApproval { request }) => {
            assert_eq!(request.risk, ApprovalRisk::Medium);
        }
        other => panic!("unexpected policy decision: {other:?}"),
    }
}

#[tokio::test]
async fn ordinary_and_external_tools_follow_the_two_mode_table() {
    for mode in [ApprovalMode::Ask, ApprovalMode::Auto] {
        let policy = Policy::new(mode, true, true, None);
        for name in [
            "read",
            "write",
            "edit",
            "apply_patch",
            "bash",
            "registered_external",
        ] {
            assert_mode_decision(
                mode,
                policy
                    .decide(request(name, json!({})).request())
                    .await
                    .unwrap(),
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
    for mode in [ApprovalMode::Ask, ApprovalMode::Auto] {
        let policy = Policy::new(mode, true, true, None);
        for (arguments, read_only) in &actions {
            AgentAction::parse(arguments.clone()).expect("valid production dispatch branch");
            assert_mode_decision(
                mode,
                policy
                    .decide(request("agent", arguments.clone()).request())
                    .await
                    .unwrap(),
                *read_only,
            );
        }
    }
}

#[tokio::test]
async fn malformed_agent_arguments_fail_before_policy_approval_or_execution() {
    for mode in [ApprovalMode::Ask, ApprovalMode::Auto] {
        let policy = Policy::new(mode, true, true, None);
        for arguments in [
            json!({"action":"unknown"}),
            json!({"action":"inspect"}),
            json!({"action":"wait","command":"unrecognized"}),
            json!({"action":"wait","timeout_ms":-1}),
            json!({"action":"inspect","target":{"kind":"session","agent_id":"invalid","session_id":"invalid"}}),
        ] {
            assert!(matches!(
                policy
                    .decide(request("agent", arguments).request())
                    .await
                    .unwrap(),
                ToolDecision::InvalidInvocation { .. }
            ));
        }
        let ToolDecision::InvalidInvocation { reason } = policy
            .decide(request("agent", json!({"action":"inspect"})).request())
            .await
            .unwrap()
        else {
            panic!("malformed input is distinct from a policy refusal")
        };
        assert_eq!(
            reason.as_str(),
            "invalid agent arguments: missing field `target`"
        );
        let mut mismatched = request("read", json!({}));
        mismatched.spec = request("write", json!({})).spec;
        assert!(matches!(
            policy.decide(mismatched.request()).await.unwrap(),
            ToolDecision::Deny { .. }
        ));
    }
}

#[tokio::test]
async fn cancellation_and_deadline_are_checked_without_waiting_or_approving() {
    let policy = Policy::new(ApprovalMode::Auto, true, true, None);
    let fixture = request("read", json!({}));
    let mut cancelled = fixture.request();
    cancelled.cancellation.cancel();
    cancelled.deadline = Instant::now() - Duration::from_millis(1);
    assert_eq!(
        policy.decide(cancelled).await,
        Err(ToolPolicyError::Cancelled)
    );
    let mut expired = fixture.request();
    expired.deadline = Instant::now() - Duration::from_millis(1);
    assert_eq!(policy.decide(expired).await, Err(ToolPolicyError::Failed));
}

#[tokio::test]
async fn approvals_do_not_cache_or_copy_arguments_into_prompts() {
    let workspace = tempfile::tempdir().unwrap();
    for _ in 0..2 {
        let policy = Policy::new(
            ApprovalMode::Ask,
            true,
            true,
            Some(workspace.path().to_path_buf()),
        );
        for _ in 0..2 {
            let decision = policy
                .decide(
                    request(
                        "write",
                        json!({"path":"private-path","content":"secret-fixture-content"}),
                    )
                    .request(),
                )
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
    for mode in [ApprovalMode::Ask, ApprovalMode::Auto] {
        let policy = Policy::new(mode, false, false, None);
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
                policy
                    .decide(request(name, arguments).request())
                    .await
                    .unwrap(),
                false,
            );
        }
    }
}

#[tokio::test]
async fn cross_workspace_paths_require_approval_in_ask_and_allow_in_auto() {
    let parent = tempfile::tempdir().unwrap();
    let workspace = parent.path().join("ws");
    let sibling = parent.path().join("other");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(workspace.join("inside.txt"), "ok").unwrap();
    std::fs::write(sibling.join("outside.txt"), "ext").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&sibling, workspace.join("ext-link")).unwrap();

    let ask = Policy::new(ApprovalMode::Ask, true, true, Some(workspace.clone()));
    let auto = Policy::new(ApprovalMode::Auto, true, true, Some(workspace.clone()));

    // Inside workspace read is allowed without approval in Ask mode.
    assert_eq!(
        ask.decide(request("read", json!({"path":"inside.txt"})).request())
            .await
            .unwrap(),
        ToolDecision::Allow
    );

    // Reading MiniCore's own spilled output log directory is also auto-allowed in Ask mode.
    let log_file = minicore_host_local_tools::output_log_dir().join("probe-stdout.log");
    assert_eq!(
        ask.decide(request("read", json!({"path": log_file.to_string_lossy()})).request())
            .await
            .unwrap(),
        ToolDecision::Allow
    );

    // Outside-workspace read requires Medium approval in Ask mode (including symlinks).
    let outside_abs = sibling.join("outside.txt").to_string_lossy().into_owned();
    #[allow(unused_mut)]
    let mut outside_reads = vec![outside_abs.clone(), "../other/outside.txt".to_owned()];
    #[cfg(unix)]
    outside_reads.push("ext-link/outside.txt".to_owned());
    for raw in &outside_reads {
        let decision = ask
            .decide(request("read", json!({"path": raw})).request())
            .await
            .unwrap();
        let ToolDecision::RequireApproval { request: req } = decision else {
            panic!("expected outside read approval for {raw}, got {decision:?}");
        };
        assert_eq!(req.risk, ApprovalRisk::Medium);
        assert!(req.prompt.as_str().contains("outside workspace"));
        assert!(req.prompt.as_str().contains("outside.txt"));
        assert_eq!(
            auto.decide(request("read", json!({"path": raw})).request())
                .await
                .unwrap(),
            ToolDecision::Allow
        );
    }

    // Outside-workspace write/edit/apply_patch/bash require High approval in Ask, Allow in Auto.
    let sibling_str = sibling.to_string_lossy().into_owned();
    for (tool_name, args) in [
        ("write", json!({"path": outside_abs, "content": "x"})),
        (
            "edit",
            json!({"path": "../other/outside.txt", "old_text": "a", "new_text": "b"}),
        ),
        (
            "apply_patch",
            json!({"path": "../other/outside.txt", "patch": "@@"}),
        ),
        ("bash", json!({"command": "pwd", "cwd": sibling_str})),
        ("bash", json!({"command": "pwd", "cwd": ".."})),
    ] {
        let decision = ask
            .decide(request(tool_name, args.clone()).request())
            .await
            .unwrap();
        let ToolDecision::RequireApproval { request: req } = decision else {
            panic!("expected outside High approval for {tool_name}, got {decision:?}");
        };
        assert_eq!(req.risk, ApprovalRisk::High);
        assert!(req.prompt.as_str().contains("outside workspace"));
        assert_eq!(
            auto.decide(request(tool_name, args).request())
                .await
                .unwrap(),
            ToolDecision::Allow
        );
    }
}
