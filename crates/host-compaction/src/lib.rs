//! Host-owned compaction calculations, without Session state or persistence.
//!
//! The caller supplies the currently bound model and its already prepared,
//! one-message-per-history-item request. Planning retains complete exchanges;
//! generation produces a candidate, never a committed history mutation. The
//! Session owner must still arbitrate cancellation, source applicability and
//! durable confirmation at its ordinary history-write boundary.

mod plan;
mod source;
mod utility;

pub use plan::{
    CompactionCandidate, CompactionPlan, ContextEstimate, HistorySnapshot, estimate_context,
    estimate_request_tokens, prepare,
};
pub use utility::{GenerationFailure, UtilityUsage, generate};

use minicore_host_runtime::model::ModelError;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq)]
pub enum CompactionError {
    #[error("compaction settings or current model capabilities are invalid")]
    InvalidConfiguration,
    #[error("history and its current request projection do not have usable exchange boundaries")]
    InvalidHistory,
    #[error("utility model no longer matches the prepared model capabilities")]
    ModelChanged,
    #[error("compaction was cancelled before producing a candidate")]
    Cancelled,
    #[error("compaction deadline expired before producing a candidate")]
    Timeout,
    #[error("utility response did not finish with nonempty valid summary text")]
    InvalidResponse,
    #[error("utility response requested a tool while tools were disabled")]
    UnexpectedToolCall,
    #[error("summary did not reduce the estimated request size")]
    NoProgress,
    #[error("utility request position cannot be represented by Runtime")]
    RequestIndexOverflow,
    #[error("compaction data could not be serialized")]
    Serialization,
    #[error(transparent)]
    Model(#[from] ModelError),
}
