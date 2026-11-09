use crate::agent::AgentOwner;
use crate::configuration::{ConfigurationOwner, HostConfig, PreparedHostConfiguration};
use crate::control::{CallerContext, ConnectionHandle, ConnectionRegistry};
use crate::session::{ResourceResolver, SessionController, SessionSubscription};
use crate::types::*;
use minicore_host_protocol::*;
use minicore_host_storage::{HostRecord, HostStore};
use minicore_runtime::HistoryItem;

mod collaboration;
mod configuration;
mod context;
mod controls;
mod create;
mod deletion;
mod metadata;
mod network;
mod observation;
mod persistent;
mod process;
pub use process::{HostProcessControl, HostProcessSnapshot};
mod retirement;
mod shutdown;
mod tool_edits;
mod workspace;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
pub use workspace::WorkspaceSnapshot;

/// In-process entry point. Persistent initialization and memory construction
/// share the same direct owners, input queues and Runtime execution path.
pub struct Host {
    self_weak: Weak<Self>,
    id: HostId,
    incarnation: HostIncarnation,
    access: Arc<ConnectionRegistry>,
    resources: Mutex<Option<Arc<dyn ResourceResolver>>>,
    state: Mutex<HostState>,
    store: Mutex<Option<HostStore>>,
    changed: Arc<tokio::sync::Notify>,
    process_changed: tokio::sync::Notify,
    pub(crate) configuration: Arc<ConfigurationOwner>,
}
struct HostState {
    lifecycle: HostLifecycle,
    shutdown_strategy: Option<HostShutdownStrategy>,
    shutdown_fault: Option<Fault>,
    shutdown_task: Option<tokio::task::JoinHandle<()>>,
    creating: usize,
    retirement_watch: Option<tokio::task::JoinHandle<()>>,
    retirement_epoch: u64,
    agents: BTreeMap<AgentId, Arc<AgentOwner>>,
    // The generated object identity distinguishes accepted invocations. The
    // caller's operation_id is correlation data, never an idempotency key.
    creations: BTreeMap<AgentId, AgentCreateReceipt>,
    controls: BTreeMap<OperationId, Vec<Receipt>>,
    deleting: usize,
    delete_tasks: BTreeMap<AgentId, tokio::task::JoinHandle<()>>,
    load_failures: BTreeMap<AgentId, Fault>,
    create_tasks: BTreeMap<AgentId, tokio::task::JoinHandle<()>>,
}
impl Host {
    pub fn memory(resources: Arc<dyn ResourceResolver>) -> Result<Arc<Self>, Fault> {
        Self::memory_configured(PreparedHostConfiguration::new(
            HostConfig::default(),
            resources,
            Vec::new(),
            None,
        )?)
    }
    pub fn memory_configured(candidate: PreparedHostConfiguration) -> Result<Arc<Self>, Fault> {
        let id = HostId::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        Self::construct(id, 0, candidate, None, HostLifecycle::Running)
    }
    fn construct(
        id: HostId,
        revision: ConfigRevision,
        candidate: PreparedHostConfiguration,
        store: Option<HostStore>,
        lifecycle: HostLifecycle,
    ) -> Result<Arc<Self>, Fault> {
        let incarnation = HostIncarnation::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        let access = ConnectionRegistry::new(id.clone(), incarnation.clone());
        let (configuration, key) = ConfigurationOwner::new(candidate, revision);
        access.publish_network(None, key, || Ok(()))?;
        let resources: Arc<dyn ResourceResolver> = configuration.clone();
        let owner = Arc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            id,
            incarnation,
            access,
            resources: Mutex::new(Some(resources)),
            store: Mutex::new(store),
            changed: Arc::new(tokio::sync::Notify::new()),
            process_changed: tokio::sync::Notify::new(),
            configuration,
            state: Mutex::new(HostState {
                lifecycle,
                shutdown_strategy: None,
                shutdown_fault: None,
                shutdown_task: None,
                creating: 0,
                retirement_watch: None,
                retirement_epoch: 0,
                agents: BTreeMap::new(),
                creations: BTreeMap::new(),
                controls: BTreeMap::new(),
                deleting: 0,
                delete_tasks: BTreeMap::new(),
                load_failures: BTreeMap::new(),
                create_tasks: BTreeMap::new(),
            }),
        });
        owner.access.attach_host(&owner);
        Ok(owner)
    }
    fn resources_owned(&self) -> Result<Arc<dyn ResourceResolver>, Fault> {
        self.resources
            .lock()
            .expect("Host resources mutex poisoned")
            .clone()
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host execution resources were retired",
                )
            })
    }
    fn store(&self) -> Option<HostStore> {
        self.store
            .lock()
            .expect("Host store mutex poisoned")
            .clone()
    }
    /// The embedding process is trusted. A network adapter must authenticate
    /// before it can invoke an equivalent trusted initialization path.
    pub fn connect(&self) -> Result<ConnectionHandle, Fault> {
        let connection = self.access.connect()?;
        self.publish_host_state_owned();
        Ok(connection)
    }
    pub fn info(&self, caller: &CallerContext) -> Result<HostInfo, Fault> {
        let control = self.access.snapshot(caller)?;
        let state = self.state.lock().expect("Host mutex poisoned");
        Ok(HostInfo {
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            lifecycle: state.lifecycle,
            shutdown_strategy: state.shutdown_strategy,
            shutdown_fault: state.shutdown_fault.clone(),
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
        self.resources_owned()?.validate(
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
            self.resources_owned()?,
        );
        let agent = agent.with_change_signal(Arc::clone(&self.changed));
        agent.begin_initial_creation();
        // Acceptance publishes the Agent. Following creation is an owned
        // workflow and is not re-authorized against a possibly closed caller.
        self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            if state.lifecycle != HostLifecycle::Running {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host is not accepting new Agents",
                ));
            }
            state.creating += 1;
            state.agents.insert(id.clone(), Arc::clone(&agent));
            Ok(())
        })?;
        let agent = agent.hold_root(
            self.store()
                .map(|store| Arc::new(store) as Arc<dyn Send + Sync>),
        );
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
        agent.finish_initial_creation();
        let receipt = AgentCreateReceipt {
            kind: AgentCreateKind::AgentCreate,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: request.operation_id.clone(),
            agent_id: Some(id.clone()),
            session_id,
            message_id,
            result,
        };
        let mut state = self.state.lock().expect("Host mutex poisoned");
        state.creations.insert(id, receipt.clone());
        state.creating -= 1;
        self.changed.notify_waiters();
        drop(state);
        agent.try_retire_empty_owned();
        self.remove_retired_agents_owned();
        Ok(receipt)
    }
    pub fn receipt(
        &self,
        caller: &CallerContext,
        operation_id: &OperationId,
    ) -> Result<AgentCreateReceipt, Fault> {
        let mut receipts = self.creation_receipts(caller, operation_id)?.into_iter();
        let receipt = receipts.next().ok_or_else(|| {
            Fault::new(
                FaultKind::HostReceiptNotFound,
                "no retained creation receipt for this operation",
            )
        })?;
        if receipts.next().is_some() {
            return Err(Fault::new(
                FaultKind::HostReceiptAmbiguous,
                "operation_id matches multiple invocations; use creation_receipts",
            ));
        }
        Ok(receipt)
    }
    /// All accepted invocations with this correlation ID. Reusing an operation
    /// ID neither deduplicates a request nor replaces another invocation's facts.
    pub fn creation_receipts(
        &self,
        caller: &CallerContext,
        operation_id: &OperationId,
    ) -> Result<Vec<AgentCreateReceipt>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Host mutex poisoned")
                .creations
                .values()
                .filter(|receipt| &receipt.operation_id == operation_id)
                .cloned()
                .collect())
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
        let key = SessionId::new()
            .map_err(|error| Fault::new(FaultKind::AgentInternalUnexpected, error.to_string()))?;
        let (session_id, result) = match agent.create_session(caller, key.clone(), options, false) {
            Err(error)
                if matches!(
                    error.kind(),
                    Some(
                        FaultKind::HostLifecycleUnavailable
                            | FaultKind::HostAccessInvalid
                            | FaultKind::HostAccessReadOnly
                    )
                ) =>
            {
                return Err(error);
            }
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
        let receipt = SessionCreateReceipt {
            kind: SessionCreateKind::SessionCreate,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: request.operation_id,
            agent_id: agent.id.clone(),
            session_id,
            message_id,
            result,
        };
        agent.retain_session_creation(key, receipt.clone());
        Ok(receipt)
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
        self.session_pause(
            caller,
            target,
            SessionControlRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
    }
    pub fn resume(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session_resume(
            caller,
            target,
            SessionControlRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
    }
    pub fn unload(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session_unload(
            caller,
            target,
            SessionControlRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
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
        self.agent_stop(
            caller,
            target,
            AgentStopRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
    }
    pub fn start_agent(&self, caller: &CallerContext, target: &AgentRef) -> Result<(), Fault> {
        self.agent_start(
            caller,
            target,
            AgentStartRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
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
            .cancel(caller, None, keep_inbox, None)
            .map(|(result, _)| result)
    }
    pub fn cancel_turn(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
        keep_inbox: bool,
    ) -> Result<CancelResult, Fault> {
        self.session(caller, target)?
            .cancel(caller, Some(turn_id), keep_inbox, None)
            .map(|(result, _)| result)
    }
    pub fn close_session(&self, caller: &CallerContext, target: &SessionRef) -> Result<(), Fault> {
        self.session_close(
            caller,
            target,
            SessionCloseRequest {
                operation_id: Self::operation_id()?,
            },
        )
        .map(|_| ())
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
        // The observation token contains only a Weak owner and a bounded state
        // snapshot. An abandoned wait cannot retain Session bodies or runners.
        let mut states = self.session_states(caller, target)?;
        loop {
            let turn = self.turn_info(caller, target, id)?;
            if turn.state == TurnState::Finished {
                return self.turn_result(caller, target, id);
            }
            states.changed().await?;
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // This monitor is observational, not an accepted execution/save owner.
        if let Some(task) = self
            .state
            .get_mut()
            .expect("Host mutex poisoned")
            .retirement_watch
            .take()
        {
            task.abort();
        }
    }
}
