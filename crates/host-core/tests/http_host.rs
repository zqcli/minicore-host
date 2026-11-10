//! Real public in-process Host routing through Runtime and production reqwest/SSE
//! adapter to a local scripted HTTP fixture. This is not a live model/TUI test.
#[path = "support/input_context.rs"]
mod input_context;
#[path = "../../host-provider/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;
use minicore_host_core::{
    BoundResources, CallerContext, ConnectionHandle, Host, ResourceFuture, ResourceResolver,
    TurnResult,
};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopOptions, Text,
    model::{AssistantPart, Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use support::{Fixture, Reply};
struct ConfiguredResources {
    model: Arc<OpenAiChatModel>,
}
impl ResourceResolver for ConfiguredResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if config.model.as_str() != self.model.descriptor().model_ref.as_str() {
            return Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "model reference is not configured",
            ));
        }
        Ok(())
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.validate(config)?;
            let prompt = Text::new(format!(
                "Host framework rules\n{}",
                config.role.instructions
            ));
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(Some(prompt))),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
    )
}
#[tokio::test]
async fn host_two_turns_reach_real_http_and_keep_role_and_history_exact() {
    let server = Fixture::start(vec![
        Reply::sse(answer("first actual fixture response 世界")),
        Reply::sse(answer("second actual fixture response")),
    ])
    .await;
    let (host, connection, target) = http_host(&server.endpoint, "Be precise").await;
    let caller = connection.caller();
    let mut message_ids = Vec::new();
    for (index, text) in ["first task", "follow-up task"].into_iter().enumerate() {
        let mut states = host.session_states(&caller, &target).unwrap();
        let accepted = host
            .send(
                &caller,
                SendMessage::new(
                    MessageInput {
                        message_id: MessageId::new().unwrap(),
                        mode: MessageMode::FollowUp,
                        text: text.into(),
                        in_reply_to: None,
                    },
                    target.clone(),
                ),
            )
            .unwrap();
        assert_eq!(accepted.result.delivery, Delivery::Accepted);
        message_ids.push(accepted.message_id);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let turns = host.turns(&caller, &target).unwrap();
                if turns.len() == index + 1 && turns.iter().all(|t| t.state == TurnState::Finished)
                {
                    assert_eq!(turns[index].outcome, Some(TurnOutcome::Completed));
                    break;
                }
                states.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    let history = host.history(&caller, &target).unwrap();
    assert_eq!(history.len(), 4);
    assert!(history.iter().any(|item|matches!(item,HistoryItem::Assistant(a) if a.content.iter().any(|part|matches!(part,AssistantPart::Text(text) if text.as_ref()=="first actual fixture response 世界")))));
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .head
            .starts_with("POST /v1/chat/completions HTTP/1.1")
    );
    for request in &requests {
        assert_eq!(request.body["model"], "fixture-model");
        let messages = request.body["messages"].as_array().unwrap();
        assert_eq!(messages.iter().filter(|m| m["role"] == "system").count(), 1);
        assert_eq!(messages[0]["content"], "Host framework rules\nBe precise");
    }
    let messages = requests[1].body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_user(&messages[1], &message_ids[0], "first task");
    assert_user(&messages[3], &message_ids[1], "follow-up task");
}

fn assert_user(message: &Value, id: &MessageId, original: &str) {
    assert_eq!(message["role"], "user");
    let (context, text) = input_context::decode(message["content"].as_str().unwrap());
    assert_eq!(
        context,
        input_context::Context {
            message_id: id.clone(),
            source: MessageSource::User,
            in_reply_to: None,
        }
    );
    assert_eq!(
        text.as_bytes(),
        original.as_bytes(),
        "full original input bytes are retained"
    );
}

async fn http_host(endpoint: &str, role: &str) -> (Arc<Host>, ConnectionHandle, SessionRef) {
    let descriptor = ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([RuntimeReasoning::Auto]),
        true,
    )
    .unwrap();
    let provider = Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(endpoint, "fixture-model", descriptor),
            SecretApiKey::new("fixture-not-a-real-secret").unwrap(),
        )
        .unwrap(),
    );
    let host = Host::memory(Arc::new(ConfiguredResources { model: provider })).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/chat".parse().unwrap();
    config.role.instructions = role.into();
    // This fixture verifies verbatim large history, not utility summarization.
    config.session_defaults.execution.compaction.enabled = false;
    let created = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "HTTP fixture".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    (host, connection, target)
}

fn message(text: &str) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: text.to_owned(),
        in_reply_to: None,
    }
}

async fn send_turn(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    input: &MessageInput,
    index: usize,
) -> TurnResult {
    let mut states = host.session_states(caller, target).unwrap();
    let receipt = host
        .send(caller, SendMessage::new(input.clone(), target.clone()))
        .unwrap();
    assert_eq!(receipt.result.delivery, Delivery::Accepted);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(turn) = host.turns(caller, target).unwrap().get(index)
                && turn.state == TurnState::Finished
            {
                return host.turn_result(caller, target, &turn.id).unwrap();
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}

fn payload_reply(body: String) -> Reply {
    Reply::Full {
        status: 200,
        content_type: "text/event-stream",
        headers: String::new(),
        body,
        fragment: 4093,
    }
}

#[tokio::test]
async fn host_preserves_large_input_role_response_and_next_turn_history() {
    let role = "Role 世界\n".repeat(30_000);
    let input = "请求 🐈\n".repeat(30_000);
    let output = "回复 世界🐈\n".repeat(30_000);
    assert!(role.len() > 262_144 && input.len() > 262_144 && output.len() > 262_144);
    let server = Fixture::start(vec![
        payload_reply(answer(&output)),
        Reply::sse(answer("follow-up complete")),
    ])
    .await;
    let (host, connection, target) = http_host(&server.endpoint, &role).await;
    let caller = connection.caller();
    let empty = host
        .send(&caller, SendMessage::new(message(""), target.clone()))
        .unwrap_err();
    assert_eq!(empty.kind(), Some(FaultKind::SessionInputInvalid));
    assert_eq!(server.request_count(), 0);
    let input_message = message(&input);
    let first = send_turn(&host, &caller, &target, &input_message, 0).await;
    assert_eq!(first.info.outcome, Some(TurnOutcome::Completed));
    assert!(first.partials.is_empty());
    assert_eq!(first.history.len(), 2);
    assert!(matches!(&first.history[0], HistoryItem::User(user) if user.input.as_text() == input));
    assert!(
        matches!(&first.history[1], HistoryItem::Assistant(assistant) if matches!(&assistant.content[0], AssistantPart::Text(text) if text.as_ref() == output))
    );
    let follow_up = message("follow-up");
    let second = send_turn(&host, &caller, &target, &follow_up, 1).await;
    assert_eq!(second.info.outcome, Some(TurnOutcome::Completed));
    assert_eq!(host.history(&caller, &target).unwrap().len(), 4);
    let requests = server.finish().await;
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(
            request.body["messages"][0]["content"],
            format!("Host framework rules\n{role}")
        );
        assert_user(
            &request.body["messages"][1],
            &input_message.message_id,
            &input,
        );
    }
    assert_eq!(requests[1].body["messages"][2]["content"], output);
    assert_user(
        &requests[1].body["messages"][3],
        &follow_up.message_id,
        "follow-up",
    );
}

#[tokio::test]
async fn host_keeps_large_http_partial_without_fabricating_history_or_completion() {
    let partial = "unfinished 世界🐈\n".repeat(25_000);
    assert!(partial.len() > 262_144);
    let body = format!(
        "data: {}\n\n",
        json!({"choices":[{"index":0,"delta":{"content":partial},"finish_reason":null}]})
    );
    let server = Fixture::start(vec![payload_reply(body)]).await;
    let (host, connection, target) = http_host(&server.endpoint, "Keep partials exact").await;
    let caller = connection.caller();
    let result = send_turn(&host, &caller, &target, &message("partial response"), 0).await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
    assert!(result.info.fault.is_some());
    assert_eq!(result.info.requests, 1);
    assert_eq!(result.history.len(), 1);
    assert!(matches!(&result.history[0], HistoryItem::User(_)));
    assert_eq!(result.partials.len(), 1);
    assert_eq!(result.partials[0].response_index, 0);
    assert!(
        matches!(&result.partials[0].partial.content[0], AssistantPart::Text(text) if text.as_ref() == partial)
    );
    assert_eq!(host.history(&caller, &target).unwrap().len(), 1);
    assert_eq!(server.finish().await.len(), 1);
}
