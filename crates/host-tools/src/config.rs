use crate::action::{invalid, validate_name};
use minicore_host_protocol::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Partial compaction settings for `agent` tool calls; omitted fields inherit from base.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionSettingsPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub enabled: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub trigger_percent: Option<u8>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub target_percent: Option<u8>,
}
impl CompactionSettingsPatch {
    pub fn apply_to(
        &self,
        base: &CompactionSettings,
    ) -> Result<CompactionSettings, ValidationError> {
        let candidate = CompactionSettings {
            enabled: self.enabled.unwrap_or(base.enabled),
            trigger_percent: self.trigger_percent.unwrap_or(base.trigger_percent),
            target_percent: self.target_percent.unwrap_or(base.target_percent),
        };
        candidate.validate()?;
        Ok(candidate)
    }
}

/// Partial execution settings for `agent` tool calls; omitted fields inherit from base.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSettingsPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub approval: Option<ApprovalMode>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub prompt_timeout_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub model_timeout_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub policy_timeout_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub tool_timeout_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub model_retry_attempts: Option<u8>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub model_retry_base_delay_ms: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub compaction: Option<CompactionSettingsPatch>,
}
impl ExecutionSettingsPatch {
    pub fn apply_to(&self, base: &ExecutionSettings) -> Result<ExecutionSettings, ValidationError> {
        let compaction = match &self.compaction {
            Some(patch) => patch.apply_to(&base.compaction)?,
            None => base.compaction.clone(),
        };
        let candidate = ExecutionSettings {
            approval: self.approval.unwrap_or(base.approval),
            prompt_timeout_ms: self.prompt_timeout_ms.unwrap_or(base.prompt_timeout_ms),
            model_timeout_ms: self.model_timeout_ms.unwrap_or(base.model_timeout_ms),
            policy_timeout_ms: self.policy_timeout_ms.unwrap_or(base.policy_timeout_ms),
            tool_timeout_ms: self.tool_timeout_ms.unwrap_or(base.tool_timeout_ms),
            model_retry_attempts: self
                .model_retry_attempts
                .unwrap_or(base.model_retry_attempts),
            model_retry_base_delay_ms: self
                .model_retry_base_delay_ms
                .unwrap_or(base.model_retry_base_delay_ms),
            compaction,
        };
        candidate.validate()?;
        Ok(candidate)
    }
}

/// Settings patch accepted by the `agent` tool; allows partial `execution` overrides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSettingsPatch {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub role: Option<Role>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub instructions: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub model: Option<ModelRef>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub reasoning: Option<ReasoningPreference>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub tools: Option<Vec<ToolBinding>>,
    #[serde(default, skip_serializing_if = "NullablePatch::is_unchanged")]
    pub workspace: NullablePatch<WorkspaceRef>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub execution: Option<ExecutionSettingsPatch>,
}
impl AgentSettingsPatch {
    pub fn is_empty(&self) -> bool {
        self.role.is_none()
            && self.instructions.is_none()
            && self.model.is_none()
            && self.reasoning.is_none()
            && self.tools.is_none()
            && self.workspace.is_unchanged()
            && self.execution.is_none()
    }
    pub fn resolve(
        &self,
        base_execution: &ExecutionSettings,
    ) -> Result<SessionConfigPatch, ValidationError> {
        let execution = self
            .execution
            .as_ref()
            .map(|patch| patch.apply_to(base_execution))
            .transpose()?;
        Ok(SessionConfigPatch {
            role: self.role.clone(),
            instructions: self.instructions.clone(),
            model: self.model.clone(),
            reasoning: self.reasoning,
            tools: self.tools.clone(),
            workspace: self.workspace.clone(),
            execution,
        })
    }
}

/// `settings` and config changes use exactly the owner's allowed values.
/// A metadata field is parsed separately so unknown config fields remain errors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentChanges {
    pub name: Option<String>,
    pub settings: AgentSettingsPatch,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionChanges {
    pub title: NullablePatch<String>,
    pub settings: AgentSettingsPatch,
}
impl AgentChanges {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.name.is_none() && self.settings.is_empty() {
            return Err(invalid("changes must not be empty"));
        }
        if let Some(name) = &self.name {
            validate_name(name)?;
        }
        self.settings
            .resolve(&ExecutionSettings::default())
            .map_err(|error| invalid(error.to_string()))?;
        Ok(())
    }
}
impl SessionChanges {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.title.is_unchanged() && self.settings.is_empty() {
            return Err(invalid("changes must not be empty"));
        }
        if let NullablePatch::Set(title) = &self.title {
            validate_name(title)?;
        }
        self.settings
            .resolve(&ExecutionSettings::default())
            .map_err(|error| invalid(error.to_string()))?;
        Ok(())
    }
}
impl<'de> Deserialize<'de> for AgentChanges {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut fields = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let name = fields
            .remove("name")
            .map(serde_json::from_value::<String>)
            .transpose()
            .map_err(serde::de::Error::custom)?;
        let settings = serde_json::from_value(serde_json::Value::Object(fields))
            .map_err(serde::de::Error::custom)?;
        let changes = Self { name, settings };
        changes.validate().map_err(serde::de::Error::custom)?;
        Ok(changes)
    }
}
impl Serialize for AgentChanges {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        let mut fields = serde_json::to_value(&self.settings).map_err(serde::ser::Error::custom)?;
        if let Some(name) = &self.name {
            fields["name"] = name.clone().into();
        }
        fields.serialize(s)
    }
}
impl<'de> Deserialize<'de> for SessionChanges {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut fields = serde_json::Map::<String, serde_json::Value>::deserialize(d)?;
        let title = match fields.remove("title") {
            None => NullablePatch::Unchanged,
            Some(serde_json::Value::Null) => NullablePatch::Clear,
            Some(value) => {
                NullablePatch::Set(serde_json::from_value(value).map_err(serde::de::Error::custom)?)
            }
        };
        let settings = serde_json::from_value(serde_json::Value::Object(fields))
            .map_err(serde::de::Error::custom)?;
        let changes = Self { title, settings };
        changes.validate().map_err(serde::de::Error::custom)?;
        Ok(changes)
    }
}
impl Serialize for SessionChanges {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        let mut fields = serde_json::to_value(&self.settings).map_err(serde::ser::Error::custom)?;
        match &self.title {
            NullablePatch::Unchanged => {}
            NullablePatch::Clear => fields["title"] = serde_json::Value::Null,
            NullablePatch::Set(title) => fields["title"] = title.clone().into(),
        }
        fields.serialize(s)
    }
}

/// Copy values from one source snapshot, never identity, mode, default pointer,
/// history, provenance, or runtime handles. Owner resource checks still apply.
pub fn agent_config_from_settings(
    source: &AgentConfig,
    settings: &SessionConfigPatch,
) -> Result<AgentConfig, Fault> {
    let session = SessionConfig::from_agent(source, &SessionOverrides(settings.clone()))
        .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
    Ok(AgentConfig {
        role: session.role,
        instructions: session.instructions,
        session_defaults: SessionDefaults {
            model: session.model,
            reasoning: session.reasoning,
            tools: session.tools,
            workspace: session.workspace,
            execution: session.execution,
        },
    })
}

/// Values and their matching revision must be fetched together by the backend.
#[derive(Clone, Debug)]
pub struct AgentConfigSnapshot {
    pub name: String,
    pub config: AgentConfig,
    pub config_revision: ConfigRevision,
}
#[derive(Clone, Debug)]
pub struct SessionConfigSnapshot {
    pub title: Option<String>,
    pub mode: StorageMode,
    pub config: SessionConfig,
    pub config_revision: ConfigRevision,
}
#[derive(Clone, Debug)]
pub struct AgentUpdateCandidate {
    pub expected_revision: ConfigRevision,
    pub config: AgentConfig,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct SessionUpdateCandidate {
    pub expected_revision: ConfigRevision,
    pub config: SessionConfig,
    pub title: Option<String>,
}
impl AgentConfigSnapshot {
    pub fn apply(&self, changes: &AgentChanges) -> Result<AgentUpdateCandidate, Fault> {
        changes.validate()?;
        let settings = changes
            .settings
            .resolve(&self.config.session_defaults.execution)
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        Ok(AgentUpdateCandidate {
            expected_revision: self.config_revision,
            config: agent_config_from_settings(&self.config, &settings)?,
            name: changes.name.clone().unwrap_or_else(|| self.name.clone()),
        })
    }
}
impl SessionConfigSnapshot {
    pub fn apply(&self, changes: &SessionChanges) -> Result<SessionUpdateCandidate, Fault> {
        changes.validate()?;
        let settings = changes
            .settings
            .resolve(&self.config.execution)
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        Ok(SessionUpdateCandidate {
            expected_revision: self.config_revision,
            config: self
                .config
                .apply_patch(&settings)
                .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?,
            title: match &changes.title {
                NullablePatch::Unchanged => self.title.clone(),
                NullablePatch::Set(title) => Some(title.clone()),
                NullablePatch::Clear => None,
            },
        })
    }
}
