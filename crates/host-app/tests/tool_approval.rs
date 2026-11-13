//! Actual minicore-host process, stdio/WSS, production resource resolver, HTTP/SSE
//! and pinned Runtime. Only the explicitly local model replies are fixtures.
mod support;
#[allow(dead_code)]
mod tool_support;

use minicore_host_core::{
    ActivityEvent, ApprovalAnswer, ApprovalRiskData, InteractionAnswerData, InteractionRequestData,
    InteractionResolution, ObservationEvent,
};
use minicore_host_protocol::*;
use minicore_host_transport::ClientSubscription;
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::{Process, message, options, shutdown};
use tool_support::*;

async fn create(
    app: &mut App,
    workspace: &Path,
    tools: &[&str],
    mode: Option<ApprovalMode>,
) -> Value {
    let mut request = agent(workspace, tools);
    let execution = &mut request["config"]["session_defaults"]["execution"];
    match mode {
        Some(mode) => execution["approval"] = json!(mode),
        None => {
            execution.as_object_mut().unwrap().remove("approval");
        }
    }
    let receipt = app.process.rpc("agent.create", request).await;
    support::target(&receipt)
}

async fn interactions(app: &App, target: &Value) -> ClientSubscription {
    app.observer
        .subscribe(
            SubscribeRequest {
                scope: SubscriptionScope::Session {
                    agent_id: serde_json::from_value(target["agent_id"].clone()).unwrap(),
                    session_id: serde_json::from_value(target["session_id"].clone()).unwrap(),
                },
                categories: [EventCategory::Interaction].into_iter().collect(),
            },
            options(),
        )
        .await
        .unwrap()
}

async fn pending(
    app: &mut App,
    target: &Value,
    events: &mut ClientSubscription,
    call: &str,
) -> (ToolCallIdentity, InteractionId) {
    let (source, id, request) = loop {
        if let ObservationEvent::Activity(ActivityEvent::InteractionRequested {
            source,
            interaction_id,
            data,
        }) = event(events).await
        {
            break (source, interaction_id, data);
        }
    };
    assert_eq!(source.tool_call_id.0, call);
    assert_eq!(json!(source.turn.session.host_id), app.initial["host_id"]);
    assert_eq!(
        json!(source.turn.session.host_incarnation),
        app.initial["host_incarnation"]
    );
    assert_eq!(json!(source.turn.session.agent_id), target["agent_id"]);
    assert_eq!(json!(source.turn.session.session_id), target["session_id"]);
    let InteractionRequestData::Approval { risk, .. } = request else {
        panic!("actual approval")
    };
    assert_eq!(risk, ApprovalRiskData::Medium);
    let snapshot = app
        .process
        .rpc("session.state", json!({"target":target}))
        .await;
    assert_eq!(
        snapshot["data"]["active_turn_id"],
        json!(source.turn.turn_id)
    );
    assert_eq!(snapshot["data"]["load_id"], json!(source.turn.load_id));
    let waiting = snapshot["data"]["pending_interactions"].as_array().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["interaction_id"], json!(id));
    assert_eq!(waiting[0]["tool_call_id"], call);
    assert_eq!(waiting[0]["request"]["kind"], "approval");
    (source, id)
}

fn answer_request(
    source: &ToolCallIdentity,
    id: &InteractionId,
    decision: ApprovalAnswer,
) -> InteractionAnswerRequest<InteractionAnswerData> {
    InteractionAnswerRequest {
        operation_id: OperationId::new().unwrap(),
        turn_id: source.turn.turn_id.clone(),
        load_id: source.turn.load_id.clone().unwrap(),
        interaction_id: id.clone(),
        answer: InteractionAnswerData::Approval { decision },
    }
}

async fn approve(
    app: &mut App,
    target: &Value,
    source: &ToolCallIdentity,
    id: &InteractionId,
    decision: ApprovalAnswer,
) {
    let request = answer_request(source, id, decision);
    let receipt = app
        .process
        .rpc(
            "interaction.answer",
            json!({"target":target,"request":request}),
        )
        .await;
    assert_eq!(receipt["result"]["status"], "answered");
    assert_eq!(receipt["turn_id"], json!(source.turn.turn_id));
    assert_eq!(receipt["load_id"], json!(source.turn.load_id));
    assert_eq!(receipt["interaction_id"], json!(id));
}

async fn call_outcome(app: &mut App, target: &Value, turn: &Value, call: &str, expected: &str) {
    let actual = app
        .process
        .rpc(
            "toolcall.get",
            json!({"target":target,"tool":{"turn_id":turn,"tool_call_id":call}}),
        )
        .await;
    assert_eq!(actual["data"]["state"], "finished");
    assert_eq!(actual["data"]["result"]["outcome"], expected);
}

async fn complete(app: &mut App, target: &Value, turn: &Value) {
    let result = app
        .process
        .rpc("turn.wait", json!({"target":target,"turn_id":turn}))
        .await;
    assert_eq!(result["data"]["info"]["outcome"], "completed");
    assert_eq!(result["data"]["info"]["persistence"], "persisted");
}

async fn no_agent_mutation(app: &mut App) {
    let agents = app.process.rpc("agent.list", json!({})).await;
    let items = agents["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "no child Agent was created");
    for agent in items {
        assert_eq!(agent["name"], "Application fixture");
        assert_eq!(
            agent["sessions"].as_array().unwrap().len(),
            1,
            "no child Session was created"
        );
    }
}

async fn stop(mut app: App) {
    app.process.send("host.shutdown", shutdown("graceful"));
    let stopped = app.process.finish().await;
    assert!(stopped.status.success());
    stopped.protocol_only();
    stopped.safe_diagnostics();
    app.observer.close();
}

#[tokio::test]
async fn binary_omitted_approval_is_ask_and_allow_once_does_not_approve_the_next_call() {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["read", "write"]).await;
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("input"), "actual read bytes").unwrap();
    let target = create(&mut app, workspace.path(), &["read", "write"], None).await;
    let mut events = interactions(&app, &target).await;
    let config = app
        .process
        .rpc("session.get", json!({"target":target}))
        .await;
    assert_eq!(config["config"]["execution"]["approval"], "ask");
    app.process
        .rpc("message.send", message(&target, "read then write twice"))
        .await;
    provider
        .request()
        .await
        .answer(tool("read", "read", json!({"path":"input","limit":1})));
    let next = provider.request().await;
    assert_eq!(next.tool_result("read")["text"], "actual read bytes");
    next.answer(tool(
        "write-one",
        "write",
        json!({"path":"output","content":"first"}),
    ));
    let (source, id) = pending(&mut app, &target, &mut events, "write-one").await;
    assert!(!workspace.path().join("output").exists());
    let observer = app
        .observer
        .session(serde_json::from_value(target.clone()).unwrap());
    assert_eq!(
        fault(
            observer
                .answer_interaction(
                    answer_request(&source, &id, ApprovalAnswer::AllowOnce),
                    options()
                )
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::HostAccessReadOnly)
    );
    assert!(!workspace.path().join("output").exists());
    approve(&mut app, &target, &source, &id, ApprovalAnswer::AllowOnce).await;
    let next = provider.request().await;
    assert_eq!(next.tool_result("write-one")["status"], "applied");
    assert_eq!(fs::read(workspace.path().join("output")).unwrap(), b"first");
    next.answer(tool(
        "write-two",
        "write",
        json!({"path":"output","content":"second"}),
    ));
    let (source2, id2) = pending(&mut app, &target, &mut events, "write-two").await;
    assert_eq!(source.turn, source2.turn);
    assert_ne!(id, id2);
    assert_eq!(fs::read(workspace.path().join("output")).unwrap(), b"first");
    approve(&mut app, &target, &source2, &id2, ApprovalAnswer::Deny).await;
    let last = provider.request().await;
    call_outcome(
        &mut app,
        &target,
        &json!(source.turn.turn_id),
        "write-two",
        "denied",
    )
    .await;
    assert_eq!(fs::read(workspace.path().join("output")).unwrap(), b"first");
    last.answer(answer("single-call decision respected"));
    complete(&mut app, &target, &json!(source.turn.turn_id)).await;
    let before = history(&mut app.process, &target).await;
    assert_eq!(history_result(&before, "read")["outcome"], "success");
    assert_eq!(history_result(&before, "write-one")["outcome"], "success");
    assert_eq!(history_result(&before, "write-two")["outcome"], "denied");
    app.process.send("host.shutdown", shutdown("graceful"));
    let stopped = app.process.finish().await;
    assert!(stopped.status.success());
    app.observer.close();
    let mut reopened = Process::start(&app.root, None, true, Some((&app.certificate, &app.key)));
    reopened.rpc("initialize", json!({})).await;
    assert_eq!(
        reopened.rpc("session.get", json!({"target":target})).await["config"]["execution"]["approval"],
        "ask"
    );
    reopened.send("host.shutdown", shutdown("graceful"));
    assert!(reopened.finish().await.status.success());
}

#[tokio::test]
async fn binary_read_only_blocks_local_and_agent_mutations_but_reads_inspects_and_waits() {
    let mut provider = Provider::start().await;
    let tools = ["read", "write", "edit", "apply_patch", "bash", "agent"];
    let mut app = App::start(&provider, &tools).await;
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("input"), "original").unwrap();
    let target = create(
        &mut app,
        workspace.path(),
        &tools,
        Some(ApprovalMode::ReadOnly),
    )
    .await;
    let victim = create(
        &mut app,
        workspace.path(),
        &["agent"],
        Some(ApprovalMode::ReadOnly),
    )
    .await;
    let agent_ref = json!({"host_id":app.initial["host_id"],"agent_id":victim["agent_id"]});
    let execution = ExecutionSettings {
        approval: ApprovalMode::Auto,
        compaction: CompactionSettings {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    app.process
        .rpc("message.send", message(&target, "read-only policy matrix"))
        .await;
    let mut next = provider.request().await;
    let turn = only_turn(&mut app.process, &target).await;
    for (id, name, arguments) in [
        (
            "write",
            "write",
            json!({"path":"output","content":"must not write"}),
        ),
        (
            "edit",
            "edit",
            json!({"path":"input","old_text":"original","new_text":"changed"}),
        ),
        (
            "patch",
            "apply_patch",
            json!({"path":"input","patch":"*** Begin Patch\n*** Update File: input\n@@\n-original\n+changed\n*** End Patch\n"}),
        ),
        (
            "bash",
            "bash",
            json!({"command":"printf executed > shell-output"}),
        ),
        (
            "create-agent",
            "agent",
            json!({"action":"create","resource":"agent","target":{"host_id":app.initial["host_id"]},"name":"blocked","mode":"memory"}),
        ),
        (
            "create-session",
            "agent",
            json!({"action":"create","resource":"session","target":agent_ref,"text":"blocked delegated execution"}),
        ),
        (
            "send",
            "agent",
            json!({"action":"send","target":{"kind":"session","agent_id":victim["agent_id"],"session_id":victim["session_id"]},"text":"blocked send"}),
        ),
        (
            "delegate",
            "agent",
            json!({"action":"send","target":{"kind":"session","agent_id":victim["agent_id"],"session_id":victim["session_id"]},"text":"blocked delegation","delegate":true}),
        ),
        (
            "update-agent",
            "agent",
            json!({"action":"update","resource":"agent","target":agent_ref,"changes":{"name":"blocked rename"}}),
        ),
        (
            "update-policy",
            "agent",
            json!({"action":"update","resource":"session","target":target,"changes":{"execution":execution}}),
        ),
        (
            "cancel-session",
            "agent",
            json!({"action":"control","operation":"cancel_session","target":victim}),
        ),
        (
            "close-session",
            "agent",
            json!({"action":"control","operation":"close_session","target":victim}),
        ),
        (
            "close-scope",
            "agent",
            json!({"action":"control","operation":"close_scope"}),
        ),
    ] {
        next.answer(tool(id, name, arguments));
        next = provider.request().await;
        call_outcome(&mut app, &target, &turn, id, "denied").await;
    }
    next.answer(tool("read", "read", json!({"path":"input","limit":1})));
    let next = provider.request().await;
    assert_eq!(next.tool_result("read")["text"], "original");
    next.answer(tool("inspect", "agent", json!({"action":"inspect","target":{"kind":"session","agent_id":victim["agent_id"],"session_id":victim["session_id"]},"include_config":true})));
    let next = provider.request().await;
    assert!(next.tool_result("inspect").get("error").is_none());
    call_outcome(&mut app, &target, &turn, "inspect", "success").await;
    next.answer(tool(
        "wait",
        "agent",
        json!({"action":"wait","timeout_ms":0}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("wait")["status"], "timed_out");
    assert_eq!(
        fs::read(workspace.path().join("input")).unwrap(),
        b"original"
    );
    assert!(!workspace.path().join("output").exists());
    assert!(!workspace.path().join("shell-output").exists());
    assert_eq!(
        app.process
            .rpc("session.get", json!({"target":target}))
            .await["config"]["execution"]["approval"],
        "read_only"
    );
    let victim_state = app
        .process
        .rpc("session.state", json!({"target":victim}))
        .await;
    assert_eq!(victim_state["data"]["lifecycle"], "open");
    assert!(
        victim_state["data"]["next_turn"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        app.process.rpc("turn.list", json!({"target":victim})).await["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    no_agent_mutation(&mut app).await;
    next.answer(answer("read-only checks complete"));
    complete(&mut app, &target, &turn).await;
    stop(app).await;
}

#[tokio::test]
async fn binary_ask_does_not_allow_agent_mutations_to_bypass_approval() {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["agent"]).await;
    let workspace = tempfile::tempdir().unwrap();
    let target = create(&mut app, workspace.path(), &["agent"], None).await;
    let victim = create(&mut app, workspace.path(), &["agent"], None).await;
    let mut events = interactions(&app, &target).await;
    let host = app.initial["host_id"].clone();
    app.process
        .rpc("message.send", message(&target, "agent policy checks"))
        .await;
    let mut next = provider.request().await;
    let turn = only_turn(&mut app.process, &target).await;
    for (id, arguments) in [
        (
            "create",
            json!({"action":"create","resource":"agent","target":{"host_id":host},"name":"blocked","mode":"memory","text":"blocked task"}),
        ),
        (
            "send",
            json!({"action":"send","target":{"kind":"session","agent_id":victim["agent_id"],"session_id":victim["session_id"]},"text":"blocked task","delegate":true}),
        ),
        (
            "update",
            json!({"action":"update","resource":"session","target":victim,"changes":{"title":"blocked"}}),
        ),
        (
            "control",
            json!({"action":"control","operation":"close_session","target":victim}),
        ),
    ] {
        next.answer(tool(id, "agent", arguments));
        let (source, interaction) = pending(&mut app, &target, &mut events, id).await;
        no_agent_mutation(&mut app).await;
        let state = app
            .process
            .rpc("session.state", json!({"target":victim}))
            .await;
        assert!(state["data"]["next_turn"].as_array().unwrap().is_empty());
        assert_eq!(state["data"]["lifecycle"], "open");
        assert!(
            app.process
                .rpc("session.get", json!({"target":victim}))
                .await["title"]
                .is_null()
        );
        approve(
            &mut app,
            &target,
            &source,
            &interaction,
            ApprovalAnswer::Deny,
        )
        .await;
        next = provider.request().await;
        call_outcome(&mut app, &target, &turn, id, "denied").await;
    }
    no_agent_mutation(&mut app).await;
    next.answer(answer("agent calls denied individually"));
    complete(&mut app, &target, &turn).await;
    stop(app).await;
}

async fn cancel_waiting_approval(force: bool) {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["bash"]).await;
    let workspace = tempfile::tempdir().unwrap();
    let target = create(&mut app, workspace.path(), &["bash"], None).await;
    let mut events = interactions(&app, &target).await;
    app.process
        .rpc(
            "message.send",
            message(&target, "wait for approval before shell"),
        )
        .await;
    provider.request().await.answer(tool(
        "shell",
        "bash",
        json!({"command":"printf must-not-run > shell-output"}),
    ));
    let (source, id) = pending(&mut app, &target, &mut events, "shell").await;
    assert!(!workspace.path().join("shell-output").exists());
    if force {
        app.process.send("host.shutdown", shutdown("force"));
        let stopped = app.process.finish().await;
        assert!(stopped.status.success());
        stopped.protocol_only();
        stopped.safe_diagnostics();
        app.observer.close();
    } else {
        app.process.rpc("turn.cancel", json!({"target":target,"operation_id":OperationId::new().unwrap(),"turn_id":source.turn.turn_id})).await;
        let result = app
            .process
            .rpc(
                "turn.wait",
                json!({"target":target,"turn_id":source.turn.turn_id}),
            )
            .await;
        assert_eq!(result["data"]["info"]["outcome"], "cancelled");
        loop {
            if let ObservationEvent::Activity(ActivityEvent::InteractionResolved {
                interaction_id,
                data,
                ..
            }) = event(&mut events).await
            {
                assert_eq!(interaction_id, id);
                assert_eq!(data.resolution, InteractionResolution::Cancelled);
                break;
            }
        }
        stop(app).await;
    }
    assert!(!workspace.path().join("shell-output").exists());
}

#[tokio::test]
async fn binary_cancel_waiting_approval_never_starts_the_tool() {
    cancel_waiting_approval(false).await;
}
#[tokio::test]
async fn binary_force_shutdown_waiting_approval_never_starts_the_tool() {
    cancel_waiting_approval(true).await;
}

#[tokio::test]
async fn binary_policy_update_is_frozen_through_internal_loops_and_applies_next_turn() {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["write"]).await;
    let workspace = tempfile::tempdir().unwrap();
    let target = create(&mut app, workspace.path(), &["write"], None).await;
    let mut events = interactions(&app, &target).await;
    app.process
        .rpc("message.send", message(&target, "first public Turn"))
        .await;
    let first = provider.request().await;
    let turn = only_turn(&mut app.process, &target).await;
    let execution = ExecutionSettings {
        approval: ApprovalMode::Auto,
        compaction: CompactionSettings {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    };
    tokio::time::timeout(support::WAIT, async {
        loop {
            let state = app
                .process
                .rpc("session.state", json!({"target":target}))
                .await;
            if !state["data"]["metadata_saving"]
                .as_bool()
                .expect("Session exposes its metadata owner")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("accepted input activity save completes before the policy update");
    app.process.rpc("session.update", json!({"target":target,"operation_id":OperationId::new().unwrap(),"expected_config_revision":0,"changes":{"config":{"execution":execution}}})).await;
    tokio::time::timeout(support::WAIT, async {
        loop {
            let config = app
                .process
                .rpc("session.get", json!({"target":target}))
                .await;
            if config["config_revision"] == 1 {
                assert_eq!(config["config"]["execution"]["approval"], "auto");
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    first.answer(tool(
        "first",
        "write",
        json!({"path":"first","content":"approved"}),
    ));
    let (source, id) = pending(&mut app, &target, &mut events, "first").await;
    assert!(!workspace.path().join("first").exists());
    approve(&mut app, &target, &source, &id, ApprovalAnswer::AllowOnce).await;
    let finish_loop = provider.request().await;
    assert_eq!(
        fs::read(workspace.path().join("first")).unwrap(),
        b"approved"
    );
    let mut steer = message(&target, "continue inside the same public Turn");
    steer["mode"] = json!("steer");
    app.process.rpc("message.send", steer).await;
    finish_loop.answer(answer("first internal loop finished"));
    provider.request().await.answer(tool(
        "continued",
        "write",
        json!({"path":"continued","content":"must still ask"}),
    ));
    let (continued, next_id) = pending(&mut app, &target, &mut events, "continued").await;
    assert_eq!(continued.turn, source.turn);
    let frozen = app
        .process
        .rpc("turn.get", json!({"target":target,"turn_id":turn}))
        .await;
    assert_eq!(frozen["data"]["config"]["execution"]["approval"], "ask");
    assert_eq!(frozen["data"]["config_revision"], 0);
    assert_ne!(next_id, id);
    assert!(!workspace.path().join("continued").exists());
    approve(
        &mut app,
        &target,
        &continued,
        &next_id,
        ApprovalAnswer::Deny,
    )
    .await;
    provider
        .request()
        .await
        .answer(answer("public Turn finished"));
    complete(&mut app, &target, &turn).await;
    app.process
        .rpc(
            "message.send",
            message(&target, "next public Turn uses Auto"),
        )
        .await;
    provider.request().await.answer(tool(
        "next-turn",
        "write",
        json!({"path":"next","content":"automatic"}),
    ));
    let last = provider.request().await;
    assert_eq!(last.tool_result("next-turn")["status"], "applied");
    assert_eq!(
        fs::read(workspace.path().join("next")).unwrap(),
        b"automatic"
    );
    let turns = app.process.rpc("turn.list", json!({"target":target})).await;
    let next_turn = turns["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["turn_id"] != turn)
        .unwrap()["turn_id"]
        .clone();
    assert_ne!(next_turn, turn);
    let bound = app
        .process
        .rpc("turn.get", json!({"target":target,"turn_id":next_turn}))
        .await;
    assert_eq!(bound["data"]["config"]["execution"]["approval"], "auto");
    assert_eq!(bound["data"]["config_revision"], 1);
    last.answer(answer("next public Turn finished"));
    complete(&mut app, &target, &next_turn).await;
    assert!(!workspace.path().join("continued").exists());
    stop(app).await;
}
