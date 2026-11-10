//! Prepare immutable Host resource candidates outside owner locks.
//! One builder shares the compiled tool registry and weak Workspace bindings
//! across reloads; only explicitly configured credential sources are read.
use crate::{
    resolver::{ProductionResources, reasoning},
    workspaces::WorkspaceBindings,
};
use minicore_host_core::{
    HostConfig, HostConfigurationFuture, HostConfigurationPreparer, PreparedHostConfiguration,
};
use minicore_host_protocol::{Fault, FaultKind};
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_tools::{ScopeManager, ToolRegistry};
use minicore_runtime::model::{Model, ModelDescriptor};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

/// The deployment's statically compiled registry. Keep one builder across
/// reloads: agent-tool delegation scopes remain owned across public Turns.
#[derive(Clone)]
pub struct HostResourceBuilder {
    tools: Arc<ToolRegistry>,
    workspaces: Arc<WorkspaceBindings>,
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
        Self {
            tools,
            workspaces: Arc::default(),
        }
    }
    pub fn builtins() -> Result<Self, Fault> {
        let mut tools = ToolRegistry::new();
        tools.register_agent_tool(ScopeManager::new())?;
        tools.register_local_tools()?;
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
        let workspaces = Arc::clone(&self.workspaces);
        tokio::task::spawn_blocking(move || {
            let mut selected_specs = Vec::new();
            for selection in &config.tools {
                let spec = tools.spec(&selection.name).ok_or_else(|| {
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
                workspaces,
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

fn invalid(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::HostConfigInvalid, message)
}

#[cfg(test)]
#[path = "production_concurrency_tests.rs"]
mod production_concurrency_tests;
