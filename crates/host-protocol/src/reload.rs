//! Canonical Host reload requests and direct-owner progress snapshots.
use crate::{
    AgentCreateReceipt, AgentId, ConfigRevision, Fault, HostId, HostIncarnation, OperationId,
    ValidationError, invalid,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Reload only the directory bound at startup. Paths, configuration candidates,
/// credentials and private preparation callbacks are never accepted over RPC.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReloadRequest {
    pub operation_id: OperationId,
    pub expected_revision: ConfigRevision,
}

/// Stable declaration provenance belongs to the Agent owner, never AgentConfig.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfigurationSource {
    pub key: String,
    pub managed: bool,
}
impl AgentConfigurationSource {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.key.trim().is_empty() || self.key.contains('\0') {
            return Err(invalid(
                "Agent configuration key must be nonblank and contain no NUL",
            ));
        }
        Ok(())
    }
}

/// Each entry reports the actual direct-owner result; creation retains its
/// existing Agent/Session steps, including a durable Agent with a failed Session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentConfigurationResult {
    Created {
        key: String,
        receipt: Box<AgentCreateReceipt>,
    },
    Updated {
        key: String,
        agent_id: AgentId,
        config_revision: ConfigRevision,
    },
    Unmanaged {
        key: String,
        agent_id: AgentId,
        config_revision: ConfigRevision,
    },
    Unchanged {
        key: String,
        agent_id: AgentId,
        config_revision: ConfigRevision,
    },
    Failed {
        key: String,
        agent_id: Option<AgentId>,
        error: Fault,
    },
}
impl AgentConfigurationResult {
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Created { receipt, .. } => {
                receipt.result.steps.agent.succeeded() && receipt.result.steps.session.succeeded()
            }
            Self::Failed { .. } => false,
            _ => true,
        }
    }
    fn validate(&self) -> Result<(), ValidationError> {
        let key = match self {
            Self::Created { key, receipt } => {
                receipt.validate()?;
                key
            }
            Self::Updated { key, .. }
            | Self::Unmanaged { key, .. }
            | Self::Unchanged { key, .. } => key,
            Self::Failed { key, error, .. } => {
                error.validate()?;
                key
            }
        };
        AgentConfigurationSource {
            key: key.clone(),
            managed: true,
        }
        .validate()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HostReloadKind {
    #[serde(rename = "host.reload")]
    HostReload,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartField {
    NetworkListen,
    Logging,
}

/// Each value is a snapshot of facts owned by the actual configuration work.
/// The revision identifies the effective Host configuration at this phase.
/// Applying may already include Host publication while Agent work continues;
/// each terminal Agent entry reports its own actual saved result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostReloadResult {
    Preparing {
        config_revision: ConfigRevision,
    },
    Applying {
        config_revision: ConfigRevision,
    },
    Applied {
        config_revision: ConfigRevision,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        agents: Vec<AgentConfigurationResult>,
    },
    PartiallyApplied {
        config_revision: ConfigRevision,
        agents: Vec<AgentConfigurationResult>,
    },
    RestartRequired {
        config_revision: ConfigRevision,
        #[serde(
            serialize_with = "serialize_restart_fields",
            deserialize_with = "deserialize_restart_fields"
        )]
        fields: Vec<RestartField>,
    },
    Failed {
        config_revision: ConfigRevision,
        error: Fault,
    },
}
impl HostReloadResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Applied { agents, .. } => {
                for agent in agents {
                    agent.validate()?;
                }
                if agents.iter().any(|agent| !agent.succeeded()) {
                    return Err(invalid("applied reload cannot contain failed Agent work"));
                }
                Ok(())
            }
            Self::PartiallyApplied { agents, .. } => {
                for agent in agents {
                    agent.validate()?;
                }
                if agents.iter().all(AgentConfigurationResult::succeeded) {
                    return Err(invalid(
                        "partially applied reload needs an actual Agent failure",
                    ));
                }
                Ok(())
            }
            Self::RestartRequired { fields, .. } => validate_restart_fields(fields),
            Self::Failed { error, .. } => error.validate(),
            _ => Ok(()),
        }
    }
}

fn validate_restart_fields(fields: &[RestartField]) -> Result<(), ValidationError> {
    if fields.is_empty() || fields.iter().collect::<BTreeSet<_>>().len() != fields.len() {
        Err(invalid(
            "restart_required needs distinct actual changed fields",
        ))
    } else {
        Ok(())
    }
}
fn serialize_restart_fields<S: serde::Serializer>(
    fields: &[RestartField],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    validate_restart_fields(fields).map_err(serde::ser::Error::custom)?;
    fields.serialize(serializer)
}
fn deserialize_restart_fields<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<RestartField>, D::Error> {
    let fields = Vec::deserialize(deserializer)?;
    validate_restart_fields(&fields).map_err(serde::de::Error::custom)?;
    Ok(fields)
}

crate::validated_dto!(HostReloadReceipt, HostReloadReceiptWire, "HostReloadReceiptWire" {
    kind: HostReloadKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    result: HostReloadResult,
});
impl HostReloadReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()
    }
}
