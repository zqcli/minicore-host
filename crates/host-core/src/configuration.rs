//! Readable Host configuration and immutable prepared execution snapshots.
//!
//! Credentials are read only by the explicit application preparation step.
//! Saved values name their source; neither queries nor Debug expose key bytes.
use crate::{BoundResources, PreparedTurnBinding, ResourceFuture, ResourceResolver};
use minicore_host_protocol::*;
pub use minicore_host_protocol::{HostReloadResult, RestartField};
use minicore_runtime::tools::ToolSpec;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub name: String,
    pub framework_instructions: String,
    pub models: Vec<OpenAiModelSettings>,
    /// Available selections from the statically compiled tool registry.
    pub tools: Vec<ToolBinding>,
    pub network: NetworkSettings,
    pub logging: LogSettings,
}
impl Default for HostConfig {
    fn default() -> Self {
        Self {
            name: "MiniCore".into(),
            framework_instructions: String::new(),
            models: Vec::new(),
            tools: Vec::new(),
            network: NetworkSettings::default(),
            logging: LogSettings::default(),
        }
    }
}
impl From<crate::HostSettings> for HostConfig {
    fn from(value: crate::HostSettings) -> Self {
        Self {
            name: value.name,
            ..Self::default()
        }
    }
}
impl HostConfig {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.name.trim().is_empty()
            || self.name.contains('\0')
            || self.framework_instructions.contains('\0')
        {
            return Err(invalid("Host name and framework instructions are invalid"));
        }
        let mut models = BTreeSet::new();
        for model in &self.models {
            if !models.insert(&model.model)
                || model.context_window == 0
                || model.supported_reasoning.is_empty()
                || model.request_timeout_ms == Some(0)
                || model.provider_model.trim().is_empty()
            {
                return Err(invalid("model configuration is invalid or duplicated"));
            }
            environment_name(&model.api_key_env)?;
        }
        let mut tools = BTreeSet::new();
        for tool in &self.tools {
            tool.validate()
                .map_err(|error| invalid(error.to_string()))?;
            if !tools.insert(&tool.name) {
                return Err(invalid("duplicate configured tool"));
            }
        }
        if let Some(source) = &self.network.access_key_env {
            environment_name(source)?;
        }
        if self.network.listen.is_some() && self.network.access_key_env.is_none() {
            return Err(invalid(
                "network listening requires a configured access key source",
            ));
        }
        if let LogOutput::File { path } = &self.logging.output
            && (!std::path::Path::new(path).is_absolute() || path.contains('\0'))
        {
            return Err(invalid("log file must be an absolute path"));
        }
        Ok(())
    }
    pub(crate) fn restart_changes(&self, candidate: &Self) -> Vec<RestartField> {
        let mut changes = Vec::new();
        if self.network.listen != candidate.network.listen {
            changes.push(RestartField::NetworkListen);
        }
        if self.logging != candidate.logging {
            changes.push(RestartField::Logging);
        }
        changes
    }
}

/// Explicit OpenAI-compatible configuration. Capability values are declarations
/// from the user's model configuration, not discovered or guessed capabilities.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiModelSettings {
    pub model: ModelRef,
    pub endpoint: String,
    pub provider_model: String,
    pub api_key_env: String,
    pub context_window: u64,
    pub supported_reasoning: Vec<ReasoningPreference>,
    pub supports_tools: bool,
    pub request_timeout_ms: Option<u64>,
}
impl fmt::Debug for OpenAiModelSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiModelSettings")
            .field("model", &self.model)
            .field("context_window", &self.context_window)
            .field("supported_reasoning", &self.supported_reasoning)
            .field("supports_tools", &self.supports_tools)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkSettings {
    /// Listener startup value. Reload does not start, replace, or stop listeners.
    pub listen: Option<SocketAddr>,
    pub access_key_env: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSettings {
    pub level: LogLevel,
    pub output: LogOutput,
}
impl Default for LogSettings {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            output: LogOutput::Stderr,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LogOutput {
    Stderr,
    File { path: String },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDescription {
    pub model: ModelRef,
    pub context_window: u64,
    pub supported_reasoning: Vec<ReasoningPreference>,
    pub supports_tools: bool,
}
impl From<&OpenAiModelSettings> for ModelDescription {
    fn from(value: &OpenAiModelSettings) -> Self {
        Self {
            model: value.model.clone(),
            context_window: value.context_window,
            supported_reasoning: value.supported_reasoning.clone(),
            supports_tools: value.supports_tools,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfigSnapshot {
    pub config: HostConfig,
    pub config_revision: ConfigRevision,
}
/// Application assembly is fixed when the Host is constructed. The preparer
/// owns the deployment's static registry, never a Host or connection. It must
/// preserve candidate values, resolve all configured resources, and retain no
/// Host owner locks while awaiting preparation.
pub trait HostConfigurationPreparer: Send + Sync + 'static {
    fn prepare(&self, config: HostConfig) -> HostConfigurationFuture<'_>;
}
pub type HostConfigurationFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<PreparedHostConfiguration, Fault>> + Send + 'a>,
>;

/// Fully prepared candidate. Construction never publishes it. The assembler is
/// responsible for resolving every configured model and tool before returning.
/// This object cannot be decoded from business JSON or expose private handles.
pub struct PreparedHostConfiguration {
    pub(crate) config: HostConfig,
    pub(crate) resources: Arc<dyn ResourceResolver>,
    pub(crate) tools: Vec<ToolSpec>,
    pub(crate) access_key: Option<String>,
    pub(crate) preparer: Option<Arc<dyn HostConfigurationPreparer>>,
}
impl fmt::Debug for PreparedHostConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedHostConfiguration")
            .field("config", &self.config)
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}
impl PreparedHostConfiguration {
    pub fn new(
        config: HostConfig,
        resources: Arc<dyn ResourceResolver>,
        mut tools: Vec<ToolSpec>,
        access_key: Option<String>,
    ) -> Result<Self, Fault> {
        config.validate()?;
        if access_key.as_ref().is_some_and(|key| key.trim().is_empty())
            || config.network.access_key_env.is_some() != access_key.is_some()
        {
            return Err(invalid(
                "prepared network credential does not match its configured source",
            ));
        }
        tools.sort_by(|a, b| a.name().cmp(b.name()));
        let names: BTreeSet<_> = tools.iter().map(|tool| tool.name().as_str()).collect();
        if names.len() != tools.len()
            || names != config.tools.iter().map(|tool| tool.name.as_str()).collect()
        {
            return Err(invalid(
                "prepared tool declarations differ from configured selections",
            ));
        }
        Ok(Self {
            config,
            resources,
            tools,
            access_key,
            preparer: None,
        })
    }
    /// Bind the application assembler at Host initialization. Reload candidates
    /// cannot replace the assembler or the deployment's static tool registry.
    pub fn with_preparer(mut self, preparer: Arc<dyn HostConfigurationPreparer>) -> Self {
        self.preparer = Some(preparer);
        self
    }
    pub fn config(&self) -> &HostConfig {
        &self.config
    }
}

/// Read-only launch policy shared by this Host's old and new Turn bindings.
/// It contains names only, never credential values. Successful publications
/// add names for the Host lifetime; removing a configured source does not make
/// its inherited process value available to later child commands again.
#[derive(Clone)]
pub struct ToolEnvironmentExclusions(Arc<Mutex<BTreeSet<String>>>);
impl ToolEnvironmentExclusions {
    fn new(config: &HostConfig) -> Self {
        let exclusions = Self(Arc::default());
        exclusions.include(config);
        exclusions
    }
    pub(crate) fn include(&self, config: &HostConfig) {
        let mut names = self.0.lock().expect("tool environment mutex poisoned");
        names.extend(config.models.iter().map(|model| model.api_key_env.clone()));
        names.extend(config.network.access_key_env.iter().cloned());
    }
    /// Snapshot at child-launch filtering time. The lock is released before
    /// command construction or spawn; already-started children are unaffected.
    pub fn names(&self) -> Vec<String> {
        self.0
            .lock()
            .expect("tool environment mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

pub(crate) struct ConfigurationOwner {
    pub(crate) tool_environment: ToolEnvironmentExclusions,
    pub(crate) state: Mutex<ConfigurationState>,
    #[cfg(test)]
    pub(crate) save_gate: Mutex<Option<Arc<tests::SaveGate>>>,
}
pub(crate) struct ConfigurationState {
    pub(crate) preparer: Option<Arc<dyn HostConfigurationPreparer>>,
    pub(crate) snapshot: Arc<PublishedConfiguration>,
    // Accepted preparation, save, publication and actual resource cleanup.
    pub(crate) saving: bool,
    pub(crate) save_fault: Option<Fault>,
    pub(crate) failed_candidate: Option<PreparedHostConfiguration>,
    // One record per actual invocation; operation IDs are correlation only.
    pub(crate) reloads: BTreeMap<OperationId, Vec<HostReloadReceipt>>,
}
pub(crate) struct PublishedConfiguration {
    pub(crate) config: HostConfig,
    pub(crate) revision: ConfigRevision,
    pub(crate) tools: Vec<ToolSpec>,
    // Retired only after the real Host shutdown barrier has settled all work.
    pub(crate) resources: Option<Arc<dyn ResourceResolver>>,
}
impl ConfigurationOwner {
    pub(crate) fn new(
        candidate: PreparedHostConfiguration,
        revision: ConfigRevision,
    ) -> (Arc<Self>, Option<String>) {
        let owner = Arc::new(Self {
            tool_environment: ToolEnvironmentExclusions::new(&candidate.config),
            #[cfg(test)]
            save_gate: Mutex::new(None),
            state: Mutex::new(ConfigurationState {
                preparer: candidate.preparer,
                snapshot: Arc::new(PublishedConfiguration {
                    config: candidate.config,
                    revision,
                    tools: candidate.tools,
                    resources: Some(candidate.resources),
                }),
                saving: false,
                save_fault: None,
                failed_candidate: None,
                reloads: BTreeMap::new(),
            }),
        });
        (owner, candidate.access_key)
    }
    pub(crate) fn snapshot(&self) -> Arc<PublishedConfiguration> {
        Arc::clone(
            &self
                .state
                .lock()
                .expect("Host configuration mutex poisoned")
                .snapshot,
        )
    }
    pub(crate) fn retire(&self) {
        let old = {
            let mut state = self
                .state
                .lock()
                .expect("Host configuration mutex poisoned");
            let prior = &state.snapshot;
            let retired = Arc::new(PublishedConfiguration {
                config: prior.config.clone(),
                revision: prior.revision,
                tools: prior.tools.clone(),
                resources: None,
            });
            (
                std::mem::replace(&mut state.snapshot, retired),
                state.preparer.take(),
            )
        };
        drop(old);
    }
    pub(crate) fn receipts_owned(&self, operation: &OperationId) -> Vec<Receipt> {
        self.state
            .lock()
            .expect("Host configuration mutex poisoned")
            .reloads
            .get(operation)
            .into_iter()
            .flatten()
            .cloned()
            .map(Receipt::HostReload)
            .collect()
    }
    pub(crate) fn status(&self) -> (bool, Option<Fault>) {
        let state = self
            .state
            .lock()
            .expect("Host configuration mutex poisoned");
        (state.saving, state.save_fault.clone())
    }
    fn resources(snapshot: &PublishedConfiguration) -> Result<Arc<dyn ResourceResolver>, Fault> {
        snapshot.resources.clone().ok_or_else(|| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Host execution resources were retired",
            )
        })
    }
    async fn resolved(
        &self,
        snapshot: Arc<PublishedConfiguration>,
        config: &SessionConfig,
        binding: Option<PreparedTurnBinding>,
    ) -> Result<BoundResources, Fault> {
        let resources = Self::resources(&snapshot)?;
        let result = match binding {
            Some(binding) => resources.resolve_for_turn(config, binding).await,
            None => resources.resolve(config).await,
        }?;
        if self.snapshot().revision != snapshot.revision {
            return Err(Fault::new(
                FaultKind::SessionConfigConflict,
                "Host resources changed during Turn preparation",
            ));
        }
        Ok(result)
    }
}
impl ResourceResolver for ConfigurationOwner {
    fn resolve_for_compaction(
        self: Arc<Self>,
        config: SessionConfig,
    ) -> crate::CompactionResourceFuture {
        let snapshot = self.snapshot();
        Box::pin(async move {
            // This accepted utility keeps the selected publication even if a
            // later Host reload changes resources for future work.
            Self::resources(&snapshot)?
                .resolve_for_compaction(config)
                .await
        })
    }

    fn bind_tool_output(
        &self,
        execution: &minicore_runtime::ExecutionConfig,
        output: crate::ToolOutputBinding,
    ) -> minicore_runtime::ExecutionConfig {
        // Per ResourceResolver's contract this hook attaches observation only;
        // it may not select or replace already prepared execution resources.
        match Self::resources(&self.snapshot()) {
            Ok(resources) => resources.bind_tool_output(execution, output),
            Err(_) => execution.clone(),
        }
    }
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        Self::resources(&self.snapshot())?.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        let snapshot = self.snapshot();
        Box::pin(self.resolved(snapshot, config, None))
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        let snapshot = self.snapshot();
        Box::pin(self.resolved(snapshot, config, Some(binding)))
    }
}
pub(crate) fn invalid(message: impl Into<String>) -> Fault {
    Fault::new(FaultKind::HostConfigInvalid, message)
}
fn environment_name(name: &str) -> Result<(), Fault> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(invalid("credential environment name is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod credential_environment_tests;
#[cfg(test)]
mod reload_tests;
#[cfg(test)]
mod tests;
