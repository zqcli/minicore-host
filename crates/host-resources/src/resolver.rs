//! Resolve one immutable Session selection into Turn or compaction resources.
//! Turn preparation alone receives owner capabilities and executable tools;
//! compaction receives only model resources, declarations and a frozen prompt.
use crate::{approval, prompt::BoundPrompt, workspaces::WorkspaceBindings};
use minicore_host_core::{
    BoundCompactionResources, BoundResources, CompactionResourceFuture, PreparedTurnBinding,
    ResourceFuture, ResourceResolver, ToolOutputBinding,
};
use minicore_host_protocol::{Fault, FaultKind, ReasoningPreference, SessionConfig};
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{Model, ReasoningPreference as RuntimeReasoning},
    tools::ToolSet,
};
use minicore_host_tools::{BoundLocalTools, CoreTurnPort, ToolRegistry};
use minicore_host_workspace::Workspace;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

pub(super) struct ProductionResources {
    pub(super) workspaces: Arc<WorkspaceBindings>,
    pub(super) models: BTreeMap<minicore_host_protocol::ModelRef, Arc<dyn Model>>,
    pub(super) tools: Arc<ToolRegistry>,
    pub(super) enabled: std::collections::BTreeSet<String>,
    pub(super) framework: String,
}
impl ProductionResources {
    async fn prepare_turn(
        &self,
        config: &SessionConfig,
        binding: Option<PreparedTurnBinding>,
    ) -> Result<BoundResources, Fault> {
        self.validate(config)?;
        let workspace = match &config.workspace {
            Some(selection) => Some(self.workspaces.bind(selection).await?),
            None => None,
        };
        let selected = if config.tools.is_empty() {
            ToolSet::default()
        } else {
            let binding = binding.ok_or_else(|| {
                session_invalid("selected tools require an owner-prepared Turn binding")
            })?;
            let port = CoreTurnPort::prepared(binding.clone());
            if self.tools.requires_local(&config.tools) {
                let local = BoundLocalTools::production(
                    binding,
                    workspace
                        .as_ref()
                        .expect("validated local workspace")
                        .as_ref()
                        .clone(),
                )?;
                self.tools.bind_with_local(&config.tools, port, &local)?
            } else {
                self.tools.bind(&config.tools, port)?
            }
        };
        let prompt = Arc::new(BoundPrompt::new(
            &self.framework,
            &config.role.instructions,
            &config.instructions,
            workspace,
        ));
        let execution = ExecutionConfig::new(
            self.models[&config.model].clone(),
            reasoning(config.reasoning),
            selected,
            Some(Arc::new(approval::Policy::new(
                config.execution.approval,
                self.tools.has_builtin_read(),
                self.tools.has_builtin_agent(),
            ))),
            prompt,
        )
        .map_err(|error| session_invalid(error.to_string()))?;
        let settings = &config.execution;
        let options = ExecutionOptions {
            deadline: None,
            prompt_timeout: Duration::from_millis(settings.prompt_timeout_ms),
            model_timeout: Duration::from_millis(settings.model_timeout_ms),
            policy_timeout: Duration::from_millis(settings.policy_timeout_ms),
            tool_timeout: Duration::from_millis(settings.tool_timeout_ms),
            model_retry_attempts: settings.model_retry_attempts,
            model_retry_base_delay: Duration::from_millis(settings.model_retry_base_delay_ms),
        };
        options
            .validate()
            .map_err(|error| session_invalid(error.to_string()))?;
        Ok(BoundResources { execution, options })
    }
}
impl ResourceResolver for ProductionResources {
    fn bind_tool_output(
        &self,
        execution: &ExecutionConfig,
        output: ToolOutputBinding,
    ) -> ExecutionConfig {
        execution.map_tools(|tool, spec| output.wrap(tool, spec.clone()))
    }
    fn resolve_for_compaction(self: Arc<Self>, config: SessionConfig) -> CompactionResourceFuture {
        Box::pin(async move {
            self.validate(&config)?;
            let workspace = match &config.workspace {
                Some(selection) => {
                    Some(Workspace::bind(Some(selection)).await.map_err(|error| {
                        session_invalid(format!("workspace binding failed: {error}"))
                    })?)
                }
                None => None,
            };
            let mut tools = config
                .tools
                .iter()
                .map(|selection| {
                    self.tools.spec(&selection.name).cloned().ok_or_else(|| {
                        missing(format!("tool {} is not registered", selection.name))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            tools.sort_by(|left, right| left.name().cmp(right.name()));
            Ok(BoundCompactionResources {
                model: self.models[&config.model].clone(),
                reasoning: reasoning(config.reasoning),
                tools,
                prompt: Arc::new(BoundPrompt::new(
                    &self.framework,
                    &config.role.instructions,
                    &config.instructions,
                    workspace.map(Arc::new),
                )),
            })
        })
    }

    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|error| session_invalid(error.to_string()))?;
        let model = self
            .models
            .get(&config.model)
            .ok_or_else(|| missing(format!("model {} is not configured", config.model)))?;
        if !model
            .descriptor()
            .supports_reasoning(reasoning(config.reasoning))
        {
            return Err(session_invalid(format!(
                "model {} does not support the selected reasoning preference",
                config.model
            )));
        }
        for tool in &config.tools {
            if !self.enabled.contains(&tool.name) {
                return Err(missing(format!("tool {} is not configured", tool.name)));
            }
        }
        if !config.tools.is_empty() && !model.descriptor().supports_tools {
            return Err(session_invalid(format!(
                "model {} does not support tools",
                config.model
            )));
        }
        if self.tools.requires_local(&config.tools) && config.workspace.is_none() {
            return Err(session_invalid(
                "local tools require a configured workspace",
            ));
        }
        if let Some(workspace) = &config.workspace
            && !Path::new(&workspace.path).is_absolute()
        {
            return Err(session_invalid("workspace path must be absolute"));
        }
        Ok(())
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(self.prepare_turn(config, None))
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(self.prepare_turn(config, Some(binding)))
    }
}

pub(super) fn reasoning(value: ReasoningPreference) -> RuntimeReasoning {
    match value {
        ReasoningPreference::Auto => RuntimeReasoning::Auto,
        ReasoningPreference::Disabled => RuntimeReasoning::Disabled,
        ReasoningPreference::Low => RuntimeReasoning::Low,
        ReasoningPreference::Medium => RuntimeReasoning::Medium,
        ReasoningPreference::High => RuntimeReasoning::High,
        ReasoningPreference::XHigh => RuntimeReasoning::XHigh,
        ReasoningPreference::Max => RuntimeReasoning::Max,
        ReasoningPreference::Ultra => RuntimeReasoning::Ultra,
    }
}
pub(super) fn session_invalid(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::SessionConfigInvalid, message)
}
fn missing(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::SessionResourceMissing, message)
}
