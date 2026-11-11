use super::*;
use crate::CompactionProjection;
use minicore_host_protocol::TurnId;
use minicore_runtime::LoopId;
use minicore_runtime::{
    UserInput,
    history::{UserHistory, UserMessageKind},
};

fn marker_fixture() -> (
    tempfile::TempDir,
    SessionStore,
    SessionJournal,
    Vec<u8>,
    SaveRange,
) {
    let (temp, session) = fixture();
    let mut journal = session.open_journal().unwrap();
    let inputs = ["original", "retained"]
        .into_iter()
        .map(|text| HistoryEntry::Input {
            turn_id: TurnId::new().unwrap(),
            message_id: minicore_host_protocol::MessageId::new().unwrap(),
            source: minicore_host_protocol::MessageSource::User,
            mode: minicore_host_protocol::MessageMode::FollowUp,
            in_reply_to: None,
            user: UserHistory {
                loop_id: LoopId::new().unwrap(),
                kind: UserMessageKind::Prompt,
                input: UserInput::text(text).unwrap(),
            },
        })
        .collect();
    journal.accept(inputs).unwrap();
    journal.persist_pending().unwrap();
    let prefix = read_owned(&journal);
    let range = journal
        .accept(vec![
            CompactionProjection::new(
                2,
                1,
                SummaryHistory {
                    content: Text::new("actual summary fixture"),
                },
            )
            .into_entry(),
        ])
        .unwrap();
    (temp, session, journal, prefix, range)
}

#[test]
fn compaction_zero_write_continues_only_original_marker_and_confirms_once() {
    let (_temp, session, mut journal, prefix, range) = marker_fixture();
    assert_eq!(range, SaveRange { start: 2, end: 3 });
    journal.failure = Some(TestFailure::WritePrefix(0));
    assert!(journal.persist_pending().is_err());
    assert_eq!(read_owned(&journal), prefix);
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 3,
            persisted: Some(2)
        }
    );
    assert_eq!(journal.pending_range(), Some(range));
    assert!(matches!(
        journal.accept(vec![entry("must remain blocked")]),
        Err(StorageError::ReconciliationRequired)
    ));
    assert_eq!(
        journal.reconcile_pending().unwrap(),
        SaveConfirmation {
            range,
            persistence: PersistenceState::Persisted
        }
    );
    let once = read_owned(&journal);
    assert!(once.starts_with(&prefix));
    assert_eq!(
        journal.reconcile_pending().unwrap().range,
        SaveRange { start: 3, end: 3 }
    );
    assert_eq!(read_owned(&journal), once);
    drop(journal);
    let loaded = session.read_history().unwrap();
    assert_eq!(loaded.records.len(), 3);
    assert_eq!(loaded.runtime_history().len(), 2);
    assert_eq!(loaded.model_history().unwrap().len(), 2);
}

fn full_write_sync_failure(failure: TestFailure) {
    let (_temp, session, mut journal, prefix, range) = marker_fixture();
    journal.failure = Some(failure);
    assert!(journal.persist_pending().is_err());
    let written = read_owned(&journal);
    assert!(written.starts_with(&prefix));
    let readable = decode_history(&written, session.id()).unwrap().0;
    assert_eq!(readable.compaction_projection().unwrap().covered_items, 1);
    assert_eq!(journal.positions().persisted, Some(2));
    assert_eq!(journal.pending_range(), Some(range));
    assert!(matches!(
        journal.persist_pending(),
        Err(StorageError::ReconciliationRequired)
    ));
    assert_eq!(journal.reconcile_pending().unwrap().range, range);
    assert_eq!(read_owned(&journal), written);
    assert_eq!(journal.positions().persisted, Some(3));
    assert_eq!(
        journal.reconcile_pending().unwrap().range,
        SaveRange { start: 3, end: 3 }
    );
    assert_eq!(read_owned(&journal), written);
    assert!(
        journal
            .accept(vec![
                CompactionProjection::new(
                    2,
                    1,
                    SummaryHistory {
                        content: Text::new("must not replay")
                    }
                )
                .into_entry()
            ])
            .is_err()
    );
    assert_eq!(read_owned(&journal), written);
}

#[test]
fn compaction_full_write_file_sync_failure_reconciles_without_duplicate() {
    full_write_sync_failure(TestFailure::FileSync);
}

#[test]
fn compaction_full_write_directory_sync_failure_reconciles_without_duplicate() {
    full_write_sync_failure(TestFailure::DirectorySync);
}

#[test]
fn compaction_partial_write_stays_blocked_and_preserves_every_original_byte() {
    let (_temp, session, mut journal, prefix, range) = marker_fixture();
    let path = session.canonical_path().join("history.jsonl");
    journal.failure = Some(TestFailure::WritePrefix(17));
    assert!(journal.persist_pending().is_err());
    let partial = read_owned(&journal);
    assert!(partial.starts_with(&prefix));
    assert_eq!(partial.len(), prefix.len() + 17);
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(journal.pending_range(), Some(range));
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 3,
            persisted: Some(2)
        }
    );
    assert_eq!(read_owned(&journal), partial);
    drop(journal);
    assert!(session.read_history().is_err());
    assert!(session.open_journal().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), partial);
}

#[test]
fn compaction_reconcile_rejects_changed_source_prefix_without_repair() {
    let (_temp, _session, mut journal, _prefix, range) = marker_fixture();
    journal.failure = Some(TestFailure::FileSync);
    assert!(journal.persist_pending().is_err());
    let written = String::from_utf8(read_owned(&journal)).unwrap();
    let changed = written.replace("original", "modified");
    assert_eq!(changed.len(), written.len());
    corrupt_owned(&mut journal, changed.as_bytes());
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(journal.pending_range(), Some(range));
    assert_eq!(journal.positions().persisted, Some(2));
    assert_eq!(String::from_utf8(read_owned(&journal)).unwrap(), changed);
}
