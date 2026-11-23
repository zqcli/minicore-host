//! Runtime tools plus explicit owner-port fixtures, not real Host integration.
mod support;
use minicore_host_protocol::*;
use minicore_host_runtime::tools::{Tool, ToolError, ToolExecutionOutcome, ToolInvocation};
use minicore_host_tools::*;
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::*;

#[tokio::test]
async fn malformed_arguments_return_failed_diagnostics_without_entering_an_owner() {
    let port = FixtureTurn::new();
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    let result = tool
        .execute(
            ToolInvocation::new(
                "invalid-agent-call".parse().unwrap(),
                "agent".parse().unwrap(),
                json!({"action":"create","resource":"agent","mode":"memory"}),
            )
            .unwrap(),
            context(),
        )
        .await
        .unwrap();
    let ToolExecutionOutcome::Failed(output) = result else {
        panic!("invalid arguments cannot produce successful tool execution")
    };
    assert_eq!(
        output.content().as_str(),
        "invalid agent arguments: missing field `name`"
    );
    assert!(port.state.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn create_without_text_only_creates_and_never_registers_delegation() {
    let port = FixtureTurn::new();
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    let output=invoke(&tool,json!({"action":"create","resource":"agent","target":{"host_id":port.source.host_id},"name":"empty","mode":"memory"})).await;
    assert_eq!(output["kind"], "agent.create");
    assert!(output.get("message_id").is_none());
    assert_eq!(*port.state.events.lock().unwrap(), vec!["create_agent"]);
}
#[tokio::test]
async fn sessions_default_to_the_callers_agent_and_inherit_its_session_settings() {
    let port = FixtureTurn::new();
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    invoke(&tool, json!({"action":"create","resource":"session"})).await;
    let explicit = json!({"action":"create","resource":"session","settings":{"reasoning":"high"}});
    invoke(&tool, explicit).await;
    let creates = port.state.session_creates.lock().unwrap().clone();
    let caller = SessionConfig::default();
    for (request, reasoning) in creates
        .iter()
        .zip([caller.reasoning, ReasoningPreference::High])
    {
        assert_eq!(request.target.host_id, port.source.host_id);
        assert_eq!(request.target.agent_id, port.source.session.agent_id);
        let settings = &request.overrides.as_ref().unwrap().0;
        assert_eq!(settings.model.as_ref(), Some(&caller.model));
        assert_eq!(settings.reasoning, Some(reasoning));
        assert_eq!(settings.execution.as_ref(), Some(&caller.execution));
    }
}

#[tokio::test]
async fn create_registers_receiver_before_acceptance_and_returns_real_facts() {
    let port = FixtureTurn::new();
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    let output=invoke(&tool,json!({"action":"create","resource":"session","target":{"host_id":port.source.host_id,"agent_id":port.source.session.agent_id},"text":"work"})).await;
    assert_eq!(output["kind"], "session.create");
    assert_eq!(output["result"]["delivery"], "accepted");
    assert_eq!(output["result"]["steps"]["message"]["status"], "succeeded");
    assert_eq!(
        *port.state.events.lock().unwrap(),
        vec![
            "create_session",
            "register_scope",
            "register_sink",
            "accept_task",
            "project_relation"
        ]
    );
    let relation = port
        .state
        .relations
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    assert_eq!(
        relation.registration.task_message_id.as_str(),
        output["message_id"].as_str().unwrap()
    );
    assert_eq!(
        relation.registration.origin.tool_call_id.as_str(),
        "fixture-call"
    );
    assert!(relation.registration.temporary);
}
#[tokio::test]
async fn failed_preparation_preserves_creation_and_does_not_accept_task() {
    let port = FixtureTurn::new();
    port.state.prepare_fails.store(true, Ordering::Release);
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    let output=invoke(&tool,json!({"action":"create","resource":"agent","target":{"host_id":port.source.host_id},"name":"reviewer","mode":"memory","text":"work"})).await;
    assert_eq!(output["result"]["steps"]["agent"]["status"], "succeeded");
    assert_eq!(output["result"]["steps"]["session"]["status"], "succeeded");
    assert_eq!(output["result"]["steps"]["message"]["status"], "failed");
    assert!(output.get("agent_id").is_some());
    assert!(output.get("message_id").is_some());
    assert!(
        !port
            .state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event == "accept_task")
    );
}
#[tokio::test]
async fn update_uses_one_snapshot_and_never_retries_a_conflict() {
    let port = FixtureTurn::new();
    port.state.update_conflict.store(true, Ordering::Release);
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    let result = tool.execute(ToolInvocation::new(
        "fixture-call".parse().unwrap(), "agent".parse().unwrap(),
        json!({"action":"update","resource":"agent","target":{"host_id":port.source.host_id,"agent_id":port.source.session.agent_id},"changes":{"model":"new-model","name":"renamed"}}),
    ).unwrap(), context()).await.unwrap();
    let ToolExecutionOutcome::Failed(output) = result else {
        panic!("a business Fault must retain the native failed tool outcome");
    };
    let expected =
        json!({"error":Fault::new(FaultKind::AgentConfigConflict, "fixture revision changed")});
    assert_eq!(
        output.content().as_str(),
        serde_json::to_string(&expected).unwrap()
    );
    assert_eq!(port.state.snapshot_reads.load(Ordering::Acquire), 1);
    let writes = port.state.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].expected_revision, 7);
    assert_eq!(
        writes[0].config.session_defaults.model.as_str(),
        "new-model"
    );
    assert_eq!(writes[0].name.as_str(), "renamed");
}
#[tokio::test]
async fn expired_turn_is_business_fault_and_cannot_use_owner_ports() {
    let port = FixtureTurn::new();
    port.active.store(false, Ordering::Release);
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    let output=invoke(&tool,json!({"action":"create","resource":"agent","target":{"host_id":port.source.host_id},"name":"empty","mode":"memory"})).await;
    assert!(output.get("error").is_some());
    assert!(port.state.events.lock().unwrap().is_empty());
}
#[tokio::test]
async fn waiting_respects_pause_cancellation_and_business_timeout() {
    let port = FixtureTurn::new();
    let tool = AgentTool::new(port.port(), ScopeManager::new());
    port.state.ready.store(true, Ordering::Release);
    port.state.paused.store(true, Ordering::Release);
    let output = invoke(&tool, json!({"action":"wait","timeout_ms":1})).await;
    assert_eq!(output["status"], "timed_out");
    port.state.paused.store(false, Ordering::Release);
    let output = invoke(&tool, json!({"action":"wait"})).await;
    assert_eq!(output["status"], "input_available");
    let context = context();
    context.cancellation.cancel();
    let result = tool
        .execute(
            ToolInvocation::new(
                "c".parse().unwrap(),
                "agent".parse().unwrap(),
                json!({"action":"wait"}),
            )
            .unwrap(),
            context,
        )
        .await;
    assert_eq!(result.unwrap_err(), ToolError::Cancelled);
    assert!(
        port.state.ready.load(Ordering::Acquire),
        "wait did not consume the input"
    );
}
#[tokio::test]
async fn registry_freezes_exact_selected_specs_and_implementations() {
    let port = FixtureTurn::new();
    let mut registry = ToolRegistry::new();
    registry
        .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
        .unwrap();
    assert!(
        registry
            .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
            .is_err()
    );
    let empty = registry.bind(&[], port.port()).unwrap();
    assert!(empty.specs().is_empty());
    let selected = [ToolBinding {
        name: "agent".into(),
    }];
    let actual = registry.bind(&selected, port.port()).unwrap();
    assert_eq!(
        actual.get(&"agent".parse().unwrap()).unwrap().spec(),
        &registry.specs()[0]
    );
    assert!(
        registry
            .bind(&[selected[0].clone(), selected[0].clone()], port.port())
            .is_err()
    );
    assert!(
        registry
            .bind(
                &[ToolBinding {
                    name: "missing".into()
                }],
                port.port()
            )
            .is_err()
    );
}
#[tokio::test]
async fn next_step_wait_registers_before_checking_and_has_no_lost_wake_in_fixture() {
    let port = FixtureTurn::new();
    let tool = Arc::new(AgentTool::new(port.port(), ScopeManager::new()));
    let task = tokio::spawn({
        let tool = Arc::clone(&tool);
        async move { invoke(tool.as_ref(), json!({"action":"wait"})).await }
    });
    tokio::task::yield_now().await;
    port.state.ready.store(true, Ordering::Release);
    port.state.changed.notify_waiters();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()["status"],
        "input_available"
    );
}

#[tokio::test]
async fn dropped_startup_wait_still_cleans_a_created_temporary_session() {
    let port = FixtureTurn::new();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    *port.state.creation_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let tool = Arc::new(AgentTool::new(port.port(), ScopeManager::new()));
    let context = context();
    let cancellation = context.cancellation.clone();
    let args = json!({"action":"create","resource":"session","target":{"host_id":port.source.host_id,"agent_id":port.source.session.agent_id},"text":"work"});
    let caller = tokio::spawn({
        let tool = Arc::clone(&tool);
        async move {
            tool.execute(
                ToolInvocation::new(
                    "fixture-call".parse().unwrap(),
                    "agent".parse().unwrap(),
                    args,
                )
                .unwrap(),
                context,
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if port
                .state
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == "create_session")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancellation.cancel();
    caller.abort();
    gate.add_permits(1);
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let changed = port.state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if port
                .state
                .events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event == "cleanup_created")
            {
                break;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
    assert!(
        !port
            .state
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event == "accept_task")
    );
}
