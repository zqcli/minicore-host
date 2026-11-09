//! Ordinary tools receive their frozen workspace and actual Turn record owner,
//! not the agent tool's business port or a model-supplied source identity.
use minicore_host_core::{
    OwnerFuture, PreparedTurnBinding, ToolEnvironmentExclusions, ToolOutputLease, ToolOutputSink,
    TurnCapability, TurnToolCleanup,
};
use minicore_host_local_tools::{CommandEnvironment, LocalToolSink, LocalTools, RecordError};
use minicore_host_protocol::{Fault, FaultKind};
use minicore_host_workspace::Workspace;
use minicore_runtime::tools::{Tool, ToolContext, ToolError, ToolFuture, ToolInvocation, ToolSpec};
use std::sync::Arc;

/// Actual Session-bound recording, distinct from an observer or a standalone
/// no-op sink. `begin` runs before the local worker starts, from the real Runtime
/// invocation. The output lease carries its owner-minted Turn and Load identity.
/// All callbacks are short, synchronous ownership handoffs; necessary saving
/// remains owned by the receiving Session rather than by event subscribers.
pub trait LocalToolInvocationSink: LocalToolSink {
    fn begin(
        &self,
        invocation: &ToolInvocation,
        output: ToolOutputLease,
    ) -> Result<(), RecordError>;
}

pub struct BoundLocalTools {
    tools: Arc<LocalTools>,
    capability: TurnCapability,
    sink: Arc<dyn LocalToolInvocationSink>,
}
impl BoundLocalTools {
    /// Production uses the original Session owner, never a standalone no-op
    /// sink. The adapter retains no second result store or execution scheduler.
    pub fn production(binding: PreparedTurnBinding, workspace: Workspace) -> Result<Self, Fault> {
        Self::prepare(
            binding,
            workspace,
            Arc::new(crate::local_records::SessionLocalToolSink::default()),
        )
    }

    pub fn prepare(
        binding: PreparedTurnBinding,
        workspace: Workspace,
        sink: Arc<dyn LocalToolInvocationSink>,
    ) -> Result<Self, Fault> {
        let environment = Arc::new(HostCommandEnvironment(
            binding.tool_environment_exclusions()?,
        ));
        let tools = Arc::new(LocalTools::new(workspace, sink.clone(), environment));
        binding.register_tool_cleanup(Arc::new(LocalCleanup(tools.clone())))?;
        Ok(Self {
            tools,
            capability: binding.capability(),
            sink,
        })
    }
    pub(crate) fn tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(BoundLocalTool {
            tool: self.tools.tool(name)?,
            capability: self.capability.clone(),
            sink: self.sink.clone(),
        }))
    }
}
struct HostCommandEnvironment(ToolEnvironmentExclusions);
impl CommandEnvironment for HostCommandEnvironment {
    fn excluded_names(&self) -> Vec<String> {
        self.0.names()
    }
}
struct BoundLocalTool {
    tool: Arc<dyn Tool>,
    capability: TurnCapability,
    sink: Arc<dyn LocalToolInvocationSink>,
}
impl Tool for BoundLocalTool {
    fn spec(&self) -> &ToolSpec {
        self.tool.spec()
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, context: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            self.capability
                .check_active()
                .map_err(|_| ToolError::Internal)?;
            let output = ToolOutputSink::current()
                .ok_or(ToolError::Internal)?
                .retain_owned()
                .map_err(|_| ToolError::Internal)?;
            self.sink
                .begin(&invocation, output)
                .map_err(|_| ToolError::Internal)?;
            self.tool.execute(invocation, context).await
        })
    }
}
struct LocalCleanup(Arc<LocalTools>);
impl TurnToolCleanup for LocalCleanup {
    fn finish(&self) -> OwnerFuture<'_, Vec<Fault>> {
        Box::pin(async move {
            let completions = self.0.finish().await;
            if let Some(completion) = completions.iter().find(|result| !result.cleanup_confirmed) {
                return Err(Fault::new(
                    FaultKind::TurnInternalUnexpected,
                    format!(
                        "tool call {} did not confirm actual worker cleanup",
                        completion.tool_call_id
                    ),
                ));
            }
            Ok(completions
                .iter()
                .filter(|result| result.error == Some(ToolError::Internal))
                .map(|result| {
                    Fault::new(
                        FaultKind::TurnInternalUnexpected,
                        format!(
                            "tool call {} returned an internal fault after actual cleanup",
                            result.tool_call_id
                        ),
                    )
                })
                .collect())
        })
    }
}
