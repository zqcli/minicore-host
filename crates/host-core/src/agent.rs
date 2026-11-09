use crate::control::{CallerContext, ConnectionRegistry};
use crate::session::{HistoryBackend, MemoryHistory, ResourceResolver, SessionController};
use crate::types::{AgentInfo, SessionLoadFailure};
use minicore_host_storage::{AgentRecord, AgentStore};

mod persistent;
use minicore_host_protocol::*;
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(crate) struct AgentOwner {
    pub(crate) id: AgentId,
    host_id: HostId,
    incarnation: HostIncarnation,
    access: Arc<ConnectionRegistry>,
    resources: Arc<dyn ResourceResolver>,
    state: Mutex<AgentState>,
    enabled: Arc<AtomicBool>,
    store: Option<AgentStore>,
}
struct AgentState {
    info: AgentInfo,
    sessions: BTreeMap<SessionId, Arc<SessionController>>,
    load_failures: BTreeMap<SessionId, Fault>,
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
            resources,
            enabled: Arc::new(AtomicBool::new(true)),
            store: None,
            state: Mutex::new(AgentState {
                info: AgentInfo {
                    id,
                    host_id,
                    name,
                    mode: StorageMode::Memory,
                    lifecycle: AgentLifecycle::Active,
                    config_revision: 0,
                    config,
                    default_session_id: None,
                },
                sessions: BTreeMap::new(),
                load_failures: BTreeMap::new(),
            }),
        })
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
        options: SessionOptions,
        make_default: bool,
    ) -> Result<Arc<SessionController>, Fault> {
        let id = SessionId::new()
            .map_err(|error| Fault::new(FaultKind::AgentInternalUnexpected, error.to_string()))?;
        self.access.accept(caller, || {
            self.create_session_owned(id, options, make_default)
        })
    }
    pub(crate) fn create_session_owned(
        &self,
        id: SessionId,
        options: SessionOptions,
        make_default: bool,
    ) -> Result<Arc<SessionController>, Fault> {
        if self.store.is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent Agent creation requires the asynchronous owner entry",
            ));
        }
        // Snapshot only direct-owner values, then release this lock before child
        // construction. Publication rechecks the actual lifecycle boundary.
        let config = {
            let state = self.state.lock().expect("Agent mutex poisoned");
            if state.info.lifecycle != AgentLifecycle::Active {
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
            SessionConfig::from_agent(&state.info.config, &options.overrides.unwrap_or_default())
                .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?
        };
        self.resources.validate(&config)?;
        let backend: Arc<dyn HistoryBackend> = Arc::new(MemoryHistory);
        let session = SessionController::new(
            id.clone(),
            self.id.clone(),
            self.host_id.clone(),
            self.incarnation.clone(),
            config,
            Arc::clone(&self.access),
            Arc::clone(&self.resources),
            backend,
            Vec::new(),
            Arc::clone(&self.enabled),
        );
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        if state.info.lifecycle != AgentLifecycle::Active {
            return Err(Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "Agent stopped before Session publication",
            ));
        }
        state.sessions.insert(id.clone(), Arc::clone(&session));
        if make_default {
            state.info.default_session_id = Some(id);
        }
        Ok(session)
    }
    pub(crate) fn set_default(
        &self,
        caller: &CallerContext,
        session: Option<SessionId>,
    ) -> Result<(), Fault> {
        if self.store.is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent default updates require the asynchronous owner entry",
            ));
        }
        let child = session.as_ref().map(|id| self.session(id)).transpose()?;
        self.access.accept(caller, || {
            if child.as_ref().is_some_and(|child| !child.is_open_owned()) {
                return Err(Fault::new(
                    FaultKind::AgentDefaultSessionUnavailable,
                    "default Session must be open",
                ));
            }
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if session
                .as_ref()
                .is_some_and(|id| !state.sessions.contains_key(id))
            {
                return Err(Fault::new(
                    FaultKind::AgentSessionNotFound,
                    "Session does not belong to this Agent",
                ));
            }
            state.info.default_session_id = session;
            Ok(())
        })
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
        if self.store.is_some() {
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
        self.resources.validate(
            &SessionConfig::from_agent(&config, &SessionOverrides::default())
                .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?,
        )?;
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
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
            Ok(state.info.clone())
        })
    }
    pub(crate) fn set_enabled(
        self: &Arc<Self>,
        caller: &CallerContext,
        enabled: bool,
    ) -> Result<(), Fault> {
        let children = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            if enabled && state.info.lifecycle == AgentLifecycle::Stopping {
                return Err(Fault::new(
                    FaultKind::AgentLifecycleUnavailable,
                    "Agent cannot start until its stop work has settled",
                ));
            }
            if (!enabled
                && matches!(
                    state.info.lifecycle,
                    AgentLifecycle::Stopping | AgentLifecycle::Stopped
                ))
                || (enabled && state.info.lifecycle == AgentLifecycle::Active)
            {
                return Ok(None);
            }
            self.enabled.store(enabled, Ordering::SeqCst);
            state.info.lifecycle = if enabled {
                AgentLifecycle::Active
            } else {
                AgentLifecycle::Stopping
            };
            Ok(Some(state.sessions.values().cloned().collect::<Vec<_>>()))
        })?;
        let Some(children) = children else {
            return Ok(());
        };
        for child in &children {
            child.set_agent_enabled(enabled);
        }
        if enabled {
            return Ok(());
        }
        if children.iter().all(|child| child.is_quiescent_owned()) {
            self.state
                .lock()
                .expect("Agent mutex poisoned")
                .info
                .lifecycle = AgentLifecycle::Stopped;
        } else {
            let owner = Arc::clone(self);
            // This task owns the accepted Agent stop after its caller returns.
            // Start is rejected until it has joined every direct child's work.
            let executor = children
                .iter()
                .find_map(|child| child.executor())
                .expect("active children have an execution runtime");
            executor.spawn(async move {
                for child in children {
                    child.wait_quiescent_owned().await;
                }
                let mut state = owner.state.lock().expect("Agent mutex poisoned");
                if state.info.lifecycle == AgentLifecycle::Stopping {
                    state.info.lifecycle = AgentLifecycle::Stopped;
                }
            });
        }
        Ok(())
    }
}
