use crate::action::{invalid, validate_name};
use minicore_host_protocol::*;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// `settings` and config changes use exactly the owner's allowed values.
/// A metadata field is parsed separately so unknown config fields remain errors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentChanges {
    pub name: Option<String>,
    pub settings: SessionConfigPatch,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionChanges {
    pub title: NullablePatch<String>,
    pub settings: SessionConfigPatch,
}
impl AgentChanges {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.name.is_none() && self.settings.is_empty() {
            return Err(invalid("changes must not be empty"));
        }
        if let Some(name) = &self.name {
            validate_name(name)?;
        }
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
        Ok(AgentUpdateCandidate {
            expected_revision: self.config_revision,
            config: agent_config_from_settings(&self.config, &changes.settings)?,
            name: changes.name.clone().unwrap_or_else(|| self.name.clone()),
        })
    }
}
impl SessionConfigSnapshot {
    pub fn apply(&self, changes: &SessionChanges) -> Result<SessionUpdateCandidate, Fault> {
        changes.validate()?;
        Ok(SessionUpdateCandidate {
            expected_revision: self.config_revision,
            config: self
                .config
                .apply_patch(&changes.settings)
                .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?,
            title: match &changes.title {
                NullablePatch::Unchanged => self.title.clone(),
                NullablePatch::Set(title) => Some(title.clone()),
                NullablePatch::Clear => None,
            },
        })
    }
}
