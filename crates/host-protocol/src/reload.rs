//! Canonical Host reload requests and direct-owner progress snapshots.
use crate::{
    ConfigRevision, Fault, HostId, HostIncarnation, OperationId, ValidationError, invalid,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// `C` is the application's concrete readable HostConfig. Private credentials,
/// resource handles, preparation callbacks and waiting options are not JSON.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostReloadRequest<C> {
    pub operation_id: OperationId,
    pub expected_revision: ConfigRevision,
    pub candidate: C,
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
/// Preparing/applying do not claim publication or a confirmed save. The revision
/// is the unchanged effective version until the complete candidate is applied.
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
    },
    RestartRequired {
        config_revision: ConfigRevision,
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
            Self::RestartRequired { fields, .. }
                if fields.is_empty()
                    || fields.iter().collect::<BTreeSet<_>>().len() != fields.len() =>
            {
                Err(invalid(
                    "restart_required needs distinct actual changed fields",
                ))
            }
            Self::Failed { error, .. } => error.validate(),
            _ => Ok(()),
        }
    }
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
