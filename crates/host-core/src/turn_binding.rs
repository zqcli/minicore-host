//! Trusted capabilities bound to one active public Turn.
//! No capability is decoded from model arguments or persisted history.
use crate::{
    control::{CallerContext, ConnectionRegistry, InternalAuthority},
    session::SessionController,
    *,
};
use minicore_host_protocol::*;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

/// Factual execution identity supplied by an authenticated Turn capability.
/// This value is not authorization and cannot be deserialized from tool JSON.
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
    cleanup: Arc<Mutex<Option<ToolCleanupHandles>>>,
}
type ToolCleanupHandles = Vec<Arc<dyn crate::TurnToolCleanup>>;
impl PreparedTurnBinding {
    pub(crate) fn new(
        owner: &Arc<SessionController>,
        source: TurnSource,
        access: &Arc<ConnectionRegistry>,
    ) -> Self {
        Self {
            cleanup: Arc::new(Mutex::new(Some(Vec::new()))),
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
    /// Bind the Host's cumulative credential-name exclusions without retaining
    /// the Host itself or changing this Turn's selected execution resources.
    pub fn tool_environment_exclusions(&self) -> Result<ToolEnvironmentExclusions, Fault> {
        Ok(self
            .capability
            .host()?
            .configuration
            .tool_environment
            .clone())
    }
    /// Register actual execution owners before this candidate is claimed. The
    /// handles cannot authorize tool execution and are never run on rejection.
    pub fn register_tool_cleanup(
        &self,
        cleanup: Arc<dyn crate::TurnToolCleanup>,
    ) -> Result<(), Fault> {
        let mut handles = self
            .cleanup
            .lock()
            .expect("prepared cleanup mutex poisoned");
        handles
            .as_mut()
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionConfigInvalid,
                    "tool cleanup registration is closed for this Turn binding",
                )
            })?
            .push(cleanup);
        Ok(())
    }
    pub(crate) fn activate(&self) -> (ActivationLease, ToolCleanupHandles) {
        let cleanup = self
            .cleanup
            .lock()
            .expect("prepared cleanup mutex poisoned")
            .take()
            .expect("candidate activates only once");
        self.capability
            .authority
            .active
            .store(true, Ordering::SeqCst);
        (
            ActivationLease(Arc::clone(&self.capability.authority)),
            cleanup,
        )
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
    pub fn agent_summary(&self, target: &AgentRef) -> Result<crate::AgentSummary, Fault> {
        self.host()?.agent_summary(&self.caller()?, target)
    }
    pub fn session_snapshot(&self, target: &SessionRef) -> Result<SessionInfo, Fault> {
        self.host()?.session_info(&self.caller()?, target)
    }
    pub fn session_state(&self, target: &SessionRef) -> Result<SessionSnapshot, Fault> {
        self.host()?.session_state(&self.caller()?, target)
    }
    pub fn sessions(&self, target: &AgentRef) -> Result<Vec<crate::SessionSummary>, Fault> {
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
    pub async fn turn_result_snapshot(
        &self,
        target: &SessionRef,
        id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        self.host()?
            .turn_result_snapshot(&self.caller()?, target, id)
            .await
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
        let source = self.source();
        let context = minicore_host_diagnostics::Context {
            host_id: Some(source.host_id.clone()),
            host_incarnation: Some(source.host_incarnation.clone()),
            agent_id: Some(source.session.agent_id.clone()),
            session_id: Some(source.session.session_id.clone()),
            turn_id: Some(source.turn.turn_id.clone()),
            message_id: Some(input.message_id.clone()),
            in_reply_to: input.in_reply_to.clone(),
            ..Default::default()
        };
        let result = self
            .host()
            .and_then(|host| host.send_delegated_owned(self, registration, input));
        if let Err(fault) = &result {
            crate::diagnostics::message_error(
                context,
                &registration.child,
                &registration.task_message_id,
                None,
                "delegated_send",
                fault,
            )
            .emit();
        }
        result
    }
    pub fn record_delegation(&self, origin: &ToolCallRef, registration: &DelegationRegistration) {
        if let Some(owner) = self.authority.owner.upgrade() {
            owner.record_delegation_owned(origin, registration);
        }
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

impl TurnCapability {
    pub async fn history_read(
        &self,
        target: &SessionRef,
        request: &HistoryReadRequest,
    ) -> Result<SessionHistorySnapshot, Fault> {
        self.host()?
            .history_read(&self.caller()?, target, request)
            .await
    }
}
