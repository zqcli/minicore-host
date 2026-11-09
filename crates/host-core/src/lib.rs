//! Direct owners and the in-process Host entry point.
//!
//! A connection supplies authority, not an entity selector. Every operation
//! resolves through Host -> Agent -> SessionController; Runtime is bound to
//! one Turn and never owns public scheduling.

mod agent;
mod backend;
mod collaboration;
mod control;
pub use collaboration::*;
mod host;
mod persistence;
mod prompt_history;
pub use prompt_history::prepare_prompt_history;
mod session;
mod types;

pub use backend::JournalHistory;
pub use control::{CallerContext, ConnectionHandle};
pub use host::Host;
pub use session::{
    ApprovalAnswer, BoundResources, HistoryBackend, InteractionAnswerData, LoadFuture,
    MemoryHistory, ResourceFuture, ResourceResolver, SaveFuture, SessionSubscription,
    ToolOutputBinding, ToolOutputSink,
};
pub use types::*;

mod observation;
pub use observation::{
    ActivityEvent, ApprovalRiskData, EventSubscription, HostStateData, InteractionRequestData,
    InteractionResolution, InteractionResolvedData, MessageEventData, ModelFinishedData,
    ModelResponseData, ObservationEvent, ObservationNotification, PendingInteractionData,
    StateEvent, ToolCallData, ToolCallState, ToolCallStateData, ToolFinishedData,
    ToolInvocationData, ToolProgressData, ToolResultData, TurnSnapshot, TurnStateData,
};
