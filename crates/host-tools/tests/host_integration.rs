//! Production Host owners and Runtime. Only the model is a scripted fixture.
#[path = "../../host-core/tests/support/input_context.rs"]
mod input_context;
use futures_util::{StreamExt, stream};
use minicore_host_core::{
    BoundResources, Host, PreparedTurnBinding, ResourceFuture, ResourceResolver,
};
use minicore_host_protocol::*;
use minicore_host_tools::{AgentToolFactory, CoreTurnPort, ScopeManager, ToolRegistry};
use minicore_runtime::{
    ExecutionConfig, HistoryItem, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelMessage,
        ModelRequest, ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

struct FixtureModel {
    descriptor: ModelDescriptor,
    agent: Mutex<Option<AgentRef>>,
    round: AtomicUsize,
    six_actions: bool,
    close_scope: bool,
    create_agent: bool,
    child_mode: StorageMode,
    child: Mutex<Option<SessionRef>>,
    child_gate: Arc<Semaphore>,
    calls: mpsc::UnboundedSender<(usize, ModelRequest)>,
}
impl Model for FixtureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let is_child = request.messages().iter().any(
            |message| matches!(message,ModelMessage::User(text) if input_context::decode(text).1=="child work"),
        );
        if is_child {
            let gate = Arc::clone(&self.child_gate);
            self.calls.send((usize::MAX, request)).unwrap();
            return Box::pin(async move {
                let events = stream::once(async move {
                    gate.acquire().await.unwrap().forget();
                    stream::iter([
                        Ok(ModelEvent::text_delta("actual child result")),
                        Ok(ModelEvent::Finish {
                            reason: ModelFinishReason::Stop,
                        }),
                    ])
                })
                .flatten();
                Ok(Box::pin(events) as ModelStream)
            });
        }
        let round = self.round.fetch_add(1, Ordering::AcqRel);
        let prior = request
            .messages()
            .iter()
            .rev()
            .find_map(|message| match message {
                ModelMessage::Tool { output, .. } => {
                    serde_json::from_str::<Value>(output.content().as_str()).ok()
                }
                _ => None,
            });
        self.calls.send((round, request)).unwrap();
        let events = if self.close_scope && round == 1 {
            let call = "close-root".parse().unwrap();
            vec![
                ModelEvent::ToolCallStart {
                    tool_call_id: call,
                    tool_name: "agent".parse().unwrap(),
                },
                ModelEvent::tool_call_arguments_delta(
                    "close-root".parse().unwrap(),
                    json!({"action":"control","operation":"close_scope"}).to_string(),
                ),
                ModelEvent::ToolCallEnd {
                    tool_call_id: "close-root".parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ]
        } else if self.six_actions && round < 6 {
            if round == 1 {
                let prior = prior.unwrap();
                *self.child.lock().unwrap() = Some(SessionRef {
                    agent_id: serde_json::from_value(prior["agent_id"].clone()).unwrap(),
                    session_id: serde_json::from_value(prior["session_id"].clone()).unwrap(),
                });
            }
            let child = self.child.lock().unwrap().clone();
            let arguments = match round {
                0 => {
                    json!({"action":"create","resource":"session","target":self.agent.lock().unwrap().clone().unwrap(),"mode":"memory"})
                }
                1 => {
                    let child = child.unwrap();
                    json!({"action":"send","target":{"kind":"session","agent_id":child.agent_id,"session_id":child.session_id},"text":"child work","delegate":true})
                }
                2 => json!({"action":"wait"}),
                3 => {
                    let child = child.unwrap();
                    json!({"action":"inspect","target":{"kind":"session","agent_id":child.agent_id,"session_id":child.session_id},"include_config":true,"include_output":true,"history_page":{"cursor":null,"limit":50}})
                }
                4 => {
                    json!({"action":"update","resource":"session","target":child.unwrap(),"changes":{"title":"retained child"}})
                }
                5 => {
                    json!({"action":"control","operation":"close_session","target":child.unwrap()})
                }
                _ => unreachable!(),
            };
            let call = format!("action-{round}").parse().unwrap();
            vec![
                ModelEvent::ToolCallStart {
                    tool_call_id: call,
                    tool_name: "agent".parse().unwrap(),
                },
                ModelEvent::tool_call_arguments_delta(
                    format!("action-{round}").parse().unwrap(),
                    arguments.to_string(),
                ),
                ModelEvent::ToolCallEnd {
                    tool_call_id: format!("action-{round}").parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ]
        } else if !self.six_actions && round == 0 {
            let target = self.agent.lock().unwrap().clone().unwrap();
            let call = "upstream-delegate".parse().unwrap();
            let arguments = if self.create_agent {
                json!({"action":"create","resource":"agent","target":{"host_id":target.host_id},"name":"delegated reviewer","from_agent_id":target.agent_id,"mode":self.child_mode,"text":"child work"})
            } else {
                json!({"action":"create","resource":"session","target":target,"mode":self.child_mode,"text":"child work"})
            };
            vec![
                ModelEvent::ToolCallStart {
                    tool_call_id: call,
                    tool_name: "agent".parse().unwrap(),
                },
                ModelEvent::tool_call_arguments_delta(
                    "upstream-delegate".parse().unwrap(),
                    arguments.to_string(),
                ),
                ModelEvent::ToolCallEnd {
                    tool_call_id: "upstream-delegate".parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ]
        } else {
            vec![
                ModelEvent::text_delta(if round == 1 {
                    "launch accepted"
                } else {
                    "received child result"
                }),
                ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                },
            ]
        };
        Box::pin(
            async move { Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as ModelStream) },
        )
    }
}
struct Resources {
    model: Arc<FixtureModel>,
    tools: ToolRegistry,
}
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            let mut bound = self.resolve(config).await?;
            let selected = self
                .tools
                .bind(&config.tools, CoreTurnPort::prepared(binding))?;
            bound.execution = ExecutionConfig::new(
                self.model.clone(),
                RuntimeReasoning::Auto,
                selected,
                None,
                Arc::new(DefaultPromptProvider::new(None)),
            )
            .unwrap();
            Ok(bound)
        })
    }
}
async fn settled(
    host: &Host,
    caller: &minicore_host_core::CallerContext,
    target: &SessionRef,
    count: usize,
) -> Vec<minicore_host_core::TurnInfo> {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let turns = host.turns(caller, target).unwrap();
            if turns.len() == count && turns.iter().all(|turn| turn.state == TurnState::Finished) {
                return turns;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn real_host_delegation_survives_parent_turn_and_client_disconnect() {
    run_host_delegation(false, false, false).await;
}
#[tokio::test]
async fn real_agent_delegation_copies_config_and_reclaims_temporary_owners() {
    run_host_delegation(true, false, false).await;
}
async fn run_host_delegation(create_agent: bool, delete_persistent: bool, delete_root: bool) {
    let (calls, mut requests) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let model = Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        agent: Mutex::new(None),
        round: AtomicUsize::new(0),
        six_actions: false,
        close_scope: false,
        child_mode: if delete_persistent {
            StorageMode::Persistent
        } else {
            StorageMode::Memory
        },
        create_agent,
        child: Mutex::new(None),
        child_gate: Arc::clone(&gate),
        calls,
    });
    let mut tools = ToolRegistry::new();
    tools
        .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
        .unwrap();
    let resources = Arc::new(Resources {
        model: Arc::clone(&model),
        tools,
    });
    let directory =
        std::env::temp_dir().join(format!("task051-delegation-{}", SessionId::new().unwrap()));
    let host = if delete_persistent {
        std::fs::create_dir(&directory).unwrap();
        Host::open_persistent(
            &directory,
            minicore_host_core::HostSettings {
                name: "explicit deletion fixture".into(),
            },
            resources,
        )
        .await
        .unwrap()
    } else {
        Host::memory(resources).unwrap()
    };
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.role.name = "Source role".into();
    config.role.instructions = "Preserved source instructions, not the task text".into();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    config.session_defaults.tools = vec![ToolBinding {
        name: "agent".into(),
    }];
    let source_config = config.clone();
    let created = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "root".into(),
                mode: if delete_persistent {
                    StorageMode::Persistent
                } else {
                    StorageMode::Memory
                },
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let root = SessionRef {
        agent_id: created.agent_id.clone().unwrap(),
        session_id: created.session_id.unwrap(),
    };
    *model.agent.lock().unwrap() = Some(AgentRef {
        host_id: created.host_id,
        agent_id: root.agent_id.clone(),
    });
    host.send(
        &connection.caller(),
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "root work".into(),
                in_reply_to: None,
            },
            root.clone(),
        ),
    )
    .unwrap();
    let child_request = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (round, request) = requests.recv().await.unwrap();
            if round == usize::MAX {
                break request;
            }
        }
    })
    .await
    .unwrap();
    let turns = settled(&host, &connection.caller(), &root, 1).await;
    let parent = host
        .turn_result(&connection.caller(), &root, &turns[0].id)
        .unwrap();
    let tool = parent
        .history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(result) => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool.call_id.as_str(), "upstream-delegate");
    let launch: Value = serde_json::from_str(tool.output.content().as_str()).unwrap();
    assert_eq!(launch["result"]["delivery"], "accepted");
    let (context, body) = child_request
        .messages()
        .iter()
        .find_map(|message| {
            let ModelMessage::User(text) = message else {
                return None;
            };
            Some(input_context::decode(text))
        })
        .unwrap();
    assert_eq!(body, "child work");
    assert_eq!(
        context.source,
        MessageSource::Agent {
            source: root.clone()
        }
    );
    assert_eq!(
        serde_json::to_value(context.message_id).unwrap(),
        launch["message_id"]
    );
    assert!(context.in_reply_to.is_none());
    let child = SessionRef {
        agent_id: serde_json::from_value(launch["agent_id"].clone()).unwrap(),
        session_id: serde_json::from_value(launch["session_id"].clone()).unwrap(),
    };
    assert_ne!(root, child);
    let child_agent = AgentRef {
        host_id: host.info(&connection.caller()).unwrap().host_id,
        agent_id: child.agent_id.clone(),
    };
    if create_agent {
        assert_ne!(root.agent_id, child.agent_id);
        let info = host.agent_info(&connection.caller(), &child_agent).unwrap();
        assert_eq!(info.name, "delegated reviewer");
        assert_eq!(info.config, source_config);
        assert_eq!(info.default_session_id, Some(child.session_id.clone()));
    }
    assert!(
        host.session_state(&connection.caller(), &child)
            .unwrap()
            .data
            .active_turn_id
            .is_some()
    );
    if delete_root && !delete_persistent {
        let agent = model.agent.lock().unwrap().clone().unwrap();
        let receipt = host
            .agent_delete(
                &connection.caller(),
                &agent,
                AgentDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    scope: AgentDeleteScope::Sessions {
                        session_ids: vec![root.session_id.clone(), child.session_id.clone()],
                        close: true,
                    },
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.agent_info(&connection.caller(), &agent).is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            host.receipt_get(&connection.caller(), &receipt.operation_id)
                .unwrap(),
            Receipt::AgentDelete(receipt)
        );
        return;
    }
    if delete_root {
        let receipt = host
            .session_delete(
                &connection.caller(),
                &root,
                SessionDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    close: true,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.session_info(&connection.caller(), &root).is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let child_state = host.session_state(&connection.caller(), &child).unwrap();
        assert_eq!(child_state.data.lifecycle, SessionLifecycle::Closed);
        assert!(child_state.data.deletion.is_none());
        let child_path = directory
            .join("agents")
            .join(child.agent_id.as_str())
            .join("sessions")
            .join(child.session_id.as_str());
        assert!(child_path.join("session.json").exists());
        assert!(child_path.join("history.jsonl").exists());
        assert_eq!(
            host.receipt_get(&connection.caller(), &receipt.operation_id)
                .unwrap(),
            Receipt::SessionDelete(receipt)
        );
        host.shutdown(
            &connection.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Force,
            },
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.info(&connection.caller()).unwrap().lifecycle != HostLifecycle::Stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        return;
    }
    connection.close();
    let replacement = host.connect().unwrap();
    gate.add_permits(1);
    let turns = settled(&host, &replacement.caller(), &root, 2).await;
    let continued = host
        .turn_result(&replacement.caller(), &root, &turns[1].id)
        .unwrap();
    let notification = continued
        .history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::User(input) => serde_json::from_str::<Value>(input.input.as_text()).ok(),
            _ => None,
        })
        .find(|value| value.get("task_message_id").is_some())
        .unwrap();
    assert_eq!(
        notification["source"],
        serde_json::to_value(&child).unwrap()
    );
    assert_eq!(notification["task_message_id"], launch["message_id"]);
    assert_eq!(notification["origin"]["tool_call_id"], "upstream-delegate");
    assert_eq!(notification["results"][0]["outcome"], "completed");
    assert_eq!(
        notification["results"][0]["output"][0],
        "actual child result"
    );
    let original = host
        .turn_result(&replacement.caller(), &root, &turns[0].id)
        .unwrap();
    let original_tool = original
        .history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(result) => Some(result),
            _ => None,
        })
        .unwrap();
    assert_eq!(original_tool.output, tool.output);
    if delete_persistent {
        // The persistent child retains its own context after completion. Only
        // this explicit deletion closes/removes it, after the parent received
        // both the actual launch ToolResult and completion notification.
        if create_agent {
            host.agent_delete(
                &replacement.caller(),
                &child_agent,
                AgentDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    scope: AgentDeleteScope::Sessions {
                        session_ids: vec![child.session_id.clone()],
                        close: true,
                    },
                },
            )
            .unwrap();
        } else {
            host.session_delete(
                &replacement.caller(),
                &child,
                SessionDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    close: true,
                },
            )
            .unwrap();
        }
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if host.session_info(&replacement.caller(), &child).is_err()
                && (!create_agent
                    || host
                        .agent_info(&replacement.caller(), &child_agent)
                        .is_err())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    if delete_persistent {
        let retained = host.history(&replacement.caller(), &root).unwrap();
        assert!(retained.iter().any(
            |item| matches!(item,HistoryItem::ToolResult(result) if result.output == tool.output)
        ));
        assert!(retained.iter().any(|item| matches!(item,HistoryItem::User(input) if serde_json::from_str::<Value>(input.input.as_text()).ok().as_ref() == Some(&notification))));
        let child_path = directory
            .join("agents")
            .join(child.agent_id.as_str())
            .join("sessions")
            .join(child.session_id.as_str());
        assert!(!child_path.exists());
        host.shutdown(
            &replacement.caller(),
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Force,
            },
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.info(&replacement.caller()).unwrap().lifecycle != HostLifecycle::Stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // This path was generated and created by this test only.
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[tokio::test]
async fn all_six_actions_execute_through_real_owners_and_one_runtime_turn() {
    let (calls, mut requests) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let model = Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        agent: Mutex::new(None),
        round: AtomicUsize::new(0),
        six_actions: true,
        close_scope: false,
        create_agent: false,
        child_mode: StorageMode::Memory,
        child: Mutex::new(None),
        child_gate: Arc::clone(&gate),
        calls,
    });
    let mut tools = ToolRegistry::new();
    tools
        .register(Arc::new(AgentToolFactory::new(ScopeManager::new())))
        .unwrap();
    let host = Host::memory(Arc::new(Resources {
        model: Arc::clone(&model),
        tools,
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    config.session_defaults.tools = vec![ToolBinding {
        name: "agent".into(),
    }];
    let created = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "root".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let root = SessionRef {
        agent_id: created.agent_id.clone().unwrap(),
        session_id: created.session_id.unwrap(),
    };
    *model.agent.lock().unwrap() = Some(AgentRef {
        host_id: created.host_id,
        agent_id: root.agent_id.clone(),
    });
    host.send(
        &connection.caller(),
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "root work".into(),
                in_reply_to: None,
            },
            root.clone(),
        ),
    )
    .unwrap();
    // Request 2 emits wait. Its preceding input-take boundary has already passed,
    // so child completion is now guaranteed to remain consumable for that tool.
    tokio::time::timeout(Duration::from_secs(5), async {
        while requests.recv().await.unwrap().0 != 2 {}
    })
    .await
    .unwrap();
    gate.add_permits(1);
    let turns = settled(&host, &connection.caller(), &root, 1).await;
    let result = host
        .turn_result(&connection.caller(), &root, &turns[0].id)
        .unwrap();
    let outputs = result
        .history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(result) => Some((
                result.call_id.as_str(),
                serde_json::from_str::<Value>(result.output.content().as_str()).unwrap(),
            )),
            _ => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(outputs.len(), 6);
    assert_eq!(outputs["action-0"]["kind"], "session.create");
    assert_eq!(outputs["action-1"]["kind"], "message.send");
    assert_eq!(outputs["action-2"]["status"], "input_available");
    assert!(
        outputs["action-3"]["data"]["history"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.to_string().contains("actual child result"))
    );
    assert_eq!(outputs["action-4"]["title"], "retained child");
    assert_eq!(outputs["action-5"]["kind"], "session.close");
    let child = model.child.lock().unwrap().clone().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if host.session_info(&connection.caller(), &child).is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        host.session_info(&connection.caller(), &root)
            .unwrap()
            .title
            .is_none()
    );
}

#[tokio::test]
async fn actual_factory_binding_failure_keeps_input_queued_without_a_turn() {
    let (calls, mut requests) = mpsc::unbounded_channel();
    let model = Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        agent: Mutex::new(None),
        round: AtomicUsize::new(0),
        six_actions: false,
        close_scope: false,
        create_agent: false,
        child_mode: StorageMode::Memory,
        child: Mutex::new(None),
        child_gate: Arc::new(Semaphore::new(0)),
        calls,
    });
    let host = Host::memory(Arc::new(Resources {
        model,
        tools: ToolRegistry::new(),
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    config.session_defaults.tools = vec![ToolBinding {
        name: "missing".into(),
    }];
    let created = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "binding failure".into(),
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
    let input = MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: "must remain queued".into(),
        in_reply_to: None,
    };
    host.send(
        &connection.caller(),
        SendMessage::new(input.clone(), target.clone()),
    )
    .unwrap();
    let mut states = host.session_states(&connection.caller(), &target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if states.snapshot().unwrap().data.block_reason.is_some() {
                break;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        host.turns(&connection.caller(), &target)
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        host.message(&connection.caller(), &target, &input.message_id)
            .unwrap()
            .binding,
        minicore_host_core::MessageBinding::Queued
    ));
    assert!(matches!(
        requests.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    let state = host.session_state(&connection.caller(), &target).unwrap();
    assert_eq!(state.data.next_turn.len(), 1);
    assert!(state.data.active_turn_id.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_close_scope_does_not_join_its_own_runtime_tool() {
    let (calls, mut requests) = mpsc::unbounded_channel();
    let model = Arc::new(FixtureModel {
        descriptor: ModelDescriptor::new(
            "fixture/model".parse().unwrap(),
            128000,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        agent: Mutex::new(None),
        round: AtomicUsize::new(0),
        six_actions: false,
        close_scope: true,
        create_agent: false,
        child_mode: StorageMode::Memory,
        child: Mutex::new(None),
        child_gate: Arc::new(Semaphore::new(0)),
        calls,
    });
    let scopes = ScopeManager::new();
    let mut tools = ToolRegistry::new();
    tools
        .register(Arc::new(AgentToolFactory::new(Arc::clone(&scopes))))
        .unwrap();
    let host = Host::memory(Arc::new(Resources {
        model: Arc::clone(&model),
        tools,
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    config.session_defaults.tools = vec![ToolBinding {
        name: "agent".into(),
    }];
    let created = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "root".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let root = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    *model.agent.lock().unwrap() = Some(AgentRef {
        host_id: created.host_id,
        agent_id: root.agent_id.clone(),
    });
    host.send(
        &connection.caller(),
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "root work".into(),
                in_reply_to: None,
            },
            root.clone(),
        ),
    )
    .unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(5), async {
        while requests.recv().await.unwrap().0 != 1 {}
        loop {
            if host.agents(&connection.caller()).unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    if let Err(error) = finished {
        panic!(
            "close scope timed out: {error}; agents={:?}; root={:?}; sessions={:?}",
            host.agents(&connection.caller()),
            host.session_state(&connection.caller(), &root),
            host.sessions(
                &connection.caller(),
                model.agent.lock().unwrap().as_ref().unwrap()
            )
            .map(|sessions| sessions
                .into_iter()
                .map(|info| host.session_state(
                    &connection.caller(),
                    &SessionRef {
                        agent_id: info.agent_id,
                        session_id: info.id
                    }
                ))
                .collect::<Vec<_>>())
        );
    }
    assert!(host.session_info(&connection.caller(), &root).is_err());
    assert_eq!(model.child_gate.available_permits(), 0);
    // The child never needed a model finish to permit actual close/cancel/join.
    // The root may reach a request boundary after the control acceptance and
    // before its independent close owner cancels the still-current Turn.
}

#[tokio::test]
async fn task051_session_deletion_preserves_received_launch_and_completion_history() {
    run_host_delegation(false, true, false).await;
}
#[tokio::test]
async fn task051_agent_deletion_preserves_received_launch_and_completion_history() {
    run_host_delegation(true, true, false).await;
}

#[tokio::test]
async fn task051_root_delete_closes_real_scope_without_deleting_independent_child_data() {
    run_host_delegation(true, true, true).await;
}

#[tokio::test]
async fn task051_agent_cascade_and_root_scope_share_real_memory_child_cleanup() {
    run_host_delegation(false, false, true).await;
}
