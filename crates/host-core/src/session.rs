use crate::control::{CallerContext, ConnectionRegistry};
use crate::types::*;
use futures_util::FutureExt;
use minicore_host_protocol::*;
use minicore_host_storage::{HistoryEntry, SessionRecord, SessionStore};

mod controls;
mod history;
mod metadata;
mod retirement;
pub(crate) use retirement::SessionShutdownStatus;
mod persistent;
use minicore_runtime::{
    AgentLoop, ExecutionConfig, HistoryFuture, HistoryItem, LoopHandle, LoopHistory, LoopId,
    LoopInputs, LoopOptions, LoopOutcome, LoopReport, LoopRequest, Text, UserInput,
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, watch};

/// A connection-scoped state subscription. It never exposes a raw watch handle
/// that could keep receiving private state after that connection is revoked.
pub struct SessionSubscription {
    owner: Weak<SessionController>,
    snapshots: watch::Receiver<SessionSnapshot>,
    revocation: watch::Receiver<bool>,
    access: Arc<ConnectionRegistry>,
    caller: CallerContext,
}
impl SessionSubscription {
    pub fn snapshot(&self) -> Result<SessionSnapshot, Fault> {
        self.access.read(&self.caller, || {
            self.owner
                .upgrade()
                .ok_or_else(|| {
                    Fault::new(FaultKind::AgentSessionNotFound, "Session was reclaimed")
                })?
                .ensure_visible()?;
            Ok(self.snapshots.borrow().clone())
        })
    }
    pub async fn changed(&mut self) -> Result<(), Fault> {
        self.snapshot()?;
        tokio::select! {
            _ = self.revocation.changed() => Err(Fault::new(FaultKind::HostAccessInvalid, "subscription connection is no longer valid")),
            changed = self.snapshots.changed() => {
                changed.map_err(|_| Fault::new(FaultKind::SessionLifecycleClosed, "state subscription owner has ended"))?;
                self.snapshot().map(|_| ())
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
pub trait HistoryBackend: Send + Sync + std::any::Any {
    fn mode(&self) -> StorageMode;
    fn load(&self) -> LoadFuture<'_>;
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a>;
    /// Explicitly continue only the original retained append. Never reload,
    /// accept another batch, or infer confirmation from readable bytes.
    fn reconcile(&self) -> SaveFuture<'_> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "this history backend cannot reconcile the original save",
            ))
        })
    }
    /// Called by a fenced owner after actual workers have returned. Implementors
    /// retaining external storage handles must release them here, not merely
    /// publish a persisted acknowledgement.
    fn release(&self) -> Pin<Box<dyn Future<Output = Result<(), Fault>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }
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
    partial_accepted: bool,
    terminal_accepted: bool,
}
struct SessionState {
    info: SessionInfo,
    lifecycle: SessionLifecycle,
    paused: bool,
    pause_generation: u64,
    failure_paused: bool,
    unloaded_with_input: bool,
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
    controls: BTreeMap<OperationId, Vec<Receipt>>,
    completion_order: Vec<TurnId>,
    history: Vec<HistoryItem>,
    history_loaded: bool,
    preparing: bool,
    saving: bool,
    history_work: Option<history::HistorySaveWork>,
    history_task: Option<tokio::task::JoinHandle<()>>,
    resume_after_saves: Option<u64>,
    resume_after_load: Option<u64>,
    host_draining: bool,
    host_stopping: bool,
    releasing: bool,
    released: bool,
    reclaimed: bool,
    block_reason: Option<Fault>,
    executor: Option<tokio::runtime::Handle>,
    driver_running: bool,
    driver_task: Option<tokio::task::JoinHandle<()>>,
    record_base: Option<SessionRecord>,
    record_work: Option<metadata::SessionRecordWork>,
    record_task: Option<tokio::task::JoinHandle<()>>,
    load_task: Option<tokio::task::JoinHandle<()>>,
    close_fault: Option<Fault>,
}

pub(crate) struct SessionController {
    pub(crate) id: SessionId,
    pub(crate) agent_id: AgentId,
    host_id: HostId,
    incarnation: HostIncarnation,
    access: Arc<ConnectionRegistry>,
    resources: Mutex<Option<Arc<dyn ResourceResolver>>>,
    backend: Arc<dyn HistoryBackend>,
    state: Mutex<SessionState>,
    store: Mutex<Option<SessionStore>>,
    root_lease: Mutex<Option<Arc<dyn Send + Sync>>>,
    // A completion notification carries no parent state or mutation authority.
    parent_changed: Option<Arc<Notify>>,
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
                agent_config_revision: 0,
                id: id.clone(),
                agent_id: agent_id.clone(),
                title: None,
                mode: backend.mode(),
                config_revision: 0,
                config,
            },
            lifecycle: SessionLifecycle::Open,
            paused: false,
            pause_generation: 0,
            failure_paused: false,
            unloaded_with_input: false,
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
            controls: BTreeMap::new(),
            completion_order: Vec::new(),
            history,
            history_loaded: backend.mode() == StorageMode::Memory,
            preparing: false,
            saving: false,
            history_work: None,
            history_task: None,
            resume_after_saves: None,
            resume_after_load: None,
            host_draining: false,
            host_stopping: false,
            releasing: false,
            released: false,
            reclaimed: false,
            block_reason: None,
            executor: tokio::runtime::Handle::try_current().ok(),
            driver_running: false,
            driver_task: None,
            record_base: None,
            record_work: None,
            record_task: None,
            load_task: None,
            close_fault: None,
        };
        let snapshot = Self::make_snapshot(&state, &host_id, &incarnation, &agent_id, &id);
        let (snapshots, _) = watch::channel(snapshot);
        Arc::new(Self {
            id,
            agent_id,
            host_id,
            incarnation,
            access,
            resources: Mutex::new(Some(resources)),
            backend,
            store: Mutex::new(None),
            root_lease: Mutex::new(None),
            parent_changed: None,
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
        let control = Self::control_facts(state);
        SessionSnapshot {
            host_id: host_id.clone(),
            host_incarnation: incarnation.clone(),
            agent_id: agent_id.clone(),
            session_id: id.clone(),
            data: SessionStateData {
                mode: state.info.mode,
                controller_revision: state.controller_revision,
                lifecycle: state.lifecycle,
                scheduling: control.scheduling,
                pause_reasons: control.pause_reasons,
                residency: state.residency,
                load_id: state.load_id.clone(),
                runner_revision: state.runner_revision,
                active_turn_id: state.active.as_ref().map(|active| active.info.id.clone()),
                next_step: state.next_step.iter().map(QueuedInput::summary).collect(),
                next_turn: state.next_turn.iter().map(QueuedInput::summary).collect(),
                preparing: state.preparing,
                saving: state.saving,
                metadata_saving: state.record_work.as_ref().is_some_and(|work| work.running),
                metadata_fault: state
                    .record_work
                    .as_ref()
                    .and_then(|work| work.fault.clone())
                    .or_else(|| state.close_fault.clone()),
                block_reason: control.block_reason,
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
        if let Some(parent_changed) = &self.parent_changed {
            parent_changed.notify_waiters();
        }
    }
    fn available(state: &SessionState) -> Result<(), Fault> {
        if state.reclaimed {
            return Err(Fault::new(
                FaultKind::AgentSessionNotFound,
                "memory Session was reclaimed",
            ));
        }
        if state.releasing || state.released {
            return Err(Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Session resources were retired",
            ));
        }
        Ok(())
    }
    fn open(state: &SessionState) -> Result<(), Fault> {
        Self::available(state)?;
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
    fn schedulable(&self, state: &SessionState) -> bool {
        if !self.access.execution_allowed_owned()
            || state.host_stopping
            || state.releasing
            || state.released
            || state.reclaimed
        {
            return false;
        }
        state.lifecycle == SessionLifecycle::Open
            && !state.paused
            && !state.failure_paused
            && !state.unloaded_with_input
            && state.agent_enabled.load(Ordering::SeqCst)
            && state.block_reason.is_none()
            && state.resume_after_saves.is_none()
    }
    pub(crate) fn is_open_owned(&self) -> bool {
        let state = self.state.lock().expect("Session mutex poisoned");
        state.lifecycle == SessionLifecycle::Open
            && !state.reclaimed
            && !state.releasing
            && !state.released
    }
    pub(crate) fn info(&self, caller: &CallerContext) -> Result<SessionInfo, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            Ok(self
                .state
                .lock()
                .expect("Session mutex poisoned")
                .info
                .clone())
        })
    }
    pub(crate) fn snapshot(&self, caller: &CallerContext) -> Result<SessionSnapshot, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            Ok(self.snapshots.borrow().clone())
        })
    }
    pub(crate) fn subscribe_state(
        self: &Arc<Self>,
        caller: &CallerContext,
    ) -> Result<SessionSubscription, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            Ok(SessionSubscription {
                owner: Arc::downgrade(self),
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
            self.ensure_visible()?;
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
            self.ensure_visible()?;
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
        self.access.ensure_execution_owned()?;
        let runtime = UserInput::text(&message.text)
            .map_err(|_| Fault::new(FaultKind::SessionInputInvalid, "input must not be empty"))?;
        let receipt = self.access.accept_business(caller, || {
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
        self.access.ensure_execution_owned()?;
        let runtime = UserInput::text(&message.text)
            .map_err(|_| Fault::new(FaultKind::SessionInputInvalid, "input must not be empty"))?;
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
        self.access.ensure_execution_owned()?;
        if state.host_stopping {
            return Err(Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Session is stopping",
            ));
        }
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
        self.resources_owned()?.validate(&candidate)?;
        let updated = self.access.accept_execution(caller, || {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            Self::open(&state)?;
            if state.info.mode == StorageMode::Persistent {
                return Err(Fault::new(
                    FaultKind::HostMethodUnavailable,
                    "persistent configuration updates require the asynchronous owner entry",
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
                .is_some_and(|fault| fault.kind() == Some(FaultKind::SessionResourceMissing))
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
            Self::available(&state)?;
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
    pub(crate) fn is_quiescent_owned(&self) -> bool {
        let state = self.state.lock().expect("Session mutex poisoned");
        state.active.is_none()
            && !state.preparing
            && !state.saving
            && !state.driver_running
            && state.history_work.is_none()
            && state.record_work.is_none()
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
            if state.releasing || state.released || state.reclaimed {
                return;
            }
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
            self.ensure_visible()?;
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
            self.ensure_visible()?;
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
            self.ensure_visible()?;
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
    #[cfg(test)]
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
            || state.preparing
            || !self.schedulable(&state)
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
        let previous = state.driver_task.take();
        state.driver_task = Some(executor.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            owner.drive().await;
        }));
    }
    async fn drive(self: Arc<Self>) {
        loop {
            let preparation = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                if !self.schedulable(&state)
                    || state.preparing
                    || state.active.is_some()
                    || (state.next_step.is_empty() && state.next_turn.is_empty())
                {
                    if state.lifecycle == SessionLifecycle::Closing
                        && state.info.mode == StorageMode::Memory
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
                    self.publish(&mut state);
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
                {
                    let mut state = self.state.lock().expect("Session mutex poisoned");
                    state.residency = Residency::Loading;
                    self.publish(&mut state);
                }
                match AssertUnwindSafe(self.backend.load())
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        Err(Fault::new(
                            FaultKind::SessionHistoryLoadFailed,
                            "history loader failed unexpectedly",
                        ))
                    }) {
                    Ok(history) => {
                        let mut state = self.state.lock().expect("Session mutex poisoned");
                        state.history = history;
                        state.history_loaded = true;
                        // Validated history is Controller-owned; no Runner has
                        // been bound yet. Do not leave a completed load stuck
                        // in loading if pause/cancel prevents Turn binding.
                        state.residency = Residency::Unloaded;
                        state.load_id = None;
                        self.finish_load_resume(&mut state);
                    }
                    Err(fault) => {
                        self.preparation_failed(fault);
                        return;
                    }
                }
            }
            let resources = AssertUnwindSafe(self.prepare_resources(&preparation.0))
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
                    if state.lifecycle == SessionLifecycle::Closing
                        && state.info.mode == StorageMode::Memory
                    {
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
                if !self.schedulable(&state) || state.info.config_revision != preparation.1 {
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
                    partial_accepted: false,
                    terminal_accepted: false,
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
        state.resume_after_load = None;
        if fault.kind() == Some(FaultKind::SessionHistoryLoadFailed) {
            state.residency = Residency::Unloaded;
            state.load_id = None;
        }
        state.block_reason = Some(fault);
        state.driver_running = false;
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
                if active.info.state == TurnState::Cancelling
                    || state.host_stopping
                    || !self.access.execution_allowed_owned()
                {
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
                    active.partial_accepted = false;
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
                active.handle = None;
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
                        LoopOutcome::Completed if active.info.state == TurnState::Cancelling => {
                            TurnOutcome::Cancelled
                        }
                        LoopOutcome::Completed => TurnOutcome::Completed,
                        LoopOutcome::Cancelled(_) => TurnOutcome::Cancelled,
                        _ => TurnOutcome::Failed,
                    });
                    active.info.state = TurnState::Finalizing;
                    if active.info.outcome == Some(TurnOutcome::Failed) {
                        state.failure_paused = true;
                    }
                }
                self.publish(&mut state);
                return;
            }
            let next = {
                let mut state = self.state.lock().expect("Session mutex poisoned");
                let schedulable = self.schedulable(&state);
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
                        LoopOutcome::Failed(failure) => (
                            TurnOutcome::Failed,
                            (failure.kind != minicore_runtime::LoopFailureKind::History)
                                .then(|| runtime_fault(failure)),
                        ),
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
        let mut state = self.session.state.lock().expect("Session mutex poisoned");
        if state.host_stopping || !self.session.access.execution_allowed_owned() {
            let handle = state
                .active
                .as_mut()
                .filter(|active| active.info.id == self.turn_id)
                .and_then(|active| {
                    if matches!(active.info.state, TurnState::Starting | TurnState::Running) {
                        active.info.state = TurnState::Cancelling;
                    }
                    active.handle.clone()
                });
            drop(state);
            if let Some(handle) = handle {
                handle.shutdown();
            }
            return Vec::new();
        }
        // Already bound remainder is handoff storage, not a third inbox. Pause
        // never withdraws it. Force does not consume even a bound remainder.
        let mut values = self
            .initial
            .lock()
            .expect("bound input mutex poisoned")
            .drain(..)
            .map(|input| input.runtime)
            .collect::<Vec<_>>();
        if self.session.schedulable(&state)
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
            let result = self
                .session
                .accept_history(&self.turn_id, loop_id, offset, items)
                .await;
            // Observe the Host's accepted force fence at the Runtime boundary,
            // even if direct-owner traversal has not reached this Session yet.
            // Save completion is never cancelled, but subsequent business is.
            if !self.session.access.execution_allowed_owned() {
                let handle = {
                    let state = self.session.state.lock().expect("Session mutex poisoned");
                    state
                        .active
                        .as_ref()
                        .filter(|active| {
                            active.info.id == self.turn_id && active.loop_id == Some(loop_id)
                        })
                        .and_then(|active| active.handle.clone())
                };
                if let Some(handle) = handle {
                    handle.shutdown();
                }
            }
            result.map_err(|_| {
                DiagnosticSummary::new(
                    DiagnosticCode::ContextFailed,
                    DiagnosticCategory::Context,
                    Text::new("Session could not confirm its history save boundary"),
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
