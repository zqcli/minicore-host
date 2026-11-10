//! New recovery probes using real Host/Runtime and agent-tool execution.
//! The model stream is an explicit test fixture. No capability or owner is faked.
#[path = "support/gated_host.rs"]
mod gated_host;
#[path = "../../host-core/tests/support/input_context.rs"]
mod input_context;
use gated_host::{Fixture, Resources};
use minicore_host_core::{Host, TurnSource};
use minicore_host_protocol::*;
use minicore_host_tools::{AgentToolFactory, ScopeManager, ToolRegistry};
use minicore_runtime::model::{ModelEvent, ModelMessage};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

impl Fixture {
    async fn open(path: &std::path::Path) -> Self {
        let (calls, called) = mpsc::unbounded_channel();
        let mut tools = ToolRegistry::new();
        tools
            .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
            .unwrap();
        let host = Host::open_persistent(
            path,
            minicore_host_core::HostSettings {
                name: "persistent inspect fixture".into(),
            },
            Arc::new(Resources { calls, tools }),
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        Self {
            host,
            connection,
            calls: called,
        }
    }
    async fn persistent_agent(&self) -> SessionRef {
        let mut config = AgentConfig::default();
        config.role.instructions = "original bound instructions".into();
        config.session_defaults.tools = vec![ToolBinding {
            name: "agent".into(),
        }];
        config.session_defaults.execution.compaction.enabled = false;
        let receipt = self
            .host
            .create_agent_async(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "persistent inspect fixture".into(),
                    mode: StorageMode::Persistent,
                    config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .await
            .unwrap();
        SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        }
    }
    async fn stop(&self) {
        self.host
            .shutdown(
                &self.connection.caller(),
                HostShutdownRequest {
                    operation_id: OperationId::new().unwrap(),
                    strategy: HostShutdownStrategy::Graceful,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.host.info(&self.connection.caller()).unwrap().lifecycle
                != HostLifecycle::Stopped
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        self.connection.close();
    }
    async fn agent(&self) -> SessionRef {
        let mut config = AgentConfig::default();
        config.role.instructions = "original bound instructions".into();
        config.session_defaults.tools = vec![ToolBinding {
            name: "agent".into(),
        }];
        let receipt = self
            .host
            .create_agent_async(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "inspect recovery fixture".into(),
                    mode: StorageMode::Memory,
                    config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .await
            .unwrap();
        SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        }
    }
}
fn inspect(source: &TurnSource, output: bool) -> Value {
    json!({
        "action": "inspect",
        "target": {
            "kind": "turn",
            "agent_id": source.session.agent_id,
            "session_id": source.session.session_id,
            "turn_id": source.turn.turn_id,
        },
        "include_output": output,
    })
}
fn assert_tool_summary(value: &Value, source: &TurnSource, state: &str) {
    assert_eq!(value["turn_id"], json!(source.turn.turn_id));
    assert_eq!(value["session_id"], json!(source.session.session_id));
    assert_eq!(value["data"]["state"], state);
    assert!(
        value["data"].get("config").is_none(),
        "Turn tool inspect leaked the full bound config: {value}"
    );
    assert!(!value.to_string().contains("original bound instructions"));
}
fn assert_public_binding(f: &Fixture, source: &TurnSource) {
    let detailed = f
        .host
        .turn_snapshot(
            &f.connection.caller(),
            &source.session,
            &source.turn.turn_id,
        )
        .unwrap();
    assert_eq!(
        detailed.data.config.role.instructions,
        "original bound instructions"
    );
    assert_eq!(detailed.data.state.config_revision, 0);
}

#[tokio::test]
async fn active_turn_inspect_is_lightweight_while_public_get_keeps_bound_configuration() {
    let mut f = Fixture::new();
    let session = f.agent().await;
    f.send(&session, "inspect the active Turn");
    let first = f.next(&session).await;
    f.host
        .update_session(
            &f.connection.caller(),
            &session,
            0,
            &SessionConfigPatch {
                role: Some(Role {
                    instructions: "next Turn instructions".into(),
                    ..Role::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    first.tool("inspect-active", inspect(&first.source, false));
    let next = f.next(&session).await;
    assert_tool_summary(&next.last_tool_result(), &first.source, "running");
    assert_public_binding(&f, &first.source);
    next.events
        .send(ModelEvent::text_delta("visible while running"))
        .unwrap();
    let mut range = inspect(&first.source, true);
    range["response_index"] = json!(1);
    range["output_range"] = json!({"channel":"text","start":0,"end":21,"max_bytes":21});
    next.tool("inspect-current-range", range);
    let next = f.next(&session).await;
    let ranged = next.last_tool_result();
    assert_tool_summary(&ranged, &first.source, "running");
    assert_eq!(ranged["data"]["output"]["text"], "visible while running");
    assert_eq!(ranged["data"]["response_index"], 1);
    assert!(ranged["data"].get("history").is_none());
    next.complete("actual first result");
    f.finished(&first.source).await;
    assert_public_binding(&f, &first.source);
}

#[tokio::test]
async fn completed_turn_inspect_omits_config_with_and_without_actual_output() {
    let mut f = Fixture::new();
    let session = f.agent().await;
    let original_message = f.send(&session, "complete the original Turn");
    let first = f.next(&session).await;
    first.complete("actual retained completion");
    f.finished(&first.source).await;
    f.send(&session, "inspect the original completion");
    let next = f.next(&session).await;
    next.tool("inspect-completed", inspect(&first.source, false));
    let next = f.next(&session).await;
    let summary = next.last_tool_result();
    assert_tool_summary(&summary, &first.source, "finished");
    assert!(summary["data"].get("history").is_none());
    next.tool("inspect-completed-output", inspect(&first.source, true));
    let next = f.next(&session).await;
    let with_output = next.last_tool_result();
    assert_tool_summary(&with_output, &first.source, "finished");
    assert!(with_output["data"]["history"].is_array());
    assert_eq!(
        with_output["data"]["history"][0]["message_id"],
        json!(original_message)
    );
    assert_eq!(
        with_output["data"]["history"][0]["source"],
        json!({"kind":"user"})
    );
    assert_eq!(with_output["data"]["history"][0]["mode"], "follow_up");
    assert!(
        with_output["data"]["history"]
            .to_string()
            .contains("actual retained completion")
    );
    assert_eq!(with_output["data"]["partials"], json!([]));
    assert_public_binding(&f, &first.source);
    next.complete("inspections complete");
    f.finished(&next.source).await;
}

#[tokio::test]
async fn agent_inspect_reads_saved_turn_after_restart_without_inventing_execution_metadata() {
    let path = std::env::temp_dir().join(format!(
        "minicore-inspect-saved-{}",
        OperationId::new().unwrap()
    ));
    let mut f = Fixture::open(&path).await;
    let target = f.persistent_agent().await;
    f.send(&target, "save the original answer");
    let first = f.next(&target).await;
    first.complete("actual saved answer — 原文完整保留");
    f.finished(&first.source).await;
    let old_source = first.source.clone();
    f.stop().await;
    drop(f);

    let mut f = Fixture::open(&path).await;
    let public = f
        .host
        .turn_result_snapshot(&f.connection.caller(), &target, &old_source.turn.turn_id)
        .await
        .unwrap();
    assert_ne!(
        public.source.session.host_incarnation,
        old_source.host_incarnation
    );
    assert!(public.source.load_id.is_none());
    assert!(public.data.info.is_none());
    assert!(public.data.config.is_none());
    assert_eq!(
        public.data.saved_terminal.as_ref().unwrap().outcome,
        TurnOutcome::Completed
    );
    assert!(
        f.host
            .turns(&f.connection.caller(), &target)
            .unwrap()
            .is_empty()
    );

    f.send(&target, "inspect the original saved turn");
    let current = f.next(&target).await;
    current.tool("inspect-saved-turn", inspect(&old_source, true));
    let after_tool = f.next(&target).await;
    let actual = after_tool.last_tool_result();
    assert_eq!(actual["turn_id"], json!(old_source.turn.turn_id));
    assert_eq!(
        actual["host_incarnation"],
        json!(public.source.session.host_incarnation)
    );
    assert_eq!(
        actual["data"]["history"],
        serde_json::to_value(public.data.history).unwrap()
    );
    assert!(
        actual["data"]["history"]
            .to_string()
            .contains("actual saved answer — 原文完整保留")
    );
    assert_eq!(
        actual["data"]["saved_terminal"],
        serde_json::to_value(public.data.saved_terminal.unwrap()).unwrap()
    );
    assert_eq!(actual["data"]["partials"], json!([]));
    for field in [
        "info",
        "state",
        "config",
        "load_id",
        "config_revision",
        "runner_revision",
    ] {
        assert!(
            actual["data"].get(field).is_none(),
            "invented saved execution metadata: {actual}"
        );
    }
    assert!(!actual.to_string().contains("original bound instructions"));
    after_tool.complete("inspection complete");
    f.finished(&after_tool.source).await;
    f.stop().await;
    drop(f);
    std::fs::remove_dir_all(path).unwrap();
}

fn input_row(message: &MessageInput, source: MessageSource) -> Value {
    let mut row = json!({
        "kind":"user", "text":message.text, "message_id":message.message_id,
        "mode":message.mode, "source":source,
    });
    if let Some(reply) = &message.in_reply_to {
        row["in_reply_to"] = json!(reply);
    }
    row
}
fn user_rows(items: Value) -> Vec<Value> {
    items
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "user")
        .cloned()
        .collect()
}
async fn history_rows(f: &Fixture, target: &SessionRef) -> Vec<Value> {
    let history = f
        .host
        .history_read(
            &f.connection.caller(),
            target,
            &HistoryReadRequest {
                cursor: None,
                limit: u64::MAX,
            },
        )
        .await
        .unwrap();
    user_rows(serde_json::to_value(history.data.items).unwrap())
}
async fn result_rows(f: &Fixture, source: &TurnSource) -> Vec<Value> {
    let result = f
        .host
        .turn_result_snapshot(
            &f.connection.caller(),
            &source.session,
            &source.turn.turn_id,
        )
        .await
        .unwrap();
    user_rows(serde_json::to_value(result.data.history).unwrap())
}

#[tokio::test]
async fn identical_inputs_keep_identity_mode_and_reply_in_live_held_and_saved_history() {
    let path = std::env::temp_dir().join(format!(
        "minicore-input-history-{}",
        OperationId::new().unwrap()
    ));
    let mut f = Fixture::open(&path).await;
    let target = f.persistent_agent().await;
    let first = MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: "same original body\n原文 — no inferred source".into(),
        in_reply_to: None,
    };
    let second = MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::Steer,
        text: first.text.clone(),
        in_reply_to: Some(first.message_id.clone()),
    };
    let expected = vec![
        input_row(&first, MessageSource::User),
        input_row(&second, MessageSource::User),
    ];
    let mut sources = Vec::new();
    for message in [&first, &second] {
        f.host
            .send(
                &f.connection.caller(),
                SendMessage::new(message.clone(), target.clone()),
            )
            .unwrap();
        let call = f.next(&target).await;
        call.complete("actual completed answer");
        f.finished(&call.source).await;
        sources.push(call.source);
    }
    assert_eq!(history_rows(&f, &target).await, expected);
    // The first result must still use its original slice after another Turn.
    for (index, source) in sources.iter().enumerate() {
        assert_eq!(result_rows(&f, source).await, vec![expected[index].clone()]);
    }
    f.host
        .session_unload(
            &f.connection.caller(),
            &target,
            SessionControlRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
    // Execution reloads the validated append-only native history and provenance.
    f.send(&target, "reload then query earlier inputs");
    let later = f.next(&target).await;
    later.complete("reload complete");
    f.finished(&later.source).await;
    for (index, source) in sources.iter().enumerate() {
        assert_eq!(result_rows(&f, source).await, vec![expected[index].clone()]);
    }
    let live = history_rows(&f, &target).await;
    assert_eq!(&live[..2], &expected);
    f.stop().await;
    drop(f);
    let f = Fixture::open(&path).await;
    assert!(
        f.host
            .turns(&f.connection.caller(), &target)
            .unwrap()
            .is_empty()
    );
    assert_eq!(history_rows(&f, &target).await, live);
    for (index, source) in sources.iter().enumerate() {
        assert_eq!(result_rows(&f, source).await, vec![expected[index].clone()]);
    }
    // History exposes these identities without restoring the old message queue.
    for message in [&first, &second] {
        assert!(
            f.host
                .message(&f.connection.caller(), &target, &message.message_id)
                .is_err()
        );
    }
    f.stop().await;
    drop(f);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn actual_child_input_and_completion_notification_keep_sources_after_restart() {
    let path = std::env::temp_dir().join(format!(
        "minicore-agent-history-{}",
        OperationId::new().unwrap()
    ));
    let mut f = Fixture::open(&path).await;
    let parent = f.persistent_agent().await;
    f.send(&parent, "delegate actual persistent work");
    let start = f.next(&parent).await;
    start.tool(
        "delegate-persistent",
        json!({
            "action":"create", "resource":"session", "target":{
                "host_id":start.source.host_id, "agent_id":parent.agent_id,
            }, "mode":"persistent", "text":"actual child input\n完整任务原文",
        }),
    );
    let a = tokio::time::timeout(Duration::from_secs(5), f.calls.recv())
        .await
        .unwrap()
        .unwrap();
    let b = tokio::time::timeout(Duration::from_secs(5), f.calls.recv())
        .await
        .unwrap()
        .unwrap();
    let (parent_call, child_call) = if a.source.session == parent {
        (a, b)
    } else {
        (b, a)
    };
    assert_eq!(parent_call.source.session, parent);
    let launch = parent_call.last_tool_result();
    assert_eq!(launch["result"]["delivery"], "accepted");
    let child = child_call.source.session.clone();
    assert_eq!(launch["session_id"], json!(child.session_id));
    let child_input = MessageInput {
        message_id: serde_json::from_value(launch["message_id"].clone()).unwrap(),
        mode: MessageMode::FollowUp,
        text: "actual child input\n完整任务原文".into(),
        in_reply_to: None,
    };
    parent_call.complete("delegation launched");
    f.finished(&parent_call.source).await;
    child_call.complete("actual child answer — 完整结果");
    f.finished(&child_call.source).await;
    let notification = f.next(&parent).await;
    let text = notification
        .request
        .messages()
        .iter()
        .rev()
        .find_map(|message| match message {
            ModelMessage::User(text) => Some(text.as_ref()),
            _ => None,
        })
        .unwrap();
    let (context, body) = input_context::decode(text);
    assert_eq!(
        context.source,
        MessageSource::Agent {
            source: child.clone()
        }
    );
    assert_eq!(context.in_reply_to, Some(child_input.message_id.clone()));
    assert!(body.contains("actual child answer — 完整结果"));
    let notification_input = MessageInput {
        message_id: context.message_id,
        mode: MessageMode::FollowUp,
        text: body.to_owned(),
        in_reply_to: context.in_reply_to,
    };
    notification.complete("notification consumed");
    f.finished(&notification.source).await;
    let child_expected = input_row(
        &child_input,
        MessageSource::Agent {
            source: parent.clone(),
        },
    );
    let notification_expected = input_row(
        &notification_input,
        MessageSource::Agent {
            source: child.clone(),
        },
    );
    assert_eq!(history_rows(&f, &child).await, vec![child_expected.clone()]);
    assert_eq!(
        result_rows(&f, &child_call.source).await,
        vec![child_expected.clone()]
    );
    let parent_history = history_rows(&f, &parent).await;
    assert_eq!(parent_history.last(), Some(&notification_expected));
    assert_eq!(
        result_rows(&f, &notification.source).await,
        vec![notification_expected.clone()]
    );
    f.stop().await;
    drop(f);

    let f = Fixture::open(&path).await;
    assert_eq!(history_rows(&f, &child).await, vec![child_expected.clone()]);
    assert_eq!(
        result_rows(&f, &child_call.source).await,
        vec![child_expected]
    );
    assert_eq!(history_rows(&f, &parent).await, parent_history);
    assert_eq!(
        result_rows(&f, &notification.source).await,
        vec![notification_expected]
    );
    assert!(
        f.host
            .turns(&f.connection.caller(), &child)
            .unwrap()
            .is_empty()
    );
    assert!(
        f.host
            .turns(&f.connection.caller(), &parent)
            .unwrap()
            .is_empty()
    );
    f.stop().await;
    drop(f);
    std::fs::remove_dir_all(path).unwrap();
}
