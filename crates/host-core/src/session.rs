use crate::control::{CallerContext, ConnectionRegistry};
use crate::types::*;
use futures_util::FutureExt;
use minicore_host_protocol::*;
use minicore_host_storage::{HistoryEntry, SessionRecord, SessionStore};

mod persistent;
use minicore_runtime::{
    AgentLoop, BoundedText, ExecutionConfig, HistoryFuture, HistoryItem, LoopHandle, LoopHistory,
    LoopId, LoopInputs, LoopOptions, LoopOutcome, LoopReport, LoopRequest, UserInput,
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, watch};

/// A connection-scoped state subscription. It never exposes a raw watch handle
/// that could keep receiving private state after that connection is revoked.
pub struct SessionSubscription {
    snapshots: watch::Receiver<SessionSnapshot>,
    revocation: watch::Receiver<bool>,
    access: Arc<ConnectionRegistry>,
    caller: CallerContext,
}
impl SessionSubscription {
    pub fn snapshot(&self) -> Result<SessionSnapshot, Fault> {
        self.access
            .read(&self.caller, || Ok(self.snapshots.borrow().clone()))
    }
    pub async fn changed(&mut self) -> Result<(), Fault> {
        self.access.read(&self.caller, || Ok(()))?;
        tokio::select! {
            _ = self.revocation.changed() => Err(Fault::new(FaultKind::HostAccessInvalid, "subscription connection is no longer valid")),
            changed = self.snapshots.changed() => {
                changed.map_err(|_| Fault::new(FaultKind::SessionLifecycleClosed, "state subscription owner has ended"))?;
                self.access.read(&self.caller, || Ok(()))
            }
        }
    }
}

pub type ResourceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<BoundResources, Fault>> + Send + 'a>>;

/// The application composes resources. An immutable result is bound for the
/// whole public Turn, including later internal loops. No default substitution.
pub trait ResourceResolver: Send + Sync + 'static {
    /// Pure structural/reference validation. Do not call a model, tool, network,
    /// or filesystem here; resource preparation belongs in `resolve`.
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault>;
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a>;
}
#[derive(Clone)]
pub struct BoundResources {
    pub execution: ExecutionConfig,
    pub options: LoopOptions,
}

pub type SaveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PersistenceState, Fault>> + Send + 'a>>;
/// Narrow save ownership boundary. The implementation must keep accepted
/// batches through cancellation and perform disk work outside owner state locks.
pub type LoadFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<HistoryItem>, Fault>> + Send + 'a>>;
pub trait HistoryBackend: Send + Sync + 'static {
    fn mode(&self) -> StorageMode;
    fn load(&self) -> LoadFuture<'_>;
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a>;
}
pub struct MemoryHistory;
impl HistoryBackend for MemoryHistory {
    fn mode(&self) -> StorageMode {
        StorageMode::Memory
    }
    fn load(&self) -> LoadFuture<'_> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn append<'a>(&'a self, _entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        Box::pin(async { Ok(PersistenceState::NotApplicable) })
    }
}

#[derive(Clone)]
struct QueuedInput {
    message: MessageInput,
    source: MessageSource,
    runtime: UserInput,
    queue: InputQueue,
    order: u64,
}
impl QueuedInput {
    fn summary(&self) -> InputSummary {
        InputSummary {
            message_id: self.message.message_id.clone(),
            mode: self.message.mode,
            queue: self.queue,
            order: self.order,
        }
    }
}
struct ActiveTurn {
    info: TurnInfo,
    loop_id: Option<LoopId>,
    handle: Option<LoopHandle>,
    accepted: usize,
    response_base: u64,
    bound_pending: VecDeque<QueuedInput>,
    start_history: usize,
    partials: Vec<PartialResult>,
    report: Option<Arc<LoopReport>>,
}
struct SessionState {
    info: SessionInfo,
    lifecycle: SessionLifecycle,
    paused: bool,
    failure_paused: bool,
    agent_enabled: Arc<AtomicBool>,
    residency: Residency,
    load_id: Option<LoadId>,
    controller_revision: u64,
    runner_revision: u64,
    next_step: VecDeque<QueuedInput>,
    next_turn: VecDeque<QueuedInput>,
    next_step_order: u64,
    next_turn_order: u64,
    messages: Vec<MessageRecord>,
    active: Option<ActiveTurn>,
    completed: BTreeMap<TurnId, TurnResult>,
    completion_order: Vec<TurnId>,
    history: Vec<HistoryItem>,
    history_loaded: bool,
    preparing: bool,
    saving: bool,
    block_reason: Option<Fault>,
    executor: Option<tokio::runtime::Handle>,
    driver_running: bool,
    driver_task: Option<tokio::task::JoinHandle<()>>,
}

pub(crate) struct SessionController {
    pub(crate) id: SessionId,
    pub(crate) agent_id: AgentId,
    host_id: HostId,
    incarnation: HostIncarnation,
    access: Arc<ConnectionRegistry>,
    resources: Arc<dyn ResourceResolver>,
    backend: Arc<dyn HistoryBackend>,
    state: Mutex<SessionState>,
    store: Option<SessionStore>,
    provenance: Option<SessionRecord>,
    changed: Notify,
    snapshots: watch::Sender<SessionSnapshot>,
}

impl SessionController {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        id: SessionId,
        agent_id: AgentId,
        host_id: HostId,
        incarnation: HostIncarnation,
        config: SessionConfig,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
        backend: Arc<dyn HistoryBackend>,
        history: Vec<HistoryItem>,
        agent_enabled: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let state = SessionState {
            info: SessionInfo {
                id: id.clone(),
                agent_id: agent_id.clone(),
                title: None,
                mode: backend.mode(),
                config_revision: 0,
                config,
            },
            lifecycle: SessionLifecycle::Open,
            paused: false,
            failure_paused: false,
            agent_enabled,
            residency: Residency::Unloaded,
            load_id: None,
            controller_revision: 0,
            runner_revision: 0,
            next_step: VecDeque::new(),
            next_turn: VecDeque::new(),
            next_step_order: 0,
            next_turn_order: 0,
            messages: Vec::new(),
            active: None,
            completed: BTreeMap::new(),
            completion_order: Vec::new(),
            history,
            history_loaded: backend.mode() == StorageMode::Memory,
            preparing: false,
            saving: false,
            block_reason: None,
            executor: tokio::runtime::Handle::try_current().ok(),
            driver_running: false,
            driver_task: None,
        };
        let snapshot = Self::make_snapshot(&state, &host_id, &incarnation, &agent_id, &id);
        let (snapshots, _) = watch::channel(snapshot);
        Arc::new(Self {
            id,
            agent_id,
            host_id,
            incarnation,
            access,
            resources,
            backend,
            store: None,
            provenance: None,
            state: Mutex::new(state),
            changed: Notify::new(),
            snapshots,
        })
    }
    fn make_snapshot(
        state: &SessionState,
        host_id: &HostId,
        incarnation: &HostIncarnation,
        agent_id: &AgentId,
        id: &SessionId,
    ) -> SessionSnapshot {
        SessionSnapshot {
            host_id: host_id.clone(),
            host_incarnation: incarnation.clone(),
            agent_id: agent_id.clone(),
            session_id: id.clone(),
            data: SessionStateData {
                mode: state.info.mode,
                controller_revision: state.controller_revision,
                lifecycle: state.lifecycle,
                scheduling: if state.paused || state.failure_paused {
                    Scheduling::Paused
                } else {
                    Scheduling::Auto
                },
                pause_reasons: [
                    state.paused.then(|| "explicit".to_owned()),
                    state.failure_paused.then(|| "execution_failure".to_owned()),
                ]
                .into_iter()
                .flatten()
                .collect(),
                residency: state.residency,
                load_id: state.load_id.clone(),
                runner_revision: state.runner_revision,
                active_turn_id: state.active.as_ref().map(|active| active.info.id.clone()),
                next_step: state.next_step.iter().map(QueuedInput::summary).collect(),
                next_turn: state.next_turn.iter().map(QueuedInput::summary).collect(),
                preparing: state.preparing,
                saving: state.saving,
                block_reason: state.block_reason.clone(),
                last_result: state.completion_order.last().map(|id| TurnRef {
                    session_id: state.info.id.clone(),
                    turn_id: id.clone(),
                }),
            },
        }
    }
    fn publish(&self, state: &mut SessionState) {
        state.controller_revision = state
            .controller_revision
            .checked_add(1)
            .expect("controller revision exhausted");
        self.snapshots.send_replace(Self::make_snapshot(
            state,
            &self.host_id,
            &self.incarnation,
            &self.agent_id,
            &self.id,
        ));
        self.changed.notify_waiters();
    }
    fn open(state: &SessionState) -> Result<(), Fault> {
        match state.lifecycle {
            SessionLifecycle::Open => Ok(()),
            SessionLifecycle::Closing => Err(Fault::new(
                FaultKind::SessionLifecycleClosing,
                "Session is closing",
            )),
            SessionLifecycle::Closed => Err(Fault::new(
                FaultKind::SessionLifecycleClosed,
                "Session is closed",
            )),
        }
    }
    fn schedulable(state: &SessionState) -> bool {
        state.lifecycle == SessionLifecycle::Open
            && !state.paused
            && !state.failure_paused
            && state.agent_enabled.load(Ordering::SeqCst)
            && state.block_reason.is_none()
    }
    pub(crate) fn is_open_owned(&self) -> bool {
        self.state.lock().expect("Session mutex poisoned").lifecycle == SessionLifecycle::Open
    }
    pub(crate) fn info(&self, caller: &CallerContext) -> Result<SessionInfo, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Session mutex poisoned")
                .info
                .clone())
        })
    }
    pub(crate) fn snapshot(&self, caller: &CallerContext) -> Result<SessionSnapshot, Fault> {
        self.access
            .read(caller, || Ok(self.snapshots.borrow().clone()))
    }
    pub(crate) fn subscribe_state(
        &self,
        caller: &CallerContext,
    ) -> Result<SessionSubscription, Fault> {
        self.access.read(caller, || {
            Ok(SessionSubscription {
                snapshots: self.snapshots.subscribe(),
                revocation: caller.revocation(),
                access: Arc::clone(&self.access),
                caller: caller.clone(),
            })
        })
    }
    pub(crate) fn message(
        &self,
        caller: &CallerContext,
        id: &MessageId,
    ) -> Result<MessageRecord, Fault> {
        self.access.read(caller, || {
            self.state
                .lock()
                .expect("Session mutex poisoned")
                .messages
                .iter()
                .rev()
                .find(|message| &message.input.message_id == id)
                .cloned()
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionInputNotFound,
                        "message is not held by this Session",
                    )
                })
        })
    }
    pub(crate) fn history(&self, caller: &CallerContext) -> Result<Vec<HistoryItem>, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            if !state.history_loaded {
                return Err(Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "history has not yet been loaded and validated",
                ));
            }
            Ok(state.history.clone())
        })
    }

    pub(crate) fn send(
        self: &Arc<Self>,
        caller: &CallerContext,
        message: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        self.ensure_executor()?;
        let runtime = UserInput::text(&message.text).map_err(|_| {
            Fault::new(
                FaultKind::SessionInputInvalid,
                "input is empty or exceeds the pinned Runtime's structural text limit",
            )
        })?;
        let receipt = self.access.accept(caller, || {
            self.accept_message(message, runtime, MessageSource::User)
        })?;
        self.wake();
        Ok(receipt)
    }
    /// Used only by an already-accepted direct-owner creation workflow.
    pub(crate) fn send_owned(
        self: &Arc<Self>,
        message: MessageInput,
    ) -> Result<MessageAccepted, Fault> {
        self.ensure_executor()?;
        let runtime = UserInput::text(&message.text).map_err(|_| {
            Fault::new(
                FaultKind::SessionInputInvalid,
                "input is empty or exceeds the pinned Runtime's structural text limit",
            )
        })?;
        let receipt = self.accept_message(message, runtime, MessageSource::User)?;
        self.wake();
        Ok(receipt)
    }
    fn accept_message(
        &self,
        message: MessageInput,
        runtime: UserInput,
        source: MessageSource,
    ) -> Result<MessageAccepted, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        Self::open(&state)?;
        if !state.agent_enabled.load(Ordering::SeqCst) {
            return Err(Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "owning Agent is stopped",
            ));
        }
        let queue = if state
            .active
            .as_ref()
            .is_some_and(|active| active.info.state == TurnState::Cancelling)
        {
            InputQueue::NextTurn
        } else {
            message.mode.requested_queue()
        };
        let next_order = match queue {
            InputQueue::NextStep => &mut state.next_step_order,
            InputQueue::NextTurn => &mut state.next_turn_order,
        };
        let order = *next_order;
        *next_order = order.checked_add(1).ok_or_else(|| {
            Fault::new(
                FaultKind::SessionInternalUnexpected,
                "queue order exhausted",
            )
        })?;
        let entry = QueuedInput {
            message: message.clone(),
            source,
            runtime,
            queue,
            order,
        };
        state.messages.push(MessageRecord {
            input: message.clone(),
            queue,
            order,
            binding: MessageBinding::Queued,
        });
        match queue {
            InputQueue::NextStep => state.next_step.push_back(entry),
            InputQueue::NextTurn => state.next_turn.push_back(entry),
        }
        self.publish(&mut state);
        let mut facts = MessageAcceptedFacts::new(queue, state.controller_revision);
        facts.load_id = state.load_id.clone();
        facts.start_error = if state.active.is_none() {
            state.block_reason.clone()
        } else {
            None
        };
        Ok(MessageAccepted::new(
            self.host_id.clone(),
            self.incarnation.clone(),
            SessionRef {
                agent_id: self.agent_id.clone(),
                session_id: self.id.clone(),
            },
            message.message_id,
            facts,
        ))
    }
    pub(crate) fn pause(&self, caller: &CallerContext) -> Result<(), Fault> {
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            state.paused = true;
            self.publish(&mut state);
            Ok(())
        })
    }
    pub(crate) fn resume(self: &Arc<Self>, caller: &CallerContext) -> Result<(), Fault> {
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if let Some(fault) = &state.block_reason {
                return Err(fault.clone());
            }
            state.paused = false;
            state.failure_paused = false;
            self.publish(&mut state);
            Ok(())
        })?;
        self.wake();
        Ok(())
    }
    pub(crate) fn update(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected_revision: u64,
        patch: &SessionConfigPatch,
    ) -> Result<SessionInfo, Fault> {
        let original = self.info(caller)?;
        if original.config_revision != expected_revision {
            return Err(Fault::new(
                FaultKind::SessionConfigConflict,
                "Session configuration revision changed",
            ));
        }
        let candidate = original
            .config
            .apply_patch(patch)
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        self.resources.validate(&candidate)?;
        let updated = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if state.info.mode == StorageMode::Persistent {
                return Err(Fault::new(
                    FaultKind::HostMethodUnavailable,
                    "persistent configuration updates are not implemented in this foundation stage",
                ));
            }
            if state.info.config_revision != expected_revision {
                return Err(Fault::new(
                    FaultKind::SessionConfigConflict,
                    "Session configuration revision changed",
                ));
            }
            state.info.config_revision = expected_revision.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "configuration revision exhausted",
                )
            })?;
            state.info.config = candidate;
            if state
                .block_reason
                .as_ref()
                .is_some_and(|fault| fault.kind() == FaultKind::SessionResourceMissing)
            {
                state.block_reason = None;
            }
            self.publish(&mut state);
            Ok(state.info.clone())
        })?;
        self.wake();
        Ok(updated)
    }
    pub(crate) fn remove(
        &self,
        caller: &CallerContext,
        message_id: &MessageId,
    ) -> Result<(), Fault> {
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let position = state
                .next_step
                .iter()
                .position(|input| &input.message.message_id == message_id)
                .map(|index| (InputQueue::NextStep, index))
                .or_else(|| {
                    state
                        .next_turn
                        .iter()
                        .position(|input| &input.message.message_id == message_id)
                        .map(|index| (InputQueue::NextTurn, index))
                });
            let Some((queue, index)) = position else {
                let bound = state.messages.iter().any(|input| {
                    &input.input.message_id == message_id
                        && matches!(input.binding, MessageBinding::Bound { .. })
                });
                return Err(Fault::new(
                    if bound {
                        FaultKind::SessionInputAlreadyBound
                    } else {
                        FaultKind::SessionInputNotFound
                    },
                    "input is not an unbound queue entry",
                ));
            };
            let removed = match queue {
                InputQueue::NextStep => state.next_step.remove(index),
                InputQueue::NextTurn => state.next_turn.remove(index),
            }
            .expect("located input");
            mark_message(&mut state, &removed, MessageBinding::Removed);
            self.publish(&mut state);
            Ok(())
        })
    }
    pub(crate) fn cancel(
        &self,
        caller: &CallerContext,
        target: Option<&TurnId>,
        keep_inbox: bool,
    ) -> Result<CancelResult, Fault> {
        let (result, handle) = self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let mut handle = None;
            let status = match state.active.as_mut() {
                Some(active) if target.is_none_or(|id| id == &active.info.id) => {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        active.info.state = TurnState::Cancelling;
                        handle = active.handle.clone();
                        CancelStatus::Claimed
                    } else {
                        CancelStatus::AlreadyEnding
                    }
                }
                _ => CancelStatus::NotActive,
            };
            // Session cancellation with no active Turn still owns its current inbox.
            let clear = !keep_inbox
                && (status == CancelStatus::Claimed
                    || (target.is_none() && status == CancelStatus::NotActive));
            let cleared = if clear {
                clear_inbox(&mut state)
            } else {
                Vec::new()
            };
            self.publish(&mut state);
            Ok((CancelResult { status, cleared }, handle))
        })?;
        if let Some(handle) = handle {
            handle.cancel();
        }
        Ok(result)
    }
    pub(crate) fn close(&self, caller: &CallerContext) -> Result<(), Fault> {
        if self.store.is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "persistent close requires the asynchronous owner entry",
            ));
        }
        let handle = self.access.accept(caller, || self.close_owned())?;
        if let Some(handle) = handle {
            handle.shutdown();
        }
        Ok(())
    }
    fn close_owned(&self) -> Result<Option<LoopHandle>, Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.lifecycle == SessionLifecycle::Closed {
            return Ok(None);
        }
        state.lifecycle = SessionLifecycle::Closing;
        clear_inbox(&mut state);
        let handle = if let Some(active) = state.active.as_mut() {
            if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                active.info.state = TurnState::Cancelling;
            }
            active.handle.clone()
        } else {
            None
        };
        if state.active.is_none() && !state.preparing && !state.saving {
            state.lifecycle = SessionLifecycle::Closed;
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
        self.publish(&mut state);
        Ok(handle)
    }
    pub(crate) fn unload(&self, caller: &CallerContext) -> Result<(), Fault> {
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if state.active.is_some() || state.preparing || state.saving {
                return Err(Fault::new(
                    FaultKind::SessionBusy,
                    "Session still owns execution or save work",
                ));
            }
            if !state.next_step.is_empty() || !state.next_turn.is_empty() {
                state.paused = true;
            }
            state.residency = Residency::Unloaded;
            state.load_id = None;
            self.publish(&mut state);
            Ok(())
        })
    }
    pub(crate) fn is_quiescent_owned(&self) -> bool {
        let state = self.state.lock().expect("Session mutex poisoned");
        state.active.is_none() && !state.preparing && !state.saving
    }
    pub(crate) async fn wait_quiescent_owned(&self) {
        let mut snapshots = self.snapshots.subscribe();
        loop {
            if self.is_quiescent_owned() {
                return;
            }
            if snapshots.changed().await.is_err() {
                return;
            }
        }
    }
    pub(crate) fn set_agent_enabled(self: &Arc<Self>, enabled: bool) {
        let handle = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let handle = if !enabled {
                state.active.as_mut().and_then(|active| {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        active.info.state = TurnState::Cancelling;
                        active.handle.clone()
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            self.publish(&mut state);
            handle
        };
        if let Some(handle) = handle {
            handle.cancel();
        }
        if enabled {
            self.wake();
        }
    }
    pub(crate) fn turn_info(&self, caller: &CallerContext, id: &TurnId) -> Result<TurnInfo, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            state
                .active
                .as_ref()
                .filter(|active| &active.info.id == id)
                .map(|active| active.info.clone())
                .or_else(|| state.completed.get(id).map(|result| result.info.clone()))
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionTurnNotFound,
                        "Turn is not held by this Session",
                    )
                })
        })
    }
    pub(crate) fn turns(&self, caller: &CallerContext) -> Result<Vec<TurnInfo>, Fault> {
        self.access.read(caller, || {
            let state = self.state.lock().expect("Session mutex poisoned");
            let mut turns = state
                .completion_order
                .iter()
                .filter_map(|id| state.completed.get(id).map(|result| result.info.clone()))
                .collect::<Vec<_>>();
            if let Some(active) = &state.active {
                turns.push(active.info.clone());
            }
            Ok(turns)
        })
    }
    pub(crate) fn turn_result(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        self.access.read(caller, || {
            self.state
                .lock()
                .expect("Session mutex poisoned")
                .completed
                .get(id)
                .cloned()
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionTurnNotFound,
                        "completed Turn result is not held by this Session",
                    )
                })
        })
    }
    pub(crate) async fn wait_turn(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnResult, Fault> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let info = self.turn_info(caller, id)?;
            if info.state == TurnState::Finished {
                return self.turn_result(caller, id);
            }
            notified.await;
        }
    }
    fn ensure_executor(&self) -> Result<(), Fault> {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.executor.is_none() {
            state.executor = tokio::runtime::Handle::try_current().ok();
        }
        if state.executor.is_none() {
            return Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "execution requires a live Tokio runtime before input can be accepted",
            ));
        }
        Ok(())
    }
    pub(crate) fn executor(&self) -> Option<tokio::runtime::Handle> {
        self.state
            .lock()
            .expect("Session mutex poisoned")
            .executor
            .clone()
    }
    fn wake(self: &Arc<Self>) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        if state.driver_running
            || !Self::schedulable(&state)
            || (state.next_step.is_empty() && state.next_turn.is_empty())
        {
            return;
        }
        let Some(executor) = state
            .executor
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())
        else {
            return;
        };
        state.executor = Some(executor.clone());
        state.driver_running = true;
        let owner = Arc::clone(self);
        state.driver_task = Some(executor.spawn(async move {
            owner.drive().await;
        }));
    }
    async fn drive(self: Arc<Self>) {
        loop {
            let preparation = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                if !Self::schedulable(&state)
                    || state.active.is_some()
                    || (state.next_step.is_empty() && state.next_turn.is_empty())
                {
                    if state.lifecycle == SessionLifecycle::Closing
                        && state.active.is_none()
                        && !state.preparing
                        && !state.saving
                    {
                        state.lifecycle = SessionLifecycle::Closed;
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                        self.publish(&mut state);
                    }
                    state.driver_running = false;
                    state.driver_task.take();
                    return;
                }
                state.preparing = true;
                let candidate = (state.info.config.clone(), state.info.config_revision);
                self.publish(&mut state);
                candidate
            };
            let needs_load = !self
                .state
                .lock()
                .expect("Session mutex poisoned")
                .history_loaded;
            if needs_load {
                match self.backend.load().await {
                    Ok(history) => {
                        let mut state = self.state.lock().expect("Session mutex poisoned");
                        state.history = history;
                        state.history_loaded = true;
                    }
                    Err(fault) => {
                        self.preparation_failed(fault);
                        return;
                    }
                }
            }
            let resources =
                AssertUnwindSafe(async { self.resources.resolve(&preparation.0).await })
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(Fault::new(
                            FaultKind::SessionInternalUnexpected,
                            "resource preparation panicked; accepted inputs remain queued",
                        ))
                    });
            let resources = match resources {
                Ok(resources) => resources,
                Err(fault) => {
                    let mut state = self.state.lock().expect("Session mutex poisoned");
                    state.preparing = false;
                    if state.info.config_revision != preparation.1 {
                        continue;
                    }
                    state.block_reason = Some(fault);
                    state.driver_running = false;
                    state.driver_task.take();
                    if state.lifecycle == SessionLifecycle::Closing {
                        state.lifecycle = SessionLifecycle::Closed;
                    }
                    self.publish(&mut state);
                    return;
                }
            };
            let turn_id = match TurnId::new() {
                Ok(id) => id,
                Err(error) => {
                    self.preparation_failed(Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        error.to_string(),
                    ));
                    return;
                }
            };
            let load_id = match LoadId::new() {
                Ok(id) => id,
                Err(error) => {
                    self.preparation_failed(Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        error.to_string(),
                    ));
                    return;
                }
            };
            let batch = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                state.preparing = false;
                if !Self::schedulable(&state) || state.info.config_revision != preparation.1 {
                    self.publish(&mut state);
                    continue;
                }
                let batch = take_batch(&mut state, true, &turn_id);
                if batch.is_empty() {
                    continue;
                }
                let load_id = state.load_id.get_or_insert(load_id).clone();
                state.residency = Residency::Loaded;
                let start_history = state.history.len();
                state.active = Some(ActiveTurn {
                    info: TurnInfo {
                        id: turn_id.clone(),
                        session_id: self.id.clone(),
                        load_id,
                        state: TurnState::Starting,
                        config_revision: preparation.1,
                        outcome: None,
                        persistence: if state.info.mode == StorageMode::Memory {
                            PersistenceState::NotApplicable
                        } else {
                            PersistenceState::Pending
                        },
                        requests: 0,
                        tool_rounds: 0,
                        fault: None,
                        persistence_fault: None,
                    },
                    loop_id: None,
                    handle: None,
                    accepted: 0,
                    response_base: 0,
                    bound_pending: VecDeque::new(),
                    start_history,
                    partials: Vec::new(),
                    report: None,
                });
                self.publish(&mut state);
                batch
            };
            self.run_turn(turn_id, batch, resources).await;
        }
    }
    fn preparation_failed(&self, fault: Fault) {
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.preparing = false;
        state.block_reason = Some(fault);
        state.driver_running = false;
        state.driver_task.take();
        self.publish(&mut state);
    }
    async fn run_turn(
        self: &Arc<Self>,
        turn_id: TurnId,
        mut batch: Vec<QueuedInput>,
        resources: BoundResources,
    ) {
        loop {
            let started = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let Some(active) = state
                    .active
                    .as_ref()
                    .filter(|active| active.info.id == turn_id)
                else {
                    return;
                };
                if active.info.state == TurnState::Cancelling {
                    None
                } else {
                    let history: Arc<[HistoryItem]> = state.history.clone().into();
                    let first = batch.remove(0);
                    let inputs = Arc::new(SessionInputs {
                        session: Arc::clone(self),
                        turn_id: turn_id.clone(),
                        initial: Mutex::new(batch.iter().cloned().collect()),
                    });
                    let callback = Arc::new(SessionHistory {
                        session: Arc::clone(self),
                        turn_id: turn_id.clone(),
                    });
                    let mut options = resources.options.clone();
                    options.history = Some(callback);
                    let active = state.active.as_mut().expect("checked active Turn");
                    active.report = None;
                    active.bound_pending.push_back(first.clone());
                    active.bound_pending.extend(batch.drain(..));
                    active.accepted = 0;
                    let started = AgentLoop::start(
                        LoopRequest::new(history, first.runtime, resources.execution.clone()),
                        options,
                        inputs,
                    );
                    match &started {
                        Ok(owner) => {
                            active.loop_id = Some(owner.id());
                            active.handle = Some(owner.handle());
                            active.info.state = TurnState::Running;
                            state.runner_revision = state
                                .runner_revision
                                .checked_add(1)
                                .expect("runner revision exhausted");
                        }
                        Err(_) => {
                            active.info.state = TurnState::Finalizing;
                        }
                    }
                    self.publish(&mut state);
                    Some(started)
                }
            };
            let Some(started) = started else {
                self.finalize(&turn_id, TurnOutcome::Cancelled, None).await;
                return;
            };
            let report = match started {
                Ok(owner) => match owner.join().await {
                    Ok(report) => report,
                    Err(error) => {
                        self.finalize(
                            &turn_id,
                            TurnOutcome::Failed,
                            Some(Fault::new(
                                FaultKind::TurnInternalUnexpected,
                                error.to_string(),
                            )),
                        )
                        .await;
                        return;
                    }
                },
                Err(error) => {
                    self.finalize(
                        &turn_id,
                        TurnOutcome::Failed,
                        Some(Fault::new(
                            FaultKind::TurnInternalUnexpected,
                            error.to_string(),
                        )),
                    )
                    .await;
                    return;
                }
            };
            {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let active = state.active.as_mut().expect("owned active Turn");
                active.info.requests += u64::from(report.requests);
                active.info.tool_rounds += report.tool_rounds;
                active.report = Some(Arc::clone(&report));
            }
            if let Err(fault) = self.reconcile_report(&turn_id, &report).await {
                // Ownership of all accepted records remains with this Session.
                // A failed save blocks new work; no reappend or automatic retry.
                let mut state = self.state.lock().expect("Session mutex poisoned");
                state.block_reason = Some(fault.clone());
                if let Some(active) = state.active.as_mut() {
                    active.info.persistence = PersistenceState::Failed;
                    active.info.persistence_fault = Some(fault);
                    active.info.fault = match &report.outcome {
                        LoopOutcome::Failed(failure)
                            if failure.kind != minicore_runtime::LoopFailureKind::History =>
                        {
                            Some(runtime_fault(failure))
                        }
                        _ => None,
                    };
                    active.info.outcome = Some(match report.outcome {
                        LoopOutcome::Completed => TurnOutcome::Completed,
                        LoopOutcome::Cancelled(_) => TurnOutcome::Cancelled,
                        _ => TurnOutcome::Failed,
                    });
                    active.info.state = TurnState::Finalizing;
                }
                self.publish(&mut state);
                return;
            }
            let next = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let schedulable = Self::schedulable(&state);
                let active = state.active.as_mut().expect("owned active Turn");
                active.handle = None;
                active.response_base += u64::from(report.requests);
                let continue_same = report.outcome == LoopOutcome::Completed
                    && active.info.state != TurnState::Cancelling
                    && schedulable
                    && !state.next_step.is_empty();
                if continue_same {
                    let next = take_batch(&mut state, false, &turn_id);
                    self.publish(&mut state);
                    Some(next)
                } else {
                    let active = state.active.as_mut().expect("owned active Turn");
                    let (outcome, fault) = match &report.outcome {
                        LoopOutcome::Completed if active.info.state == TurnState::Cancelling => {
                            (TurnOutcome::Cancelled, None)
                        }
                        LoopOutcome::Completed => (TurnOutcome::Completed, None),
                        LoopOutcome::Cancelled(_) => (TurnOutcome::Cancelled, None),
                        LoopOutcome::Failed(failure) => {
                            (TurnOutcome::Failed, Some(runtime_fault(failure)))
                        }
                        _ => (
                            TurnOutcome::Failed,
                            Some(Fault::new(
                                FaultKind::TurnInternalUnexpected,
                                "unknown Runtime outcome",
                            )),
                        ),
                    };
                    active.info.state = TurnState::Finalizing;
                    active.info.outcome = Some(outcome);
                    active.info.fault = fault;
                    self.publish(&mut state);
                    None
                }
            };
            if let Some(next) = next {
                batch = next;
                continue;
            }
            let (outcome, fault) = {
                let state = self.state.lock().expect("Session mutex poisoned");
                let active = state.active.as_ref().expect("owned active Turn");
                (
                    active.info.outcome.expect("terminal claim"),
                    active.info.fault.clone(),
                )
            };
            self.finalize(&turn_id, outcome, fault).await;
            return;
        }
    }
    async fn reconcile_report(
        self: &Arc<Self>,
        turn_id: &TurnId,
        report: &LoopReport,
    ) -> Result<(), Fault> {
        {
            let state = self.state.lock().expect("Session mutex poisoned");
            if let Some(fault) = &state.block_reason
                && fault.kind() == FaultKind::SessionHistoryWriteFailed
            {
                return Err(fault.clone());
            }
        }
        if report.history_failure.is_some() {
            return Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "Runtime history handoff did not confirm its save boundary",
            ));
        }
        let accepted = {
            let state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_ref().expect("owned active Turn");
            if &active.info.id != turn_id || active.loop_id != Some(report.loop_id) {
                return Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "late report does not match active execution",
                ));
            }
            active.accepted
        };
        if accepted < report.appended.len() {
            self.accept_history(
                turn_id,
                report.loop_id,
                accepted,
                &report.appended[accepted..],
            )
            .await?;
        }
        let mut entries = Vec::new();
        if let Some(partial) = &report.partial {
            let response_index = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let active = state.active.as_mut().expect("owned active Turn");
                let response_index = active.response_base + u64::from(partial.request_index);
                active.partials.push(PartialResult {
                    response_index,
                    partial: partial.clone(),
                });
                response_index
            };
            entries.push(HistoryEntry::Partial {
                turn_id: turn_id.clone(),
                loop_id: report.loop_id,
                response_index: u32::try_from(response_index).map_err(|_| {
                    Fault::new(
                        FaultKind::SessionInternalUnexpected,
                        "response index exhausted",
                    )
                })?,
                partial: partial.clone(),
            });
        }
        if !entries.is_empty() {
            self.save(turn_id, entries).await?;
        }
        Ok(())
    }
    async fn accept_history(
        &self,
        turn_id: &TurnId,
        loop_id: LoopId,
        offset: usize,
        items: &[HistoryItem],
    ) -> Result<(), Fault> {
        let entries = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let active = state.active.as_mut().ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "history callback has no active Turn",
                )
            })?;
            if &active.info.id != turn_id
                || active.loop_id != Some(loop_id)
                || active.accepted != offset
            {
                return Err(Fault::new(
                    FaultKind::SessionInternalUnexpected,
                    "history callback does not match its original execution range",
                ));
            }
            let mut entries = Vec::new();
            for item in items {
                match item {
                    HistoryItem::User(user) => {
                        let input = active.bound_pending.pop_front().ok_or_else(|| {
                            Fault::new(
                                FaultKind::SessionInternalUnexpected,
                                "Runtime returned an unbound input",
                            )
                        })?;
                        if input.runtime != user.input {
                            return Err(Fault::new(
                                FaultKind::SessionInternalUnexpected,
                                "Runtime input order differs from bound messages",
                            ));
                        }
                        entries.push(HistoryEntry::Input {
                            turn_id: turn_id.clone(),
                            message_id: input.message.message_id,
                            source: input.source,
                            mode: input.message.mode,
                            in_reply_to: input.message.in_reply_to,
                            user: user.clone(),
                        });
                    }
                    _ => {
                        let request_index = match item {
                            HistoryItem::Assistant(value) => value.request_index,
                            HistoryItem::ToolResult(value) => value.request_index,
                            _ => 0,
                        };
                        let response_index =
                            u32::try_from(active.response_base + u64::from(request_index))
                                .map_err(|_| {
                                    Fault::new(
                                        FaultKind::SessionInternalUnexpected,
                                        "response index exhausted",
                                    )
                                })?;
                        entries.push(HistoryEntry::Runtime {
                            turn_id: turn_id.clone(),
                            response_index,
                            item: item.clone(),
                        });
                    }
                }
            }
            // Accepted cursor is advanced before I/O and never rolled back on a
            // save failure. The final report supplements only the unseen suffix.
            active.accepted += items.len();
            state.history.extend_from_slice(items);
            self.publish(&mut state);
            entries
        };
        self.save(turn_id, entries).await
    }
    async fn save(&self, turn_id: &TurnId, entries: Vec<HistoryEntry>) -> Result<(), Fault> {
        {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            state.saving = true;
            if let Some(active) = state.active.as_mut()
                && &active.info.id == turn_id
                && self.backend.mode() == StorageMode::Persistent
            {
                active.info.persistence = PersistenceState::Pending;
            }
            self.publish(&mut state);
        }
        let result = AssertUnwindSafe(async { self.backend.append(entries).await })
            .catch_unwind()
            .await
            .unwrap_or_else(|_| {
                Err(Fault::new(
                    FaultKind::SessionHistoryWriteFailed,
                    "history backend panicked before confirming its save",
                ))
            });
        let mut state = self.state.lock().expect("Session mutex poisoned");
        state.saving = false;
        if let Some(active) = state
            .active
            .as_mut()
            .filter(|active| &active.info.id == turn_id)
        {
            match &result {
                Ok(persistence) => active.info.persistence = *persistence,
                Err(fault) => {
                    active.info.persistence = PersistenceState::Failed;
                    active.info.persistence_fault = Some(fault.clone());
                    state.block_reason = Some(fault.clone());
                }
            }
        }
        self.publish(&mut state);
        result.map(|_| ())
    }
    async fn finalize(&self, turn_id: &TurnId, outcome: TurnOutcome, fault: Option<Fault>) {
        {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let Some(active) = state
                .active
                .as_mut()
                .filter(|active| &active.info.id == turn_id)
            else {
                return;
            };
            active.info.state = TurnState::Finalizing;
            active.info.outcome = Some(outcome);
            active.info.fault = fault.clone();
            active.handle = None;
            if outcome == TurnOutcome::Failed {
                state.failure_paused = true;
            }
            self.publish(&mut state);
        }
        let entry = HistoryEntry::TurnFinished {
            turn_id: turn_id.clone(),
            outcome,
            reason: fault.as_ref().map(|fault| fault.message().to_owned()),
        };
        if self.save(turn_id, vec![entry]).await.is_err() {
            return;
        }
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(mut active) = state.active.take() else {
            return;
        };
        active.info.state = TurnState::Finished;
        let result = TurnResult {
            info: active.info,
            history: state.history[active.start_history..].to_vec(),
            partials: active.partials,
        };
        state.completion_order.push(turn_id.clone());
        state.completed.insert(turn_id.clone(), result);
        if state.lifecycle == SessionLifecycle::Closing {
            state.lifecycle = SessionLifecycle::Closed;
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
        self.publish(&mut state);
    }
}

fn mark_message(state: &mut SessionState, input: &QueuedInput, binding: MessageBinding) {
    if let Some(record) = state.messages.iter_mut().find(|record| {
        record.input.message_id == input.message.message_id
            && record.queue == input.queue
            && record.order == input.order
    }) {
        record.binding = binding;
    }
}
fn clear_inbox(state: &mut SessionState) -> Vec<MessageId> {
    let mut entries = state.next_step.drain(..).collect::<Vec<_>>();
    entries.extend(state.next_turn.drain(..));
    let ids = entries
        .iter()
        .map(|input| input.message.message_id.clone())
        .collect();
    for input in entries {
        mark_message(state, &input, MessageBinding::Removed);
    }
    ids
}
fn take_batch(state: &mut SessionState, new_turn: bool, turn_id: &TurnId) -> Vec<QueuedInput> {
    let mut batch = state.next_step.drain(..).collect::<Vec<_>>();
    if new_turn && let Some(input) = state.next_turn.pop_front() {
        batch.push(input);
    }
    for input in &batch {
        mark_message(
            state,
            input,
            MessageBinding::Bound {
                turn_id: turn_id.clone(),
            },
        );
    }
    batch
}

struct SessionInputs {
    session: Arc<SessionController>,
    turn_id: TurnId,
    initial: Mutex<VecDeque<QueuedInput>>,
}
impl LoopInputs for SessionInputs {
    fn take(&self) -> Vec<UserInput> {
        // Already bound remainder is handoff storage, not a third inbox. Pause
        // never withdraws these items after the initial binding boundary.
        let mut values = self
            .initial
            .lock()
            .expect("bound input mutex poisoned")
            .drain(..)
            .map(|input| input.runtime)
            .collect::<Vec<_>>();
        let mut state = self.session.state.lock().expect("Session mutex poisoned");
        if SessionController::schedulable(&state)
            && state.active.as_ref().is_some_and(|active| {
                active.info.id == self.turn_id && active.info.state == TurnState::Running
            })
        {
            let batch = take_batch(&mut state, false, &self.turn_id);
            values.extend(batch.iter().map(|input| input.runtime.clone()));
            state
                .active
                .as_mut()
                .expect("checked active Turn")
                .bound_pending
                .extend(batch);
            self.session.publish(&mut state);
        }
        values
    }
}
struct SessionHistory {
    session: Arc<SessionController>,
    turn_id: TurnId,
}
impl LoopHistory for SessionHistory {
    fn append<'a>(
        &'a self,
        loop_id: LoopId,
        offset: usize,
        items: &'a [HistoryItem],
    ) -> HistoryFuture<'a> {
        Box::pin(async move {
            self.session
                .accept_history(&self.turn_id, loop_id, offset, items)
                .await
                .map_err(|_| {
                    DiagnosticSummary::new(
                        DiagnosticCode::ContextFailed,
                        DiagnosticCategory::Context,
                        BoundedText::new("Session could not confirm its history save boundary")
                            .expect("static diagnostic"),
                        false,
                    )
                    .expect("static diagnostic")
                })
        })
    }
}

fn runtime_fault(failure: &minicore_runtime::LoopFailure) -> Fault {
    use minicore_runtime::model::ModelErrorKind;
    let kind = match failure.model_error().map(|error| error.kind()) {
        Some(ModelErrorKind::RateLimited) => FaultKind::TurnModelUpstreamRateLimited,
        Some(ModelErrorKind::ProviderUnavailable) => FaultKind::TurnModelUpstreamOverloaded,
        Some(ModelErrorKind::QuotaExceeded) => FaultKind::TurnModelUpstreamQuotaExhausted,
        Some(ModelErrorKind::AuthMissing | ModelErrorKind::AuthRejected) => {
            FaultKind::TurnModelUpstreamAuthenticationFailed
        }
        Some(ModelErrorKind::IncompleteResponse | ModelErrorKind::StreamInterrupted) => {
            FaultKind::TurnModelStreamInterrupted
        }
        Some(_) => FaultKind::TurnModelUpstreamUnclassified,
        None => FaultKind::TurnInternalUnexpected,
    };
    Fault::new(kind, failure.diagnostic.message().as_str())
}

#[cfg(test)]
mod tests;
