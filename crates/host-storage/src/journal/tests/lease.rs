use super::*;
use crate::lease_tests::{InheritedDescriptor, inherit_next_lock, inherited};

fn held(session: &SessionStore) {
    assert!(matches!(
        session.open_journal(),
        Err(StorageError::HistoryInUse)
    ));
    assert!(matches!(
        session.read_history(),
        Err(StorageError::HistoryInUse)
    ));
}

#[test]
fn writer_releases_only_after_actual_owner_finishes_with_inherited_descriptor() {
    let (_temp, session) = fixture();
    let path = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    let Backend::Persistent(backend) = &journal.backend else {
        unreachable!()
    };
    let mut child = InheritedDescriptor::new(&backend.file);
    let range = journal.accept(vec![entry("owned range")]).unwrap();
    held(&session);
    journal.failure = Some(TestFailure::FileSync);
    assert!(journal.persist_pending().is_err());
    held(&session);
    assert_eq!(journal.positions().persisted, Some(0));
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(journal.reconcile_pending().unwrap().range, range);
    held(&session); // Save confirmation does not release the live writer.
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(journal);
    let loaded = session.read_history();
    assert!(child.is_alive());
    assert_eq!(loaded.unwrap().records.len(), 1);
    let _next = session.open_journal().unwrap();
    drop(child);
    held(&session); // The old inherited descriptor cannot release a new lease.
}

#[test]
fn completed_reader_releases_shared_lease_with_inherited_descriptor() {
    let (_temp, session) = fixture();
    inherit_next_lock();
    let loaded = session.read_history();
    let mut child = inherited();
    assert_eq!(loaded.unwrap().records.len(), 0);
    let next = session.open_journal();
    assert!(child.is_alive());
    let _next = next.unwrap();
    drop(child);
    held(&session);
}

#[test]
fn failed_reader_validation_releases_shared_lease_without_repair() {
    failed_validation(false);
}

#[test]
fn failed_writer_validation_releases_exclusive_lease_without_repair() {
    failed_validation(true);
}

fn failed_validation(writer: bool) {
    let (_temp, session) = fixture();
    let path = session.canonical_path().join("history.jsonl");
    let bad = b"{broken\n";
    std::fs::write(&path, bad).unwrap();
    inherit_next_lock();
    let result = if writer {
        session.open_journal().map(|_| ())
    } else {
        session.read_history().map(|_| ())
    };
    let mut child = inherited();
    assert!(matches!(result, Err(StorageError::InvalidHistory { .. })));
    // Must reach the same format error, not become blocked on a stale lease.
    assert!(matches!(
        session.open_journal(),
        Err(StorageError::InvalidHistory { .. })
    ));
    assert!(child.is_alive());
    assert_eq!(std::fs::read(path).unwrap(), bad);
}

#[test]
fn completed_deletion_releases_its_history_lease_with_inherited_descriptor() {
    let (_temp, session) = fixture();
    let mut record = session.load().unwrap();
    record.data.closed = Some(crate::SessionClosure {
        reason: "explicit".into(),
    });
    session.save(&record).unwrap();
    let path = session.canonical_path();
    // Independently open the old inode, so its unlink does not hide a stale lock.
    let old_history = File::open(path.join("history.jsonl")).unwrap();
    inherit_next_lock();
    session.delete_closed().unwrap();
    let mut child = inherited();
    assert!(!path.exists());
    assert!(child.is_alive());
    old_history.try_lock().unwrap();
    old_history.unlock().unwrap();
}

#[test]
fn accepted_worker_retains_root_and_history_after_callers_drop() {
    use std::{sync::mpsc, time::Duration};
    let (temp, session) = fixture();
    let path = temp.path().join("host");
    let history = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    let session_id = session.id().clone();
    let range = journal.accept(vec![entry("accepted save")]).unwrap();
    let (ready, started) = mpsc::channel();
    let (release, proceed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        proceed.recv_timeout(Duration::from_secs(5)).unwrap();
        journal.persist_pending().unwrap()
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(session);
    assert!(matches!(
        DataRoot::acquire(&path),
        Err(StorageError::RootInUse)
    ));
    assert!(matches!(
        File::open(&history).unwrap().try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    release.send(()).unwrap();
    assert_eq!(worker.join().unwrap().range, range);
    let _next = DataRoot::acquire(&path).unwrap();
    let bytes = std::fs::read(history).unwrap();
    assert_eq!(
        decode_history(&bytes, &session_id).unwrap().0.records.len(),
        1
    );
}
