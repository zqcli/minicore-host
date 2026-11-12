//! The real CoreTurnPort on an actual bound Runtime Turn. Only the model is a
//! gated test double; inspection uses the production direct owner methods.
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use minicore_host_tools::{CoreTurnPort, InspectAction, TurnPort};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc};

struct GatedModel {
    descriptor: ModelDescriptor,
    entered: Semaphore,
    release: Semaphore,
}
impl Model for GatedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        Box::pin(async {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Ok(Box::pin(futures_util::stream::iter([
                Ok(ModelEvent::text_delta("done")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
struct CaptureBinding {
    model: Arc<GatedModel>,
    ports: mpsc::UnboundedSender<Arc<CoreTurnPort>>,
}
impl ResourceResolver for CaptureBinding {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        self.ports
            .send(CoreTurnPort::prepared(binding))
            .ok()
            .unwrap();
        self.resolve(config)
    }
}
#[tokio::test]
async fn agent_inspect_reuses_owner_summaries_and_only_includes_requested_agent_config() {
    let model = Arc::new(GatedModel {
        descriptor: ModelDescriptor::new(
            "test/model".parse().unwrap(),
            4096,
            BTreeSet::from([RuntimeReasoning::Auto]),
            true,
        )
        .unwrap(),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let (ports, mut received) = mpsc::unbounded_channel();
    let host = Host::memory(Arc::new(CaptureBinding {
        model: model.clone(),
        ports,
    }))
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut config = AgentConfig::default();
    config.role.instructions = "large private role body ".repeat(1024);
    let receipt = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "inspected".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let agent = AgentRef {
        host_id: receipt.host_id,
        agent_id: receipt.agent_id.unwrap(),
    };
    let session = SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: receipt.session_id.unwrap(),
    };
    host.send(
        &caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                text: "run model".into(),
                mode: MessageMode::FollowUp,
                in_reply_to: None,
            },
            session.clone(),
        ),
    )
    .unwrap();
    let port = received.recv().await.unwrap();
    model.entered.acquire().await.unwrap().forget();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !host
            .session_state(&caller, &session)
            .unwrap()
            .data
            .owned_work
            .model
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let expected =
        serde_json::to_value(host.agent_details(&caller, &agent).unwrap().sessions).unwrap();
    for include_config in [false, true] {
        let request: InspectAction = serde_json::from_value(serde_json::json!({
            "target":{"kind":"agent","host_id":agent.host_id,"agent_id":agent.agent_id},
            "include_config":include_config
        }))
        .unwrap();
        let result = port.inspect(request).await.unwrap();
        assert_eq!(result["data"]["sessions"], expected);
        assert_eq!(result["data"].get("config").is_some(), include_config);
        assert!(result["data"]["sessions"][0].get("config").is_none());
        assert_eq!(result["data"]["sessions"][0]["owned_work"]["model"], true);
    }
    model.release.add_permits(1);
    let turn = host
        .session_state(&caller, &session)
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    host.wait_turn(&caller, &session, &turn).await.unwrap();
}
