use crate::control::{CallerContext, ConnectionRegistry};
use crate::session::{HistoryBackend, MemoryHistory, ResourceResolver, SessionController};
use crate::types::{AgentInfo, SessionLoadFailure};
use minicore_host_storage::{AgentRecord, AgentStore};

mod collaboration;
mod controls;
mod create;
mod deletion;
mod diagnostics;
mod discovery;
mod metadata;
mod persistent;
mod retirement;
pub(crate) mod shutdown;
use minicore_host_protocol::*;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tracing::{Instrument, instrument::WithSubscriber};

pub(crate) struct AgentOwner {
    pub(crate) id: AgentId,
    host_id: HostId,
    incarnation: HostIncarnation,
    access: Arc<ConnectionRegistry>,
    resources: Mutex<Option<Arc<dyn ResourceResolver>>>,
    state: Mutex<AgentState>,
    enabled: Arc<AtomicBool>,
    store: Mutex<Option<AgentStore>>,
    changed: Arc<tokio::sync::Notify>,
    root_lease: Mutex<Option<Arc<dyn Send + Sync>>>,
}
struct AgentState {
    info: AgentInfo,
    sessions: BTreeMap<SessionId, Arc<SessionController>>,
    load_failures: BTreeMap<SessionId, Fault>,
    creations: BTreeMap<SessionId, SessionCreateReceipt>,
    public_creations: std::collections::BTreeSet<SessionId>,
    controls: BTreeMap<OperationId, Vec<Receipt>>,
    create_tasks: BTreeMap<SessionId, tokio::task::JoinHandle<()>>,
    creating: usize,
    creation_committed: bool,
    resources_released: bool,
    host_shutdown_phase: u8,
    retirement_joining: bool,
    executor: Option<tokio::runtime::Handle>,
    stop_task: Option<tokio::task::JoinHandle<()>>,
    record_base: Option<AgentRecord>,
    record_work: Option<metadata::AgentRecordWork>,
    record_task: Option<tokio::task::JoinHandle<()>>,
    closing: std::collections::BTreeSet<SessionId>,
    close_tasks: BTreeMap<SessionId, tokio::task::JoinHandle<()>>,
    close_running: std::collections::BTreeSet<SessionId>,
    deleting_sessions: std::collections::BTreeSet<SessionId>,
    delete_tasks: BTreeMap<SessionId, tokio::task::JoinHandle<()>>,
    deleting: bool,
}
struct SynchronousCreation<'a> {
    owner: &'a AgentOwner,
}
impl Drop for SynchronousCreation<'_> {
    fn drop(&mut self) {
        self.owner
            .state
            .lock()
            .expect("Agent mutex poisoned")
            .creating -= 1;
        self.owner.changed.notify_waiters();
    }
}
impl AgentOwner {
    pub(crate) fn new(
        id: AgentId,
        host_id: HostId,
        incarnation: HostIncarnation,
        name: String,
        config: AgentConfig,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: id.clone(),
            host_id: host_id.clone(),
            incarnation,
            access,
            resources: Mutex::new(Some(resources)),
            enabled: Arc::new(AtomicBool::new(true)),
            store: Mutex::new(None),
            root_lease: Mutex::new(None),
            changed: Arc::new(tokio::sync::Notify::new()),
            state: Mutex::new(AgentState {
                info: AgentInfo {
                    deletion: None,
                    id,
                    host_id,
                    name,
                    mode: StorageMode::Memory,
                    lifecycle: AgentLifecycle::Active,
                    config_revision: 0,
                    config,
                    default_session_id: None,
                    metadata_saving: false,
                    metadata_fault: None,
                },
                sessions: BTreeMap::new(),
                load_failures: BTreeMap::new(),
                creations: BTreeMap::new(),
                public_creations: Default::default(),
                controls: BTreeMap::new(),
                create_tasks: BTreeMap::new(),
                creating: 0,
                creation_committed: false,
                resources_released: false,
                host_shutdown_phase: 0,
                retirement_joining: false,
                executor: tokio::runtime::Handle::try_current().ok(),
                stop_task: None,
                record_base: None,
                record_work: None,
                record_task: None,
                closing: Default::default(),
                close_tasks: BTreeMap::new(),
                close_running: Default::default(),
                deleting_sessions: Default::default(),
                delete_tasks: BTreeMap::new(),
                deleting: false,
            }),
        })
    }
    fn resources_owned(&self) -> Result<Arc<dyn ResourceResolver>, Fault> {
        self.resources
            .lock()
            .expect("Agent resources mutex poisoned")
            .clone()
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host execution resources were retired",
                )
            })
    }
    fn store(&self) -> Option<AgentStore> {
        self.store
            .lock()
            .expect("Agent store mutex poisoned")
            .clone()
    }
    fn root_lease(&self) -> Option<Arc<dyn Send + Sync>> {
        self.root_lease
            .lock()
            .expect("Agent lease mutex poisoned")
            .clone()
    }
    pub(crate) fn hold_root(self: Arc<Self>, lease: Option<Arc<dyn Send + Sync>>) -> Arc<Self> {
        *self.root_lease.lock().expect("Agent lease mutex poisoned") = lease;
        self
    }
    pub(crate) fn with_change_signal(
        mut self: Arc<Self>,
        changed: Arc<tokio::sync::Notify>,
    ) -> Arc<Self> {
        Arc::get_mut(&mut self).expect("unpublished Agent").changed = changed;
        self
    }
    pub(crate) fn info(&self, caller: &CallerContext) -> Result<AgentInfo, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Agent mutex poisoned")
                .info
                .clone())
        })
    }
    pub(crate) fn session(&self, id: &SessionId) -> Result<Arc<SessionController>, Fault> {
        let state = self.state.lock().expect("Agent mutex poisoned");
        if let Some(fault) = state.load_failures.get(id) {
            return Err(fault.clone());
        }
        state.sessions.get(id).cloned().ok_or_else(|| {
            Fault::new(
                FaultKind::AgentSessionNotFound,
                "Session does not belong to this Agent",
            )
        })
    }

    pub(crate) fn default_session(&self) -> Result<Arc<SessionController>, Fault> {
        let state = self.state.lock().expect("Agent mutex poisoned");
        let id = state.info.default_session_id.as_ref().ok_or_else(|| {
            Fault::new(
                FaultKind::AgentDefaultSessionUnavailable,
                "Agent has no default Session",
            )
        })?;
        state.sessions.get(id).cloned().ok_or_else(|| {
            Fault::new(
                FaultKind::AgentDefaultSessionUnavailable,
                "Agent default Session is unavailable",
            )
        })
    }
    pub(crate) fn list_sessions(
        &self,
        caller: &CallerContext,
    ) -> Result<Vec<Arc<SessionController>>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Agent mutex poisoned")
                .sessions
                .values()
                .cloned()
                .collect())
        })
    }
    pub(crate) fn create_session(
        &self,
        caller: &CallerContext,
        id: SessionId,
        options: SessionOptions,
        make_default: bool,
    ) -> Result<Arc<SessionController>, Fault> {
        self.access.accept_business(caller, || {
            self.create_session_owned(id, options, make_default)
        })
    }
    pub(crate) fn create_session_owned(
        &self,
        id: SessionId,
        options: SessionOptions,
        make_default: bool,
    ) -> Result<Arc<SessionController>, Fault> {
        self.access.ensure_execution_owned()?;
        if self.store().is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent Agent creation requires the asynchronous owner entry",
            ));
        }
        // Snapshot only direct-owner values, then release this lock before child
        // construction. Publication rechecks the actual lifecycle boundary.
        let (config, source_revision) = {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if state.info.lifecycle != AgentLifecycle::Active
                && !(make_default && state.info.lifecycle == AgentLifecycle::Creating)
            {
                return Err(Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "Agent is not accepting Session creation",
                ));
            }
            state
                .info
                .mode
                .session_mode(options.mode)
                .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
            let config = SessionConfig::from_agent(
                &state.info.config,
                &options.overrides.unwrap_or_default(),
            )
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
            state.creating += 1;
            (config, state.info.config_revision)
        };
        let _creation = SynchronousCreation { owner: self };
        self.resources_owned()?.validate(&config)?;
        let backend: Arc<dyn HistoryBackend> = Arc::new(MemoryHistory);
        let session = SessionController::new(
            id.clone(),
            self.id.clone(),
            self.host_id.clone(),
            self.incarnation.clone(),
            config,
            Arc::clone(&self.access),
            self.resources_owned()?,
            backend,
            Vec::new(),
            Arc::clone(&self.enabled),
        );
        let session = session
            .hold_root(self.root_lease())
            .notify_parent_on_change(Arc::clone(&self.changed));
        session.set_source_revision(source_revision);
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        self.access.ensure_execution_owned()?;
        if state.info.lifecycle != AgentLifecycle::Active
            && !(make_default && state.info.lifecycle == AgentLifecycle::Creating)
        {
            return Err(Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "Agent stopped before Session publication",
            ));
        }
        state.sessions.insert(id.clone(), Arc::clone(&session));
        if make_default {
            state.info.default_session_id = Some(id);
        }
        drop(state);
        session.publish_initial();
        Ok(session)
    }
    pub(crate) fn set_default(
        &self,
        caller: &CallerContext,
        session: Option<SessionId>,
    ) -> Result<(), Fault> {
        if self.store().is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent default updates require the asynchronous owner entry",
            ));
        }
        let child = session.as_ref().map(|id| self.session(id)).transpose()?;
        if child.as_ref().is_some_and(|child| !child.is_open_owned()) {
            return Err(Fault::new(
                FaultKind::AgentDefaultSessionUnavailable,
                "default Session must be open",
            ));
        }
        let diagnostic = self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            Self::writable(&state)?;
            if session
                .as_ref()
                .is_some_and(|id| !state.sessions.contains_key(id))
            {
                return Err(Fault::new(
                    FaultKind::AgentSessionNotFound,
                    "Session does not belong to this Agent",
                ));
            }
            if session
                .as_ref()
                .is_some_and(|id| state.closing.contains(id))
            {
                return Err(Fault::new(
                    FaultKind::AgentDefaultSessionUnavailable,
                    "default Session is closing",
                ));
            }
            state.info.default_session_id = session;
            Ok(self.metadata_result_record(self.diagnostic_context(), &state.info, None))
        })?;
        diagnostic.emit();
        Ok(())
    }
    pub(crate) fn clear_default_if(&self, id: &SessionId) {
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        if state.info.default_session_id.as_ref() == Some(id) {
            state.info.default_session_id = None;
        }
    }
    pub(crate) fn update(
        &self,
        caller: &CallerContext,
        expected_revision: u64,
        patch: &AgentConfigPatch,
    ) -> Result<AgentInfo, Fault> {
        if self.store().is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent configuration updates require the asynchronous owner entry",
            ));
        }
        let original = self.info(caller)?;
        if original.config_revision != expected_revision {
            return Err(Fault::new(
                FaultKind::AgentConfigConflict,
                "Agent configuration revision changed",
            ));
        }
        let config = original
            .config
            .apply_patch(patch)
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        self.resources_owned()?
            .validate(
                &SessionConfig::from_agent(&config, &SessionOverrides::default()).map_err(
                    |error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()),
                )?,
            )
            .inspect_err(|fault| self.metadata_refusal_record(None, fault).emit())?;
        let (info, diagnostic) = self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            Self::writable(&state)?;
            if state.info.config_revision != expected_revision {
                return Err(Fault::new(
                    FaultKind::AgentConfigConflict,
                    "Agent configuration revision changed",
                ));
            }
            state.info.config_revision = expected_revision.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "configuration revision exhausted",
                )
            })?;
            state.info.config = config;
            Ok((
                state.info.clone(),
                self.metadata_result_record(self.diagnostic_context(), &state.info, None),
            ))
        })?;
        diagnostic.emit();
        Ok(info)
    }
    pub(crate) fn set_enabled(
        self: &Arc<Self>,
        caller: &CallerContext,
        enabled: bool,
        operation: Option<OperationId>,
    ) -> Result<Option<Receipt>, Fault> {
        let span = self.diagnostic_context().span();
        let (children, receipt, diagnostic) = self.access.accept(caller, || {
            if enabled {
                self.access.ensure_execution_owned()?;
            }
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if enabled
                && !matches!(
                    state.info.lifecycle,
                    AgentLifecycle::Active | AgentLifecycle::Stopped
                )
            {
                return Err(Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "Agent can start only after initialization or stop work has settled",
                ));
            }
            if !enabled
                && matches!(
                    state.info.lifecycle,
                    AgentLifecycle::Retiring | AgentLifecycle::Retired
                )
            {
                return Err(Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "retiring Agents cannot reenter stop or start",
                ));
            }
            if (!enabled
                && matches!(
                    state.info.lifecycle,
                    AgentLifecycle::Stopping | AgentLifecycle::Stopped
                ))
                || (enabled && state.info.lifecycle == AgentLifecycle::Active)
            {
                let receipt = self.enabled_receipt(&mut state, operation.clone(), enabled);
                return Ok((None, receipt, None));
            }
            self.enabled.store(enabled, Ordering::SeqCst);
            state.info.lifecycle = if enabled {
                AgentLifecycle::Active
            } else {
                AgentLifecycle::Stopping
            };
            let children = state.sessions.values().cloned().collect::<Vec<_>>();
            let diagnostic = self.lifecycle_record(
                if enabled {
                    "agent.started"
                } else {
                    "agent.stop_accepted"
                },
                if enabled { "active" } else { "stopping" },
                state.info.mode,
                operation.clone(),
            );
            let receipt = self.enabled_receipt(&mut state, operation.clone(), enabled);
            Ok((Some(children), receipt, Some(diagnostic)))
        })?;
        crate::diagnostics::emit(diagnostic);
        let Some(children) = children else {
            return Ok(receipt);
        };
        for child in &children {
            child.set_agent_enabled(enabled);
        }
        if enabled {
            return Ok(receipt);
        }
        let executor = self
            .state
            .lock()
            .expect("Agent mutex poisoned")
            .executor
            .clone()
            .or_else(|| children.iter().find_map(|child| child.executor()));
        if self.stop_settled(operation.clone()) {
            return Ok(receipt);
        }
        let executor = executor.expect("accepted child work has an execution runtime");
        let owner = Arc::clone(self);
        let task = executor.spawn(
            async move {
                loop {
                    // Every child publishes to this narrow notification token too.
                    // A child checked earlier may acquire independent metadata/load
                    // work while we await a later sibling; its completion must wake
                    // the aggregate without giving it access to Agent state.
                    let notified = owner.changed.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    let children = owner
                        .state
                        .lock()
                        .expect("Agent mutex poisoned")
                        .sessions
                        .values()
                        .cloned()
                        .collect::<Vec<_>>();
                    for child in children {
                        child.wait_quiescent_owned().await;
                    }
                    if owner.stop_settled(operation.clone()) {
                        return;
                    }
                    notified.await;
                }
            }
            .instrument(span)
            .with_current_subscriber(),
        );
        self.state.lock().expect("Agent mutex poisoned").stop_task = Some(task);
        Ok(receipt)
    }
    fn stop_settled(&self, operation: Option<OperationId>) -> bool {
        let children = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            if state.creating != 0 || state.info.metadata_saving || !state.close_running.is_empty()
            {
                return false;
            }
            state.sessions.values().cloned().collect::<Vec<_>>()
        };
        if !children.iter().all(|child| child.is_quiescent_owned()) {
            return false;
        }
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        if state.creating != 0 || state.info.metadata_saving || !state.close_running.is_empty() {
            return false;
        }
        let diagnostic = if state.info.lifecycle == AgentLifecycle::Stopping {
            state.info.lifecycle = AgentLifecycle::Stopped;
            Some(self.lifecycle_record("agent.stopped", "stopped", state.info.mode, operation))
        } else {
            None
        };
        self.changed.notify_waiters();
        drop(state);
        crate::diagnostics::emit(diagnostic);
        true
    }
}
