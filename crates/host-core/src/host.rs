use crate::agent::AgentOwner;
use crate::control::{CallerContext, ConnectionHandle, ConnectionRegistry};
use crate::session::{ResourceResolver, SessionController, SessionSubscription};
use crate::types::*;
use minicore_host_protocol::*;
use minicore_host_storage::{HostRecord, HostStore};
use minicore_runtime::HistoryItem;

mod create;
mod metadata;
mod persistent;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// In-process entry point. Persistent initialization and memory construction
/// share the same direct owners, input queues and Runtime execution path.
pub struct Host {
    id: HostId,
    incarnation: HostIncarnation,
    access: Arc<ConnectionRegistry>,
    resources: Arc<dyn ResourceResolver>,
    state: Mutex<HostState>,
    store: Option<HostStore>,
    settings: Option<HostRecord<HostSettings>>,
}
struct HostState {
    lifecycle: HostLifecycle,
    agents: BTreeMap<AgentId, Arc<AgentOwner>>,
    creations: BTreeMap<OperationId, AgentCreateReceipt>,
    load_failures: BTreeMap<AgentId, Fault>,
    create_tasks: BTreeMap<OperationId, tokio::task::JoinHandle<()>>,
}
impl Host {
    pub fn memory(resources: Arc<dyn ResourceResolver>) -> Result<Arc<Self>, Fault> {
        let id = HostId::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        let incarnation = HostIncarnation::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        let access = ConnectionRegistry::new(id.clone(), incarnation.clone());
        Ok(Arc::new(Self {
            id,
            incarnation,
            access,
            resources,
            store: None,
            settings: None,
            state: Mutex::new(HostState {
                lifecycle: HostLifecycle::Running,
                agents: BTreeMap::new(),
                creations: BTreeMap::new(),
                load_failures: BTreeMap::new(),
                create_tasks: BTreeMap::new(),
            }),
        }))
    }
    /// The embedding process is trusted. A network adapter must authenticate
    /// before it can invoke an equivalent trusted initialization path.
    pub fn connect(&self) -> Result<ConnectionHandle, Fault> {
        self.access.connect()
    }
    pub fn info(&self, caller: &CallerContext) -> Result<HostInfo, Fault> {
        let control = self.access.snapshot(caller)?;
        let lifecycle = self.state.lock().expect("Host mutex poisoned").lifecycle;
        Ok(HostInfo {
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            lifecycle,
            control,
        })
    }
    fn agent(&self, caller: &CallerContext, id: &AgentId) -> Result<Arc<AgentOwner>, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Host mutex poisoned");
            if let Some(fault) = state.load_failures.get(id) {
                return Err(fault.clone());
            }
            state.agents.get(id).cloned().ok_or_else(|| {
                Fault::new(
                    FaultKind::HostAgentNotFound,
                    "Agent is not registered in this Host",
                )
            })
        })
    }

    fn session(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<Arc<SessionController>, Fault> {
        // Deliberately no Host Session index or direct Session fast path.
        self.agent(caller, &target.agent_id)?
            .session(&target.session_id)
    }
    fn check_host(&self, id: &HostId) -> Result<(), Fault> {
        if id == &self.id {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::HostOwnershipMismatch,
                "the target belongs to another Host",
            ))
        }
    }
    pub fn create_agent(
        &self,
        caller: &CallerContext,
        request: AgentCreate,
    ) -> Result<AgentCreateReceipt, Fault> {
        request
            .validate()
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        if request.mode != StorageMode::Memory {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent entity startup is not implemented in this foundation entry point",
            ));
        }
        self.resources.validate(
            &SessionConfig::from_agent(
                &request.config,
                &request
                    .default_session
                    .clone()
                    .unwrap_or_default()
                    .overrides
                    .unwrap_or_default(),
            )
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?,
        )?;
        let id = AgentId::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        let agent = AgentOwner::new(
            id.clone(),
            self.id.clone(),
            self.incarnation.clone(),
            request.name,
            request.config,
            Arc::clone(&self.access),
            Arc::clone(&self.resources),
        );
        let agent = agent.hold_root(
            self.store
                .as_ref()
                .map(|store| Arc::new(store.clone()) as Arc<dyn Send + Sync>),
        );
        // Acceptance publishes the Agent. Following creation is an owned
        // workflow and is not re-authorized against a possibly closed caller.
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            if state.lifecycle != HostLifecycle::Running {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host is not accepting new Agents",
                ));
            }
            state.agents.insert(id.clone(), Arc::clone(&agent));
            Ok(())
        })?;
        let session_id = SessionId::new()
            .map_err(|error| Fault::new(FaultKind::AgentInternalUnexpected, error.to_string()));
        let session = session_id.and_then(|id| {
            agent.create_session_owned(id, request.default_session.unwrap_or_default(), true)
        });
        let message_id = request
            .initial_message
            .as_ref()
            .map(|message| message.message_id.clone());
        let (session_id, result) = match session {
            Err(error) => (
                None,
                AgentCreateResult::without_delivery(AgentCreateSteps {
                    agent: CreateStep::Succeeded,
                    session: CreateStep::Failed { error },
                    message: message_id.as_ref().map(|_| CreateStep::NotAttempted),
                }),
            ),
            Ok(session) => {
                let id = session.id.clone();
                let steps = AgentCreateSteps {
                    agent: CreateStep::Succeeded,
                    session: CreateStep::Succeeded,
                    message: None,
                };
                let result = if let Some(message) = request.initial_message {
                    match session.send_owned(message) {
                        Ok(receipt) => AgentCreateResult::accepted(
                            AgentCreateSteps {
                                message: Some(CreateStep::Succeeded),
                                ..steps
                            },
                            receipt.result,
                        )
                        .expect("formed valid accepted creation"),
                        Err(error) => AgentCreateResult::without_delivery(AgentCreateSteps {
                            message: Some(CreateStep::Failed { error }),
                            ..steps
                        }),
                    }
                } else {
                    AgentCreateResult::without_delivery(steps)
                };
                (Some(id), result)
            }
        };
        let receipt = AgentCreateReceipt {
            kind: AgentCreateKind::AgentCreate,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: request.operation_id.clone(),
            agent_id: Some(id),
            session_id,
            message_id,
            result,
        };
        self.state
            .lock()
            .expect("Host mutex poisoned")
            .creations
            .insert(request.operation_id, receipt.clone());
        Ok(receipt)
    }
    pub fn receipt(
        &self,
        caller: &CallerContext,
        operation_id: &OperationId,
    ) -> Result<AgentCreateReceipt, Fault> {
        self.access.read(caller, || {
            self.state
                .lock()
                .expect("Host mutex poisoned")
                .creations
                .get(operation_id)
                .cloned()
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::HostMethodUnavailable,
                        "no retained creation receipt for this operation",
                    )
                })
        })
    }
    pub fn agent_info(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<AgentInfo, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?.info(caller)
    }
    pub fn agents(&self, caller: &CallerContext) -> Result<Vec<AgentInfo>, Fault> {
        let agents = self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Host mutex poisoned")
                .agents
                .values()
                .cloned()
                .collect::<Vec<_>>())
        })?;
        agents.into_iter().map(|agent| agent.info(caller)).collect()
    }
    pub fn create_session(
        &self,
        caller: &CallerContext,
        request: SessionCreate,
    ) -> Result<SessionCreateReceipt, Fault> {
        self.check_host(&request.target.host_id)?;
        let agent = self.agent(caller, &request.target.agent_id)?;
        let options = SessionOptions {
            mode: request.mode,
            overrides: request.overrides,
        };
        let message_id = request
            .initial_message
            .as_ref()
            .map(|message| message.message_id.clone());
        let (session_id, result) = match agent.create_session(caller, options, false) {
            Err(error) => (
                None,
                SessionCreateResult::without_delivery(SessionCreateSteps {
                    session: CreateStep::Failed { error },
                    message: message_id.as_ref().map(|_| CreateStep::NotAttempted),
                }),
            ),
            Ok(session) => {
                let id = session.id.clone();
                let steps = SessionCreateSteps {
                    session: CreateStep::Succeeded,
                    message: None,
                };
                let result = if let Some(message) = request.initial_message {
                    match session.send_owned(message) {
                        Ok(receipt) => SessionCreateResult::accepted(
                            SessionCreateSteps {
                                message: Some(CreateStep::Succeeded),
                                ..steps
                            },
                            receipt.result,
                        )
                        .expect("formed valid accepted creation"),
                        Err(error) => SessionCreateResult::without_delivery(SessionCreateSteps {
                            message: Some(CreateStep::Failed { error }),
                            ..steps
                        }),
                    }
                } else {
                    SessionCreateResult::without_delivery(steps)
                };
                (Some(id), result)
            }
        };
        Ok(SessionCreateReceipt {
            kind: SessionCreateKind::SessionCreate,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: request.operation_id,
            agent_id: agent.id.clone(),
            session_id,
            message_id,
            result,
        })
    }
    pub fn send(
        &self,
        caller: &CallerContext,
        request: SendMessage,
    ) -> Result<MessageAccepted, Fault> {
        let session = match &request.target {
            MessageTarget::AgentDefault { host_id, agent_id } => {
                self.check_host(host_id)?;
                self.agent(caller, agent_id)?.default_session()?
            }
            MessageTarget::Session {
                agent_id,
                session_id,
            } => self.agent(caller, agent_id)?.session(session_id)?,
        };
        session.send(caller, request.input())
    }
    pub fn session_info(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionInfo, Fault> {
        self.session(caller, target)?.info(caller)
    }
    pub fn session_state(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionSnapshot, Fault> {
        self.session(caller, target)?.snapshot(caller)
    }
    pub fn session_states(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionSubscription, Fault> {
        self.session(caller, target)?.subscribe_state(caller)
    }
    pub fn sessions(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<Vec<SessionInfo>, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .list_sessions(caller)?
            .into_iter()
            .map(|session| session.info(caller))
            .collect()
    }
    pub fn message(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        message_id: &MessageId,
    ) -> Result<MessageRecord, Fault> {
        self.session(caller, target)?.message(caller, message_id)
    }
    pub fn history(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<Vec<HistoryItem>, Fault> {
        self.session(caller, target)?.history(caller)
    }
    pub fn pause(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session(caller, target)?.pause(caller)
    }
    pub fn resume(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session(caller, target)?.resume(caller)
    }
    pub fn unload(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session(caller, target)?.unload(caller)
    }
    pub fn update_session(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        expected_revision: u64,
        patch: &SessionConfigPatch,
    ) -> Result<SessionInfo, Fault> {
        self.session(caller, target)?
            .update(caller, expected_revision, patch)
    }
    pub fn update_agent(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        expected_revision: u64,
        patch: &AgentConfigPatch,
    ) -> Result<AgentInfo, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .update(caller, expected_revision, patch)
    }
    pub fn set_default_session(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        session_id: Option<SessionId>,
    ) -> Result<(), Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_default(caller, session_id)
    }
    pub fn stop_agent(&self, caller: &CallerContext, target: &AgentRef) -> Result<(), Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_enabled(caller, false)
    }
    pub fn start_agent(&self, caller: &CallerContext, target: &AgentRef) -> Result<(), Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .set_enabled(caller, true)
    }
    pub fn remove_input(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &MessageId,
    ) -> Result<(), Fault> {
        self.session(caller, target)?.remove(caller, id)
    }
    pub fn cancel_session(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        keep_inbox: bool,
    ) -> Result<CancelResult, Fault> {
        self.session(caller, target)?
            .cancel(caller, None, keep_inbox)
    }
    pub fn cancel_turn(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
        keep_inbox: bool,
    ) -> Result<CancelResult, Fault> {
        self.session(caller, target)?
            .cancel(caller, Some(turn_id), keep_inbox)
    }
    pub fn close_session(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.agent(caller, &target.agent_id)?
            .close_session(caller, &target.session_id)
    }
    pub fn turns(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<Vec<TurnInfo>, Fault> {
        self.session(caller, target)?.turns(caller)
    }
    pub fn turn_info(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnInfo, Fault> {
        self.session(caller, target)?.turn_info(caller, id)
    }
    pub fn turn_result(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        self.session(caller, target)?.turn_result(caller, id)
    }
    pub async fn wait_turn(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        self.session(caller, target)?.wait_turn(caller, id).await
    }
}
