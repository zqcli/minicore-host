#![allow(dead_code)]
//! Explicit owner-port fixtures. These are NOT a real Host integration.
use minicore_host_protocol::*;
use minicore_host_tools::*;
use minicore_runtime::tools::{Tool, ToolContext, ToolExecutionOutcome, ToolInvocation};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

pub fn id() -> String {
    minicore_host_protocol::new_id().unwrap()
}
pub fn session() -> SessionRef {
    SessionRef {
        agent_id: AgentId::new().unwrap(),
        session_id: SessionId::new().unwrap(),
    }
}
pub fn input() -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: "task".into(),
        in_reply_to: None,
    }
}
pub fn origin(port: &FixtureTurn) -> ToolCallRef {
    ToolCallRef {
        turn_id: port.source.turn.turn_id.clone(),
        tool_call_id: ToolCallId("fixture-call".into()),
    }
}
pub fn unsupported() -> Fault {
    Fault::new(
        FaultKind::HostMethodUnavailable,
        "not implemented by this explicit fixture",
    )
}
#[derive(Default)]
pub struct FixtureState {
    pub events: Mutex<Vec<String>>,
    pub relations: Mutex<HashMap<SessionRef, Arc<FixtureDelegation>>>,
    pub hooks: Mutex<HashMap<SessionRef, Arc<dyn ScopeHooks>>>,
    pub notifications: Mutex<Vec<(SessionRef, MessageInput)>>,
    pub releases: Mutex<Vec<DelegationDisposition>>,
    pub changed: Notify,
    pub ready: AtomicBool,
    pub paused: AtomicBool,
    pub draining: AtomicBool,
    pub root_close_gate: Mutex<Option<Arc<Semaphore>>>,
    pub creation_gate: Mutex<Option<Arc<Semaphore>>>,
    pub finish_inline: AtomicBool,
    pub prepare_fails: AtomicBool,
    pub notify_fails: AtomicBool,
    pub close_fails: AtomicBool,
    pub snapshot_reads: AtomicUsize,
    pub writes: Mutex<Vec<AgentUpdateCandidate>>,
    pub update_conflict: AtomicBool,
}
pub struct FixtureTurn {
    pub source: ToolSource,
    pub active: AtomicBool,
    pub state: Arc<FixtureState>,
}
impl FixtureTurn {
    pub fn new() -> Arc<Self> {
        let session = session();
        Arc::new(Self {
            source: ToolSource {
                host_id: HostId::new().unwrap(),
                host_incarnation: HostIncarnation::new().unwrap(),
                turn: TurnRef {
                    session_id: session.session_id.clone(),
                    turn_id: TurnId::new().unwrap(),
                },
                session,
            },
            active: AtomicBool::new(true),
            state: Arc::new(FixtureState::default()),
        })
    }
    pub fn for_session(&self, session: SessionRef) -> Arc<Self> {
        Arc::new(Self {
            source: ToolSource {
                session: session.clone(),
                turn: TurnRef {
                    session_id: session.session_id,
                    turn_id: TurnId::new().unwrap(),
                },
                ..self.source.clone()
            },
            active: AtomicBool::new(true),
            state: Arc::clone(&self.state),
        })
    }
    pub fn port(self: &Arc<Self>) -> Arc<dyn TurnPort> {
        self.clone()
    }
    pub fn event(&self, event: &str) {
        self.state.events.lock().unwrap().push(event.into());
    }
    pub fn accepted(&self, target: SessionRef, input: MessageInput) -> MessageAccepted {
        MessageAccepted::new(
            self.source.host_id.clone(),
            self.source.host_incarnation.clone(),
            target,
            input.message_id,
            MessageAcceptedFacts::new(input.mode.requested_queue(), 1),
        )
    }
}
impl TurnPort for FixtureTurn {
    fn source(&self) -> &ToolSource {
        &self.source
    }
    fn check_active(&self) -> Result<(), Fault> {
        if self.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "fixture Turn no longer active",
            ))
        }
    }
    fn agent_snapshot(&self, _: &AgentRef) -> Result<AgentConfigSnapshot, Fault> {
        self.check_active()?;
        self.state.snapshot_reads.fetch_add(1, Ordering::Relaxed);
        Ok(AgentConfigSnapshot {
            name: "fixture name".into(),
            config: AgentConfig::default(),
            config_revision: 7,
        })
    }
    fn session_snapshot(&self, _: &SessionRef) -> Result<SessionConfigSnapshot, Fault> {
        self.check_active()?;
        Ok(SessionConfigSnapshot {
            title: Some("fixture title".into()),
            config: SessionConfig::default(),
            config_revision: 9,
            mode: StorageMode::Memory,
        })
    }
    fn create_agent(&self, request: AgentCreate) -> PortFuture<'_, AgentCreateReceipt> {
        Box::pin(async move {
            self.check_active()?;
            assert!(request.initial_message.is_none());
            self.event("create_agent");
            let child = session();
            Ok(AgentCreateReceipt {
                kind: AgentCreateKind::AgentCreate,
                host_id: self.source.host_id.clone(),
                host_incarnation: self.source.host_incarnation.clone(),
                operation_id: request.operation_id,
                agent_id: Some(child.agent_id),
                session_id: Some(child.session_id),
                message_id: None,
                result: AgentCreateResult::without_delivery(AgentCreateSteps {
                    agent: CreateStep::Succeeded,
                    session: CreateStep::Succeeded,
                    message: None,
                }),
            })
        })
    }
    fn create_session(&self, request: SessionCreate) -> PortFuture<'_, SessionCreateReceipt> {
        Box::pin(async move {
            self.check_active()?;
            assert!(request.initial_message.is_none());
            self.event("create_session");
            Ok(SessionCreateReceipt {
                kind: SessionCreateKind::SessionCreate,
                host_id: self.source.host_id.clone(),
                host_incarnation: self.source.host_incarnation.clone(),
                operation_id: request.operation_id,
                agent_id: request.target.agent_id,
                session_id: Some(SessionId::new().unwrap()),
                message_id: None,
                result: SessionCreateResult::without_delivery(SessionCreateSteps {
                    session: CreateStep::Succeeded,
                    message: None,
                }),
            })
        })
    }
    fn create_agent_for_delegation(
        &self,
        request: AgentCreate,
    ) -> PortFuture<'_, CreatedForDelegation<AgentCreateReceipt>> {
        Box::pin(async move {
            let receipt = self.create_agent(request).await?;
            let gate = self.state.creation_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            Ok(CreatedForDelegation {
                receipt,
                cleanup: Some(Arc::new(FixtureCreationCleanup(Arc::clone(&self.state)))),
            })
        })
    }
    fn create_session_for_delegation(
        &self,
        request: SessionCreate,
    ) -> PortFuture<'_, CreatedForDelegation<SessionCreateReceipt>> {
        Box::pin(async move {
            let receipt = self.create_session(request).await?;
            let gate = self.state.creation_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            Ok(CreatedForDelegation {
                receipt,
                cleanup: Some(Arc::new(FixtureCreationCleanup(Arc::clone(&self.state)))),
            })
        })
    }
    fn send(&self, request: SendMessage) -> PortFuture<'_, MessageAccepted> {
        Box::pin(async move {
            self.check_active()?;
            self.event("plain_send");
            let target = match &request.target {
                MessageTarget::Session {
                    agent_id,
                    session_id,
                } => SessionRef {
                    agent_id: agent_id.clone(),
                    session_id: session_id.clone(),
                },
                _ => return Err(unsupported()),
            };
            Ok(self.accepted(target, request.input()))
        })
    }
    fn send_delegated<'a>(
        &'a self,
        registration: &'a DelegationRegistration,
        input: MessageInput,
    ) -> PortFuture<'a, MessageAccepted> {
        Box::pin(async move {
            self.check_active()?;
            let relation = self
                .state
                .relations
                .lock()
                .unwrap()
                .get(&registration.child)
                .cloned()
                .expect("sink must be registered before send");
            self.event("accept_task");
            let receipt = self.accepted(registration.child.clone(), input.clone());
            relation.accepted.lock().unwrap().push(input.message_id);
            if self.state.finish_inline.load(Ordering::Acquire) {
                relation
                    .sink
                    .handoff(completion(registration, TurnOutcome::Completed))?;
            }
            Ok(receipt)
        })
    }
    fn inspect(&self, request: InspectAction) -> PortFuture<'_, Value> {
        Box::pin(async move {
            self.check_active()?;
            Ok(json!({"fixture_target":request.target}))
        })
    }
    fn update_agent(&self, _: AgentRef, candidate: AgentUpdateCandidate) -> PortFuture<'_, Value> {
        Box::pin(async move {
            self.check_active()?;
            self.state.writes.lock().unwrap().push(candidate);
            if self.state.update_conflict.load(Ordering::Acquire) {
                Err(Fault::new(
                    FaultKind::AgentConfigConflict,
                    "fixture revision changed",
                ))
            } else {
                Ok(json!({"fixture_updated":true}))
            }
        })
    }
    fn update_session(&self, _: SessionRef, _: SessionUpdateCandidate) -> PortFuture<'_, Value> {
        Box::pin(async { Err(unsupported()) })
    }
    fn cancel_turn(
        &self,
        _: SessionRef,
        _: TurnCancelRequest,
    ) -> PortFuture<'_, TurnCancelReceipt> {
        Box::pin(async { Err(unsupported()) })
    }
    fn cancel_session(
        &self,
        _: SessionRef,
        _: SessionCancelRequest,
    ) -> PortFuture<'_, SessionCancelReceipt> {
        Box::pin(async { Err(unsupported()) })
    }
    fn close_session(
        &self,
        _: SessionRef,
        _: SessionCloseRequest,
    ) -> PortFuture<'_, SessionCloseReceipt> {
        Box::pin(async { Err(unsupported()) })
    }
    fn wait_input(&self) -> PortFuture<'_, ()> {
        Box::pin(async move {
            loop {
                let changed = self.state.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                self.check_active()?;
                if self.state.ready.load(Ordering::Acquire)
                    && !self.state.paused.load(Ordering::Acquire)
                {
                    return Ok(());
                }
                changed.await;
            }
        })
    }
    fn prepare_delegation(
        &self,
        registration: DelegationRegistration,
        sink: Arc<dyn DelegationSink>,
    ) -> PortFuture<'_, Arc<dyn DelegationPort>> {
        Box::pin(async move {
            self.check_active()?;
            if self.state.draining.load(Ordering::Acquire)
                || self.state.prepare_fails.load(Ordering::Acquire)
            {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "fixture registration rejected",
                ));
            }
            self.event("register_sink");
            let relation = Arc::new(FixtureDelegation {
                registration: registration.clone(),
                sink,
                state: Arc::clone(&self.state),
                accepted: Mutex::new(Vec::new()),
            });
            self.state
                .relations
                .lock()
                .unwrap()
                .insert(registration.child, relation.clone());
            Ok(relation as Arc<dyn DelegationPort>)
        })
    }
    fn prepare_scope(&self, hooks: Arc<dyn ScopeHooks>) -> Result<Arc<dyn ScopePort>, Fault> {
        self.check_active()?;
        self.event("register_scope");
        self.state
            .hooks
            .lock()
            .unwrap()
            .insert(self.source.session.clone(), hooks);
        Ok(Arc::new(FixtureScope {
            root: self.source.session.clone(),
            state: Arc::clone(&self.state),
        }))
    }
    fn record_delegation(&self, _: &ToolCallRef, _: &DelegationRegistration) {
        self.event("project_relation");
    }
}
pub struct FixtureDelegation {
    pub registration: DelegationRegistration,
    pub sink: Arc<dyn DelegationSink>,
    pub state: Arc<FixtureState>,
    pub accepted: Mutex<Vec<MessageId>>,
}
impl DelegationPort for FixtureDelegation {
    fn notify(&self, input: MessageInput) -> PortFuture<'_, MessageAccepted> {
        Box::pin(async move {
            self.state.events.lock().unwrap().push("notify".into());
            if self.state.notify_fails.load(Ordering::Acquire) {
                return Err(Fault::new(
                    FaultKind::SessionLifecycleClosed,
                    "fixture parent is closed",
                ));
            }
            self.state
                .notifications
                .lock()
                .unwrap()
                .push((self.registration.parent.clone(), input.clone()));
            Ok(MessageAccepted::new(
                self.registration.host_id.clone(),
                self.registration.host_incarnation.clone(),
                self.registration.parent.clone(),
                input.message_id,
                MessageAcceptedFacts::new(input.mode.requested_queue(), 1),
            ))
        })
    }
    fn release(&self, disposition: DelegationDisposition) -> PortFuture<'_, ()> {
        Box::pin(async move {
            self.state.releases.lock().unwrap().push(disposition);
            self.state.changed.notify_waiters();
            Ok(())
        })
    }
    fn close(&self, _: OperationId) -> PortFuture<'_, ()> {
        Box::pin(async move {
            self.state
                .events
                .lock()
                .unwrap()
                .push(format!("close:{}", self.registration.child.session_id));
            self.state.changed.notify_waiters();
            if self.state.close_fails.load(Ordering::Acquire) {
                return Err(Fault::new(
                    FaultKind::SessionHistoryWriteFailed,
                    "fixture close has an unresolved save",
                ));
            }
            Ok(())
        })
    }
}
struct FixtureScope {
    root: SessionRef,
    state: Arc<FixtureState>,
}
impl ScopePort for FixtureScope {
    fn seal(&self) -> Result<(), Fault> {
        self.state
            .hooks
            .lock()
            .unwrap()
            .get(&self.root)
            .unwrap()
            .closing()
            .cancel();
        Ok(())
    }
    fn close_root(&self, _: OperationId) -> PortFuture<'_, ()> {
        Box::pin(async move {
            let hooks = self
                .state
                .hooks
                .lock()
                .unwrap()
                .get(&self.root)
                .cloned()
                .unwrap();
            hooks.close_members().await?;
            let gate = self.state.root_close_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            hooks.root_closed();
            self.state.events.lock().unwrap().push("close_root".into());
            self.state.changed.notify_waiters();
            Ok(())
        })
    }
}
pub fn completion(
    registration: &DelegationRegistration,
    outcome: TurnOutcome,
) -> DelegationCompletion {
    let turn_id = TurnId::new().unwrap();
    let info = minicore_host_core::TurnInfo {
        id: turn_id.clone(),
        session_id: registration.child.session_id.clone(),
        load_id: LoadId::new().unwrap(),
        state: TurnState::Finished,
        config_revision: 0,
        outcome: Some(outcome),
        persistence: PersistenceState::NotApplicable,
        requests: 1,
        tool_rounds: 0,
        phase: None,
        model_retry: None,
        model_attempts: vec![1],
        fault: None,
        persistence_fault: None,
        cleanup_faults: Vec::new(),
    };
    DelegationCompletion {
        results: vec![minicore_host_core::TurnResult {
            info,
            history: vec![],
            partials: vec![],
        }],
        discarded_inputs: Vec::new(),
        inputs: vec![DelegationInputRange {
            turn: TurnRef {
                session_id: registration.child.session_id.clone(),
                turn_id,
            },
            message_ids: vec![registration.task_message_id.clone()],
            delegated_message_ids: vec![registration.task_message_id.clone()],
        }],
    }
}
pub fn context() -> ToolContext {
    ToolContext {
        cancellation: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(10),
        progress: Default::default(),
    }
}
pub async fn invoke(tool: &dyn Tool, args: Value) -> Value {
    let result = tool
        .execute(
            ToolInvocation::new(
                "fixture-call".parse().unwrap(),
                "agent".parse().unwrap(),
                args,
            )
            .unwrap(),
            context(),
        )
        .await
        .unwrap();
    let ToolExecutionOutcome::Completed(output) = result else {
        panic!()
    };
    serde_json::from_str(output.content().as_str()).unwrap()
}
pub async fn released(state: &FixtureState, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let changed = state.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if state.releases.lock().unwrap().len() >= count {
                return;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
}

struct FixtureCreationCleanup(Arc<FixtureState>);
impl CreationCleanup for FixtureCreationCleanup {
    fn close(&self, _: OperationId) -> PortFuture<'_, ()> {
        Box::pin(async move {
            self.0.events.lock().unwrap().push("cleanup_created".into());
            self.0.changed.notify_waiters();
            Ok(())
        })
    }
}
