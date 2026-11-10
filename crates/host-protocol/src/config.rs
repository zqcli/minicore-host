use crate::{ValidationError, invalid, validated_dto};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeSet, fmt, str::FromStr};

/// Preserves Runtime's full reference string, including provider/namespace.
/// The 256-byte grammar restriction is inherited from Runtime, not a Host quota.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ModelRef(String);
impl ModelRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for ModelRef {
    type Err = ValidationError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || value.len() > 256
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
            })
        {
            Err(invalid(
                "model reference violates the Runtime reference grammar",
            ))
        } else {
            Ok(Self(value.to_owned()))
        }
    }
}
impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl<'de> Deserialize<'de> for ModelRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningPreference {
    #[default]
    Auto,
    Disabled,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
    Ultra,
}

/// Execution policy for this Session's tools, frozen with the next public Turn.
/// An omitted value defaults to `Ask`, including older saved configurations.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Auto,
    #[default]
    Ask,
    ReadOnly,
}

validated_dto!(Role, RoleWire, "RoleWire" {
    name: String,
    description: String,
    instructions: String,
});
impl Role {
    pub fn validate(&self) -> Result<(), ValidationError> {
        nonblank(&self.name, "role name")
    }
}
impl Default for Role {
    fn default() -> Self {
        Self {
            name: "Assistant".into(),
            description: String::new(),
            instructions: String::new(),
        }
    }
}

validated_dto!(ToolBinding, ToolBindingWire, "ToolBindingWire" { name: String });
impl ToolBinding {
    pub fn validate(&self) -> Result<(), ValidationError> {
        nonblank(&self.name, "tool name")
    }
}
validated_dto!(WorkspaceRef, WorkspaceRefWire, "WorkspaceRefWire" { path: String });
impl WorkspaceRef {
    pub fn validate(&self) -> Result<(), ValidationError> {
        nonblank(&self.path, "workspace path")
    }
}

validated_dto!(CompactionSettings, CompactionSettingsWire, "CompactionSettingsWire" {
    enabled: bool,
    trigger_percent: u8,
    target_percent: u8,
});
impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_percent: 95,
            target_percent: 50,
        }
    }
}
impl CompactionSettings {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.target_percent == 0
            || self.target_percent >= self.trigger_percent
            || self.trigger_percent > 100
        {
            Err(invalid(
                "compaction requires 0 < target_percent < trigger_percent <= 100",
            ))
        } else {
            Ok(())
        }
    }
}

validated_dto!(
    /// Supported execution values only. No application capacities, counts,
    /// entity quotas, input-size caps, or mutable process-global directory.
    ExecutionSettings, ExecutionSettingsWire, "ExecutionSettingsWire" {
        #[serde(default)]
        approval: ApprovalMode,
        prompt_timeout_ms: u64,
        model_timeout_ms: u64,
        policy_timeout_ms: u64,
        tool_timeout_ms: u64,
        model_retry_attempts: u8,
        model_retry_base_delay_ms: u64,
        compaction: CompactionSettings,
    }
);
impl Default for ExecutionSettings {
    fn default() -> Self {
        Self {
            approval: ApprovalMode::default(),
            prompt_timeout_ms: 30_000,
            model_timeout_ms: 600_000,
            policy_timeout_ms: 30_000,
            tool_timeout_ms: 1_800_000,
            model_retry_attempts: 2,
            model_retry_base_delay_ms: 100,
            compaction: CompactionSettings::default(),
        }
    }
}
impl ExecutionSettings {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.prompt_timeout_ms == 0
            || self.model_timeout_ms == 0
            || self.policy_timeout_ms == 0
            || self.tool_timeout_ms == 0
        {
            return Err(invalid("execution timeouts must be positive milliseconds"));
        }
        // These are the existing Runtime driver's legal retry/deadline ranges.
        if self.model_timeout_ms > 86_400_000
            || !(1..=4).contains(&self.model_retry_attempts)
            || !(1..=30_000).contains(&self.model_retry_base_delay_ms)
        {
            return Err(invalid(
                "execution model timing or retry value is outside Runtime's supported range",
            ));
        }
        self.compaction.validate()
    }
}

validated_dto!(SessionDefaults, SessionDefaultsWire, "SessionDefaultsWire" {
    model: ModelRef,
    reasoning: ReasoningPreference,
    tools: Vec<ToolBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceRef>,
    execution: ExecutionSettings,
});
impl Default for SessionDefaults {
    /// `default` is a selection reference, not proof of a configured provider.
    /// Direct owners still resolve it using their current Host resources.
    fn default() -> Self {
        Self {
            model: "default"
                .parse()
                .expect("literal has valid Runtime grammar"),
            reasoning: ReasoningPreference::Auto,
            tools: Vec::new(),
            workspace: None,
            execution: ExecutionSettings::default(),
        }
    }
}
impl SessionDefaults {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_session_fields(&self.execution, &self.tools, self.workspace.as_ref())
    }
}
fn validate_session_fields(
    execution: &ExecutionSettings,
    tools: &[ToolBinding],
    workspace: Option<&WorkspaceRef>,
) -> Result<(), ValidationError> {
    execution.validate()?;
    let mut names = BTreeSet::new();
    for tool in tools {
        tool.validate()?;
        if !names.insert(&tool.name) {
            return Err(invalid("duplicate tool binding"));
        }
    }
    if let Some(workspace) = workspace {
        workspace.validate()?;
    }
    Ok(())
}

validated_dto!(#[derive(Default)] AgentConfig, AgentConfigWire, "AgentConfigWire" {
    role: Role,
    session_defaults: SessionDefaults,
});
impl AgentConfig {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.role.validate()?;
        self.session_defaults.validate()
    }
    /// Build a complete independent candidate. A failed candidate leaves the
    /// original untouched; ownership/version/resource checks remain with owner.
    pub fn apply_patch(&self, patch: &AgentConfigPatch) -> Result<Self, ValidationError> {
        let candidate = Self {
            role: patch.role.clone().unwrap_or_else(|| self.role.clone()),
            session_defaults: patch
                .session_defaults
                .clone()
                .unwrap_or_else(|| self.session_defaults.clone()),
        };
        candidate.validate()?;
        Ok(candidate)
    }
}

validated_dto!(SessionConfig, SessionConfigWire, "SessionConfigWire" {
    role: Role,
    model: ModelRef,
    reasoning: ReasoningPreference,
    tools: Vec<ToolBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<WorkspaceRef>,
    execution: ExecutionSettings,
});
impl Default for SessionConfig {
    fn default() -> Self {
        Self::from_agent(&AgentConfig::default(), &SessionOverrides::default())
            .expect("defaults are structurally valid")
    }
}
impl SessionConfig {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.role.validate()?;
        validate_session_fields(&self.execution, &self.tools, self.workspace.as_ref())
    }
    pub fn from_agent(
        agent: &AgentConfig,
        overrides: &SessionOverrides,
    ) -> Result<Self, ValidationError> {
        agent.validate()?;
        let defaults = &agent.session_defaults;
        Self {
            role: agent.role.clone(),
            model: defaults.model.clone(),
            reasoning: defaults.reasoning,
            tools: defaults.tools.clone(),
            workspace: defaults.workspace.clone(),
            execution: defaults.execution.clone(),
        }
        .apply_patch(&overrides.0)
    }
    pub fn apply_patch(&self, patch: &SessionConfigPatch) -> Result<Self, ValidationError> {
        let mut candidate = self.clone();
        if let Some(value) = &patch.role {
            candidate.role = value.clone();
        }
        if let Some(value) = &patch.model {
            candidate.model = value.clone();
        }
        if let Some(value) = patch.reasoning {
            candidate.reasoning = value;
        }
        if let Some(value) = &patch.tools {
            candidate.tools = value.clone();
        }
        if let Some(value) = &patch.execution {
            candidate.execution = value.clone();
        }
        match &patch.workspace {
            NullablePatch::Unchanged => {}
            NullablePatch::Set(value) => candidate.workspace = Some(value.clone()),
            NullablePatch::Clear => candidate.workspace = None,
        }
        candidate.validate()?;
        Ok(candidate)
    }
    /// Structural validation is insufficient for current resource availability.
    /// Host supplies the narrow validator without changing the selected values.
    pub fn validate_resources(
        &self,
        resources: &impl ConfigResources,
    ) -> Result<(), ValidationError> {
        self.validate()?;
        resources.validate_model(&self.model, self.reasoning)?;
        for tool in &self.tools {
            resources.validate_tool(tool)?;
        }
        if let Some(workspace) = &self.workspace {
            resources.validate_workspace(workspace)?;
        }
        Ok(())
    }
}

/// Read-only resource validation. Implementations must not invoke a model or
/// execute tools to check configuration, or substitute a different resource.
pub trait ConfigResources {
    fn validate_model(
        &self,
        model: &ModelRef,
        reasoning: ReasoningPreference,
    ) -> Result<(), ValidationError>;
    fn validate_tool(&self, tool: &ToolBinding) -> Result<(), ValidationError>;
    fn validate_workspace(&self, workspace: &WorkspaceRef) -> Result<(), ValidationError>;
}

/// Omission and explicit null differ. Only the workspace field is clearable.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum NullablePatch<T> {
    #[default]
    Unchanged,
    Set(T),
    Clear,
}
impl<T> NullablePatch<T> {
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}
impl<T: Serialize> Serialize for NullablePatch<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(value) => value.serialize(s),
            Self::Clear => s.serialize_none(),
            Self::Unchanged => Err(serde::ser::Error::custom("unchanged patch must be omitted")),
        }
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for NullablePatch<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Option::<T>::deserialize(d).map(|value| value.map(Self::Set).unwrap_or(Self::Clear))
    }
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfigPatch {
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
    pub execution: Option<ExecutionSettings>,
}
impl SessionConfigPatch {
    pub fn is_empty(&self) -> bool {
        self.role.is_none()
            && self.model.is_none()
            && self.reasoning.is_none()
            && self.tools.is_none()
            && self.workspace.is_unchanged()
            && self.execution.is_none()
    }
}

/// Creation omission inherits the single selected Agent version; this remains
/// a distinct type from updates, whose omissions retain existing Session data.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionOverrides(pub SessionConfigPatch);

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigPatch {
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
    pub session_defaults: Option<SessionDefaults>,
}
impl AgentConfigPatch {
    pub fn is_empty(&self) -> bool {
        self.role.is_none() && self.session_defaults.is_none()
    }
}

fn nonblank(value: &str, name: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(invalid(format!(
            "{name} must be nonblank and contain no NUL"
        )))
    } else {
        Ok(())
    }
}
