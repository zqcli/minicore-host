#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

mod command;
mod environment;
mod files;
mod records;
mod schema;
mod scope;

pub use environment::{CommandEnvironment, InheritCommandEnvironment};
pub use records::*;
pub use schema::local_tool_specs;
pub use scope::LocalToolCompletion;

use std::{sync::Arc, time::Instant};

use minicore_host_workspace::Workspace;
use minicore_runtime::tools::{
    Tool, ToolContext, ToolError, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput,
    ToolSpec,
};
use serde_json::{Value, json};

/// One frozen workspace and narrow, already Turn-bound recording capability.
/// Retain this handle until `finish` joins every actual execution.
/// Separate Turns sharing file mutations must receive clones of one Workspace
/// binding, not independently opened bindings for the same path.
pub struct LocalTools {
    binding: Arc<Binding>,
    scope: Arc<scope::Scope>,
}

struct Binding {
    workspace: Workspace,
    sink: Arc<dyn LocalToolSink>,
    environment: Arc<dyn CommandEnvironment>,
}

impl LocalTools {
    pub fn new(
        workspace: Workspace,
        sink: Arc<dyn LocalToolSink>,
        environment: Arc<dyn CommandEnvironment>,
    ) -> Self {
        Self {
            binding: Arc::new(Binding {
                workspace,
                sink,
                environment,
            }),
            scope: Arc::new(scope::Scope::default()),
        }
    }

    /// Returns only a named implementation. The caller selects the actual ToolSet.
    pub fn tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let spec = local_tool_specs()
            .into_iter()
            .find(|spec| spec.name().as_str() == name)?;
        Some(Arc::new(LocalTool {
            binding: Arc::clone(&self.binding),
            scope: Arc::clone(&self.scope),
            spec,
        }))
    }

    /// Close this Turn's admission, then join its workers. This is idempotent;
    /// dropping a waiter does not drop the workers or their cleanup collector.
    /// Runtime's own join is not sufficient after cancellation or a deadline.
    pub async fn finish(&self) -> Arc<[LocalToolCompletion]> {
        self.scope.finish().await
    }
}

struct LocalTool {
    binding: Arc<Binding>,
    scope: Arc<scope::Scope>,
    spec: ToolSpec,
}

impl Tool for LocalTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation, context: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            let prepared = (|| {
                if invocation.tool_name() != self.spec.name() {
                    return Err(ToolError::InvalidInvocation);
                }
                check(&context)?;
                Action::parse(self.spec.name().as_str(), invocation.arguments().clone())
            })();
            let action = match prepared {
                Ok(action) => action,
                Err(error) => {
                    // Rejection happened before an actual worker was started.
                    self.binding
                        .sink
                        .complete(invocation.tool_call_id(), true)
                        .map_err(|_| ToolError::Internal)?;
                    return Err(error);
                }
            };
            let binding = Arc::clone(&self.binding);
            let sink = Arc::clone(&binding.sink);
            let id = invocation.tool_call_id().clone();
            let completed_id = id.clone();
            let completion = self.scope.start(id.clone(), async move {
                let mut outcome = match action {
                    Action::Read(input) => {
                        scope::OwnedOutcome::settled(files::read(binding, input, context).await)
                    }
                    Action::Edit(input) => {
                        scope::OwnedOutcome::settled(files::edit(binding, id, input, context).await)
                    }
                    Action::Write(input) => scope::OwnedOutcome::settled(
                        files::write(binding, id, input, context).await,
                    ),
                    Action::Bash(input) => command::run(binding, id, input, context).await,
                };
                // Actual I/O and all its output/observations are settled here,
                // even if Runtime already dropped its waiting ToolFuture.
                if sink
                    .complete(&completed_id, outcome.cleanup_confirmed)
                    .is_err()
                {
                    outcome.result = Err(ToolError::Internal);
                }
                outcome
            });
            let completion = match completion {
                Ok(completion) => completion,
                Err(error) => {
                    self.binding
                        .sink
                        .complete(invocation.tool_call_id(), true)
                        .map_err(|_| ToolError::Internal)?;
                    return Err(error);
                }
            };
            completion.await.map_err(|_| ToolError::Internal)?
        })
    }
}

enum Action {
    Read(files::ReadInput),
    Edit(files::EditInput),
    Write(files::WriteInput),
    Bash(command::BashInput),
}

impl Action {
    fn parse(name: &str, value: Value) -> Result<Self, ToolError> {
        let invalid = |_| ToolError::InvalidInvocation;
        match name {
            "read" => serde_json::from_value(value)
                .map(Self::Read)
                .map_err(invalid),
            "edit" => serde_json::from_value(value)
                .map(Self::Edit)
                .map_err(invalid),
            "write" => serde_json::from_value(value)
                .map(Self::Write)
                .map_err(invalid),
            "bash" => serde_json::from_value(value)
                .map(Self::Bash)
                .map_err(invalid),
            _ => Err(ToolError::InvalidInvocation),
        }
    }
}

fn check(context: &ToolContext) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        Err(ToolError::Cancelled)
    } else if Instant::now() >= context.deadline {
        Err(ToolError::TimedOut)
    } else {
        Ok(())
    }
}

fn completed(value: impl serde::Serialize) -> Result<ToolExecutionOutcome, ToolError> {
    let text = serde_json::to_string(&value).map_err(|_| ToolError::Internal)?;
    Ok(ToolExecutionOutcome::Completed(
        ToolOutput::new(text).map_err(|_| ToolError::Internal)?,
    ))
}

fn business_error(code: &str, message: impl AsRef<str>) -> Result<ToolExecutionOutcome, ToolError> {
    completed(json!({"status":"failed","error":{"code":code,"message":message.as_ref()}}))
}
