use minicore_host_protocol::{
    ConnectionInfo, ControlStateData, Fault, FaultKind, HostId, HostIncarnation, HostLifecycle,
};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU8, Ordering},
};
use subtle::ConstantTimeEq;

/// Trusted context created by a live access handle. It cannot be deserialized
/// from business JSON, and retaining it does not keep the connection valid.
#[derive(Clone)]
pub struct CallerContext {
    registry: Weak<ConnectionRegistry>,
    identity: Arc<ConnectionIdentity>,
}
impl CallerContext {
    pub(crate) fn connection_number(&self) -> u64 {
        self.identity.number
    }
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
    /// Resolves when this original connection is invalidated. Transport cleanup
    /// observes this signal; authority is already revoked at the owner boundary.
    pub fn closed(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        let mut validity = self.identity.validity.subscribe();
        async move {
            if *validity.borrow() {
                let _ = validity.changed().await;
            }
        }
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

/// Configured credentials never participate in public serialization or Debug.
struct AccessKey(String);
impl std::fmt::Debug for AccessKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessKey([REDACTED])")
    }
}
impl AccessKey {
    fn new(value: String) -> Result<Self, Fault> {
        if value.trim().is_empty() {
            return Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "network configuration requires a nonempty access key",
            ));
        }
        Ok(Self(value))
    }
    fn matches(&self, supplied: &str) -> bool {
        bool::from(self.0.as_bytes().ct_eq(supplied.as_bytes()))
    }
}

struct ConnectionIdentity {
    number: u64,
    network: bool,
    validity: tokio::sync::watch::Sender<bool>,
}
struct RegistryState {
    network_key: Option<AccessKey>,
    next_number: u64,
    connections: HashMap<u64, Weak<ConnectionIdentity>>,
    operator: Option<u64>,
    revision: u64,
}

pub(crate) struct ConnectionRegistry {
    pub(crate) observation: Arc<crate::observation::ObservationHub>,
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
            observation: crate::observation::ObservationHub::new(),
            host_id,
            incarnation,
            host_phase: AtomicU8::new(0),
            execution_enabled: Arc::new(AtomicBool::new(true)),
            state: Mutex::new(RegistryState {
                network_key: None,
                next_number: 0,
                connections: HashMap::new(),
                operator: None,
                revision: 0,
            }),
        })
    }
    pub(crate) fn connect(self: &Arc<Self>) -> Result<ConnectionHandle, Fault> {
        let mut state = self.state.lock().expect("connection mutex poisoned");
        self.register(&mut state, false)
    }
    pub(crate) fn connect_network(
        self: &Arc<Self>,
        supplied_key: &str,
    ) -> Result<ConnectionHandle, Fault> {
        let mut state = self.state.lock().expect("connection mutex poisoned");
        if !state
            .network_key
            .as_ref()
            .is_some_and(|key| key.matches(supplied_key))
        {
            return Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "network authentication failed",
            ));
        }
        self.register(&mut state, true)
    }
    pub(crate) fn require_network_enabled(&self) -> Result<(), Fault> {
        let state = self.state.lock().expect("connection mutex poisoned");
        if state.network_key.is_none() {
            return Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "network access is not configured",
            ));
        }
        Ok(())
    }
    /// The embedding owner prepares/validates configuration outside this lock.
    /// `publish` must only check versions and commit prepared state: no I/O,
    /// child calls, or reentry. A failed commit leaves authentication unchanged.
    pub(crate) fn publish_network<T>(
        &self,
        caller: Option<&CallerContext>,
        candidate: Option<String>,
        publish: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        let candidate = candidate.map(AccessKey::new).transpose()?;
        let mut state = self.state.lock().expect("connection mutex poisoned");
        if let Some(caller) = caller {
            self.check(&state, caller, true)?;
        }
        let changed = match (&state.network_key, &candidate) {
            (None, None) => false,
            (Some(old), Some(new)) => !old.matches(&new.0),
            _ => true,
        };
        let result = publish()?;
        if changed {
            state.network_key = candidate;
            let mut released_operator = false;
            let operator = state.operator;
            state.connections.retain(|number, stored| {
                if let Some(identity) = stored.upgrade() {
                    if !identity.network {
                        return true;
                    }
                    identity.validity.send_replace(false);
                    self.observation.close_connection(identity.number);
                }
                released_operator |= operator == Some(*number);
                false
            });
            if released_operator {
                state.operator = None;
                state.revision = state
                    .revision
                    .checked_add(1)
                    .expect("control revision exhausted");
            }
        }
        Ok(result)
    }
    fn register(
        self: &Arc<Self>,
        state: &mut RegistryState,
        network: bool,
    ) -> Result<ConnectionHandle, Fault> {
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
        let identity = Arc::new(ConnectionIdentity {
            number,
            network,
            validity,
        });
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
        self.observation.close_connection(identity.number);
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
    pub(crate) fn control_state_owned(&self) -> ControlStateData {
        let state = self.state.lock().expect("connection mutex poisoned");
        ControlStateData {
            occupied: state.operator.is_some(),
            revision: state.revision,
        }
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

#[cfg(test)]
mod network_tests {
    use super::*;
    fn registry() -> Arc<ConnectionRegistry> {
        ConnectionRegistry::new(HostId::new().unwrap(), HostIncarnation::new().unwrap())
    }
    const FIRST: &str = "fixture-first-network-access-key";
    const SECOND: &str = "fixture-second-network-access-key";

    #[test]
    fn disabled_or_invalid_configuration_never_authenticates_and_secrets_are_redacted() {
        let registry = registry();
        assert!(registry.require_network_enabled().is_err());
        assert!(registry.connect_network("").is_err());
        for invalid in ["", " \t\n"] {
            assert!(
                registry
                    .publish_network(None, Some(invalid.into()), || Ok(()))
                    .is_err()
            );
        }
        let key = AccessKey::new(FIRST.into()).unwrap();
        assert_eq!(format!("{key:?}"), "AccessKey([REDACTED])");
        registry
            .publish_network(None, Some(FIRST.into()), || Ok(()))
            .unwrap();
        assert!(registry.connect_network("").is_err());
        assert!(registry.connect_network(SECOND).is_err());
        let operator = registry.connect_network(FIRST).unwrap();
        assert!(operator.initial_info().can_operate);
    }

    #[test]
    fn publication_failure_and_same_key_preserve_original_connections() {
        let registry = registry();
        registry
            .publish_network(None, Some(FIRST.into()), || Ok(()))
            .unwrap();
        let operator = registry.connect_network(FIRST).unwrap();
        let before = registry.snapshot(&operator.caller()).unwrap();
        let failure = Fault::new(
            FaultKind::HostMethodUnavailable,
            "fixture candidate cannot be published",
        );
        assert_eq!(
            registry.publish_network(
                Some(&operator.caller()),
                Some(SECOND.into()),
                || Err::<(), _>(failure.clone())
            ),
            Err(failure)
        );
        assert!(registry.connect_network(SECOND).is_err());
        assert_eq!(
            registry.publish_network(Some(&operator.caller()), Some(FIRST.into()), || Ok(42)),
            Ok(42)
        );
        assert_eq!(registry.snapshot(&operator.caller()).unwrap(), before);
        assert!(registry.accept(&operator.caller(), || Ok(())).is_ok());
        assert!(
            !registry
                .connect_network(FIRST)
                .unwrap()
                .initial_info()
                .can_operate
        );
    }

    #[tokio::test]
    async fn successful_key_commit_invalidates_network_only_before_returning() {
        let registry = registry();
        registry
            .publish_network(None, Some(FIRST.into()), || Ok(()))
            .unwrap();
        let operator = registry.connect_network(FIRST).unwrap();
        let observer = registry.connect_network(FIRST).unwrap();
        let local = registry.connect().unwrap();
        let revocation = operator.closed();
        let mut published = false;
        registry
            .publish_network(Some(&operator.caller()), Some(SECOND.into()), || {
                published = true;
                Ok(())
            })
            .unwrap();
        assert!(published);
        revocation.await;
        for caller in [operator.caller(), observer.caller()] {
            assert_eq!(
                registry.read(&caller, || Ok(())).unwrap_err().kind(),
                Some(FaultKind::HostAccessInvalid)
            );
        }
        let control = registry.snapshot(&local.caller()).unwrap();
        assert!(!control.occupied);
        assert_eq!(control.revision, 2);
        let replacement = registry.connect_network(SECOND).unwrap();
        assert!(replacement.initial_info().can_operate);
        operator.close();
        observer.close();
        assert_eq!(
            registry.snapshot(&replacement.caller()).unwrap().revision,
            3
        );
        assert_eq!(
            registry
                .accept(&local.caller(), || Ok(()))
                .unwrap_err()
                .kind(),
            Some(FaultKind::HostAccessReadOnly)
        );
    }

    #[test]
    fn local_operator_survives_rotation_and_readonly_cannot_publish() {
        let registry = registry();
        let local = registry.connect().unwrap();
        registry
            .publish_network(None, Some(FIRST.into()), || Ok(()))
            .unwrap();
        let network = registry.connect_network(FIRST).unwrap();
        let mut called = false;
        assert_eq!(
            registry
                .publish_network(Some(&network.caller()), Some(SECOND.into()), || {
                    called = true;
                    Ok(())
                })
                .unwrap_err()
                .kind(),
            Some(FaultKind::HostAccessReadOnly)
        );
        assert!(!called);
        registry
            .publish_network(Some(&local.caller()), Some(SECOND.into()), || Ok(()))
            .unwrap();
        assert_eq!(registry.snapshot(&local.caller()).unwrap().revision, 1);
        assert!(registry.accept(&local.caller(), || Ok(())).is_ok());
        assert!(
            !registry
                .connect_network(SECOND)
                .unwrap()
                .initial_info()
                .can_operate
        );
    }

    #[test]
    fn acceptance_and_key_commit_have_one_linearization_boundary() {
        let registry = registry();
        registry
            .publish_network(None, Some(FIRST.into()), || Ok(()))
            .unwrap();
        let network = registry.connect_network(FIRST).unwrap();
        let (accepted_tx, accepted_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let caller = network.caller();
        let accepting_registry = registry.clone();
        let accepted = std::thread::spawn(move || {
            accepting_registry.accept(&caller, || {
                accepted_tx.send(()).unwrap();
                finish_rx.recv().unwrap();
                Ok("owner accepted before publication")
            })
        });
        accepted_rx.recv().unwrap();
        let publishing_registry = registry.clone();
        let (commit_tx, commit_rx) = std::sync::mpsc::channel();
        let publishing = std::thread::spawn(move || {
            publishing_registry
                .publish_network(None, Some(SECOND.into()), || {
                    commit_tx.send(()).unwrap();
                    Ok(())
                })
                .unwrap();
        });
        assert!(commit_rx.try_recv().is_err());
        finish_tx.send(()).unwrap();
        assert_eq!(
            accepted.join().unwrap().unwrap(),
            "owner accepted before publication"
        );
        publishing.join().unwrap();
        commit_rx.recv().unwrap();
        assert_eq!(
            registry
                .accept(&network.caller(), || Ok(()))
                .unwrap_err()
                .kind(),
            Some(FaultKind::HostAccessInvalid)
        );
    }
}
