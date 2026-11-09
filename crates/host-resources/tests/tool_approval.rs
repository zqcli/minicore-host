//! Explicit local HTTP/SSE model fixture, real production resources + Host +
//! Runtime. The registered external fixture records actual execute invocations.
#[allow(dead_code)]
#[path = "task042/support.rs"]
mod support;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use minicore_host_tools::{ToolFactory, ToolRegistry, TurnPort};
use minicore_runtime::{
    HistoryItem,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput,
        ToolResultOutcome, ToolSpec,
    },
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::*;

#[derive(Clone)]
struct RegisteredExternalFixture {
    spec: ToolSpec,
    executions: Arc<AtomicUsize>,
}
impl ToolFactory for RegisteredExternalFixture {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn bind(&self, _: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
        Ok(Arc::new(self.clone()))
    }
}
impl Tool for RegisteredExternalFixture {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute(&self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'_> {
        self.executions.fetch_add(1, Ordering::AcqRel);
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("actual external fixture execution").unwrap(),
            ))
        })
    }
}

async fn pending(host: &Host, caller: &CallerContext, target: &SessionRef) -> SessionSnapshot {
    let mut changes = host.session_states(caller, target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = host.session_state(caller, target).unwrap();
            if !snapshot.data.pending_interactions.is_empty() {
                return snapshot;
            }
            changes.changed().await.unwrap();
        }
    })
    .await
    .expect("actual Runtime approval waiting point")
}

async fn external_scenario(mode: ApprovalMode, decision: ApprovalAnswer) {
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(RegisteredExternalFixture {
            spec: ToolSpec::new(
                "registered_external".parse().unwrap(),
                "Registered external test fixture with an execution counter",
                json!({"type":"object","properties":{},"additionalProperties":false}),
            )
            .unwrap(),
            executions: executions.clone(),
        }))
        .unwrap();
    let builder = HostResourceBuilder::new(Arc::new(registry));
    let final_gate = Arc::new(tokio::sync::Semaphore::new(0));
    let mut server = Server::start(vec![
        (
            tool_answer("external-call", "registered_external", json!({})),
            None,
        ),
        (answer("external call settled"), Some(final_gate.clone())),
    ])
    .await;
    let mut host_config = config(&server.endpoint);
    host_config.tools = selection(&["registered_external"]);
    let host = Host::memory_configured(prepare(&builder, host_config).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut create = request(selection(&["registered_external"]), None);
    create.config.session_defaults.execution.approval = mode;
    let receipt = host.create_agent(&caller, create).unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    send(&host, &caller, &target, "call registered fixture");
    let first = server.request().await;
    assert_eq!(first["tools"][0]["function"]["name"], "registered_external");
    if mode == ApprovalMode::Ask {
        let snapshot = pending(&host, &caller, &target).await;
        assert_eq!(executions.load(Ordering::Acquire), 0);
        let waiting = &snapshot.data.pending_interactions[0];
        assert_eq!(waiting.tool_call_id.0, "external-call");
        assert_eq!(waiting.name, "registered_external");
        assert!(matches!(
            waiting.request,
            InteractionRequestData::Approval { .. }
        ));
        let request = InteractionAnswerRequest {
            operation_id: OperationId::new().unwrap(),
            turn_id: snapshot.data.active_turn_id.clone().unwrap(),
            load_id: snapshot.data.load_id.clone().unwrap(),
            interaction_id: waiting.interaction_id.clone(),
            answer: InteractionAnswerData::Approval { decision },
        };
        let observer = host.connect().unwrap();
        assert_eq!(
            host.answer_interaction(&observer.caller(), &target, request.clone())
                .unwrap_err()
                .kind(),
            Some(FaultKind::HostAccessReadOnly)
        );
        let mut wrong = request.clone();
        wrong.load_id = LoadId::new().unwrap();
        assert_eq!(
            host.answer_interaction(&caller, &target, wrong)
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionOwnershipMismatch)
        );
        let mut wrong = request.clone();
        wrong.turn_id = TurnId::new().unwrap();
        assert_eq!(
            host.answer_interaction(&caller, &target, wrong)
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionTurnNotFound)
        );
        let mut wrong = request.clone();
        wrong.interaction_id = InteractionId::new().unwrap();
        assert_eq!(
            host.answer_interaction(&caller, &target, wrong)
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionInteractionNotPending)
        );
        assert_eq!(executions.load(Ordering::Acquire), 0);
        let accepted = host
            .answer_interaction(&caller, &target, request.clone())
            .unwrap();
        assert_eq!(accepted.turn_id, request.turn_id);
        assert_eq!(accepted.load_id, request.load_id);
        assert_eq!(accepted.interaction_id, request.interaction_id);
        assert_eq!(accepted.result.status, InteractionAnswerStatus::Answered);
        assert_eq!(
            host.answer_interaction(&caller, &target, request)
                .unwrap_err()
                .kind(),
            Some(FaultKind::SessionInteractionNotPending)
        );
    }
    server.request().await;
    final_gate.add_permits(1);
    let turns = settled(&host, &caller, &target, 1).await;
    assert_eq!(turns[0].outcome, Some(TurnOutcome::Completed));
    let allowed = mode == ApprovalMode::Auto
        || (mode == ApprovalMode::Ask && decision == ApprovalAnswer::AllowOnce);
    assert_eq!(executions.load(Ordering::Acquire), usize::from(allowed));
    let history = host.history(&caller, &target).unwrap();
    let result = history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(result) if result.call_id.as_str() == "external-call" => {
                Some(result)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(
        result.outcome,
        if allowed {
            ToolResultOutcome::Success
        } else {
            ToolResultOutcome::Denied
        }
    );
    assert!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .pending_interactions
            .is_empty()
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
}

#[tokio::test]
async fn registered_external_ask_preserves_original_identity_and_waits_for_allow_once() {
    external_scenario(ApprovalMode::Ask, ApprovalAnswer::AllowOnce).await;
}
#[tokio::test]
async fn registered_external_ask_deny_does_not_execute() {
    external_scenario(ApprovalMode::Ask, ApprovalAnswer::Deny).await;
}
#[tokio::test]
async fn registered_external_read_only_does_not_execute() {
    external_scenario(ApprovalMode::ReadOnly, ApprovalAnswer::AllowOnce).await;
}
#[tokio::test]
async fn registered_external_explicit_auto_executes() {
    external_scenario(ApprovalMode::Auto, ApprovalAnswer::AllowOnce).await;
}
