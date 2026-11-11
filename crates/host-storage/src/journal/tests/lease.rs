use super::*;
#[cfg(unix)]
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

#[cfg(windows)]
#[test]
fn windows_unrelated_spawn_does_not_extend_history_or_root_leases() {
    let (temp, session) = fixture();
    let path = temp.path().join("host");
    let mut journal = session.open_journal().unwrap();
    let mut child = crate::lease_tests::ChildProbe::unrelated();
    journal.accept(vec![entry("actual owner")]).unwrap();
    held(&session);
    journal.persist_pending().unwrap();
    drop(journal);
    assert!(child.is_alive());
    assert_eq!(session.read_history().unwrap().records.len(), 1);
    let journal = session.open_journal().unwrap();
    held(&session);
    drop(session);
    assert!(matches!(
        DataRoot::acquire(&path),
        Err(StorageError::RootInUse)
    ));
    drop(journal);
    assert!(child.is_alive());
    let _next = DataRoot::acquire(&path).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_history_leases_exclude_write_handles_and_writable_file_mappings() {
    use winsafe::{HFILE, co};
    let (_temp, session) = fixture();
    let path = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry("nonempty mapped file")]).unwrap();
    journal.persist_pending().unwrap();
    let rejected = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect_err("live writer must reject other write handles");
    assert_eq!(rejected.raw_os_error(), Some(32));
    drop(journal);

    let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    held(&session);
    drop(writer);

    let (file, _) = HFILE::CreateFile(
        path.to_str().unwrap(),
        co::GENERIC::READ | co::GENERIC::WRITE,
        Some(co::FILE_SHARE::READ | co::FILE_SHARE::WRITE | co::FILE_SHARE::DELETE),
        None,
        co::DISPOSITION::OPEN_EXISTING,
        co::FILE_ATTRIBUTE::NORMAL,
        None,
        None,
        None,
    )
    .unwrap();
    let mapping = file
        .CreateFileMapping(None, co::PAGE::READWRITE, None, None, None)
        .unwrap();
    let view = mapping.MapViewOfFile(co::FILE_MAP::WRITE, 0, None).unwrap();
    // Only the real writable mapping remains; this cannot be explained by a
    // forgotten ordinary write handle or the mandatory LockFileEx byte range.
    drop(file);
    held(&session);
    drop(view);
    drop(mapping);
    assert_eq!(session.read_history().unwrap().records.len(), 1);
    let mut reopened = session.open_journal().unwrap();
    reopened
        .accept(vec![entry("after mapping closed")])
        .unwrap();
    reopened.persist_pending().unwrap();
    drop(reopened);
    assert_eq!(session.read_history().unwrap().records.len(), 2);
}

#[test]
#[cfg(unix)]
fn writer_releases_only_after_actual_owner_finishes_with_inherited_descriptor() {
    let (_temp, session) = fixture();
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
    let bytes = read_owned(&journal);
    assert!(!bytes.is_empty());
    assert_eq!(journal.reconcile_pending().unwrap().range, range);
    held(&session); // Save confirmation does not release the live writer.
    assert_eq!(read_owned(&journal), bytes);
    drop(journal);
    let loaded = session.read_history();
    assert!(child.is_alive());
    assert_eq!(loaded.unwrap().records.len(), 1);
    let _next = session.open_journal().unwrap();
    drop(child);
    held(&session); // The old inherited descriptor cannot release a new lease.
}

#[test]
#[cfg(unix)]
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
#[cfg(unix)]
fn failed_reader_validation_releases_shared_lease_without_repair() {
    failed_validation(false);
}

#[test]
#[cfg(unix)]
fn failed_writer_validation_releases_exclusive_lease_without_repair() {
    failed_validation(true);
}

#[cfg(unix)]
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
#[cfg(unix)]
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

#[test]
fn releasing_one_reader_does_not_unlock_another_active_reader() {
    use crate::lease_tests::on_next_lock;
    use std::{sync::mpsc, time::Duration};
    let (temp, session) = fixture();
    let path = temp.path().join("host");
    let history = session.canonical_path().join("history.jsonl");
    let reader = session.clone();
    let (ready, reading) = mpsc::channel();
    let (release, proceed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        on_next_lock(move |_| {
            ready.send(()).unwrap();
            proceed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        reader.read_history()
    });
    reading.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(session.read_history().unwrap().records.is_empty());
    assert!(matches!(
        session.open_journal(),
        Err(StorageError::HistoryInUse)
    ));
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
    assert!(worker.join().unwrap().unwrap().records.is_empty());
    let _next = DataRoot::acquire(&path).unwrap();
    let next_history = File::open(history).unwrap();
    next_history.try_lock().unwrap();
    next_history.unlock().unwrap();
}

#[test]
fn failed_deletion_releases_its_history_lease_without_deleting_original_bytes() {
    use crate::lease_tests::on_next_lock;
    use std::{cell::RefCell, rc::Rc};
    let (_temp, session) = fixture();
    let mut record = session.load().unwrap();
    record.data.closed = Some(crate::SessionClosure {
        reason: "explicit".into(),
    });
    session.save(&record).unwrap();
    let path = session.canonical_path().join("history.jsonl");
    let original = std::fs::read(&path).unwrap();
    let contender = File::open(&path).unwrap();
    let retained = Rc::new(RefCell::new(None));
    let child = retained.clone();
    let changed = path.clone();
    on_next_lock(move |file| {
        #[cfg(unix)]
        let process = InheritedDescriptor::new(file);
        #[cfg(windows)]
        let process = {
            let _ = file;
            crate::lease_tests::ChildProbe::unrelated()
        };
        *child.borrow_mut() = Some(process);
        // External permission mutation makes the next guarded removal fail.
        crate::test_support::set_mode(&changed, 0o666);
    });
    let result = session.delete_closed();
    let mut child = retained.borrow_mut().take().unwrap();
    assert!(matches!(result, Err(StorageError::UnsafePath { .. })));
    assert!(child.is_alive());
    assert_eq!(std::fs::read(path).unwrap(), original);
    contender.try_lock().unwrap();
    contender.unlock().unwrap();
}
