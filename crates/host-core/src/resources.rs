//! Immutable resource contracts supplied by the application and bound by a Session.
use crate::{PreparedTurnBinding, ToolOutputBinding};
use minicore_host_protocol::*;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{Model, ReasoningPreference},
    prompt::PromptProvider,
    tools::ToolSpec,
};
use std::{future::Future, pin::Pin, sync::Arc};

pub type ResourceFuture<'a> =
    Pin<Box<dyn Future<Output = Result<BoundResources, Fault>> + Send + 'a>>;

/// The application composes resources. An immutable result is bound for the
/// whole public Turn, including later internal loops. No default substitution.
pub trait ResourceResolver: Send + Sync + 'static {
    /// Pure structural/reference validation. Do not call a model, tool, network,
    /// or filesystem here; resource preparation belongs in `resolve`.
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault>;
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a>;
    /// Prepare the complete selected tool set before any input is claimed.
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        _binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        self.resolve(config)
    }
    /// Prepare model, fixed prompt and selected schemas without a Turn or tool
    /// execution authority. The returned future owns its immutable resource view.
    fn resolve_for_compaction(self: Arc<Self>, config: SessionConfig) -> CompactionResourceFuture {
        Box::pin(async move {
            let bound = self.resolve(&config).await?;
            let names = config
                .tools
                .iter()
                .map(|tool| tool.name.parse())
                .collect::<Result<_, _>>()
                .map_err(|_| {
                    Fault::new(
                        FaultKind::SessionConfigInvalid,
                        "invalid selected tool name",
                    )
                })?;
            Ok(BoundCompactionResources {
                model: bound.execution.model().clone(),
                reasoning: bound.execution.reasoning(),
                tools: bound.execution.tools().specs_for(&names),
                prompt: bound.execution.prompt().clone(),
            })
        })
    }
    /// Attach only the original Turn's output sink to already-prepared tools.
    /// This infallible, non-I/O step must not select resources or prepare business authority.
    /// Those fallible steps belong in `resolve`, before input is consumed.
    fn bind_tool_output(
        &self,
        execution: &ExecutionConfig,
        _output: ToolOutputBinding,
    ) -> ExecutionConfig {
        execution.clone()
    }
}
#[derive(Clone)]
pub struct BoundResources {
    pub execution: ExecutionConfig,
    pub options: LoopOptions,
}

pub type CompactionResourceFuture =
    Pin<Box<dyn Future<Output = Result<BoundCompactionResources, Fault>> + Send>>;
#[derive(Clone)]
pub struct BoundCompactionResources {
    pub model: Arc<dyn Model>,
    pub reasoning: ReasoningPreference,
    pub tools: Vec<ToolSpec>,
    pub prompt: Arc<dyn PromptProvider>,
}
