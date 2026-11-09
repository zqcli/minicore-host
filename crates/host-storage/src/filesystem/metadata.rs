//! File-local coordination, never a business-owner or root-wide I/O lock.
//!
//! Atomic replacement must not unlink a metadata inode while another operation
//! from this same DataRoot is validating its read snapshot. Unknown filesystem
//! mutations do not take these gates and remain subject to the normal checks.

use crate::{StorageError, error::io_error};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak},
};

#[derive(Default)]
pub(super) struct MetadataGates {
    entries: Mutex<HashMap<PathBuf, Weak<RwLock<()>>>>,
}

impl MetadataGates {
    pub fn get(self: &Arc<Self>, path: PathBuf) -> Result<MetadataGate, StorageError> {
        // No filesystem work or waiting on a file gate occurs under this index.
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| poisoned("locate metadata gate", &path))?;
        let lock = entries
            .get(&path)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let lock = Arc::new(RwLock::new(()));
                entries.insert(path.clone(), Arc::downgrade(&lock));
                lock
            });
        Ok(MetadataGate {
            index: self.clone(),
            path,
            lock: Some(lock),
        })
    }
}

pub(super) struct MetadataGate {
    index: Arc<MetadataGates>,
    path: PathBuf,
    // Taken only by Drop while holding the index, so concurrent final drops
    // cannot both observe another reference and strand a dead Weak entry.
    lock: Option<Arc<RwLock<()>>>,
}

impl MetadataGate {
    fn lock(&self) -> &RwLock<()> {
        self.lock
            .as_deref()
            .expect("a live metadata gate has its lock")
    }

    pub fn read(&self) -> Result<RwLockReadGuard<'_, ()>, StorageError> {
        self.lock()
            .read()
            .map_err(|_| poisoned("lock metadata snapshot", &self.path))
    }

    pub fn write(&self) -> Result<RwLockWriteGuard<'_, ()>, StorageError> {
        self.lock()
            .write()
            .map_err(|_| poisoned("lock metadata publication", &self.path))
    }

    #[cfg(test)]
    pub fn try_write(&self) -> std::sync::TryLockResult<RwLockWriteGuard<'_, ()>> {
        self.lock().try_write()
    }
}

impl Drop for MetadataGate {
    fn drop(&mut self) {
        // Keep the index bounded by live operations, not every historical path.
        // Acquisition and retiring this lease's strong reference occur under
        // the same index lock. Guards borrow the lease and have already ended.
        if let Ok(mut entries) = self.index.entries.lock() {
            drop(self.lock.take());
            if entries
                .get(&self.path)
                .is_some_and(|gate| gate.strong_count() == 0)
            {
                entries.remove(&self.path);
            }
        }
    }
}

fn poisoned(operation: &'static str, path: &Path) -> StorageError {
    io_error(
        operation,
        path,
        std::io::Error::other("metadata coordination was poisoned"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_gates_share_only_live_matching_paths() {
        let index = Arc::new(MetadataGates::default());
        let first = index.get("agent.json".into()).unwrap();
        let same = index.get("agent.json".into()).unwrap();
        let other = index.get("session.json".into()).unwrap();
        assert!(Arc::ptr_eq(
            first.lock.as_ref().unwrap(),
            same.lock.as_ref().unwrap()
        ));
        assert!(!Arc::ptr_eq(
            first.lock.as_ref().unwrap(),
            other.lock.as_ref().unwrap()
        ));
        drop(first);
        assert_eq!(index.entries.lock().unwrap().len(), 2);
        drop(same);
        assert_eq!(index.entries.lock().unwrap().len(), 1);
        drop(other);
        assert!(index.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn queued_metadata_operations_keep_one_gate_until_the_last_lease_retires() {
        let index = Arc::new(MetadataGates::default());
        let first = index.get("object.json".into()).unwrap();
        let snapshot = first.read().unwrap();
        let waiting = index.get("object.json".into()).unwrap();
        let (ready, received) = std::sync::mpsc::channel();
        let (finish, finished) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let _publication = waiting.write().unwrap();
            finished
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        });
        received
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let later = index.get("object.json".into()).unwrap();
        assert!(Arc::ptr_eq(
            first.lock.as_ref().unwrap(),
            later.lock.as_ref().unwrap()
        ));
        drop(snapshot);
        drop(first);
        assert_eq!(index.entries.lock().unwrap().len(), 1);
        finish.send(()).unwrap();
        writer.join().unwrap();
        assert_eq!(index.entries.lock().unwrap().len(), 1);
        drop(later);
        assert!(index.entries.lock().unwrap().is_empty());
        let fresh = index.get("object.json".into()).unwrap();
        assert!(fresh.try_write().is_ok());
    }

    #[test]
    fn simultaneous_final_metadata_gate_drops_retire_every_path() {
        let index = Arc::new(MetadataGates::default());
        let start = std::sync::Barrier::new(2);
        for number in 0..256 {
            let path = format!("session-{number}/session.json");
            let first = index.get(path.clone().into()).unwrap();
            let last = index.get(path.into()).unwrap();
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    start.wait();
                    drop(first);
                });
                start.wait();
                drop(last);
            });
            assert!(index.entries.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn poisoned_metadata_coordination_returns_an_error_without_recovery() {
        let index = Arc::new(MetadataGates::default());
        let path = PathBuf::from("session.json");
        let retained = index.get(path.clone()).unwrap();
        let writer = index.get(path.clone()).unwrap();
        assert!(
            std::thread::spawn(move || {
                let _publication = writer.write().unwrap();
                panic!("injected metadata writer panic");
            })
            .join()
            .is_err()
        );
        assert!(matches!(retained.read(), Err(StorageError::Io { .. })));
        assert!(matches!(retained.write(), Err(StorageError::Io { .. })));
        drop(retained);
        assert!(index.entries.lock().unwrap().is_empty());

        let poisoned = index.clone();
        assert!(
            std::thread::spawn(move || {
                let _index = poisoned.entries.lock().unwrap();
                panic!("injected metadata index panic");
            })
            .join()
            .is_err()
        );
        assert!(matches!(index.get(path), Err(StorageError::Io { .. })));
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn review_parallel_gate_reacquisition_never_splits_one_live_path() {
        let index = Arc::new(MetadataGates::default());
        let active_writers = AtomicUsize::new(0);
        let start = std::sync::Barrier::new(8);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    start.wait();
                    for _ in 0..512 {
                        let gate = index.get("same-object.json".into()).unwrap();
                        let _publication = gate.write().unwrap();
                        assert_eq!(active_writers.fetch_add(1, Ordering::SeqCst), 0);
                        std::thread::yield_now();
                        assert_eq!(active_writers.fetch_sub(1, Ordering::SeqCst), 1);
                    }
                });
            }
        });
        assert_eq!(active_writers.load(Ordering::SeqCst), 0);
        assert!(index.entries.lock().unwrap().is_empty());
    }
}
