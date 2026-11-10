//! Delegated completion, scope and cleanup permits that may outlive a Turn.
use crate::{control::ConnectionRegistry, session::SessionController, *};
use minicore_host_protocol::*;
use serde::Serialize;
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
#[derive(Clone, Debug, Serialize)]
pub struct DelegationInputRange {
    pub turn: TurnRef,
    pub message_ids: Vec<MessageId>,
    /// Inputs in this shared Turn that the Session accepted for this delegation.
    /// Includes real bindings cancelled before the first model request.
    pub delegated_message_ids: Vec<MessageId>,
}
#[derive(Clone, Debug)]
pub struct DelegationCompletion {
    /// Actual results for the input range, including failed/cancelled facts.
    /// Reuses the owner result and Runtime history; no fabricated final text.
    pub results: Vec<TurnResult>,
    /// Real Turn input ranges. Normally consumed inputs; a task bound before
    /// cancellation of the first model request also belongs to that real Turn.
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
        let registration = &self.state.registration;
        let context = minicore_host_diagnostics::Context {
            host_id: Some(registration.host_id.clone()),
            host_incarnation: Some(registration.host_incarnation.clone()),
            agent_id: Some(registration.child.agent_id.clone()),
            session_id: Some(registration.child.session_id.clone()),
            message_id: Some(input.message_id.clone()),
            in_reply_to: input.in_reply_to.clone(),
            ..Default::default()
        };
        let result = async {
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
            let (receipt, record) = access.accept_internal_owned(|| {
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
            })?;
            record.emit();
            parent.wake_owned();
            Ok(receipt)
        }
        .await;
        if let Err(fault) = &result {
            crate::diagnostics::message_error(
                context,
                &registration.parent,
                &registration.task_message_id,
                Some(&registration.origin),
                "delegation_notify",
                fault,
            )
            .emit();
        }
        result
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
    pub(crate) host: Weak<Host>,
    pub(crate) target: SessionRef,
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DelegationAssociation {
    pub child: SessionRef,
    pub task_message_id: MessageId,
}
