use crate::{
    AgentConfigSnapshot, AgentUpdateCandidate, InspectAction, SessionConfigSnapshot,
    SessionUpdateCandidate,
};
pub use minicore_host_core::{
    DelegationCompletion, DelegationDisposition, DelegationInputRange, DelegationRegistration,
    TurnSource as ToolSource,
};
use minicore_host_protocol::*;
use serde_json::Value;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, Fault>> + Send + 'a>>;

/// Narrow owner operations bound by the backend to one admitted public Turn.
/// Implementations must check the opaque owner's active capability at each
/// acceptance boundary. An external connection CallerContext is NOT a valid
/// implementation. Nothing here exposes Host shutdown, secrets, or deletion.
pub trait TurnPort: Send + Sync + 'static {
    fn source(&self) -> &ToolSource;
    fn check_active(&self) -> Result<(), Fault>;
    fn agent_snapshot(&self, target: &AgentRef) -> Result<AgentConfigSnapshot, Fault>;
    fn session_snapshot(&self, target: &SessionRef) -> Result<SessionConfigSnapshot, Fault>;
    fn create_agent(&self, request: AgentCreate) -> PortFuture<'_, AgentCreateReceipt>;
    fn create_session(&self, request: SessionCreate) -> PortFuture<'_, SessionCreateReceipt>;
    fn create_agent_for_delegation(
        &self,
        request: AgentCreate,
    ) -> PortFuture<'_, CreatedForDelegation<AgentCreateReceipt>>;
    fn create_session_for_delegation(
        &self,
        request: SessionCreate,
    ) -> PortFuture<'_, CreatedForDelegation<SessionCreateReceipt>>;
    fn send(&self, request: SendMessage) -> PortFuture<'_, MessageAccepted>;
    fn send_delegated<'a>(
        &'a self,
        registration: &'a DelegationRegistration,
        input: MessageInput,
    ) -> PortFuture<'a, MessageAccepted>;
    fn inspect(&self, request: InspectAction) -> PortFuture<'_, Value>;
    fn update_agent(
        &self,
        target: AgentRef,
        candidate: AgentUpdateCandidate,
    ) -> PortFuture<'_, Value>;
    fn update_session(
        &self,
        target: SessionRef,
        candidate: SessionUpdateCandidate,
    ) -> PortFuture<'_, Value>;
    fn cancel_turn(
        &self,
        target: SessionRef,
        request: TurnCancelRequest,
    ) -> PortFuture<'_, TurnCancelReceipt>;
    fn cancel_session(
        &self,
        target: SessionRef,
        request: SessionCancelRequest,
    ) -> PortFuture<'_, SessionCancelReceipt>;
    fn close_session(
        &self,
        target: SessionRef,
        request: SessionCloseRequest,
    ) -> PortFuture<'_, SessionCloseReceipt>;
    /// Subscribe before checking under the Session's same claim boundary.
    /// Only consumable next-step can resolve this future. Do not consume it;
    /// next-turn/paused do not wake it. The adapter owns deadlines/cancellation.
    fn wait_input(&self) -> PortFuture<'_, ()>;
    /// Register the sink in the target owner BEFORE its task message is accepted.
    /// Registration must reject Host draining, including reuse of an old Session.
    /// No target-idle requirement. Core owns matching consumed input ranges.
    fn prepare_delegation(
        &self,
        registration: DelegationRegistration,
        sink: Arc<dyn DelegationSink>,
    ) -> PortFuture<'_, Arc<dyn DelegationPort>>;
    /// A root-owned control permit is independent of this Turn's cancellation.
    /// Core registers its closure hook with the root Session before returning.
    fn prepare_scope(&self, hooks: Arc<dyn ScopeHooks>) -> Result<Arc<dyn ScopePort>, Fault>;
    /// Actual association projection, not a second ToolCall lifecycle. The owner
    /// may keep it only while the original ToolCall remains retained.
    fn record_delegation(&self, origin: &ToolCallRef, registration: &DelegationRegistration);
}

/// Independent, non-serializable permit tied to the registered relationship,
/// not the parent's old Turn/Load/connection. The target owner enforces accepted
/// vs. consumed task identity and uses the existing standard receive path.
pub trait DelegationPort: Send + Sync + 'static {
    /// Send a new notification once to the original direct parent. Select steer
    /// if busy, follow_up if idle at the receive boundary. Respect paused,
    /// cancelling, closing, Host draining/stopping, and the scope closing token.
    fn notify(&self, input: MessageInput) -> PortFuture<'_, MessageAccepted>;
    /// Unregister an unaccepted preparation, or finish after result handoff.
    /// Never discard accepted execution or necessary save work.
    fn release(&self, disposition: DelegationDisposition) -> PortFuture<'_, ()>;
    /// Temporary members use the actual owner close/join path after handoff.
    fn close(&self, operation_id: OperationId) -> PortFuture<'_, ()>;
}

/// Registered with the root's direct owner. Root close must seal this scope
/// before accepting more delegations, then await descendant cleanup. This hook
/// never closes the root itself and therefore cannot join its own close task.
pub trait ScopeHooks: Send + Sync + 'static {
    fn closing(&self) -> CancellationToken;
    fn root_closed(&self);
    fn close_members(&self) -> PortFuture<'_, ()>;
}

/// The root scope owns this permit and the cleanup future; no tool invocation
/// awaits its own root Turn. The owner must join registered result handoffs
/// before publishing the root Session closed or Host stopped.
pub trait ScopePort: Send + Sync + 'static {
    fn seal(&self) -> Result<(), Fault>;
    fn close_root(&self, operation_id: OperationId) -> PortFuture<'_, ()>;
}

/// A narrow transfer of actual execution data, not an observation event or a
/// retained result service. The actual owner calls this only after its activity
/// has settled (including preserved inbox and necessary saves), before release.
pub trait DelegationSink: Send + Sync + 'static {
    fn handoff(&self, result: DelegationCompletion) -> Result<(), Fault>;
}

/// Only an actually created temporary memory Session grants this capability.
/// It can clean that one object after a startup call is cancelled, without
/// restoring the ended Turn's ordinary authority or accepting another task.
pub trait CreationCleanup: Send + Sync + 'static {
    fn close(&self, operation_id: OperationId) -> PortFuture<'_, ()>;
}
pub struct CreatedForDelegation<R> {
    pub receipt: R,
    pub cleanup: Option<Arc<dyn CreationCleanup>>,
}
