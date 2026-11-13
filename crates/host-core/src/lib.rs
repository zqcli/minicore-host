//! Direct owners and the in-process Host entry point.
//!
//! A connection supplies authority, not an entity selector. Every operation
//! resolves through Host -> Agent -> SessionController; Runtime is bound to
//! one Turn and never owns public scheduling.

mod agent;
mod backend;
mod collaboration;
mod turn_binding;
pub use turn_binding::{PreparedTurnBinding, TurnCapability, TurnSource};
mod configuration;
mod context;
mod control;
mod diagnostics;
pub use collaboration::*;
pub use configuration::{
    HostConfig, HostConfigSnapshot, HostConfigurationFuture, HostConfigurationPreparer,
    HostReloadResult, LogLevel, LogOutput, LogSettings, ModelDescription, NetworkSettings,
    OpenAiModelApi, OpenAiModelSettings, PreparedHostConfiguration, RestartField,
    ToolEnvironmentExclusions,
};
pub use context::*;
mod host;
mod persistence;
mod prompt_history;
pub use prompt_history::prepare_prompt_history;
mod query;
pub use query::*;
mod discovery;
pub use discovery::*;
mod session;
mod types;

pub use backend::{
    HistoryBackend, HistoryReadFuture, JournalHistory, LoadFuture, MemoryHistory, SaveFuture,
};
mod resources;
pub use control::{CallerContext, ConnectionHandle};
pub use host::{Host, HostProcessControl, HostProcessSnapshot, WorkspaceSnapshot};
pub use resources::{
    BoundCompactionResources, BoundResources, CompactionResourceFuture, ResourceFuture,
    ResourceResolver,
};
pub use session::{
    ApprovalAnswer, InteractionAnswerData, SessionSubscription, ToolOutputBinding, ToolOutputLease,
    ToolOutputSink, TurnToolCleanup,
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
