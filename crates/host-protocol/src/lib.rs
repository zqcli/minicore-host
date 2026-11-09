//! MiniCore Host v4.93 shared value contracts.
//!
//! These types do not authorize callers, schedule work, or promise availability.
//! Direct owners validate authorization and resource references at acceptance.
//! Accepted receipts describe facts at formation, never execution completion.

#![forbid(unsafe_code)]

mod config;
mod fault;
mod id;
mod ownership;

pub use config::*;
pub use fault::*;
pub use id::*;
pub use ownership::*;

/// Unsigned, monotonically increasing versions, scoped to the owning object and
/// Host incarnation. Owners must use checked arithmetic rather than wraparound.
pub type Revision = u64;
pub type ConfigRevision = Revision;
pub type ControllerRevision = Revision;

/// A structural validation error. It is not itself a business Fault; the owner
/// maps it according to the operation whose candidate it was validating.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ValidationError {}

pub(crate) fn invalid(message: impl Into<String>) -> ValidationError {
    ValidationError(message.into())
}
