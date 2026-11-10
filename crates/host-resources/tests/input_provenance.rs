//! Permanent regressions for the independently proven ordinary-input omission.
//! No live-model quality claim: only provider replies are scripted.
#[allow(dead_code)]
#[path = "support/production.rs"]
mod base;
#[path = "input_provenance/compaction.rs"]
mod compaction;
#[path = "input_provenance/input.rs"]
mod input;
#[path = "../../host-core/tests/support/input_context.rs"]
mod input_context;
#[path = "input_provenance/support.rs"]
mod support;
