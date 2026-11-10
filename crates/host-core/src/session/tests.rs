use super::*;
use crate::backend::JournalHistory;
use minicore_host_storage::{
    AgentData, AgentRecord, DataRoot, HistoryRecord, HostRecord, SessionData, SessionRecord,
    SessionStore,
};
use minicore_runtime::{
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Semaphore, mpsc};

mod support;
pub(super) use support::original_save_fixture;
use support::*;

mod admission;
mod compaction;
mod deletion;
mod discovery;
mod history_read;
mod resource_resume;
mod retirement;
mod save_boundaries;
mod save_continuation;
mod terminal_claims;
