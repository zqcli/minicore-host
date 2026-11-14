use crate::ResourceResolver;
use crate::agent::AgentOwner;
use crate::configuration::{ConfigurationOwner, HostConfig, PreparedHostConfiguration};
use crate::control::{CallerContext, ConnectionHandle, ConnectionRegistry};
use crate::session::{SessionController, SessionSubscription};
use crate::types::*;
use minicore_host_protocol::*;
use minicore_host_runtime::HistoryItem;
use minicore_host_storage::{HostRecord, HostStore};

mod collaboration;
mod configuration;
mod controls;
mod create;
mod deletion;
mod diagnostics;
mod directories;
mod discovery;
mod metadata;
mod network;
mod observation;
mod persistent;
mod process;
mod query;
pub use process::{HostProcessControl, HostProcessSnapshot};
mod retirement;
mod shutdown;
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
    // Construction mode remains factual after shutdown releases the store.
    mode: StorageMode,
    access: Arc<ConnectionRegistry>,
    resources: Mutex<Option<Arc<dyn ResourceResolver>>>,
    state: Mutex<HostState>,
    store: Mutex<Option<HostStore>>,
    changed: Arc<tokio::sync::Notify>,
    process_changed: tokio::sync::Notify,
    pub(crate) configuration: Arc<ConfigurationOwner>,
    logging_health: std::sync::OnceLock<minicore_host_diagnostics::Health>,
    #[cfg(test)]
    directory_gate: Mutex<Option<Arc<directories::tests::DirectoryGate>>>,
}
struct HostState {
    lifecycle: HostLifecycle,
    shutdown_strategy: Option<HostShutdownStrategy>,
    shutdown_fault: Option<Fault>,
    shutdown_task: Option<tokio::task::JoinHandle<()>>,
    creating: usize,
    directory_writes: usize,
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
        let owner = Self::construct(id, 0, candidate, None, HostLifecycle::Running)?;
        owner
            .lifecycle_record("host.started", "running", StorageMode::Memory, None)
            .emit();
        Ok(owner)
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
            mode: if store.is_some() {
                StorageMode::Persistent
            } else {
                StorageMode::Memory
            },
            access,
            resources: Mutex::new(Some(resources)),
            store: Mutex::new(store),
            changed: Arc::new(tokio::sync::Notify::new()),
            process_changed: tokio::sync::Notify::new(),
            configuration,
            logging_health: std::sync::OnceLock::new(),
            #[cfg(test)]
            directory_gate: Mutex::new(None),
            state: Mutex::new(HostState {
                lifecycle,
                shutdown_strategy: None,
                shutdown_fault: None,
                shutdown_task: None,
                creating: 0,
                directory_writes: 0,
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
