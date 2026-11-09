//! Production model/tool/workspace composition, outside the Host owner locks.
//!
//! No model requests, business tools, environment scans, dynamic module loading,
//! or process-wide cwd changes occur during preparation. Each candidate builds
//! new immutable provider clients; each Turn binds one actual selected ToolSet.
use minicore_host_core::{
    BoundResources, HostConfig, HostConfigurationFuture, HostConfigurationPreparer,
    PreparedHostConfiguration, PreparedTurnBinding, ResourceFuture, ResourceResolver,
};
use minicore_host_protocol::{Fault, FaultKind, ReasoningPreference, SessionConfig};
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_tools::{AgentToolFactory, CoreTurnPort, ScopeManager, ToolRegistry};
use minicore_host_workspace::Workspace;
use minicore_runtime::{
    ExecutionConfig, LoopOptions, Text,
    model::{Model, ModelDescriptor, ModelMessage, ReasoningPreference as RuntimeReasoning},
    prompt::{PromptFuture, PromptProvider, PromptRequest},
    tools::ToolSet,
};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

/// The deployment's statically compiled registry. Keep one builder across
/// reloads: agent-tool delegation scopes remain owned across public Turns.
#[derive(Clone)]
pub struct HostResourceBuilder {
    tools: Arc<ToolRegistry>,
}
/// Explicit private inputs, for embedding and isolated tests. Never serialized.
pub struct HostCredentials {
    pub model_keys: BTreeMap<minicore_host_protocol::ModelRef, SecretApiKey>,
    pub network_access_key: Option<String>,
}
impl std::fmt::Debug for HostCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostCredentials([REDACTED])")
    }
}

impl HostResourceBuilder {
    pub fn new(tools: Arc<ToolRegistry>) -> Self {
        Self { tools }
    }
    pub fn builtins() -> Result<Self, Fault> {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(AgentToolFactory::new(ScopeManager::new())))?;
        Ok(Self::new(Arc::new(tools)))
    }
    /// Explicitly read only the credential environment names in this candidate.
    /// Calling this method does not apply the result to any Host.
    pub async fn prepare(&self, config: HostConfig) -> Result<PreparedHostConfiguration, Fault> {
        config.validate()?;
        let mut model_keys = BTreeMap::new();
        for model in &config.models {
            model_keys.insert(
                model.model.clone(),
                SecretApiKey::from_env(&model.api_key_env)
                    .map_err(|error| invalid(format!("model {}: {error}", model.model)))?,
            );
        }
        let network_access_key = config
            .network
            .access_key_env
            .as_ref()
            .map(|name| {
                std::env::var(name)
                    .map_err(|_| invalid("configured network access key is unavailable"))
            })
            .transpose()?;
        self.prepare_with_credentials(
            config,
            HostCredentials {
                model_keys,
                network_access_key,
            },
        )
        .await
    }
    /// Override private inputs for this candidate only. The bound production
    /// reload preparer still reads the configured environment sources. An
    /// embedding with another credential source binds its own preparer before
    /// Host construction through PreparedHostConfiguration::with_preparer.
    pub async fn prepare_with_credentials(
        &self,
        config: HostConfig,
        mut credentials: HostCredentials,
    ) -> Result<PreparedHostConfiguration, Fault> {
        config.validate()?;
        let tools = Arc::clone(&self.tools);
        let preparer = Arc::new(self.clone());
        tokio::task::spawn_blocking(move || {
            let declared = tools.specs();
            let mut selected_specs = Vec::new();
            for selection in &config.tools {
                let spec = declared
                    .iter()
                    .find(|spec| spec.name().as_str() == selection.name)
                    .ok_or_else(|| {
                        invalid(format!(
                            "tool {} is not statically registered",
                            selection.name
                        ))
                    })?;
                selected_specs.push(spec.clone());
            }
            let mut models = BTreeMap::new();
            for settings in &config.models {
                let descriptor = ModelDescriptor::new(
                    settings
                        .model
                        .as_str()
                        .parse()
                        .map_err(|_| invalid("model reference is invalid"))?,
                    settings.context_window,
                    settings
                        .supported_reasoning
                        .iter()
                        .copied()
                        .map(reasoning)
                        .collect(),
                    settings.supports_tools,
                )
                .map_err(|_| invalid("model capabilities are invalid"))?;
                let mut connection = OpenAiChatConfig::new(
                    settings.endpoint.clone(),
                    settings.provider_model.clone(),
                    descriptor,
                );
                connection.request_timeout = settings.request_timeout_ms.map(Duration::from_millis);
                let model = OpenAiChatModel::new(
                    connection,
                    credentials
                        .model_keys
                        .remove(&settings.model)
                        .ok_or_else(|| {
                            invalid(format!("model {} credential is missing", settings.model))
                        })?,
                )
                .map_err(|error| invalid(format!("model {}: {error}", settings.model)))?;
                models.insert(settings.model.clone(), Arc::new(model) as Arc<dyn Model>);
            }
            if !credentials.model_keys.is_empty() {
                return Err(invalid(
                    "credentials include an unconfigured model reference",
                ));
            }
            let access_key = credentials.network_access_key;
            let resources = Arc::new(ProductionResources {
                models,
                tools,
                enabled: config.tools.iter().map(|tool| tool.name.clone()).collect(),
                framework: config.framework_instructions.clone(),
            });
            PreparedHostConfiguration::new(config, resources, selected_specs, access_key)
                .map(|candidate| candidate.with_preparer(preparer))
        })
        .await
        .map_err(|_| invalid("resource preparation worker did not finish"))?
    }
}

impl HostConfigurationPreparer for HostResourceBuilder {
    fn prepare(&self, config: HostConfig) -> HostConfigurationFuture<'_> {
        Box::pin(HostResourceBuilder::prepare(self, config))
    }
}

struct ProductionResources {
    models: BTreeMap<minicore_host_protocol::ModelRef, Arc<dyn Model>>,
    tools: Arc<ToolRegistry>,
    enabled: std::collections::BTreeSet<String>,
    framework: String,
}
impl ProductionResources {
    async fn prepare_turn(
        &self,
        config: &SessionConfig,
        binding: Option<PreparedTurnBinding>,
    ) -> Result<BoundResources, Fault> {
        self.validate(config)?;
        let workspace = match &config.workspace {
            Some(selection) => {
                Some(Workspace::bind(Some(selection)).await.map_err(|error| {
                    session_invalid(format!("workspace binding failed: {error}"))
                })?)
            }
            None => None,
        };
        let selected = if config.tools.is_empty() {
            ToolSet::default()
        } else {
            let binding = binding.ok_or_else(|| {
                session_invalid("selected tools require an owner-prepared Turn binding")
            })?;
            self.tools
                .bind(&config.tools, CoreTurnPort::prepared(binding))?
        };
        let prompt = Arc::new(BoundPrompt {
            framework: Text::new(&self.framework),
            role: Text::new(&config.role.instructions),
            workspace,
        });
        let execution = ExecutionConfig::new(
            self.models[&config.model].clone(),
            reasoning(config.reasoning),
            selected,
            None,
            prompt,
        )
        .map_err(|error| session_invalid(error.to_string()))?;
        let settings = &config.execution;
        let options = LoopOptions {
            deadline: None,
            prompt_timeout: Duration::from_millis(settings.prompt_timeout_ms),
            model_timeout: Duration::from_millis(settings.model_timeout_ms),
            policy_timeout: Duration::from_millis(settings.policy_timeout_ms),
            tool_timeout: Duration::from_millis(settings.tool_timeout_ms),
            model_retry_attempts: settings.model_retry_attempts,
            model_retry_base_delay: Duration::from_millis(settings.model_retry_base_delay_ms),
            history: None,
        };
        options
            .validate()
            .map_err(|error| session_invalid(error.to_string()))?;
        Ok(BoundResources { execution, options })
    }
}
impl ResourceResolver for ProductionResources {
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
        if let Some(workspace) = &config.workspace
            && !Path::new(&workspace.path).is_absolute()
        {
            return Err(session_invalid("workspace path must be absolute"));
        }
        // Compaction is Session-owned and is not an AgentLoop option. Until that
        // owner is integrated, reject rather than silently ignoring this flag.
        if config.execution.compaction.enabled {
            return Err(session_invalid(
                "automatic compaction is not implemented by this resource assembler; explicitly disable it",
            ));
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

/// Frozen Turn context. Workspace owns its canonical root and is never a
/// process cwd; the model receives it as explicit context. Ordinary filesystem
/// tools can use that same Workspace backend when their static modules land.
struct BoundPrompt {
    framework: Text,
    role: Text,
    workspace: Option<Workspace>,
}
impl PromptProvider for BoundPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(async move {
            let mut prompt = minicore_host_core::prepare_prompt_history(request).await?;
            let mut prefix = Vec::new();
            if !self.framework.is_empty() {
                prefix.push(ModelMessage::System(Arc::from(self.framework.as_str())));
            }
            if !self.role.is_empty() {
                prefix.push(ModelMessage::System(Arc::from(self.role.as_str())));
            }
            if let Some(workspace) = &self.workspace {
                prefix.push(ModelMessage::System(Arc::from(format!(
                    "Workspace directory: {}",
                    workspace.root().display()
                ))));
            }
            prefix.append(&mut prompt.messages);
            prompt.messages = prefix;
            Ok(prompt)
        })
    }
}
fn reasoning(value: ReasoningPreference) -> RuntimeReasoning {
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
fn invalid(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::HostConfigInvalid, message)
}
fn session_invalid(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::SessionConfigInvalid, message)
}
fn missing(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::SessionResourceMissing, message)
}
