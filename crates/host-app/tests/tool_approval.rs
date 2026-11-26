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

async fn pending_with_risk(
    app: &mut App,
    target: &Value,
    events: &mut ClientSubscription,
    call: &str,
    expected_risk: ApprovalRiskData,
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
    assert_eq!(risk, expected_risk);
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

async fn pending(
    app: &mut App,
    target: &Value,
    events: &mut ClientSubscription,
    call: &str,
) -> (ToolCallIdentity, InteractionId) {
    pending_with_risk(app, target, events, call, ApprovalRiskData::Medium).await
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
    let mut reopened = Process::start(
        &app.root,
        &app.config_dir,
        true,
        Some((&app.certificate, &app.key)),
    );
    reopened.rpc("initialize", json!({})).await;
    assert_eq!(
        reopened.rpc("session.get", json!({"target":target})).await["config"]["execution"]["approval"],
        "ask"
    );
    reopened.send("host.shutdown", shutdown("graceful"));
    assert!(reopened.finish().await.status.success());
}

#[tokio::test]
async fn binary_cross_workspace_requires_approval_in_ask_and_allows_in_auto() {
    let mut provider = Provider::start().await;
    let tools = ["read", "write", "bash"];
    let mut app = App::start(&provider, &tools).await;
    let parent = tempfile::tempdir().unwrap();
    let workspace = parent.path().join("ws");
    let outside = parent.path().join("outside");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("ext.txt"), "external-value").unwrap();
    let outside_file = outside.join("ext.txt").to_string_lossy().into_owned();
    let outside_new = outside.join("created.txt").to_string_lossy().into_owned();

    // Ask mode: outside read requires Medium approval; outside write requires High approval.
    let ask_target = create(&mut app, &workspace, &tools, Some(ApprovalMode::Ask)).await;
    let mut events = interactions(&app, &ask_target).await;
    app.process
        .rpc("message.send", message(&ask_target, "cross-workspace ask"))
        .await;
    let next = provider.request().await;
    let turn = only_turn(&mut app.process, &ask_target).await;
    next.answer(tool(
        "ext-read",
        "read",
        json!({"path": outside_file, "limit": 1}),
    ));
    let (source, id) = pending_with_risk(
        &mut app,
        &ask_target,
        &mut events,
        "ext-read",
        ApprovalRiskData::Medium,
    )
    .await;
    approve(
        &mut app,
        &ask_target,
        &source,
        &id,
        ApprovalAnswer::AllowOnce,
    )
    .await;
    let next = provider.request().await;
    assert_eq!(next.tool_result("ext-read")["text"], "external-value");

    next.answer(tool(
        "ext-write",
        "write",
        json!({"path": outside_new, "content": "denied-write"}),
    ));
    let (source, id) = pending_with_risk(
        &mut app,
        &ask_target,
        &mut events,
        "ext-write",
        ApprovalRiskData::High,
    )
    .await;
    assert!(!outside.join("created.txt").exists());
    approve(&mut app, &ask_target, &source, &id, ApprovalAnswer::Deny).await;
    let next = provider.request().await;
    call_outcome(&mut app, &ask_target, &turn, "ext-write", "denied").await;
    assert!(!outside.join("created.txt").exists());
    next.answer(answer("ask cross-workspace checks complete"));
    complete(&mut app, &ask_target, &turn).await;

    // Auto mode: outside read and outside write succeed without approval prompts.
    let auto_target = create(&mut app, &workspace, &tools, Some(ApprovalMode::Auto)).await;
    app.process
        .rpc(
            "message.send",
            message(&auto_target, "cross-workspace auto"),
        )
        .await;
    let next = provider.request().await;
    let auto_turn = only_turn(&mut app.process, &auto_target).await;
    next.answer(tool(
        "auto-read",
        "read",
        json!({"path": outside_file, "limit": 1}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("auto-read")["text"], "external-value");
    next.answer(tool(
        "auto-write",
        "write",
        json!({"path": outside_new, "content": "auto-written"}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("auto-write")["status"], "applied");
    assert_eq!(
        fs::read_to_string(outside.join("created.txt")).unwrap(),
        "auto-written"
    );
    next.answer(answer("auto cross-workspace checks complete"));
    complete(&mut app, &auto_target, &auto_turn).await;
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
