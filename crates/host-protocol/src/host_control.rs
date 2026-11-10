//! Host shutdown requests and the direct owner's current shutdown facts.
use crate::{HostLifecycle, OperationId, ValidationError, invalid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostShutdownStrategy {
    Graceful,
    Force,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostShutdownRequest {
    pub operation_id: OperationId,
    pub strategy: HostShutdownStrategy,
}
crate::validated_dto!(HostShutdownResult, HostShutdownResultWire, "HostShutdownResultWire" {
    strategy: HostShutdownStrategy,
    lifecycle: HostLifecycle,
});
impl HostShutdownResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match (self.strategy, self.lifecycle) {
            (
                HostShutdownStrategy::Graceful,
                HostLifecycle::Draining | HostLifecycle::Stopping | HostLifecycle::Stopped,
            )
            | (HostShutdownStrategy::Force, HostLifecycle::Stopping | HostLifecycle::Stopped) => {
                Ok(())
            }
            _ => Err(invalid(
                "shutdown requires its actual stopping/draining state; force cannot drain",
            )),
        }
    }
}
crate::control_receipt!(
    HostShutdownReceipt,
    HostShutdownKind,
    HostShutdown,
    "host.shutdown",
    HostShutdownResult
);
