#![doc = include_str!("../README.md")]
#![deny(unsafe_code)]

mod command;
mod environment;
mod files;
mod patch;
mod records;
mod schema;
mod scope;

pub use environment::{CommandEnvironment, InheritCommandEnvironment};
pub use files::{ResolvedPolicyPath, resolve_policy_path};
pub use records::*;
pub use schema::local_tool_specs;
pub use scope::LocalToolCompletion;

use std::{sync::Arc, time::Instant};

use minicore_host_runtime::tools::{
    Tool, ToolContext, ToolError, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput,
    ToolSpec,
};
use minicore_host_workspace::Workspace;
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
        let spec = schema::local_tool_spec(name)?;
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
            let ToolInvocation {
                tool_call_id: id,
                tool_name,
                arguments,
            } = invocation;
            let prepared = (|| {
                if &tool_name != self.spec.name() {
                    return Err(Err(ToolError::InvalidInvocation));
                }
                check(&context).map_err(Err)?;
                Action::parse(self.spec.name().as_str(), arguments).map_err(|message| {
                    // The model can only correct arguments it is told about.
                    rejected("invalid_arguments", format!("invalid arguments: {message}"))
                })
            })();
            let action = match prepared {
                Ok(action) => action,
                Err(result) => {
                    // Rejection happened before an actual worker was started.
                    self.binding
                        .sink
                        .complete(&id, true)
                        .map_err(|_| ToolError::Internal)?;
                    return result;
                }
            };
            let binding = Arc::clone(&self.binding);
            let sink = Arc::clone(&binding.sink);
            let completed_id = id.clone();
            let rejected_id = id.clone();
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
                    Action::Patch(input) => scope::OwnedOutcome::settled(
                        files::apply_patch(binding, id, input, context).await,
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
                        .complete(&rejected_id, true)
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
    Patch(files::PatchInput),
    Bash(command::BashInput),
}

impl Action {
    fn parse(name: &str, value: Value) -> Result<Self, String> {
        let invalid = |error: serde_json::Error| describe(&error.to_string());
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
            "apply_patch" => serde_json::from_value(value)
                .map(Self::Patch)
                .map_err(invalid),
            "bash" => serde_json::from_value(value)
                .map(Self::Bash)
                .map_err(invalid),
            _ => Err(format!("unknown local tool {name}")),
        }
    }
}

/// Serde's diagnostic, without echoing an unknown name or a supplied value:
/// those are model text. The expected names come from our own input types.
fn describe(message: &str) -> String {
    for prefix in [
        "unknown field",
        "unknown variant",
        "invalid type",
        "invalid value",
    ] {
        if message.starts_with(prefix) {
            return match message.rsplit_once(", expected ") {
                Some((_, expected)) => format!("{prefix}; expected {expected}"),
                None => prefix.to_owned(),
            };
        }
    }
    message.to_owned()
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

/// A failed operation keeps its structured body but is reported as failed, so
/// the model and every frontend see the same outcome.
fn business_error(code: &str, message: impl AsRef<str>) -> Result<ToolExecutionOutcome, ToolError> {
    rejected(code, message)
}

fn rejected(code: &str, message: impl AsRef<str>) -> Result<ToolExecutionOutcome, ToolError> {
    failed(json!({"status":"failed","error":{"code":code,"message":message.as_ref()}}))
}

fn failed(value: impl serde::Serialize) -> Result<ToolExecutionOutcome, ToolError> {
    let text = serde_json::to_string(&value).map_err(|_| ToolError::Internal)?;
    Ok(ToolExecutionOutcome::Failed(
        ToolOutput::new(text).map_err(|_| ToolError::Internal)?,
    ))
}
