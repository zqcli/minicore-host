//! Owner-scoped MiniCore persistence. This crate does not schedule or recover work.
//!
//! Disk operations are synchronous: call them outside every business-state lock,
//! on a blocking worker owned by the accepted save operation. Keep storage handles
//! alive until that worker has completed. They retain the exclusive root lock.

#![forbid(unsafe_code)]

#[cfg(not(target_os = "linux"))]
compile_error!(
    "MiniCore Host storage currently supports Linux only; no weaker lock/path fallback is provided"
);

mod error;
mod filesystem;
mod journal;
mod objects;
mod records;
mod strict_json;

pub use error::*;
pub use journal::*;
pub use objects::*;
pub use records::*;
