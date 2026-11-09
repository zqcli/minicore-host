//! Independent deletion acceptance probes using real owners and gated model fixtures.
#![allow(dead_code)]
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use tokio::sync::mpsc;

struct ModelCall {
    events: mpsc::UnboundedSender<ModelEvent>,
}
struct GatedModel {
    descriptor: ModelDescriptor,
    calls: mpsc::UnboundedSender<ModelCall>,
}
impl Model for GatedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        let (events, rx) = mpsc::unbounded_channel();
        self.calls.send(ModelCall { events }).unwrap();
        Box::pin(async move {
            Ok(
                Box::pin(stream::unfold((rx, false), |(mut rx, done)| async move {
                    if done {
                        None
                    } else {
                        rx.recv().await.map(|event| {
                            let done = matches!(event, ModelEvent::Finish { .. });
                            (Ok(event), (rx, done))
                        })
                    }
                })) as ModelStream,
            )
        })
    }
}
struct Resources {
    model: Arc<GatedModel>,
    bindings: mpsc::UnboundedSender<PreparedTurnBinding>,
}
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
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
        self.bindings.send(binding).unwrap();
        self.resolve(config)
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    host: Arc<Host>,
    connection: ConnectionHandle,
    bindings: mpsc::UnboundedReceiver<PreparedTurnBinding>,
    calls: mpsc::UnboundedReceiver<ModelCall>,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (calls, called) = mpsc::unbounded_channel();
        let (bindings, bound) = mpsc::unbounded_channel();
        let resources = Arc::new(Resources {
            model: Arc::new(GatedModel {
                descriptor: ModelDescriptor::new(
                    "fixture/model".parse().unwrap(),
                    128000,
                    BTreeSet::from([ReasoningPreference::Auto]),
                    true,
                )
                .unwrap(),
                calls,
            }),
            bindings,
        });
        let host = Host::open_persistent(
            directory.path(),
            HostSettings {
                name: "acceptance fences".into(),
            },
            resources,
        )
        .await
        .unwrap();
        let connection = host.connect().unwrap();
        Self {
            directory,
            host,
            connection,
            bindings: bound,
            calls: called,
        }
    }
    async fn agent(&self, mode: StorageMode) -> SessionRef {
        let receipt = self
            .host
            .create_agent_async(
                &self.connection.caller(),
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "fence fixture".into(),
                    mode,
                    config: AgentConfig::default(),
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
    fn send(&self, target: &SessionRef, message: MessageInput) {
        self.host
            .send(
                &self.connection.caller(),
                SendMessage::new(message, target.clone()),
            )
            .unwrap();
    }
    async fn stop(&self) {
        self.host
            .shutdown(
                &self.connection.caller(),
                HostShutdownRequest {
                    operation_id: OperationId::new().unwrap(),
                    strategy: HostShutdownStrategy::Force,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.host.info(&self.connection.caller()).unwrap().lifecycle
                != HostLifecycle::Stopped
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual owners must settle before fixture cleanup");
        assert!(self.directory.path().is_dir());
    }
}
fn input(mode: MessageMode) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode,
        text: "delegated fixture input".into(),
        in_reply_to: None,
    }
}
async fn recv<T>(rx: &mut mpsc::UnboundedReceiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("owner must progress")
        .expect("sender alive")
}
// Test double for external scope hooks. These cases exercise the real core
// admission fence; explicit fixture settlement below releases the real members.
struct TestNoopScopeHooks;
impl ScopeHooks for TestNoopScopeHooks {
    fn seal(&self) {}
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn root_closed(&self) {}
}
struct Sink(mpsc::UnboundedSender<DelegationCompletion>);
impl DelegationSink for Sink {
    fn handoff(&self, completion: DelegationCompletion) -> Result<(), Fault> {
        self.0.send(completion).unwrap();
        Ok(())
    }
}
fn registration(
    cap: &TurnCapability,
    child: &SessionRef,
    first: &MessageInput,
) -> DelegationRegistration {
    let s = cap.source();
    DelegationRegistration {
        host_id: s.host_id.clone(),
        host_incarnation: s.host_incarnation.clone(),
        root: s.session.clone(),
        parent: s.session.clone(),
        child: child.clone(),
        task_message_id: first.message_id.clone(),
        origin: ToolCallRef {
            turn_id: s.turn.turn_id.clone(),
            tool_call_id: ToolCallId("bound-fixture-call".into()),
        },
        temporary: false,
    }
}

// No yield may occur between deletion acceptance and the rejected operation.
// Current-thread Tokio keeps the accepted async delete worker queued in that gap.
#[tokio::test(flavor = "current_thread")]
async fn session_delete_acceptance_expires_bound_capability_synchronously() {
    let mut f = Fixture::new().await;
    let source = f.agent(StorageMode::Persistent).await;
    let target = f.agent(StorageMode::Persistent).await;
    f.send(&source, input(MessageMode::FollowUp));
    let cap = recv(&mut f.bindings).await.capability();
    let _running_model = recv(&mut f.calls).await;
    assert!(cap.check_active().is_ok());
    let receipt = f
        .host
        .session_delete(
            &f.connection.caller(),
            &source,
            SessionDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                close: true,
            },
        )
        .unwrap();
    assert_eq!(receipt.result.lifecycle, SessionCloseLifecycle::Closing);
    let outcome = cap.send(SendMessage::new(
        input(MessageMode::FollowUp),
        target.clone(),
    ));
    assert_eq!(
        outcome.unwrap_err().kind(),
        Some(FaultKind::HostAccessInvalid)
    );
    assert!(
        f.host
            .input_list(&f.connection.caller(), &target)
            .unwrap()
            .next_turn
            .is_empty()
    );
    f.stop().await;
    assert_eq!(
        f.host
            .receipt_get(&f.connection.caller(), &receipt.operation_id)
            .unwrap(),
        Receipt::SessionDelete(receipt)
    );
}

async fn root_delete_seals_descendant_scope(delete_agent: bool) {
    let mut f = Fixture::new().await;
    let root = f.agent(StorageMode::Persistent).await;
    let child = f.agent(StorageMode::Persistent).await;
    let grandchild = f.agent(StorageMode::Persistent).await;
    f.send(&root, input(MessageMode::FollowUp));
    let cap = recv(&mut f.bindings).await.capability();
    let _root_model = recv(&mut f.calls).await;
    cap.prepare_scope(Arc::new(TestNoopScopeHooks)).unwrap();
    let task = input(MessageMode::FollowUp);
    let relation = registration(&cap, &child, &task);
    let (sink, mut child_results) = mpsc::unbounded_channel();
    let child_permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task).unwrap();
    let child_cap = recv(&mut f.bindings).await.capability();
    let child_model = recv(&mut f.calls).await;
    if delete_agent {
        f.host
            .agent_delete(
                &f.connection.caller(),
                &AgentRef {
                    host_id: cap.source().host_id.clone(),
                    agent_id: root.agent_id.clone(),
                },
                AgentDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    scope: AgentDeleteScope::Sessions {
                        session_ids: vec![root.session_id.clone()],
                        close: true,
                    },
                },
            )
            .unwrap();
    } else {
        f.host
            .session_delete(
                &f.connection.caller(),
                &root,
                SessionDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    close: true,
                },
            )
            .unwrap();
    }
    // This descendant is owned by another Agent and is still actually running.
    assert!(child_cap.check_active().is_ok());
    let task = input(MessageMode::FollowUp);
    let mut new_relation = registration(&child_cap, &grandchild, &task);
    new_relation.root = root;
    let (sink, _grandchild_results) = mpsc::unbounded_channel();
    let result = child_cap.prepare_delegation(new_relation.clone(), Arc::new(Sink(sink)));
    if result.is_ok() {
        let accepted = child_cap.send_delegated(&new_relation, task);
        panic!("post-delete descendant registration succeeded; delegated input: {accepted:?}");
    }
    assert_eq!(
        result.err().unwrap().kind(),
        Some(FaultKind::SessionLifecycleClosing)
    );
    assert!(
        f.host
            .input_list(&f.connection.caller(), &grandchild)
            .unwrap()
            .next_turn
            .is_empty()
    );
    // Settle the test hook's real member through its original result owner.
    child_model
        .events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::Stop,
        })
        .unwrap();
    recv(&mut child_results).await;
    child_permit
        .release(DelegationDisposition::ScopeClosed)
        .await
        .unwrap();
    f.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn root_session_delete_acceptance_seals_descendant_scope_synchronously() {
    root_delete_seals_descendant_scope(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn root_agent_delete_acceptance_seals_descendant_scope_synchronously() {
    root_delete_seals_descendant_scope(true).await;
}

#[tokio::test(flavor = "current_thread")]
async fn agent_delete_acceptance_expires_every_owned_turn_capability_synchronously() {
    let mut f = Fixture::new().await;
    let source = f.agent(StorageMode::Persistent).await;
    let target = f.agent(StorageMode::Persistent).await;
    let agent = AgentRef {
        host_id: f.host.info(&f.connection.caller()).unwrap().host_id,
        agent_id: source.agent_id.clone(),
    };
    let sibling = f
        .host
        .create_session_async(
            &f.connection.caller(),
            SessionCreate {
                operation_id: OperationId::new().unwrap(),
                target: agent.clone(),
                mode: Some(StorageMode::Persistent),
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let sibling = SessionRef {
        agent_id: source.agent_id.clone(),
        session_id: sibling.session_id.unwrap(),
    };
    let mut caps = Vec::new();
    let mut running_models = Vec::new();
    for session in [&source, &sibling] {
        f.send(session, input(MessageMode::FollowUp));
        caps.push(recv(&mut f.bindings).await.capability());
        running_models.push(recv(&mut f.calls).await);
    }
    let receipt = f
        .host
        .agent_delete(
            &f.connection.caller(),
            &agent,
            AgentDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                scope: AgentDeleteScope::Sessions {
                    session_ids: vec![source.session_id, sibling.session_id],
                    close: true,
                },
            },
        )
        .unwrap();
    assert_eq!(receipt.result.lifecycle, AgentDeleteLifecycle::Retiring);
    for cap in caps {
        let outcome = cap.send(SendMessage::new(
            input(MessageMode::FollowUp),
            target.clone(),
        ));
        assert_eq!(
            outcome.unwrap_err().kind(),
            Some(FaultKind::HostAccessInvalid)
        );
    }
    assert!(
        f.host
            .input_list(&f.connection.caller(), &target)
            .unwrap()
            .next_turn
            .is_empty()
    );
    f.stop().await;
    assert_eq!(
        f.host
            .receipt_get(&f.connection.caller(), &receipt.operation_id)
            .unwrap(),
        Receipt::AgentDelete(receipt)
    );
}

async fn unbound_memory_deletion_disposes_input(delete_agent: bool) {
    let mut f = Fixture::new().await;
    let parent = f.agent(StorageMode::Persistent).await;
    let child = f.agent(StorageMode::Memory).await;
    f.send(&parent, input(MessageMode::FollowUp));
    let cap = recv(&mut f.bindings).await.capability();
    let _parent_model = recv(&mut f.calls).await;
    cap.prepare_scope(Arc::new(TestNoopScopeHooks)).unwrap();
    let task = input(MessageMode::FollowUp);
    let task_id = task.message_id.clone();
    let relation = registration(&cap, &child, &task);
    let (sink, mut completions) = mpsc::unbounded_channel();
    let permit = cap
        .prepare_delegation(relation.clone(), Arc::new(Sink(sink)))
        .unwrap();
    cap.send_delegated(&relation, task).unwrap();
    // No yield: the child's actual driver is queued before the delete worker,
    // but has not bound the accepted task. It must see normal close disposition.
    if delete_agent {
        f.host
            .agent_delete(
                &f.connection.caller(),
                &AgentRef {
                    host_id: cap.source().host_id.clone(),
                    agent_id: child.agent_id.clone(),
                },
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
        f.host
            .session_delete(
                &f.connection.caller(),
                &child,
                SessionDeleteRequest {
                    operation_id: OperationId::new().unwrap(),
                    close: true,
                },
            )
            .unwrap();
    }
    let completion = tokio::time::timeout(Duration::from_secs(2), completions.recv()).await;
    assert!(
        completion.is_ok(),
        "no discarded-input handoff; retained source: {:?}",
        f.host.session_state(&f.connection.caller(), &child)
    );
    let completion = completion.unwrap().expect("actual handoff");
    assert_eq!(completion.discarded_inputs, vec![task_id]);
    assert!(completion.results.is_empty());
    assert!(completion.inputs.is_empty());
    assert!(
        f.bindings.try_recv().is_err(),
        "unbound task must not start Runtime"
    );
    let notice = permit.notify(input(MessageMode::FollowUp)).await.unwrap();
    permit
        .release(DelegationDisposition::Notified(notice))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.host.session_info(&f.connection.caller(), &child).is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("discarded input must not strand deletion in Pending");
    assert!(completions.try_recv().is_err(), "handoff must occur once");
    f.stop().await;
}

#[tokio::test(flavor = "current_thread")]
async fn session_delete_disposes_unbound_memory_delegation_before_closed() {
    unbound_memory_deletion_disposes_input(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn agent_delete_disposes_unbound_memory_delegation_before_closed() {
    unbound_memory_deletion_disposes_input(true).await;
}
