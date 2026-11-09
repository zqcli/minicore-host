//! Trusted, in-process capabilities for tools bound to an actual public Turn.
//! No capability can be decoded from model arguments or persisted history.
use crate::{
    control::{CallerContext, ConnectionRegistry, InternalAuthority},
    session::SessionController,
    *,
};
use minicore_host_protocol::*;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub type OwnerFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Fault>> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnSource {
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub session: SessionRef,
    pub turn: TurnRef,
}

/// A candidate binding supplied only to the trusted resource assembler. The
/// capability is inactive until the Session atomically claims this candidate.
#[derive(Clone)]
pub struct PreparedTurnBinding {
    capability: TurnCapability,
}
impl PreparedTurnBinding {
    pub(crate) fn new(
        owner: &Arc<SessionController>,
        source: TurnSource,
        access: &Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            capability: TurnCapability {
                access: Arc::downgrade(access),
                authority: Arc::new(TurnAuthority {
                    source,
                    owner: Arc::downgrade(owner),
                    active: AtomicBool::new(false),
                }),
            },
        }
    }
    pub fn source(&self) -> &TurnSource {
        self.capability.source()
    }
    pub fn capability(&self) -> TurnCapability {
        self.capability.clone()
    }
    pub(crate) fn activate(&self) -> ActivationLease {
        self.capability
            .authority
            .active
            .store(true, Ordering::SeqCst);
        ActivationLease(Arc::clone(&self.capability.authority))
    }
}

pub(crate) struct ActivationLease(Arc<TurnAuthority>);
impl ActivationLease {
    pub(crate) fn expire(&self) {
        self.0.active.store(false, Ordering::SeqCst);
    }
}
impl Drop for ActivationLease {
    fn drop(&mut self) {
        self.0.active.store(false, Ordering::SeqCst);
    }
}
struct TurnAuthority {
    source: TurnSource,
    owner: Weak<SessionController>,
    active: AtomicBool,
}
impl InternalAuthority for TurnAuthority {
    fn check(&self) -> Result<(), Fault> {
        if self.active.load(Ordering::SeqCst) && self.owner.strong_count() != 0 {
            Ok(())
        } else {
            Err(inactive())
        }
    }
    fn source(&self) -> MessageSource {
        MessageSource::Agent {
            source: self.source.session.clone(),
        }
    }
}

/// Ordinary tool operations always recheck this admitted Turn. Retaining the
/// capability does not retain its owner, connection, Load, or execution lifetime.
#[derive(Clone)]
pub struct TurnCapability {
    access: Weak<ConnectionRegistry>,
    authority: Arc<TurnAuthority>,
}
impl TurnCapability {
    pub fn source(&self) -> &TurnSource {
        &self.authority.source
    }
    fn access(&self) -> Result<Arc<ConnectionRegistry>, Fault> {
        self.access.upgrade().ok_or_else(inactive)
    }
    pub(crate) fn caller(&self) -> Result<CallerContext, Fault> {
        let access = self.access()?;
        Ok(CallerContext::internal(
            &access,
            Arc::clone(&self.authority) as Arc<dyn InternalAuthority>,
        ))
    }
    fn host(&self) -> Result<Arc<Host>, Fault> {
        self.access()?.host_owned()
    }
    pub fn check_active(&self) -> Result<(), Fault> {
        self.access()?.read(&self.caller()?, || Ok(()))
    }
    pub fn agent_snapshot(&self, target: &AgentRef) -> Result<AgentInfo, Fault> {
        self.host()?.agent_info(&self.caller()?, target)
    }
    pub fn session_snapshot(&self, target: &SessionRef) -> Result<SessionInfo, Fault> {
        self.host()?.session_info(&self.caller()?, target)
    }
    pub fn session_state(&self, target: &SessionRef) -> Result<SessionSnapshot, Fault> {
        self.host()?.session_state(&self.caller()?, target)
    }
    pub fn sessions(&self, target: &AgentRef) -> Result<Vec<SessionInfo>, Fault> {
        self.host()?.sessions(&self.caller()?, target)
    }
    pub fn message(&self, target: &SessionRef, id: &MessageId) -> Result<MessageRecord, Fault> {
        self.host()?.message(&self.caller()?, target, id)
    }
    pub fn history(
        &self,
        target: &SessionRef,
    ) -> Result<Vec<minicore_runtime::HistoryItem>, Fault> {
        self.host()?.history(&self.caller()?, target)
    }
    pub fn turn_info(&self, target: &SessionRef, id: &TurnId) -> Result<TurnInfo, Fault> {
        self.host()?.turn_info(&self.caller()?, target, id)
    }
    pub fn turn_result(&self, target: &SessionRef, id: &TurnId) -> Result<TurnResult, Fault> {
        self.host()?.turn_result(&self.caller()?, target, id)
    }
    pub fn turns(&self, target: &SessionRef) -> Result<Vec<TurnInfo>, Fault> {
        self.host()?.turns(&self.caller()?, target)
    }
    pub fn receipt(&self, id: &OperationId) -> Result<Receipt, Fault> {
        self.host()?.receipt_get(&self.caller()?, id)
    }
    pub async fn create_agent(&self, request: AgentCreate) -> Result<AgentCreateReceipt, Fault> {
        self.host()?
            .create_agent_async(&self.caller()?, request)
            .await
    }
    pub async fn create_session(
        &self,
        request: SessionCreate,
    ) -> Result<SessionCreateReceipt, Fault> {
        self.host()?
            .create_session_async(&self.caller()?, request)
            .await
    }
    pub fn send(&self, request: SendMessage) -> Result<MessageAccepted, Fault> {
        self.host()?.send(&self.caller()?, request)
    }
    pub fn cancel_turn(
        &self,
        target: &SessionRef,
        request: TurnCancelRequest,
    ) -> Result<TurnCancelReceipt, Fault> {
        self.host()?.turn_cancel(&self.caller()?, target, request)
    }
    pub fn cancel_session(
        &self,
        target: &SessionRef,
        request: SessionCancelRequest,
    ) -> Result<SessionCancelReceipt, Fault> {
        self.host()?
            .session_cancel(&self.caller()?, target, request)
    }
    pub fn close_session(
        &self,
        target: &SessionRef,
        request: SessionCloseRequest,
    ) -> Result<SessionCloseReceipt, Fault> {
        self.host()?.session_close(&self.caller()?, target, request)
    }
    /// The waiter subscribes before checking, never consumes input, and does not
    /// wake for next-turn or a paused Session. The Tool's context owns timeout
    /// and cancellation of this wait; source expiry is also observed here.
    pub async fn wait_input(&self) -> Result<(), Fault> {
        let owner = self.authority.owner.upgrade().ok_or_else(inactive)?;
        owner.wait_input_owned(self).await
    }
}
fn inactive() -> Fault {
    Fault::new(
        FaultKind::HostAccessInvalid,
        "the original bound Turn capability is not active",
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DelegationRegistration {
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub root: SessionRef,
    pub parent: SessionRef,
    pub child: SessionRef,
    pub task_message_id: MessageId,
    pub origin: ToolCallRef,
    pub temporary: bool,
}
#[derive(Clone, Debug)]
pub struct DelegationInputRange {
    pub turn: TurnRef,
    pub message_ids: Vec<MessageId>,
}
#[derive(Clone, Debug)]
pub struct DelegationCompletion {
    pub results: Vec<TurnResult>,
    pub inputs: Vec<DelegationInputRange>,
    /// Accepted inputs explicitly removed before Runtime consumed them.
    pub discarded_inputs: Vec<MessageId>,
}
pub trait DelegationSink: Send + Sync + 'static {
    /// Transfer actual completed data. This synchronous ownership transfer must
    /// not wait for the parent queue, persistence, observation, or cleanup.
    fn handoff(&self, completion: DelegationCompletion) -> Result<(), Fault>;
}
pub trait ScopeHooks: Send + Sync + 'static {
    fn seal(&self);
    fn close_members(&self) -> OwnerFuture<'_, ()>;
    fn root_closed(&self);
}
#[derive(Clone, Debug)]
pub enum DelegationDisposition {
    NotAccepted,
    Notified(MessageAccepted),
    NotificationFailed(Fault),
    ScopeClosed,
}

pub(crate) struct ScopeState {
    pub(crate) root: Weak<SessionController>,
    pub(crate) root_ref: SessionRef,
    pub(crate) access: Weak<ConnectionRegistry>,
    pub(crate) hooks: Arc<dyn ScopeHooks>,
    pub(crate) sealed: AtomicBool,
    pub(crate) cleanup: tokio::sync::OnceCell<()>,
    pub(crate) pending: AtomicUsize,
    pub(crate) notified_closed: AtomicBool,
}
impl ScopeState {
    pub(crate) fn check_open(&self) -> Result<(), Fault> {
        if self.sealed.load(Ordering::SeqCst) {
            Err(scope_closed())
        } else {
            Ok(())
        }
    }
    pub(crate) fn seal(&self) {
        self.sealed.store(true, Ordering::SeqCst);
        self.hooks.seal();
    }
    pub(crate) async fn cleanup(&self) -> Result<(), Fault> {
        self.seal();
        self.cleanup
            .get_or_try_init(|| self.hooks.close_members())
            .await
            .copied()
    }
    pub(crate) fn root_closed(&self) {
        if !self.notified_closed.swap(true, Ordering::SeqCst) {
            self.hooks.root_closed();
        }
    }
}
#[derive(Clone)]
pub struct ScopePermit {
    pub(crate) state: Arc<ScopeState>,
}
impl ScopePermit {
    pub fn seal(&self) -> Result<(), Fault> {
        let root = self.state.root.upgrade().ok_or_else(scope_closed)?;
        root.seal_scope_owned(&self.state)
    }
    pub async fn close_root(&self, operation: OperationId) -> Result<(), Fault> {
        if self
            .state
            .root
            .upgrade()
            .is_none_or(|root| root.retired_owned())
        {
            return Ok(());
        }
        self.seal()?;
        let access = self.state.access.upgrade().ok_or_else(scope_closed)?;
        let host = access.host_owned()?;
        host.close_internal_owned(&self.state.root_ref, operation)
            .await
    }
}

pub(crate) struct DelegationState {
    pub(crate) registration: DelegationRegistration,
    pub(crate) child: Weak<SessionController>,
    pub(crate) parent: Weak<SessionController>,
    pub(crate) scope: Arc<ScopeState>,
    pub(crate) access: Weak<ConnectionRegistry>,
    pub(crate) notified: AtomicBool,
    pub(crate) handed_off: AtomicBool,
    pub(crate) released: AtomicBool,
    pub(crate) close: tokio::sync::OnceCell<()>,
}
/// Only final notification and actual child cleanup outlive the initiating
/// Turn. This permit cannot dispatch another task or impersonate a connection.
#[derive(Clone)]
pub struct DelegationPermit {
    pub(crate) state: Arc<DelegationState>,
}
impl DelegationPermit {
    pub async fn notify(&self, input: MessageInput) -> Result<MessageAccepted, Fault> {
        if let Some(child) = self.state.child.upgrade() {
            child.wait_handoff_owned().await;
        }
        if !self.state.handed_off.load(Ordering::SeqCst) {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "the actual delegated result has not been handed off",
            ));
        }
        let access = self.state.access.upgrade().ok_or_else(scope_closed)?;
        let parent = self.state.parent.upgrade().ok_or_else(|| {
            Fault::new(
                FaultKind::AgentSessionNotFound,
                "original direct parent is no longer available",
            )
        })?;
        access
            .accept_internal_owned(|| {
                access.ensure_execution_owned()?;
                self.state.scope.check_open()?;
                if self.state.released.load(Ordering::SeqCst)
                    || self.state.notified.swap(true, Ordering::SeqCst)
                {
                    return Err(Fault::new(
                        FaultKind::SessionInputInvalid,
                        "this delegation notification was already attempted or released",
                    ));
                }
                if input.message_id == self.state.registration.task_message_id {
                    return Err(Fault::new(
                        FaultKind::SessionInputInvalid,
                        "a final notification requires a new MessageId",
                    ));
                }
                parent.receive_notification_owned(&self.state, input)
            })
            .inspect(|_| parent.wake_owned())
    }
    pub async fn release(&self, disposition: DelegationDisposition) -> Result<(), Fault> {
        if let Some(child) = self.state.child.upgrade() {
            child.wait_handoff_owned().await;
            child.release_delegation_owned(&self.state, disposition)?;
        }
        if !self.state.released.swap(true, Ordering::SeqCst) {
            self.state.scope.pending.fetch_sub(1, Ordering::SeqCst);
            if let Some(root) = self.state.scope.root.upgrade() {
                root.signal_collaboration_owned();
            }
        }
        Ok(())
    }
    pub async fn close(&self, operation: OperationId) -> Result<(), Fault> {
        self.state
            .close
            .get_or_try_init(|| async {
                let child = self.state.child.upgrade();
                let Some(child) = child else {
                    return Ok(());
                };
                child.wait_handoff_owned().await;
                if child.retired_owned() {
                    return Ok(());
                }
                self.state
                    .access
                    .upgrade()
                    .ok_or_else(scope_closed)?
                    .host_owned()?
                    .close_internal_owned(&self.state.registration.child, operation)
                    .await
            })
            .await
            .copied()
    }
}
impl TurnCapability {
    pub fn prepare_scope(&self, hooks: Arc<dyn ScopeHooks>) -> Result<ScopePermit, Fault> {
        let owner = self.authority.owner.upgrade().ok_or_else(inactive)?;
        self.access()?
            .accept_business(&self.caller()?, || owner.register_scope_owned(hooks))
    }
    pub fn prepare_delegation(
        &self,
        registration: DelegationRegistration,
        sink: Arc<dyn DelegationSink>,
    ) -> Result<DelegationPermit, Fault> {
        self.host()?
            .prepare_delegation_owned(self, registration, sink)
    }
    pub fn send_delegated(
        &self,
        registration: &DelegationRegistration,
        input: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        self.host()?.send_delegated_owned(self, registration, input)
    }
    pub fn record_delegation(&self, origin: &ToolCallRef, registration: &DelegationRegistration) {
        if let Some(owner) = self.authority.owner.upgrade() {
            owner.record_delegation_owned(origin, registration);
        }
    }
}
fn scope_closed() -> Fault {
    Fault::new(
        FaultKind::SessionLifecycleClosing,
        "the original delegation scope is closing or unavailable",
    )
}

/// Cleanup authority for exactly one memory Session created by the completed
/// startup workflow. It is neither ordinary Turn authority nor a compensating
/// rollback for persistent or pre-existing objects.
#[derive(Clone)]
pub struct CreationCleanupPermit {
    host: Weak<Host>,
    target: SessionRef,
}
impl CreationCleanupPermit {
    pub async fn close(&self, operation: OperationId) -> Result<(), Fault> {
        self.host
            .upgrade()
            .ok_or_else(scope_closed)?
            .close_internal_owned(&self.target, operation)
            .await
    }
}
impl TurnCapability {
    pub async fn create_agent_for_delegation(
        &self,
        request: AgentCreate,
    ) -> Result<(AgentCreateReceipt, Option<CreationCleanupPermit>), Fault> {
        if request.initial_message.is_some() {
            return Err(Fault::new(
                FaultKind::SessionInputInvalid,
                "delegation preparation must create an empty target first",
            ));
        }
        let host = self.host()?;
        let memory = request.mode == StorageMode::Memory;
        let receipt = host.create_agent_async(&self.caller()?, request).await?;
        let cleanup = if memory && receipt.result.steps.session.succeeded() {
            receipt
                .agent_id
                .clone()
                .zip(receipt.session_id.clone())
                .map(|(agent_id, session_id)| CreationCleanupPermit {
                    host: Arc::downgrade(&host),
                    target: SessionRef {
                        agent_id,
                        session_id,
                    },
                })
        } else {
            None
        };
        Ok((receipt, cleanup))
    }
    pub async fn create_session_for_delegation(
        &self,
        request: SessionCreate,
    ) -> Result<(SessionCreateReceipt, Option<CreationCleanupPermit>), Fault> {
        if request.initial_message.is_some() {
            return Err(Fault::new(
                FaultKind::SessionInputInvalid,
                "delegation preparation must create an empty target first",
            ));
        }
        let host = self.host()?;
        let agent_id = request.target.agent_id.clone();
        let mode = request
            .mode
            .unwrap_or(self.agent_snapshot(&request.target)?.mode);
        let receipt = host.create_session_async(&self.caller()?, request).await?;
        let cleanup = if mode == StorageMode::Memory && receipt.result.steps.session.succeeded() {
            receipt
                .session_id
                .clone()
                .map(|session_id| CreationCleanupPermit {
                    host: Arc::downgrade(&host),
                    target: SessionRef {
                        agent_id,
                        session_id,
                    },
                })
        } else {
            None
        };
        Ok((receipt, cleanup))
    }
    pub fn toolcall_get(
        &self,
        target: &SessionRef,
        tool: &ToolCallRef,
    ) -> Result<ToolCallSnapshot, Fault> {
        self.host()?.toolcall_get(&self.caller()?, target, tool)
    }
    pub fn toolcall_output_read(
        &self,
        target: &SessionRef,
        tool: &ToolCallRef,
        request: &TextReadRequest,
    ) -> Result<ToolTextSnapshot, Fault> {
        self.host()?
            .toolcall_output_read(&self.caller()?, target, tool, request)
    }
    pub fn turn_output_read(
        &self,
        target: &SessionRef,
        turn: &TurnId,
        response_index: u64,
        request: &TextReadRequest,
    ) -> Result<ModelTextSnapshot, Fault> {
        self.host()?
            .turn_output_read(&self.caller()?, target, turn, response_index, request)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DelegationAssociation {
    pub child: SessionRef,
    pub task_message_id: MessageId,
}
impl TurnCapability {
    pub async fn update_agent(
        &self,
        target: &AgentRef,
        expected_revision: u64,
        config: AgentConfig,
        name: String,
    ) -> Result<AgentInfo, Fault> {
        self.host()?
            .update_agent_candidate_owned(&self.caller()?, target, expected_revision, config, name)
            .await
    }
    pub async fn update_session(
        &self,
        target: &SessionRef,
        expected_revision: u64,
        config: SessionConfig,
        title: Option<String>,
    ) -> Result<SessionInfo, Fault> {
        self.host()?
            .update_session_candidate_owned(
                &self.caller()?,
                target,
                expected_revision,
                config,
                title,
            )
            .await
    }
}
