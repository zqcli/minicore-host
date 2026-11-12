//! Read-only context facts. Counts describe retained native history, not token
//! usage or an assembled next request. Queries never prepare a model prompt.
use crate::CompactionActivity;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    SummaryHistory,
    model::{ModelDescriptor, Usage},
};
use serde::{Deserialize, Serialize};

/// Context and execution facts from the actual Session owner. Reading this
/// snapshot neither loads a Runner nor resumes input consumption.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionContextSnapshot {
    #[serde(flatten)]
    pub source: SessionIdentity,
    pub data: SessionContextData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionContextData {
    pub controller_revision: ControllerRevision,
    pub residency: Residency,
    pub load_id: Option<LoadId>,
    /// Whether the owner has loaded and validated its execution history. A
    /// retained or cold content read does not change this execution fact.
    pub history_loaded: bool,
    /// The committed configuration for the next Turn, not the active binding.
    pub config_revision: ConfigRevision,
    pub model: ModelRef,
    pub compaction_settings: CompactionSettings,
    pub active_turn: Option<ContextTurnBinding>,
    /// Unbound queue entries only. Bound input is authored by the Session as
    /// readable native history; required save confirmation remains separate.
    pub queued_inputs: ContextQueuedInputs,
    /// None until this Session has actually accepted or loaded native history.
    /// A cold persistent Session is unknown, not an empty context. This query
    /// does not open JSONL; explicit session.load/resume owns validation.
    pub history: Option<ContextHistory>,
    /// The owner's current utility operation, or its last completed operation.
    /// Its before/after token values remain the compaction engine's estimates,
    /// and utility_usage remains distinct from ordinary model responses.
    pub compaction: Option<CompactionActivity>,
    pub history_save: ContextHistorySave,
    pub block_reason: Option<Fault>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextQueuedInputs {
    pub next_step: u64,
    pub next_turn: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextTurnBinding {
    pub turn_id: TurnId,
    pub load_id: LoadId,
    pub config_revision: ConfigRevision,
    /// Descriptor of that Turn's actual prepared model, not today's Host list.
    pub model: ContextBoundModel,
    /// Captured at Turn admission and unchanged through internal continuations.
    pub compaction_settings: CompactionSettings,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextBoundModel {
    pub model: minicore_host_runtime::model::ModelRef,
    /// The model descriptor's declared context window, not measured usage or a
    /// new admission limit. No output allowance or safety margin is invented.
    pub context_window: u64,
}

/// Value-only facts held by the existing active Turn; no extra resource owner.
pub(crate) struct BoundContextConfiguration {
    pub(crate) compaction_settings: CompactionSettings,
    pub(crate) model: ContextBoundModel,
}
impl BoundContextConfiguration {
    pub(crate) fn new(config: &SessionConfig, model: &ModelDescriptor) -> Self {
        Self {
            compaction_settings: config.execution.compaction.clone(),
            model: ContextBoundModel {
                model: model.model_ref.clone(),
                context_window: model.context_window,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextHistory {
    /// Exclusive native journal entry position, including markers and partials;
    /// not a client cursor or a persistence watermark.
    pub position: u64,
    /// Complete native items only. Excludes queued input, partial model output,
    /// finish records, and compaction markers.
    pub raw_items: u64,
    /// Committed summary (if any) plus uncovered raw suffix, including bound
    /// unsaved input. Excludes fixed instructions, tool schemas and queued input.
    pub effective_items: u64,
    pub projection: Option<ContextProjection>,
    /// Latest complete assistant record in raw history, if any. It is a past
    /// response's reported usage, never the size of today's effective context.
    /// A response without usage stays here with absent counts, rather than
    /// falling back to an older response's usage.
    pub latest_response: Option<ContextResponseUsage>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextProjection {
    pub source_position: u64,
    pub covered_items: u64,
    /// The actual committed summary; pending candidates never appear here.
    pub summary: SummaryHistory,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextResponseUsage {
    /// Zero-based item index in this Session's raw native history.
    pub raw_item_index: u64,
    pub model: minicore_host_runtime::model::ModelRef,
    pub covered_by_compaction: bool,
    /// Runtime preserves absent provider fields as None. No counters are
    /// inferred from text length, combined, or copied from utility calls.
    pub usage: Usage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextHistorySave {
    /// An original save remains owned, including a failed save awaiting explicit
    /// reconciliation. False does not assert that cold readable bytes synced.
    pub pending: bool,
    pub running: bool,
    pub fault: Option<Fault>,
}
