//! Client wait options and facts about one actual Host connection.
use crate::{HostId, HostIncarnation, Revision, ValidationError, invalid};
use serde::{Deserialize, Serialize};

pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;

crate::validated_dto!(#[derive(Default)] CallOptions, CallOptionsWire, "CallOptionsWire" {
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::present")]
    wait_timeout_ms: Option<u64>,
});
impl CallOptions {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.wait_timeout_ms == Some(0) {
            Err(invalid(
                "wait_timeout_ms must be positive integer milliseconds",
            ))
        } else {
            Ok(())
        }
    }
    pub fn timeout_ms(&self) -> u64 {
        self.wait_timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlStateData {
    pub occupied: bool,
    pub revision: Revision,
}
crate::validated_dto!(ConnectionInfo, ConnectionInfoWire, "ConnectionInfoWire" {
    host_id: HostId,
    host_incarnation: HostIncarnation,
    can_operate: bool,
    control: ControlStateData,
});
impl ConnectionInfo {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.can_operate && !self.control.occupied {
            Err(invalid(
                "an operating connection implies occupied Host control",
            ))
        } else {
            Ok(())
        }
    }
}
