use crate::{
    AgentId, Fault, HostId, HostIncarnation, OperationId, PersistenceState, Revision, SessionId,
    ValidationError, invalid,
};
use serde::{Deserialize, Serialize};

/// Actual replacement coverage in the original native transcript. This is a
/// Session-local history position, not a controller revision or new identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionChange {
    pub source_position: u64,
    pub covered_items: u64,
    pub before_tokens: u64,
    pub after_tokens: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    from = "CompactionOutcomeWire"
)]
pub enum CompactionOutcome {
    Compacted {
        change: CompactionChange,
        persistence: PersistenceState,
    },
    Unchanged,
    Cancelled,
    Failed {
        fault: Fault,
    },
}
// Serde's internally tagged unit variants ignore extra fields. Empty struct
// variants in this private wire shape keep those outcomes strict while the
// public Rust enum retains its ordinary unit constructors.
#[derive(Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum CompactionOutcomeWire {
    Compacted {
        change: CompactionChange,
        persistence: PersistenceState,
    },
    Unchanged {},
    Cancelled {},
    Failed {
        fault: Fault,
    },
}
impl From<CompactionOutcomeWire> for CompactionOutcome {
    fn from(value: CompactionOutcomeWire) -> Self {
        match value {
            CompactionOutcomeWire::Compacted {
                change,
                persistence,
            } => Self::Compacted {
                change,
                persistence,
            },
            CompactionOutcomeWire::Unchanged {} => Self::Unchanged,
            CompactionOutcomeWire::Cancelled {} => Self::Cancelled,
            CompactionOutcomeWire::Failed { fault } => Self::Failed { fault },
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompactionState {
    Preparing {},
    Summarizing {},
    Merging {},
    Committing {
        change: CompactionChange,
        persistence: PersistenceState,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::present"
        )]
        fault: Option<Fault>,
    },
    Finished {
        result: CompactionOutcome,
    },
}
impl CompactionState {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let change = match self {
            Self::Committing {
                change,
                persistence,
                fault,
            } => {
                if !matches!(
                    (persistence, fault),
                    (PersistenceState::Pending, None) | (PersistenceState::Failed, Some(_))
                ) {
                    return Err(invalid(
                        "committing compaction requires pending or failed original save facts",
                    ));
                }
                Some(change)
            }
            Self::Finished {
                result:
                    CompactionOutcome::Compacted {
                        change,
                        persistence,
                    },
            } => {
                if !matches!(
                    persistence,
                    PersistenceState::Persisted | PersistenceState::NotApplicable
                ) {
                    return Err(invalid(
                        "compacted requires confirmed persistence or memory mode",
                    ));
                }
                Some(change)
            }
            _ => None,
        };
        if let Some(change) = change
            && (change.covered_items == 0 || change.after_tokens >= change.before_tokens)
        {
            return Err(invalid(
                "compaction must cover native history and reduce its estimated request",
            ));
        }
        Ok(())
    }
    pub fn finished(&self) -> bool {
        matches!(self, Self::Finished { .. })
    }
}
crate::validated_dto!(CompactFacts, CompactFactsWire, "CompactFactsWire" {
    state: CompactionState,
    cancellation_requested: bool,
    controller_revision: Revision,
});
impl CompactFacts {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.state.validate()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CompactKind {
    #[serde(rename = "session.compact")]
    Compact,
}
crate::validated_dto!(CompactReceipt, CompactReceiptWire, "CompactReceiptWire" {
    kind: CompactKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    agent_id: AgentId,
    session_id: SessionId,
    result: CompactFacts,
});
impl CompactReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CompactCancelKind {
    #[serde(rename = "session.compact.cancel")]
    CompactCancel,
}
crate::validated_dto!(CompactCancelResult, CompactCancelResultWire, "CompactCancelResultWire" {
    cancel_claimed: bool,
    target: CompactFacts,
});
impl CompactCancelResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.target.validate()?;
        if self.cancel_claimed
            && (!self.target.cancellation_requested
                || !matches!(
                    self.target.state,
                    CompactionState::Preparing {}
                        | CompactionState::Summarizing {}
                        | CompactionState::Merging {}
                ))
        {
            return Err(invalid(
                "claimed cancellation requires a precommit target with cancellation requested",
            ));
        }
        Ok(())
    }
}
crate::validated_dto!(CompactCancelReceipt, CompactCancelReceiptWire, "CompactCancelReceiptWire" {
    kind: CompactCancelKind,
    host_id: HostId,
    host_incarnation: HostIncarnation,
    operation_id: OperationId,
    target_operation_id: OperationId,
    agent_id: AgentId,
    session_id: SessionId,
    result: CompactCancelResult,
});
impl CompactCancelReceipt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.result.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_cancel_claim_requires_cancellable_original_phase_on_both_wire_paths() {
        let change = CompactionChange {
            source_position: 8,
            covered_items: 4,
            before_tokens: 500,
            after_tokens: 100,
        };
        let fault = Fault::new(crate::FaultKind::SessionCompactionFailed, "fixture failure");
        let states = [
            (CompactionState::Preparing {}, true),
            (CompactionState::Summarizing {}, true),
            (CompactionState::Merging {}, true),
            (
                CompactionState::Committing {
                    change: change.clone(),
                    persistence: PersistenceState::Pending,
                    fault: None,
                },
                false,
            ),
            (
                CompactionState::Committing {
                    change: change.clone(),
                    persistence: PersistenceState::Failed,
                    fault: Some(fault.clone()),
                },
                false,
            ),
            (
                CompactionState::Finished {
                    result: CompactionOutcome::Compacted {
                        change: change.clone(),
                        persistence: PersistenceState::Persisted,
                    },
                },
                false,
            ),
            (
                CompactionState::Finished {
                    result: CompactionOutcome::Compacted {
                        change,
                        persistence: PersistenceState::NotApplicable,
                    },
                },
                false,
            ),
            (
                CompactionState::Finished {
                    result: CompactionOutcome::Cancelled,
                },
                false,
            ),
            (
                CompactionState::Finished {
                    result: CompactionOutcome::Unchanged,
                },
                false,
            ),
            (
                CompactionState::Finished {
                    result: CompactionOutcome::Failed { fault },
                },
                false,
            ),
        ];
        for (state, cancellable) in states {
            for cancellation_requested in [false, true] {
                for cancel_claimed in [false, true] {
                    let result = CompactCancelResult {
                        cancel_claimed,
                        target: CompactFacts {
                            state: state.clone(),
                            cancellation_requested,
                            controller_revision: 7,
                        },
                    };
                    let valid = !cancel_claimed || (cancellable && cancellation_requested);
                    assert_eq!(serde_json::to_value(&result).is_ok(), valid, "{result:?}");
                    let wire = serde_json::json!({
                        "cancel_claimed": cancel_claimed,
                        "target": { "state": state, "cancellation_requested": cancellation_requested, "controller_revision": 7 }
                    });
                    assert_eq!(
                        serde_json::from_value::<CompactCancelResult>(wire).is_ok(),
                        valid,
                        "{result:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn compact_facts_reject_unconfirmed_completion_and_invalid_save_facts() {
        for state in [
            serde_json::json!({"phase":"finished", "result":{"outcome":"compacted", "change":{"source_position":8,"covered_items":4,"before_tokens":500,"after_tokens":100}, "persistence":"pending"}}),
            serde_json::json!({"phase":"committing", "change":{"source_position":8,"covered_items":4,"before_tokens":500,"after_tokens":100},"persistence":"persisted"}),
            serde_json::json!({"phase":"preparing", "invented":true}),
            serde_json::json!({"phase":"finished", "result":{"outcome":"cancelled","invented":true}}),
            serde_json::json!({"phase":"finished", "result":{"outcome":"unchanged","invented":true}}),
        ] {
            assert!(
                serde_json::from_value::<CompactFacts>(serde_json::json!({
                    "state":state,"cancellation_requested":false,"controller_revision":1
                }))
                .is_err()
            );
        }
    }
    #[test]
    fn compact_finished_memory_and_pending_save_have_distinct_facts() {
        let change = CompactionChange {
            source_position: 8,
            covered_items: 4,
            before_tokens: 500,
            after_tokens: 100,
        };
        for state in [
            CompactionState::Committing {
                change: change.clone(),
                persistence: PersistenceState::Pending,
                fault: None,
            },
            CompactionState::Finished {
                result: CompactionOutcome::Compacted {
                    change,
                    persistence: PersistenceState::NotApplicable,
                },
            },
            CompactionState::Finished {
                result: CompactionOutcome::Cancelled,
            },
        ] {
            let facts = CompactFacts {
                state,
                cancellation_requested: false,
                controller_revision: 7,
            };
            let value = serde_json::to_value(&facts).unwrap();
            assert_eq!(
                serde_json::from_value::<CompactFacts>(value).unwrap(),
                facts
            );
        }
    }
}
