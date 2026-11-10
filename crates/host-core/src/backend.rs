use crate::session::{HistoryBackend, HistoryReadFuture, LoadFuture, SaveFuture};
use minicore_host_protocol::{Fault, FaultDetails, FaultKind, HistoryLoadDetails, StorageMode};
use minicore_host_storage::{HistoryEntry, SaveRange, SessionJournal, SessionStore};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

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
    #[cfg(test)]
    failure: Option<TestAppendFault>,
    #[cfg(test)]
    read_gate: Option<Arc<TestReadGate>>,
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
                #[cfg(test)]
                failure: None,
                #[cfg(test)]
                read_gate: None,
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
                    .map(|record| record.entry.clone())
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
    fn read_cold(&self) -> HistoryReadFuture<'_> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let current = state.lock().expect("journal mutex poisoned");
                let store = current.store()?;
                if current.journal.is_some() || current.pending.is_some() {
                    // The existing owner must publish its actual retained data.
                    // loaded_records is only an opening baseline, never a cache.
                    return Ok(None);
                }
                // Keep the short-lived reader under the same disk-only guard as
                // load/save/release. Neither Session nor access state is locked.
                #[cfg(test)]
                if let Some(gate) = &current.read_gate {
                    gate.block();
                }
                let history = store.read_history().map_err(load_fault)?;
                Ok(Some(
                    history
                        .records
                        .into_iter()
                        .map(|record| record.entry)
                        .collect(),
                ))
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
    fn read_saved_turn(&self, id: &minicore_host_protocol::TurnId) -> LoadFuture<'_> {
        let state = Arc::clone(&self.state);
        let id = id.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let current = state.lock().expect("journal mutex poisoned");
                let store = current.store()?;
                #[cfg(test)]
                if let Some(gate) = &current.read_gate {
                    gate.block();
                }
                if let Some(journal) = &current.journal {
                    // This is only the already validated opening baseline of
                    // old saved Turns, never the current execution/history cache.
                    // The Session checks current accepted ownership on both sides
                    // of this read, including finalizing and unconfirmed saves.
                    return Ok(journal
                        .loaded_records()
                        .iter()
                        .filter(|record| record.entry.turn_id() == Some(&id))
                        .map(|record| record.entry.clone())
                        .collect());
                }
                // Validate the whole file before filtering. Do not accept a
                // terminal prefix from a corrupt or incomplete history file.
                Ok(store
                    .read_history()
                    .map_err(load_fault)?
                    .records
                    .into_iter()
                    .filter(|record| record.entry.turn_id() == Some(&id))
                    .map(|record| record.entry)
                    .collect())
            })
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "saved Turn reader did not return a result",
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
                #[cfg(test)]
                if let Some(result) = current.inject_append_fault() {
                    return result;
                }
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
    fn release(&self) -> Pin<Box<dyn Future<Output = Result<(), Fault>> + Send + '_>> {
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
                Ok(())
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

// Explicit fault fixture: these tests use the real SessionJournal's validation,
// accepted range, reconciliation, sync, and file identity checks. Only the write
// interruption is injected by writing zero/a prefix/all original bytes without
// sync and returning an error. This is not a live filesystem/provider failure.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum TestAppendFault {
    NoWrite,
    FullWrite,
    Prefix(usize),
}
#[cfg(test)]
impl JournalHistory {
    pub(crate) fn gate_cold_read(&self, gate: Arc<TestReadGate>) {
        self.state.lock().unwrap().read_gate = Some(gate);
    }
    pub(crate) fn inject_next_append_fault(&self, failure: TestAppendFault) {
        self.state.lock().unwrap().failure = Some(failure);
    }
}
#[cfg(test)]
impl JournalState {
    fn inject_append_fault(
        &mut self,
    ) -> Option<Result<minicore_host_protocol::PersistenceState, Fault>> {
        use std::io::Write;
        let failure = self.failure.take()?;
        let entries = match self.pending.as_ref().expect("fixture accepted records") {
            PendingAppend::Unaccepted(entries) => entries.clone(),
            _ => unreachable!("fixture runs before journal acceptance"),
        };
        Some((|| {
            let range = self.prepare_original()?;
            let mut bytes = Vec::new();
            for (index, entry) in entries.into_iter().enumerate() {
                let record = minicore_host_storage::HistoryRecord::new(
                    self.store()?.id().clone(),
                    range.start + index as u64,
                    entry,
                );
                bytes.extend(serde_json::to_vec(&record).unwrap());
                bytes.push(b'\n');
            }
            let length = match failure {
                TestAppendFault::NoWrite => 0,
                TestAppendFault::FullWrite => bytes.len(),
                TestAppendFault::Prefix(length) => length.min(bytes.len()),
            };
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(self.store()?.canonical_path().join("history.jsonl"))
                .unwrap();
            file.write_all(&bytes[..length]).unwrap();
            Err(Fault::new(
                FaultKind::SessionHistoryWriteFailed,
                "injected interruption of the original history write before sync",
            ))
        })())
    }
}

#[cfg(test)]
#[path = "tests/task052_backend_gate.rs"]
mod task052;
#[cfg(test)]
pub(crate) use task052::TestReadGate;
