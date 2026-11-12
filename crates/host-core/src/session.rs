use crate::control::{CallerContext, ConnectionRegistry};
use crate::diagnostics::{self as diagnostic, Record};
use crate::types::*;
use futures_util::FutureExt;
use minicore_host_protocol::*;
use minicore_host_storage::{HistoryEntry, SessionRecord, SessionStore};

mod close;
mod collaboration;
mod compaction;
mod context;
mod controls;
mod deletion;
mod diagnostics;
mod discovery;
mod execution;
mod history;
mod input;
mod input_provenance;
mod interaction;
mod loading;
pub use interaction::{ApprovalAnswer, InteractionAnswerData};
mod metadata;
mod model;
mod observation;
mod query;
mod tool_edits;
mod tool_output;
pub use tool_output::{ToolOutputBinding, ToolOutputLease, ToolOutputSink};
mod tool_cleanup;
pub use tool_cleanup::TurnToolCleanup;
mod retirement;
pub(crate) use retirement::SessionShutdownStatus;
mod persistent;
use minicore_host_runtime::{
    ExecutionConfig, HistoryItem, LoopId, Text, UserInput,
    error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary},
};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;
use tokio::sync::{Notify, watch};
use tracing::Instrument;

use crate::backend::HistoryBackend;
use crate::resources::{BoundResources, ResourceResolver};
use execution::runtime_model_fault;
use input::{QueuedInput, clear_inbox, mark_message, take_batch};
pub use observation::SessionSubscription;
use tokio_util::sync::CancellationToken;

#[cfg(test)]
use crate::{HistoryReadFuture, LoadFuture, MemoryHistory, ResourceFuture, SaveFuture};
#[cfg(test)]
use minicore_host_runtime::ExecutionOptions;
#[cfg(test)]
use std::pin::Pin;

struct ActiveTurn {
    started_at: Instant,
    _activation: crate::turn_binding::ActivationLease,
    tool_cleanup: tool_cleanup::ToolCleanupWork,
    context_binding: crate::context::BoundContextConfiguration,
    info: TurnInfo,
    loop_id: LoopId,
    cancellation: CancellationToken,
    consumed_inputs: Vec<MessageId>,
    bound_inputs: Vec<MessageId>,
    start_history: usize,
    // Native position already handed to the original save owner, not a
    // durability claim. An unconfirmed range remains in HistorySaveWork.
    save_cursor: usize,
    partials: Vec<PartialResult>,
    pending_partial: Option<minicore_host_runtime::model::ModelPartial>,
    deadline_expired: bool,
    partial_accepted: bool,
    terminal_accepted: bool,
    pending_interaction: Option<interaction::PendingAnswer>,
}
impl ActiveTurn {
    fn claim_terminal(&mut self, outcome: TurnOutcome, fault: Option<Fault>) -> bool {
        let newly_claimed = self.info.outcome.is_none();
        if newly_claimed {
            if self.info.state == TurnState::Cancelling {
                self.info.outcome = Some(TurnOutcome::Cancelled);
                self.info.cleanup_faults.extend(fault);
            } else {
                self.info.outcome = Some(outcome);
                self.info.fault = fault;
            }
        }
        self._activation.expire();
        self.cancellation.cancel();
        self.info.state = TurnState::Finalizing;
        self.info.phase = Some(TurnPhase::Finalizing);
        newly_claimed
    }
}
/// A held result keeps its original native slice position in the Session's
/// append-only history. No second input index or projected transcript is stored.
struct RetainedTurnResult {
    result: TurnResult,
    start_history: usize,
}
impl std::ops::Deref for RetainedTurnResult {
    type Target = TurnResult;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

struct SessionState {
    info: SessionInfo,
    lifecycle: SessionLifecycle,
    end_reason: Option<String>,
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
    completed: BTreeMap<TurnId, RetainedTurnResult>,
    controls: BTreeMap<OperationId, Vec<Receipt>>,
    completion_order: Vec<TurnId>,
    history: Arc<Vec<HistoryItem>>,
    input_provenance: input_provenance::InputProvenance,
    history_position: u64,
    projection: Option<minicore_host_storage::CompactionProjection>,
    compaction: Option<compaction::CompactionWork>,
    compaction_task: Option<tokio::task::JoinHandle<()>>,
    last_compaction: Option<CompactionActivity>,
    finished_manual_compactions:
        BTreeMap<(OperationId, usize), compaction::FinishedManualCompaction>,
    // Accepted content in original order, not model context or save confirmation.
    // None until loaded; retained on unload and released with the Session body.
    readable_history: Option<Vec<HistoryItemData>>,
    output: BTreeMap<TurnId, observation::TurnObservation>,
    edits: minicore_host_workspace::EditJournal,
    edits_complete: bool,
    closed_published: bool,
    published: bool,
    history_loaded: bool,
    preparing: bool,
    history_work: Option<history::HistorySaveWork>,
    history_task: Option<tokio::task::JoinHandle<()>>,
    resume_after_saves: Option<u64>,
    resume_after_load: Option<u64>,
    host_draining: bool,
    host_stopping: bool,
    releasing: bool,
    released: bool,
    reclaimed: bool,
    deleting: bool,
    deletion: Option<SessionDeletionState>,
    block_reason: Option<Fault>,
    executor: Option<tokio::runtime::Handle>,
    driver_running: bool,
    driver_task: Option<tokio::task::JoinHandle<()>>,
    record_base: Option<SessionRecord>,
    record_work: Option<metadata::SessionRecordWork>,
    record_task: Option<tokio::task::JoinHandle<()>>,
    load_task: Option<tokio::task::JoinHandle<()>>,
    close_fault: Option<Fault>,
    root_scope: Option<Arc<crate::collaboration::ScopeState>>,
    scope_task: Option<tokio::task::JoinHandle<()>>,
    scope_cleanup_pending: bool,
    delegation: Option<collaboration::DelegationSlot>,
    handoff_pending: bool,
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
        let readable_history = (backend.mode() == StorageMode::Memory)
            .then(|| history.iter().map(HistoryItemData::from).collect());
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
            end_reason: None,
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
            edits_complete: history.is_empty(),
            history_position: history.len() as u64,
            history: Arc::new(history),
            input_provenance: Default::default(),
            projection: None,
            compaction: None,
            compaction_task: None,
            last_compaction: None,
            finished_manual_compactions: BTreeMap::new(),
            readable_history,
            output: BTreeMap::new(),
            edits: minicore_host_workspace::EditJournal::new(id.clone()),
            closed_published: false,
            published: false,
            history_loaded: backend.mode() == StorageMode::Memory,
            preparing: false,
            history_work: None,
            history_task: None,
            resume_after_saves: None,
            resume_after_load: None,
            host_draining: false,
            host_stopping: false,
            releasing: false,
            released: false,
            reclaimed: false,
            deleting: false,
            deletion: None,
            block_reason: None,
            executor: tokio::runtime::Handle::try_current().ok(),
            driver_running: false,
            driver_task: None,
            record_base: None,
            record_work: None,
            record_task: None,
            load_task: None,
            close_fault: None,
            root_scope: None,
            scope_task: None,
            scope_cleanup_pending: false,
            delegation: None,
            handoff_pending: false,
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
            state: Mutex::new(state),
            changed: Notify::new(),
            snapshots,
        })
    }
    fn available(state: &SessionState) -> Result<(), Fault> {
        if state.deleting {
            return Err(Fault::new(
                FaultKind::SessionBusy,
                "Session deletion has fenced new controls",
            ));
        }
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
        if state
            .root_scope
            .as_ref()
            .is_some_and(|scope| scope.sealed.load(Ordering::SeqCst))
            || state
                .delegation
                .as_ref()
                .is_some_and(|slot| slot.scope_closed())
        {
            return Err(Fault::new(
                FaultKind::SessionLifecycleClosing,
                "the root collaboration scope is closing",
            ));
        }
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
            && !state
                .root_scope
                .as_ref()
                .is_some_and(|scope| scope.sealed.load(Ordering::SeqCst))
            && !state.paused
            && state.compaction.is_none()
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
}

#[cfg(test)]
mod tests;
