use crate::session::{HistoryBackend, LoadFuture, SaveFuture};
use minicore_host_protocol::{Fault, FaultDetails, FaultKind, HistoryLoadDetails, StorageMode};
use minicore_host_storage::{HistoryEntry, SaveRange, SessionJournal, SessionStore};
use std::sync::{Arc, Mutex};

/// A save task retains the journal and its exclusive root guard even if the
/// caller abandons its future. This disk-only mutex is never an owner/access lock.
pub struct JournalHistory {
    mode: StorageMode,
    state: Arc<Mutex<JournalState>>,
}
struct JournalState {
    store: Option<SessionStore>,
    journal: Option<SessionJournal>,
    pending: Option<PendingAppend>,
}
enum PendingAppend {
    /// No journal range has been accepted yet. Opening/encoding failure cannot
    /// discard the original records or authorize a different append.
    Unaccepted(Vec<HistoryEntry>),
    Accepted(SaveRange),
}
impl JournalState {
    fn store(&self) -> Result<&SessionStore, Fault> {
        self.store.as_ref().ok_or_else(|| {
            Fault::new(
                FaultKind::SessionLifecycleClosed,
                "history resources were retired",
            )
        })
    }
    fn prepare_original(&mut self) -> Result<SaveRange, Fault> {
        if self.journal.is_none() {
            self.journal = Some(self.store()?.open_journal().map_err(load_fault)?);
        }
        match self.pending.as_ref() {
            Some(PendingAppend::Accepted(range)) => Ok(*range),
            Some(PendingAppend::Unaccepted(entries)) => {
                let range = self
                    .journal
                    .as_mut()
                    .expect("opened journal")
                    .accept(entries.clone())
                    .map_err(write_fault)?;
                self.pending = Some(PendingAppend::Accepted(range));
                Ok(range)
            }
            None => Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "there is no original history save to continue",
            )),
        }
    }
    fn persist_original(
        &mut self,
        reconcile: bool,
    ) -> Result<minicore_host_protocol::PersistenceState, Fault> {
        let range = self.prepare_original()?;
        let journal = self.journal.as_mut().expect("opened journal");
        if self
            .store
            .as_ref()
            .is_some_and(|store| store.mode() == StorageMode::Persistent)
            && !range.is_empty()
            && journal.pending_range() != Some(range)
        {
            return Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "journal no longer owns the original accepted range",
            ));
        }
        let confirmation = if reconcile {
            journal.reconcile_pending()
        } else {
            journal.persist_pending()
        }
        .map_err(write_fault)?;
        if confirmation.range != range {
            return Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "history confirmation does not cover the original range",
            ));
        }
        self.pending = None;
        Ok(confirmation.persistence)
    }
}
impl JournalHistory {
    pub fn new(store: SessionStore) -> Self {
        Self {
            mode: store.mode(),
            state: Arc::new(Mutex::new(JournalState {
                store: Some(store),
                journal: None,
                pending: None,
            })),
        }
    }
}
impl HistoryBackend for JournalHistory {
    fn mode(&self) -> StorageMode {
        self.mode
    }
    fn load(&self) -> LoadFuture<'_> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut current = state.lock().expect("journal mutex poisoned");
                if current.pending.is_some()
                    || current.journal.as_ref().is_some_and(|journal| {
                        journal.needs_reconciliation() || journal.pending_range().is_some()
                    })
                {
                    return Err(Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "the original pending save must be reconciled before history can reload",
                    ));
                }
                let store = current.store()?.clone();
                // loaded_records is an opening baseline, not a live history cache.
                // Only a quiescent owner calls load. Never reload to manufacture
                // acknowledgement of a failed original append.
                current.journal.take();
                current.journal = Some(store.open_journal().map_err(load_fault)?);
                Ok(current
                    .journal
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
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut current = state.lock().expect("journal mutex poisoned");
                current.store()?;
                if current.pending.is_some() {
                    return Err(Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "the original history save is still pending",
                    ));
                }
                current.pending = Some(PendingAppend::Unaccepted(entries));
                current.persist_original(false)
            })
            .await
            .map_err(save_worker_fault)?
        })
    }
    fn reconcile(&self) -> SaveFuture<'_> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut current = state.lock().expect("journal mutex poisoned");
                current.store()?;
                current.persist_original(true)
            })
            .await
            .map_err(save_worker_fault)?
        })
    }
    fn release(&self) -> SaveFuture<'_> {
        let state = Arc::clone(&self.state);
        let mode = self.mode;
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let mut current = state.lock().expect("journal mutex poisoned");
                if current.pending.is_some()
                    || current.journal.as_ref().is_some_and(|journal| {
                        journal.needs_reconciliation() || journal.pending_range().is_some()
                    })
                {
                    return Err(Fault::new(
                        FaultKind::SessionHistoryWriteFailed,
                        "cannot retire history resources with an unconfirmed original save",
                    ));
                }
                // The Session's accepted owner fence prevents future append/load.
                // This disk-only guard also waits for any already-started worker.
                let journal = current.journal.take();
                let store = current.store.take();
                drop(current);
                drop(journal);
                drop(store);
                Ok(if mode == StorageMode::Memory {
                    minicore_host_protocol::PersistenceState::NotApplicable
                } else {
                    minicore_host_protocol::PersistenceState::Persisted
                })
            })
            .await
            .map_err(save_worker_fault)?
        })
    }
}
fn save_worker_fault(_: tokio::task::JoinError) -> Fault {
    Fault::new(
        FaultKind::SessionHistoryWriteFailed,
        "history worker did not return a confirmation",
    )
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
