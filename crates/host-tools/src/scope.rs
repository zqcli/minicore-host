use crate::*;
use minicore_host_protocol::*;
use serde::Serialize;
use std::time::Instant;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Notify, OnceCell};
use tokio_util::sync::CancellationToken;

/// One adapter instance belongs to one Host incarnation. Only active scope
/// membership and pending handoffs live here, not a Host task/result table.
#[derive(Default)]
pub struct ScopeManager {
    state: Mutex<Registry>,
}
#[derive(Default)]
struct Registry {
    host: Option<(HostId, HostIncarnation)>,
    roots: HashMap<SessionRef, Arc<Scope>>,
    members: HashMap<SessionRef, Weak<Scope>>,
}
struct Scope {
    root: SessionRef,
    manager: Weak<ScopeManager>,
    closing: CancellationToken,
    permit: OnceCell<Arc<dyn ScopePort>>,
    members: Mutex<HashMap<SessionRef, Arc<Member>>>,
    cleanup_gate: tokio::sync::Mutex<()>,
    cleanup_done: AtomicBool,
}
struct Member {
    registration: DelegationRegistration,
    depth: usize,
    scope: Weak<Scope>,
    permit: OnceCell<Arc<dyn DelegationPort>>,
    dispatch: tokio::sync::Mutex<()>,
    accepted: AtomicBool,
    finishing: AtomicBool,
    settled: AtomicBool,
    finish_result: Mutex<Option<Result<(), Fault>>>,
    done: Notify,
}
struct Sink(Weak<Member>);

#[derive(Clone, Debug, Serialize)]
pub struct ScopeCloseAccepted {
    pub operation_id: OperationId,
    pub accepted: bool,
}

/// Invocation-local cancellation stays separate from the admitted Turn and
/// from independent child delivery permits. The Runtime cancels this token
/// before dropping a timed-out/cancelled tool future.
pub struct DispatchContext {
    origin: ToolCallRef,
    cancellation: Option<CancellationToken>,
    deadline: Option<Instant>,
}
impl DispatchContext {
    pub fn new(origin: ToolCallRef, context: &minicore_runtime::tools::ToolContext) -> Self {
        Self {
            origin,
            cancellation: Some(context.cancellation.clone()),
            deadline: Some(context.deadline),
        }
    }
    pub fn check(&self) -> Result<(), Fault> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Err(Fault::new(
                FaultKind::SessionInputInvalid,
                "startup tool was cancelled or reached its deadline before task acceptance",
            ))
        } else {
            Ok(())
        }
    }
}
impl From<ToolCallRef> for DispatchContext {
    fn from(origin: ToolCallRef) -> Self {
        Self {
            origin,
            cancellation: None,
            deadline: None,
        }
    }
}

impl ScopeManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn scope(self: &Arc<Self>, port: &Arc<dyn TurnPort>) -> Result<Arc<Scope>, Fault> {
        port.check_active()?;
        let source = port.source();
        let scope = {
            let mut registry = self.state.lock().expect("scope registry poisoned");
            let identity = (source.host_id.clone(), source.host_incarnation.clone());
            if registry.host.as_ref().is_some_and(|host| host != &identity) {
                return Err(Fault::new(
                    FaultKind::HostOwnershipMismatch,
                    "scope manager belongs to another Host incarnation",
                ));
            }
            registry.host.get_or_insert(identity);
            if let Some(scope) = registry
                .members
                .get(&source.session)
                .and_then(Weak::upgrade)
            {
                scope
            } else if let Some(scope) = registry.roots.get(&source.session) {
                Arc::clone(scope)
            } else {
                let scope = Arc::new(Scope {
                    root: source.session.clone(),
                    manager: Arc::downgrade(self),
                    closing: CancellationToken::new(),
                    permit: OnceCell::new(),
                    members: Mutex::new(HashMap::new()),
                    cleanup_gate: tokio::sync::Mutex::new(()),
                    cleanup_done: AtomicBool::new(false),
                });
                registry
                    .roots
                    .insert(source.session.clone(), Arc::clone(&scope));
                scope
            }
        };
        scope.ensure_open()?;
        let prepared = scope
            .permit
            .get_or_try_init(|| async {
                port.prepare_scope(Arc::clone(&scope) as Arc<dyn ScopeHooks>)
            })
            .await;
        if let Err(error) = prepared {
            scope.closing.cancel();
            self.remove_root(&scope);
            return Err(error);
        }
        scope.ensure_open()?;
        Ok(scope)
    }

    /// Returns the original relation for an existing member of this caller's
    /// scope. The caller cannot move that member to a new parent or root.
    fn existing(
        &self,
        source: &SessionRef,
        child: &SessionRef,
    ) -> Result<Option<Arc<Member>>, Fault> {
        let registry = self.state.lock().expect("scope registry poisoned");
        let Some(child_scope) = registry.members.get(child).and_then(Weak::upgrade) else {
            return Ok(None);
        };
        let source_scope = registry
            .members
            .get(source)
            .and_then(Weak::upgrade)
            .or_else(|| registry.roots.get(source).cloned());
        if !source_scope.is_some_and(|scope| Arc::ptr_eq(&scope, &child_scope)) {
            return Err(Fault::new(
                FaultKind::SessionOwnershipMismatch,
                "Session already belongs to another delegation scope",
            ));
        }
        child_scope.ensure_open()?;
        let member = child_scope
            .members
            .lock()
            .expect("scope members poisoned")
            .get(child)
            .cloned();
        Ok(member)
    }

    /// Ordinary messages to an existing relation keep the existing receiver.
    /// `delegate:false` neither revokes that relationship nor creates a new one.
    pub async fn send(
        self: &Arc<Self>,
        port: Arc<dyn TurnPort>,
        dispatch: impl Into<DispatchContext>,
        target: SessionRef,
        input: MessageInput,
        delegate: bool,
        temporary: bool,
    ) -> Result<MessageAccepted, Fault> {
        let dispatch = dispatch.into();
        dispatch.check()?;
        let origin = dispatch.origin.clone();
        port.check_active()?;
        if delegate && target == port.source().session {
            return Err(Fault::new(
                FaultKind::SessionOwnershipMismatch,
                "a Session cannot delegate to itself",
            ));
        }
        let existing = match self.existing(&port.source().session, &target) {
            Err(error)
                if !delegate && error.kind() == Some(FaultKind::SessionOwnershipMismatch) =>
            {
                None
            }
            result => result?,
        };
        let member = match existing {
            Some(member) => member,
            None if !delegate => return port.send(SendMessage::new(input, target)).await,
            None => {
                let scope = self.scope(&port).await?;
                let source = port.source();
                let mut registry = self.state.lock().expect("scope registry poisoned");
                scope.ensure_open()?;
                if registry.roots.contains_key(&target) {
                    return Err(Fault::new(
                        FaultKind::SessionOwnershipMismatch,
                        "a root scope holder cannot be its own delegated member",
                    ));
                }
                if let Some(other) = registry.members.get(&target).and_then(Weak::upgrade) {
                    if !Arc::ptr_eq(&scope, &other) {
                        return Err(Fault::new(
                            FaultKind::SessionOwnershipMismatch,
                            "Session already belongs to another delegation scope",
                        ));
                    }
                    let members = scope.members.lock().expect("scope members poisoned");
                    Arc::clone(members.get(&target).expect("registered member"))
                } else {
                    let mut members = scope.members.lock().expect("scope members poisoned");
                    let depth = members
                        .get(&source.session)
                        .map_or(1, |parent| parent.depth + 1);
                    let member = Arc::new(Member {
                        registration: DelegationRegistration {
                            host_id: source.host_id.clone(),
                            host_incarnation: source.host_incarnation.clone(),
                            root: scope.root.clone(),
                            parent: source.session.clone(),
                            child: target.clone(),
                            task_message_id: input.message_id.clone(),
                            origin: origin.clone(),
                            temporary,
                        },
                        depth,
                        scope: Arc::downgrade(&scope),
                        permit: OnceCell::new(),
                        dispatch: tokio::sync::Mutex::new(()),
                        accepted: AtomicBool::new(false),
                        finishing: AtomicBool::new(false),
                        settled: AtomicBool::new(false),
                        finish_result: Mutex::new(None),
                        done: Notify::new(),
                    });
                    members.insert(target.clone(), Arc::clone(&member));
                    registry.members.insert(target, Arc::downgrade(&scope));
                    member
                }
            }
        };
        let _dispatch = member.dispatch.lock().await;
        let scope = member.scope.upgrade().ok_or_else(closed)?;
        if let Err(error) = dispatch.check().and_then(|()| scope.ensure_open()) {
            if !member.accepted.load(Ordering::Acquire) && !member.finishing.load(Ordering::Acquire)
            {
                if let Some(permit) = member.permit.get() {
                    permit.release(DelegationDisposition::NotAccepted).await?;
                }
                member.finish(Ok(()));
            }
            return Err(error);
        }
        if member.finishing.load(Ordering::Acquire) || member.settled.load(Ordering::Acquire) {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "this delegation has already handed off its final result",
            ));
        }
        let prepared = member
            .permit
            .get_or_try_init(|| async {
                port.prepare_delegation(
                    member.registration.clone(),
                    Arc::new(Sink(Arc::downgrade(&member))),
                )
                .await
            })
            .await;
        let permit = match prepared {
            Ok(permit) => Arc::clone(permit),
            Err(error) => {
                member.finish(Err(error.clone()));
                self.remove_member(&member);
                return Err(error);
            }
        };
        let result = async {
            scope.ensure_open()?;
            dispatch.check()?;
            port.check_active()?;
            port.send_delegated(&member.registration, input).await
        }
        .await;
        if result.is_ok() {
            member.accepted.store(true, Ordering::Release);
            // Projection follows the real acceptance and must never turn an
            // accepted message into an error implying non-execution. The owner
            // reports a projection fault through its observation diagnostics.
            port.record_delegation(&origin, &member.registration);
        } else if !member.finishing.load(Ordering::Acquire) {
            // The owner's reject-before-accept contract makes this safe only for
            // a first task; additional rejected sends leave the original alive.
            if !member.accepted.load(Ordering::Acquire) {
                permit.release(DelegationDisposition::NotAccepted).await?;
                member.finish(Ok(()));
            }
        }
        result
    }

    /// Seal synchronously, then let an independent owner task close descendants
    /// and the root. Awaiting root close here would join this very ToolCall.
    pub fn close_scope(self: &Arc<Self>, port: &dyn TurnPort) -> Result<ScopeCloseAccepted, Fault> {
        port.check_active()?;
        let scope = {
            let registry = self.state.lock().expect("scope registry poisoned");
            registry
                .roots
                .get(&port.source().session)
                .cloned()
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionOwnershipMismatch,
                        "caller is not a root scope holder",
                    )
                })?
        };
        let operation_id = OperationId::new().map_err(id_fault)?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "scope cleanup requires a Tokio runtime",
            )
        })?;
        scope.permit.get().ok_or_else(closed)?.seal()?;
        scope.closing.cancel();
        let operation = operation_id.clone();
        runtime.spawn(async move {
            // The root owner invokes the registered cleanup hook once. A failed
            // close is not immediately retried by a second adapter cleanup pass.
            if let Some(permit) = scope.permit.get() {
                let _ = permit.close_root(operation).await;
            }
        });
        Ok(ScopeCloseAccepted {
            operation_id,
            accepted: true,
        })
    }

    fn remove_root(&self, scope: &Arc<Scope>) {
        let mut registry = self.state.lock().expect("scope registry poisoned");
        if registry
            .roots
            .get(&scope.root)
            .is_some_and(|current| Arc::ptr_eq(current, scope))
        {
            registry.roots.remove(&scope.root);
        }
    }
    fn remove_member(&self, member: &Arc<Member>) {
        let Some(scope) = member.scope.upgrade() else {
            return;
        };
        let mut registry = self.state.lock().expect("scope registry poisoned");
        let mut members = scope.members.lock().expect("scope members poisoned");
        if members
            .get(&member.registration.child)
            .is_some_and(|current| Arc::ptr_eq(current, member))
        {
            members.remove(&member.registration.child);
            registry.members.remove(&member.registration.child);
        }
    }
}
impl Scope {
    fn ensure_open(&self) -> Result<(), Fault> {
        if self.closing.is_cancelled() {
            Err(closed())
        } else {
            Ok(())
        }
    }
}
impl ScopeHooks for Scope {
    fn root_closed(&self) {
        if let Some(manager) = self.manager.upgrade() {
            let mut registry = manager.state.lock().expect("scope registry poisoned");
            if registry
                .roots
                .get(&self.root)
                .is_some_and(|scope| std::ptr::eq(scope.as_ref(), self))
            {
                registry.roots.remove(&self.root);
            }
        }
    }
    fn closing(&self) -> CancellationToken {
        self.closing.clone()
    }
    fn close_members(&self) -> PortFuture<'_, ()> {
        self.closing.cancel();
        Box::pin(async move {
            let _gate = self.cleanup_gate.lock().await;
            if self.cleanup_done.load(Ordering::Acquire) {
                return Ok(());
            }
            let mut members = self
                .members
                .lock()
                .expect("scope members poisoned")
                .values()
                .cloned()
                .collect::<Vec<_>>();
            members.sort_by_key(|member| std::cmp::Reverse(member.depth));
            let mut first_error = None;
            for member in members {
                if let Err(error) = member.stop().await {
                    first_error.get_or_insert(error);
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
            self.cleanup_done.store(true, Ordering::Release);
            Ok(())
        })
    }
}
impl Member {
    async fn wait_done(&self) -> Result<(), Fault> {
        loop {
            let changed = self.done.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.settled.load(Ordering::Acquire) {
                return self
                    .finish_result
                    .lock()
                    .expect("member result poisoned")
                    .clone()
                    .expect("settled result");
            }
            changed.await;
        }
    }
    fn finish(self: &Arc<Self>, result: Result<(), Fault>) {
        let succeeded = result.is_ok();
        *self.finish_result.lock().expect("member result poisoned") = Some(result);
        self.settled.store(true, Ordering::Release);
        self.done.notify_waiters();
        if succeeded
            && let Some(manager) = self
                .scope
                .upgrade()
                .and_then(|scope| scope.manager.upgrade())
        {
            manager.remove_member(self);
        }
    }
    async fn stop(self: &Arc<Self>) -> Result<(), Fault> {
        // Serialize against target registration/acceptance, never against a
        // lifecycle owner lock. The independent completion task need not take it.
        let _dispatch = self.dispatch.lock().await;
        if self.settled.load(Ordering::Acquire) {
            if self.wait_done().await.is_ok() {
                return Ok(());
            }
            // A later explicit scope close can continue the original owner
            // cleanup. Never retry notification or replay the child execution.
            if let Some(permit) = self.permit.get() {
                permit.close(OperationId::new().map_err(id_fault)?).await?;
                permit.release(DelegationDisposition::ScopeClosed).await?;
            }
            self.finish(Ok(()));
            return Ok(());
        }
        if let Some(permit) = self.permit.get() {
            permit.close(OperationId::new().map_err(id_fault)?).await?;
            if self.finishing.load(Ordering::Acquire) {
                return self.wait_done().await;
            }
            permit.release(DelegationDisposition::ScopeClosed).await?;
        }
        self.finish(Ok(()));
        Ok(())
    }
    async fn complete(self: Arc<Self>, completion: DelegationCompletion) {
        let outcome = async {
            let permit = self
                .permit
                .get()
                .expect("task accepted only after preparation");
            if self.registration.temporary {
                permit.close(OperationId::new().map_err(id_fault)?).await?;
            }
            let disposition = if self
                .scope
                .upgrade()
                .is_none_or(|scope| scope.closing.is_cancelled())
            {
                DelegationDisposition::ScopeClosed
            } else {
                let message = MessageInput {
                    message_id: MessageId::new().map_err(id_fault)?,
                    mode: MessageMode::FollowUp,
                    text: notification_text(&self.registration, &completion)?,
                    in_reply_to: Some(self.registration.task_message_id.clone()),
                };
                match permit.notify(message).await {
                    Ok(receipt) => DelegationDisposition::Notified(receipt),
                    Err(error) => DelegationDisposition::NotificationFailed(error),
                }
            };
            // This records the true disposition once without retaining text or
            // retrying a failed/uncertain delivery.
            permit.release(disposition).await
        }
        .await;
        self.finish(outcome);
    }
}
impl DelegationSink for Sink {
    fn handoff(&self, completion: DelegationCompletion) -> Result<(), Fault> {
        let member = self.0.upgrade().ok_or_else(closed)?;
        let consumed = completion.inputs.iter().any(|range| {
            range
                .message_ids
                .contains(&member.registration.task_message_id)
        });
        let discarded = completion
            .discarded_inputs
            .contains(&member.registration.task_message_id);
        if !consumed && !discarded {
            return Err(Fault::new(
                FaultKind::SessionInputInvalid,
                "completion includes neither actual consumption nor explicit removal of its task input",
            ));
        }
        if (consumed && completion.results.is_empty())
            || (!consumed && !completion.results.is_empty())
            || completion.results.iter().any(|result| {
                result.info.state != TurnState::Finished || result.info.outcome.is_none()
            })
        {
            return Err(Fault::new(
                FaultKind::SessionInputInvalid,
                "only actually consumed tasks can report settled Turn results",
            ));
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "result handoff requires a Tokio runtime",
            )
        })?;
        if member.finishing.swap(true, Ordering::AcqRel) {
            return Err(Fault::new(
                FaultKind::SessionInputAlreadyBound,
                "delegation result was already handed off",
            ));
        }
        runtime.spawn(member.complete(completion));
        Ok(())
    }
}
fn closed() -> Fault {
    Fault::new(
        FaultKind::SessionLifecycleClosing,
        "delegation scope is closed",
    )
}
fn id_fault(error: IdGenerationError) -> Fault {
    Fault::new(FaultKind::HostInternalUnexpected, error.to_string())
}
fn notification_text(
    registration: &DelegationRegistration,
    completion: &DelegationCompletion,
) -> Result<String, Fault> {
    use minicore_runtime::{HistoryItem, model::AssistantPart};
    // Transfer only the actual last response's public text and marked partial
    // text, not the child's history, reasoning, tool trace, or replay metadata.
    let results = completion
        .results
        .iter()
        .map(|result| {
            let output = result.history.iter().rev().find_map(|item| match item {
                HistoryItem::Assistant(response) => Some(
                    response
                        .content
                        .iter()
                        .filter_map(|part| match part {
                            AssistantPart::Text(text) => Some(text.as_ref()),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            });
            let partials = result
                .partials
                .iter()
                .map(|partial| {
                    serde_json::json!({
                        "response_index":partial.response_index,
                        "text":partial.partial.content.iter().filter_map(|part|match part {
                            AssistantPart::Text(text) => Some(text.as_ref()), _ => None,
                        }).collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({"turn_id":result.info.id,"outcome":result.info.outcome,
            "persistence":result.info.persistence,"fault":result.info.fault,
            "output":output,"partials":partials})
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({
        "source":registration.child,"task_message_id":registration.task_message_id,
        "origin":registration.origin,"inputs":completion.inputs,"discarded_inputs":completion.discarded_inputs,
        "status":if completion.results.is_empty() {"not_executed"} else {"ended"},"results":results,
    }))
    .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))
}
