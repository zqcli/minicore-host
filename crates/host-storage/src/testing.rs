//! Explicit test fixtures for real journal ownership and stored bytes.
//!
//! This non-default feature is for development dependencies. It does not provide
//! production persistence, recovery, or confirmation APIs. Callers synchronize
//! their scenarios before inspecting or corrupting an owned history file.

mod read;

pub use read::read_history_bytes;
use std::{io, path::Path};

use crate::{SaveConfirmation, SessionJournal, StorageError};

#[derive(Clone, Copy, Debug)]
pub enum AppendFailure {
    BeforeWrite,
    AfterPrefix(usize),
    BeforeFileSync,
}

/// Run the real pending append, failing at the named existing storage seam.
/// The original accepted range and writer remain owned for reconciliation.
pub fn persist_pending_with_failure(
    journal: &mut SessionJournal,
    failure: AppendFailure,
) -> Result<SaveConfirmation, StorageError> {
    journal.persist_pending_with_failure_for_test(failure)
}

/// Corrupt actual bytes through the existing owner's handle. This does not
/// update its stamp, accepted range, pending bytes, or persistence confirmation.
pub fn overwrite_owned_history(journal: &mut SessionJournal, bytes: &[u8]) -> io::Result<()> {
    journal.overwrite_owned_history_for_test(bytes)
}

pub fn read_history_text(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read_history_bytes(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
