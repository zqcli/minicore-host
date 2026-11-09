use super::*;
use crate::{AgentData, AgentRecord, DataRoot, HostRecord, SessionData, SessionRecord};
use minicore_host_protocol::{AgentConfig, AgentId, HostId, SessionConfig, StorageMode};
use minicore_runtime::{BoundedText, history::SummaryHistory};

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
            content: BoundedText::new(text).unwrap(),
        },
    }
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
    let path = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    let range = journal
        .accept(vec![entry("completed but unsynced")])
        .unwrap();
    journal.failure = Some(TestFailure::FileSync);
    assert!(journal.persist_pending().is_err());
    let bytes_before = std::fs::read(&path).unwrap();
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
    assert_eq!(std::fs::read(&path).unwrap(), bytes_before);
    assert_eq!(journal.positions().persisted, Some(1));
}
#[test]
fn partial_write_stays_blocked_and_reopen_never_truncates() {
    let (_temp, session) = fixture();
    let path = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry("first confirmed")]).unwrap();
    journal.persist_pending().unwrap();
    let original = std::fs::read(&path).unwrap();
    journal.accept(vec![entry("second range")]).unwrap();
    journal.failure = Some(TestFailure::WritePrefix(17));
    assert!(journal.persist_pending().is_err());
    let partial = std::fs::read(&path).unwrap();
    assert!(partial.starts_with(&original));
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), partial);
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
    let path = session.canonical_path().join("history.jsonl");
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry("original")]).unwrap();
    journal.persist_pending().unwrap();
    journal.accept(vec![entry("next")]).unwrap();
    journal.failure = Some(TestFailure::FileSync);
    assert!(journal.persist_pending().is_err());
    let original = std::fs::read_to_string(&path).unwrap();
    let changed = original.replace("original", "altered!");
    assert_eq!(changed.len(), original.len());
    std::fs::write(&path, &changed).unwrap();
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);
    assert_eq!(journal.positions().persisted, Some(1));
}
