use minicore_host_protocol::{
    ConnectionInfo, ControlStateData, Fault, FaultKind, HostId, HostIncarnation, HostLifecycle,
};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// Trusted context created by a live access handle. It cannot be deserialized
/// from business JSON, and retaining it does not keep the connection valid.
#[derive(Clone)]
pub struct CallerContext {
    registry: Weak<ConnectionRegistry>,
    identity: Arc<ConnectionIdentity>,
}
impl CallerContext {
    pub(crate) fn revocation(&self) -> tokio::sync::watch::Receiver<bool> {
        self.identity.validity.subscribe()
    }
}
impl std::fmt::Debug for CallerContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallerContext").finish_non_exhaustive()
    }
}

/// One trusted in-process connection. Drop or close invalidates even cloned
/// caller contexts. Existing read-only handles are never promoted.
pub struct ConnectionHandle {
    registry: Arc<ConnectionRegistry>,
    identity: Arc<ConnectionIdentity>,
    initial: ConnectionInfo,
}
impl ConnectionHandle {
    pub fn caller(&self) -> CallerContext {
        CallerContext {
            registry: Arc::downgrade(&self.registry),
            identity: Arc::clone(&self.identity),
        }
    }
    pub fn initial_info(&self) -> &ConnectionInfo {
        &self.initial
    }
    pub fn close(&self) {
        self.registry.close(&self.identity);
    }
}
impl Drop for ConnectionHandle {
    fn drop(&mut self) {
        self.close();
    }
}

struct ConnectionIdentity {
    number: u64,
    validity: tokio::sync::watch::Sender<bool>,
}
struct RegistryState {
    next_number: u64,
    connections: HashMap<u64, Weak<ConnectionIdentity>>,
    operator: Option<u64>,
    revision: u64,
}

pub(crate) struct ConnectionRegistry {
    host_id: HostId,
    incarnation: HostIncarnation,
    state: Mutex<RegistryState>,
    // Admission fences, not a second lifecycle owner. Updated only by Host.
    host_phase: AtomicU8,
    execution_enabled: Arc<AtomicBool>,
}
impl ConnectionRegistry {
    pub(crate) fn new(host_id: HostId, incarnation: HostIncarnation) -> Arc<Self> {
        Arc::new(Self {
            host_id,
            incarnation,
            host_phase: AtomicU8::new(0),
            execution_enabled: Arc::new(AtomicBool::new(true)),
            state: Mutex::new(RegistryState {
                next_number: 0,
                connections: HashMap::new(),
                operator: None,
                revision: 0,
            }),
        })
    }
    pub(crate) fn connect(self: &Arc<Self>) -> Result<ConnectionHandle, Fault> {
        let mut state = self.state.lock().expect("connection mutex poisoned");
        let number = state.next_number;
        state.next_number = number.checked_add(1).ok_or_else(|| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "connection sequence exhausted",
            )
        })?;
        let can_operate = state.operator.is_none();
        if can_operate {
            state.revision = state.revision.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::HostInternalUnexpected,
                    "control revision exhausted",
                )
            })?;
            state.operator = Some(number);
        }
        let (validity, _) = tokio::sync::watch::channel(true);
        let identity = Arc::new(ConnectionIdentity { number, validity });
        state.connections.insert(number, Arc::downgrade(&identity));
        let initial = ConnectionInfo {
            host_id: self.host_id.clone(),
            host_incarnation: self.incarnation.clone(),
            can_operate,
            control: ControlStateData {
                occupied: state.operator.is_some(),
                revision: state.revision,
            },
        };
        Ok(ConnectionHandle {
            registry: Arc::clone(self),
            identity,
            initial,
        })
    }
    fn close(&self, identity: &Arc<ConnectionIdentity>) {
        let mut state = self.state.lock().expect("connection mutex poisoned");
        if !state
            .connections
            .get(&identity.number)
            .and_then(Weak::upgrade)
            .is_some_and(|stored| Arc::ptr_eq(&stored, identity))
        {
            return;
        }
        identity.validity.send_replace(false);
        state.connections.remove(&identity.number);
        if state.operator == Some(identity.number) {
            state.operator = None;
            state.revision = state
                .revision
                .checked_add(1)
                .expect("control revision exhausted");
        }
    }
    fn check(
        &self,
        state: &RegistryState,
        caller: &CallerContext,
        write: bool,
    ) -> Result<(), Fault> {
        let same_registry = caller
            .registry
            .upgrade()
            .is_some_and(|registry| std::ptr::eq(registry.as_ref(), self));
        let alive = state
            .connections
            .get(&caller.identity.number)
            .and_then(Weak::upgrade)
            .is_some_and(|identity| Arc::ptr_eq(&identity, &caller.identity));
        if !same_registry || !alive {
            return Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "the original connection is no longer valid for this Host",
            ));
        }
        if write && state.operator != Some(caller.identity.number) {
            return Err(Fault::new(
                FaultKind::HostAccessReadOnly,
                "this connection has read-only access",
            ));
        }
        Ok(())
    }
    /// This is the shared linearization boundary for connection invalidation
    /// and direct-owner acceptance. The closure must be short and synchronous.
    /// Lock order: this registry, then a single direct owner's state.
    pub(crate) fn accept<T>(
        &self,
        caller: &CallerContext,
        action: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        let state = self.state.lock().expect("connection mutex poisoned");
        self.check(&state, caller, true)?;
        action()
    }
    /// Root business admission shares the connection/Host shutdown boundary.
    pub(crate) fn accept_business<T>(
        &self,
        caller: &CallerContext,
        action: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        self.accept(caller, || {
            self.ensure_business_owned()?;
            action()
        })
    }
    /// Controls may drain accepted work, but cannot restart stopped execution.
    pub(crate) fn accept_execution<T>(
        &self,
        caller: &CallerContext,
        action: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        self.accept(caller, || {
            self.ensure_execution_owned()?;
            action()
        })
    }
    pub(crate) fn ensure_business_owned(&self) -> Result<(), Fault> {
        if self.host_phase.load(Ordering::SeqCst) != 0 {
            Err(Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Host no longer accepts new business work",
            ))
        } else {
            Ok(())
        }
    }
    pub(crate) fn ensure_execution_owned(&self) -> Result<(), Fault> {
        if self.execution_allowed_owned() {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Host has stopped business execution",
            ))
        }
    }
    pub(crate) fn execution_allowed_owned(&self) -> bool {
        self.execution_enabled.load(Ordering::SeqCst)
    }
    // Caller holds the Host boundary (and the registry acceptance boundary for
    // a client request). Children only consume these lock-free fences.
    pub(crate) fn set_host_lifecycle_owned(&self, lifecycle: HostLifecycle) {
        let phase = match lifecycle {
            HostLifecycle::Running => 0,
            HostLifecycle::Draining => 1,
            HostLifecycle::Starting | HostLifecycle::Stopping | HostLifecycle::Stopped => 2,
        };
        self.host_phase.store(phase, Ordering::SeqCst);
        self.execution_enabled.store(phase < 2, Ordering::SeqCst);
    }
    pub(crate) fn read<T>(
        &self,
        caller: &CallerContext,
        action: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        let state = self.state.lock().expect("connection mutex poisoned");
        self.check(&state, caller, false)?;
        action()
    }
    pub(crate) fn snapshot(&self, caller: &CallerContext) -> Result<ControlStateData, Fault> {
        let state = self.state.lock().expect("connection mutex poisoned");
        self.check(&state, caller, false)?;
        Ok(ControlStateData {
            occupied: state.operator.is_some(),
            revision: state.revision,
        })
    }
}
