//! Readable Host values, without owners, credentials or runtime resources.
use minicore_host_protocol::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, net::SocketAddr};

use crate::invalid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub name: String,
    #[serde(default)]
    pub framework_instructions: String,
    pub models: Vec<OpenAiModelSettings>,
    /// Available selections from the statically compiled tool registry.
    pub tools: Vec<ToolBinding>,
    /// Explicit directory-picker roots. Empty exposes no directories.
    #[serde(default)]
    pub workspace_roots: Vec<String>,
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
            workspace_roots: Vec::new(),
            network: NetworkSettings::default(),
            logging: LogSettings::default(),
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
        for root in &self.workspace_roots {
            if !std::path::Path::new(root).is_absolute() || root.contains('\0') {
                return Err(invalid("workspace roots must be absolute directory paths"));
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
    pub fn restart_changes(&self, candidate: &Self) -> Vec<RestartField> {
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

/// Explicit provider wire protocol. Endpoint paths do not select or negotiate it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiModelApi {
    ChatCompletions,
    Responses,
}

/// Explicit OpenAI-compatible configuration. Capability values are declarations
/// from the user's model configuration, not discovered or guessed capabilities.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiModelSettings {
    pub model: ModelRef,
    pub api: OpenAiModelApi,
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
            .field("api", &self.api)
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
    pub api: OpenAiModelApi,
    pub context_window: u64,
    pub supported_reasoning: Vec<ReasoningPreference>,
    pub supports_tools: bool,
}
impl From<&OpenAiModelSettings> for ModelDescription {
    fn from(value: &OpenAiModelSettings) -> Self {
        Self {
            model: value.model.clone(),
            api: value.api,
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
fn environment_name(name: &str) -> Result<(), Fault> {
    if name.is_empty() || name.contains(['=', '\0']) {
        return Err(invalid("credential environment name is invalid"));
    }
    Ok(())
}
