//! Owner-scoped MiniCore persistence. This crate does not schedule or recover work.
//!
//! Disk operations are synchronous: call them outside every business-state lock,
//! on a blocking worker owned by the accepted save operation. Keep storage handles
//! alive until that worker has completed. They retain the exclusive root lock.

#![forbid(unsafe_code)]

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!(
    "MiniCore Host storage requires Linux or macOS filesystem primitives; no weaker lock/path fallback is provided"
);

mod compaction;
mod error;
mod file_handle;
mod filesystem;
mod journal;
mod objects;
mod records;
mod strict_json;

pub use compaction::*;
pub use error::*;
pub use journal::*;
pub use objects::*;
pub use records::*;

/// Open a stable Host identity while retaining its exclusive root guard.
///
/// ```no_run
/// use minicore_host_protocol::HostId;
/// use minicore_host_storage::{DataRoot, HostRecord};
/// # fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let root = DataRoot::acquire("./host-data")?;
/// let existing = root.load_host::<serde_json::Value>()?;
/// let record = match existing {
///     Some(record) => record,
///     None => HostRecord::new(HostId::new()?, serde_json::json!({}), 0),
/// };
/// let host = root.host(record.id.clone());
/// host.save_host(&record)?;
/// // The Host initializes only its direct Agents through their owners.
/// # Ok(())
/// # }
/// ```
pub const STORAGE_SCHEMA_VERSION: u32 = SCHEMA_VERSION;

#[cfg(test)]
mod lease_tests;
