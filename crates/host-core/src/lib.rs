//! Direct owners and the in-process Host entry point.
//!
//! A connection supplies authority, not an entity selector. Every operation
//! resolves through Host -> Agent -> SessionController; Runtime is bound to
//! one Turn and never owns public scheduling.

mod agent;
mod backend;
mod control;
mod host;
mod persistence;
mod session;
mod types;

pub use backend::JournalHistory;
pub use control::{CallerContext, ConnectionHandle};
pub use host::Host;
pub use session::{
    BoundResources, HistoryBackend, LoadFuture, MemoryHistory, ResourceFuture, ResourceResolver,
    SaveFuture, SessionSubscription, ToolOutputBinding, ToolOutputSink,
};
pub use types::*;

mod observation;
pub use observation::{
    ActivityEvent, EventSubscription, HostStateData, MessageEventData, ModelFinishedData,
    ModelResponseData, ObservationEvent, ObservationNotification, StateEvent, ToolCallData,
    ToolCallState, ToolCallStateData, ToolFinishedData, ToolInvocationData, ToolProgressData,
    ToolResultData, TurnStateData,
};
