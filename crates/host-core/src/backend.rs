use crate::session::{HistoryBackend, LoadFuture, SaveFuture};
use minicore_host_protocol::{Fault, FaultDetails, FaultKind, HistoryLoadDetails, StorageMode};
use minicore_host_storage::{HistoryEntry, SessionJournal, SessionStore};
use std::sync::{Arc, Mutex};

/// A save task retains the journal and its exclusive root guard even if the
/// caller abandons its future. No business-state lock participates in disk I/O.
pub struct JournalHistory {
    store: SessionStore,
    journal: Arc<Mutex<Option<SessionJournal>>>,
}
impl JournalHistory {
    pub fn new(store: SessionStore) -> Self {
        Self {
            store,
            journal: Arc::new(Mutex::new(None)),
        }
    }
}
impl HistoryBackend for JournalHistory {
    fn mode(&self) -> StorageMode {
        self.store.mode()
    }
    fn load(&self) -> LoadFuture<'_> {
        let store = self.store.clone();
        let journal = Arc::clone(&self.journal);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut current = journal.lock().expect("journal mutex poisoned");
                if current.as_ref().is_some_and(|value| {
                    value.needs_reconciliation() || value.pending_range().is_some()
                }) {
                    return Err(Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "the original pending save must be reconciled before history can reload",
                    ));
                }
                // loaded_records is an opening baseline, not a live history cache.
                // Only a quiescent owner calls load, and this disk-only lock also
                // excludes appends. Reopen and validate the actual complete file.
                current.take();
                *current = Some(store.open_journal().map_err(load_fault)?);
                Ok(current
                    .as_ref()
                    .expect("opened journal")
                    .loaded_records()
                    .iter()
                    .filter_map(|record| record.entry.runtime_item())
                    .collect())
            })
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "history reader did not return a result",
                )
            })?
        })
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        let store = self.store.clone();
        let journal = Arc::clone(&self.journal);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut current = journal.lock().expect("journal mutex poisoned");
                if current.is_none() {
                    *current = Some(store.open_journal().map_err(load_fault)?);
                }
                let current = current.as_mut().expect("opened journal");
                current.accept(entries).map_err(write_fault)?;
                Ok(current.persist_pending().map_err(write_fault)?.persistence)
            })
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::SessionHistoryWriteFailed,
                    "history save did not return a confirmation",
                )
            })?
        })
    }
}
fn load_fault(error: minicore_host_storage::StorageError) -> Fault {
    let position = match &error {
        minicore_host_storage::StorageError::InvalidHistory { position, .. } => Some(*position),
        minicore_host_storage::StorageError::UnsupportedSchema { position, .. } => *position,
        _ => None,
    };
    let fault = Fault::new(FaultKind::SessionHistoryLoadFailed, error.to_string());
    if let Some(position) = position {
        fault
            .with_details(FaultDetails::HistoryLoad(HistoryLoadDetails {
                line: Some(position.line),
                byte_offset: Some(position.byte_offset),
            }))
            .expect("storage positions satisfy the history load schema")
    } else {
        fault
    }
}
fn write_fault(error: minicore_host_storage::StorageError) -> Fault {
    Fault::new(FaultKind::SessionHistoryWriteFailed, error.to_string())
}
