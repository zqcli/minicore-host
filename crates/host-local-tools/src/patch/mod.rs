//! Exact single-file patch parsing and text assembly. Adapted from
//! minicore-agent f36a87366d8e2fe0a1c8216463887902e471c739's apply_patch module.
//! The Host's existing file worker owns all I/O, mutation ordering and records.
//! There are no patch byte or matching-work quotas; the actual tool cancellation
//! and deadline are checked throughout parsing, matching and assembly.

mod apply;
mod codex;
mod unified;

pub(crate) use apply::apply_single_file_patch;
