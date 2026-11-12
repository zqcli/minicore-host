use super::*;
use crate::{AgentData, AgentRecord, DataRoot, HostRecord, SessionData, SessionRecord};
use minicore_host_protocol::{AgentConfig, AgentId, HostId, SessionConfig, StorageMode};
use minicore_host_runtime::{Text, history::SummaryHistory};

fn fixture() -> (tempfile::TempDir, SessionStore) {
    let temp = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(temp.path().join("host")).unwrap();
    let host = root.host(HostId::new().unwrap());
    host.save_host(&HostRecord::new(
        host.id().clone(),
        serde_json::json!({"test":true}),
        1,
    ))
    .unwrap();
    let agent = host.agent(AgentId::new().unwrap(), StorageMode::Persistent);
    agent
        .save(&AgentRecord::new(
            agent.id().clone(),
            host.id().clone(),
            AgentData {
                name: "test".into(),
                mode: StorageMode::Persistent,
                config: AgentConfig::default(),
                config_revision: 1,
                default_session_id: None,
            },
        ))
        .unwrap();
    let session = agent
        .session(SessionId::new().unwrap(), StorageMode::Persistent)
        .unwrap();
    session
        .save(&SessionRecord::new(
            session.id().clone(),
            agent.id().clone(),
            SessionData {
                title: None,
                mode: StorageMode::Persistent,
                agent_config_revision: 1,
                config_revision: 1,
                effective_config: SessionConfig::default(),
                parent: None,
                closed: None,
            },
        ))
        .unwrap();
    (temp, session)
}
fn entry(text: &str) -> HistoryEntry {
    HistoryEntry::Summary {
        summary: SummaryHistory {
            content: Text::new(text),
        },
    }
}

// Windows byte-range locks prevent unrelated handles from reading a writer's
// file. Inspect the actual owned file without releasing the lease or disturbing
// its cursor; accepted/persisted assertions still observe real stored bytes.
fn read_owned(journal: &SessionJournal) -> Vec<u8> {
    let Backend::Persistent(backend) = &journal.backend else {
        panic!("persistent fixture")
    };
    let mut file = &*backend.file;
    let position = file.stream_position().unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    file.seek(SeekFrom::Start(position)).unwrap();
    bytes
}

fn corrupt_owned(journal: &mut SessionJournal, bytes: &[u8]) {
    // This is an explicit corruption fixture. Windows cannot use a second
    // write handle through the mandatory lease, so mutate through its owner.
    journal.overwrite_owned_history_for_test(bytes).unwrap();
}

#[test]
fn failed_zero_byte_write_reconciles_original_range_once() {
    let (_temp, session) = fixture();
    let mut journal = session.open_journal().unwrap();
    let range = journal.accept(vec![entry("accepted")]).unwrap();
    journal.failure = Some(TestFailure::WritePrefix(0));
    assert!(journal.persist_pending().is_err());
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 1,
            persisted: Some(0)
        }
    );
    assert_eq!(journal.pending_range(), Some(range));
    assert!(matches!(
        journal.persist_pending(),
        Err(StorageError::ReconciliationRequired)
    ));
    assert!(matches!(
        journal.accept(vec![entry("must not sneak in")]),
        Err(StorageError::ReconciliationRequired)
    ));
    assert_eq!(journal.reconcile_pending().unwrap().range, range);
    drop(journal);
    assert_eq!(session.read_history().unwrap().records.len(), 1);
}
#[test]
fn complete_write_failed_sync_is_readable_but_not_confirmed_then_synced_without_duplicate() {
    let (_temp, session) = fixture();
    let mut journal = session.open_journal().unwrap();
    let range = journal
        .accept(vec![entry("completed but unsynced")])
        .unwrap();
    journal.failure = Some(TestFailure::FileSync);
    assert!(journal.persist_pending().is_err());
    let bytes_before = read_owned(&journal);
    assert!(!bytes_before.is_empty());
    assert_eq!(
        decode_history(&bytes_before, session.id())
            .unwrap()
            .0
            .records
            .len(),
        1
    );
    assert_eq!(journal.positions().persisted, Some(0));
    assert_eq!(journal.reconcile_pending().unwrap().range, range);
    assert_eq!(read_owned(&journal), bytes_before);
    assert_eq!(journal.positions().persisted, Some(1));
}
#[test]
fn partial_write_stays_blocked_and_reopen_never_truncates() {
    let (_temp, session) = fixture();
    let path = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry("first confirmed")]).unwrap();
    journal.persist_pending().unwrap();
    let original = read_owned(&journal);
    journal.accept(vec![entry("second range")]).unwrap();
    journal.failure = Some(TestFailure::WritePrefix(17));
    assert!(journal.persist_pending().is_err());
    let partial = read_owned(&journal);
    assert!(partial.starts_with(&original));
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(read_owned(&journal), partial);
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 2,
            persisted: Some(1)
        }
    );
    drop(journal);
    assert!(session.open_journal().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), partial);
}
#[test]
fn reconciliation_rejects_changed_prefix_even_when_suffix_matches() {
    let (_temp, session) = fixture();
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry("original")]).unwrap();
    journal.persist_pending().unwrap();
    journal.accept(vec![entry("next")]).unwrap();
    journal.failure = Some(TestFailure::FileSync);
    assert!(journal.persist_pending().is_err());
    let original = String::from_utf8(read_owned(&journal)).unwrap();
    let changed = original.replace("original", "altered!");
    assert_eq!(changed.len(), original.len());
    corrupt_owned(&mut journal, changed.as_bytes());
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(String::from_utf8(read_owned(&journal)).unwrap(), changed);
    assert_eq!(journal.positions().persisted, Some(1));
}

#[test]
fn directory_sync_failure_does_not_confirm_a_readable_fully_written_range() {
    let (_temp, session) = fixture();
    let mut journal = session.open_journal().unwrap();
    let range = journal.accept(vec![entry("file sync succeeded")]).unwrap();
    journal.failure = Some(TestFailure::DirectorySync);
    assert!(matches!(
        journal.persist_pending(),
        Err(StorageError::Io {
            operation: "injected history directory sync failure",
            ..
        })
    ));
    let before = read_owned(&journal);
    assert!(!before.is_empty());
    assert_eq!(journal.positions().persisted, Some(0));
    assert_eq!(journal.reconcile_pending().unwrap().range, range);
    assert_eq!(read_owned(&journal), before);
    assert_eq!(journal.positions().persisted, Some(1));
}

#[test]
fn file_replacement_during_sync_cannot_be_confirmed_by_append_or_reconciliation() {
    for reconcile in [false, true] {
        let (_temp, session) = fixture();
        let path = session.canonical_path().join("history.jsonl");
        let mut journal = session.open_journal().unwrap();
        journal.accept(vec![entry("accepted")]).unwrap();
        if reconcile {
            journal.failure = Some(TestFailure::FileSync);
            assert!(journal.persist_pending().is_err());
        }
        journal.failure = Some(TestFailure::ReplaceFileAfterSync);
        let result = if reconcile {
            journal.reconcile_pending()
        } else {
            journal.persist_pending()
        };
        assert!(matches!(result, Err(StorageError::UncertainWrite)));
        assert_eq!(journal.positions().persisted, Some(0));
        assert_eq!(std::fs::read(path).unwrap(), b"replacement");
    }
}
#[test]
fn directory_replacement_during_sync_cannot_be_confirmed() {
    let (_temp, session) = fixture();
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry("accepted")]).unwrap();
    journal.failure = Some(TestFailure::ReplaceDirectoryAfterSync);
    #[cfg(unix)]
    assert!(matches!(
        journal.persist_pending(),
        Err(StorageError::UncertainWrite)
    ));
    #[cfg(windows)]
    {
        // The hook must observe the OS rejecting the directory replacement.
        journal.persist_pending().unwrap();
        assert_eq!(journal.positions().persisted, Some(1));
        assert_eq!(
            decode_history(&read_owned(&journal), session.id())
                .unwrap()
                .0
                .records
                .len(),
            1
        );
    }
    #[cfg(unix)]
    {
        assert_eq!(journal.positions().persisted, Some(0));
        assert_eq!(
            std::fs::read(session.canonical_path().join("history.jsonl")).unwrap(),
            b"replacement"
        );
    }
}

mod compaction;

mod lease;
