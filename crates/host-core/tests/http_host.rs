//! Real public in-process Host routing through Runtime and production reqwest/SSE
//! adapter to a local scripted HTTP fixture. This is not a live model/TUI test.
#[path = "../../host-provider/tests/support/mod.rs"]
#[allow(dead_code)]
mod support;
use minicore_host_core::{BoundResources, Host, ResourceFuture, ResourceResolver};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopOptions, Text,
    model::{AssistantPart, Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::json;
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
    let descriptor = ModelDescriptor::new(
        "fixture/chat".parse().unwrap(),
        128_000,
        BTreeSet::from([RuntimeReasoning::Auto]),
        true,
    )
    .unwrap();
    let provider = Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(&server.endpoint, "fixture-model", descriptor),
            SecretApiKey::new("fixture-not-a-real-secret").unwrap(),
        )
        .unwrap(),
    );
    let host = Host::memory(Arc::new(ConfiguredResources { model: provider })).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/chat".parse().unwrap();
    config.role.instructions = "Be precise".into();
    let created = host
        .create_agent(
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
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
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
    assert_eq!(messages[1]["content"], "first task");
    assert_eq!(messages[3]["content"], "follow-up task");
}
