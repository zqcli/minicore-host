use minicore_host_protocol::{
    AgentConfig, AgentId, HostId, MessageId, MessageMode, MessageSource, PersistenceState,
    SessionConfig, SessionId, StorageMode, TurnId, TurnOutcome,
};
use minicore_host_storage::*;
use minicore_runtime::{
    LoopId, UserInput,
    history::{AssistantHistory, HistoryItem, UserHistory, UserMessageKind},
    model::{AssistantPart, ModelFinishReason, ReasoningPreference, Usage},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};
use tempfile::TempDir;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    label: String,
}
struct Fixture {
    temp: TempDir,
    root: DataRoot,
    host: HostStore,
    agent: AgentStore,
    session: SessionStore,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = DataRoot::acquire(temp.path().join("host")).unwrap();
        let host = root.host(HostId::new().unwrap());
        host.save_host(&HostRecord::new(
            host.id().clone(),
            HostConfig {
                label: "test".into(),
            },
            1,
        ))
        .unwrap();
        let agent = host.agent(AgentId::new().unwrap(), StorageMode::Persistent);
        agent
            .save(&AgentRecord::new(
                agent.id().clone(),
                host.id().clone(),
                AgentData {
                    name: "Agent".into(),
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
        Self {
            temp,
            root,
            host,
            agent,
            session,
        }
    }
    fn history_path(&self) -> std::path::PathBuf {
        self.session.canonical_path().join("history.jsonl")
    }
}
fn input(turn_id: TurnId, loop_id: LoopId) -> HistoryEntry {
    HistoryEntry::Input {
        turn_id,
        message_id: MessageId::new().unwrap(),
        source: MessageSource::User,
        mode: MessageMode::FollowUp,
        in_reply_to: None,
        user: UserHistory {
            loop_id,
            kind: UserMessageKind::Prompt,
            input: UserInput::text("real input").unwrap(),
        },
    }
}
fn response(turn_id: TurnId, loop_id: LoopId) -> HistoryEntry {
    HistoryEntry::Runtime {
        turn_id,
        response_index: 0,
        item: HistoryItem::Assistant(AssistantHistory {
            loop_id,
            request_index: 0,
            model: "test-model".parse().unwrap(),
            reasoning: ReasoningPreference::Auto,
            content: vec![AssistantPart::Text("real response".into())],
            provider_replay: None,
            finish_reason: ModelFinishReason::Stop,
            usage: Usage::default(),
        }),
    }
}
fn record_bytes(session: &SessionStore, entries: Vec<HistoryEntry>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (position, entry) in entries.into_iter().enumerate() {
        serde_json::to_writer(
            &mut bytes,
            &HistoryRecord::new(session.id().clone(), position as u64, entry),
        )
        .unwrap();
        bytes.push(b'\n');
    }
    bytes
}

#[test]
fn lock_is_exclusive_for_aliases_and_releases_after_last_child() {
    let f = Fixture::new();
    assert!(matches!(
        DataRoot::acquire(f.root.canonical_path()),
        Err(StorageError::RootInUse)
    ));
    let alias = f.temp.path().join("alias");
    symlink(f.root.canonical_path(), &alias).unwrap();
    assert!(matches!(
        DataRoot::acquire(&alias),
        Err(StorageError::RootInUse)
    ));
    let path = f.root.canonical_path().to_path_buf();
    let journal = f.session.open_journal().unwrap();
    drop(f.session);
    drop(f.agent);
    drop(f.host);
    drop(f.root);
    assert!(matches!(
        DataRoot::acquire(&path),
        Err(StorageError::RootInUse)
    ));
    drop(journal);
    DataRoot::acquire(&path).unwrap();
}
#[test]
fn independent_roots_can_run_together_and_stale_markers_do_not_lock() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::write(a.join(".host.lock"), "stale").unwrap();
    let _first = DataRoot::acquire(&a).unwrap();
    let _second = DataRoot::acquire(&b).unwrap();
}
#[test]
fn lock_probe() {
    let Ok(path) = std::env::var("MINICORE_STORAGE_LOCK_PROBE") else {
        return;
    };
    if std::env::var("MINICORE_STORAGE_LOCK_PROBE_MODE").unwrap() == "blocked" {
        assert!(matches!(
            DataRoot::acquire(path),
            Err(StorageError::RootInUse)
        ));
    } else {
        let _guard = DataRoot::acquire(path).unwrap();
        // Simulate process exit without running Rust destructors. The OS must
        // release the actual lock even though no cleanup marker can be removed.
        std::process::exit(0);
    }
}
#[test]
fn cross_process_lock_and_abrupt_exit_release() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("host");
    let root = DataRoot::acquire(&path).unwrap();
    let executable = std::env::current_exe().unwrap();
    let child = |mode| {
        Command::new(&executable)
            .args(["--exact", "lock_probe", "--nocapture"])
            .env("MINICORE_STORAGE_LOCK_PROBE", &path)
            .env("MINICORE_STORAGE_LOCK_PROBE_MODE", mode)
            .status()
            .unwrap()
    };
    assert!(child("blocked").success());
    drop(root);
    assert!(child("crash").success());
    DataRoot::acquire(&path).unwrap();
}
#[test]
fn stable_identity_discovery_and_atomic_object_roundtrip() {
    let f = Fixture::new();
    let id = f.host.id().clone();
    let root_path = f.root.canonical_path().to_owned();
    let bytes = fs::read(root_path.join("host.json")).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert!(value.get("host_incarnation").is_none());
    assert!(value["data"].get("id").is_none());
    assert_eq!(f.root.load_host::<HostConfig>().unwrap().unwrap().id, id);
    assert_eq!(f.host.list_agents().unwrap(), vec![f.agent.id().clone()]);
    assert_eq!(
        f.agent.list_sessions().unwrap(),
        vec![f.session.id().clone()]
    );
    let mut record = f.agent.load().unwrap();
    record.data.name = "renamed".into();
    record.data.config_revision += 1;
    f.agent.save(&record).unwrap();
    assert_eq!(f.agent.load().unwrap(), record);
    assert!(f.session.canonical_path().exists());
    drop(f.session);
    drop(f.agent);
    drop(f.host);
    drop(f.root);
    let root = DataRoot::acquire(root_path).unwrap();
    assert_eq!(root.load_host::<HostConfig>().unwrap().unwrap().id, id);
}
#[test]
fn unsupported_objects_and_ownership_mismatch_are_not_rewritten() {
    let f = Fixture::new();
    let path = f.session.canonical_path().join("session.json");
    let original = fs::read(&path).unwrap();
    for (field, value) in [
        ("schema_version", serde_json::json!(2)),
        ("agent_id", serde_json::json!(AgentId::new().unwrap())),
        ("id", serde_json::json!(SessionId::new().unwrap())),
    ] {
        let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
        json[field] = value;
        let bad = serde_json::to_vec(&json).unwrap();
        fs::write(&path, &bad).unwrap();
        assert!(f.session.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), bad);
    }
    fs::write(&path, &original).unwrap();
    assert!(f.session.load().is_ok());
}
#[test]
fn memory_records_and_journal_create_no_files() {
    let f = Fixture::new();
    let agent_id = AgentId::new().unwrap();
    let agent = f.host.agent(agent_id.clone(), StorageMode::Memory);
    let saved = agent
        .save(&AgentRecord::new(
            agent_id.clone(),
            f.host.id().clone(),
            AgentData {
                name: "memory".into(),
                mode: StorageMode::Memory,
                config: AgentConfig::default(),
                config_revision: 1,
                default_session_id: None,
            },
        ))
        .unwrap();
    assert_eq!(saved.persistence, PersistenceState::NotApplicable);
    assert!(matches!(
        agent.session(SessionId::new().unwrap(), StorageMode::Persistent),
        Err(StorageError::InvalidStorageMode)
    ));
    let session = agent
        .session(SessionId::new().unwrap(), StorageMode::Memory)
        .unwrap();
    session
        .save(&SessionRecord::new(
            session.id().clone(),
            agent_id.clone(),
            SessionData {
                mode: StorageMode::Memory,
                agent_config_revision: 1,
                config_revision: 1,
                effective_config: SessionConfig::default(),
                parent: None,
                closed: None,
            },
        ))
        .unwrap();
    let mut journal = session.open_journal().unwrap();
    let range = journal
        .accept(vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())])
        .unwrap();
    let confirmation = journal.persist_pending().unwrap();
    assert_eq!(confirmation.range, range);
    assert_eq!(confirmation.persistence, PersistenceState::NotApplicable);
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 1,
            persisted: None
        }
    );
    assert!(
        !f.root
            .canonical_path()
            .join("agents")
            .join(agent_id.as_str())
            .exists()
    );
    let memory_child = f
        .agent
        .session(SessionId::new().unwrap(), StorageMode::Memory)
        .unwrap();
    let mut memory_journal = memory_child.open_journal().unwrap();
    memory_journal
        .accept(vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())])
        .unwrap();
    assert!(!memory_child.canonical_path().exists());
}
#[test]
fn writes_confirm_exact_boundaries_and_reopen_native_content() {
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let mut journal = f.session.open_journal().unwrap();
    let first = vec![
        input(turn.clone(), loop_id),
        response(turn.clone(), loop_id),
    ];
    let first_range = journal.accept(first.clone()).unwrap();
    assert_eq!(first_range, SaveRange { start: 0, end: 2 });
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 2,
            persisted: Some(0)
        }
    );
    assert_eq!(fs::read(f.history_path()).unwrap(), b"");
    assert_eq!(journal.persist_pending().unwrap().range, first_range);
    assert_eq!(journal.positions().persisted, Some(2));
    let end = HistoryEntry::TurnFinished {
        turn_id: turn,
        outcome: TurnOutcome::Completed,
        reason: None,
    };
    assert_eq!(
        journal.accept(vec![end]).unwrap(),
        SaveRange { start: 2, end: 3 }
    );
    journal.persist_pending().unwrap();
    drop(journal);
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.records.len(), 3);
    assert_eq!(loaded.runtime_history().len(), 2);
    assert_eq!(loaded.records[0].entry, first[0]);
    let mut reopened = f.session.open_journal().unwrap();
    assert_eq!(
        reopened.positions(),
        SavePositions {
            accepted: 3,
            persisted: Some(3)
        }
    );
    assert_eq!(reopened.take_loaded_history(), loaded);
    assert!(reopened.loaded_records().is_empty());
}
#[test]
fn duplicate_writer_and_pending_or_duplicate_batches_are_rejected() {
    let f = Fixture::new();
    let mut journal = f.session.open_journal().unwrap();
    assert!(matches!(
        f.session.open_journal(),
        Err(StorageError::HistoryInUse)
    ));
    let entry = input(TurnId::new().unwrap(), LoopId::new().unwrap());
    let record = HistoryRecord::new(f.session.id().clone(), 0, entry);
    journal.accept_batch(vec![record.clone()]).unwrap();
    assert!(matches!(
        journal.accept_batch(vec![record.clone()]),
        Err(StorageError::PendingBatch)
    ));
    journal.persist_pending().unwrap();
    assert!(matches!(
        journal.accept_batch(vec![record]),
        Err(StorageError::InvalidHistory {
            problem: HistoryProblem::DuplicatePosition { .. },
            ..
        })
    ));
}
#[test]
fn complete_final_record_without_newline_is_read_and_append_preserves_prefix() {
    let f = Fixture::new();
    let mut bytes = record_bytes(
        &f.session,
        vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())],
    );
    bytes.pop();
    fs::write(f.history_path(), &bytes).unwrap();
    assert_eq!(f.session.read_history().unwrap().records.len(), 1);
    let mut journal = f.session.open_journal().unwrap();
    journal
        .accept(vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())])
        .unwrap();
    journal.persist_pending().unwrap();
    drop(journal);
    let result = fs::read(f.history_path()).unwrap();
    assert!(result.starts_with(&bytes));
    assert_eq!(result[bytes.len()], b'\n');
    assert_eq!(f.session.read_history().unwrap().records.len(), 2);
}
#[test]
fn corruption_truncation_duplicate_unknown_fields_are_never_repaired() {
    let f = Fixture::new();
    let good = record_bytes(
        &f.session,
        vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())],
    );
    let mut unknown: serde_json::Value = serde_json::from_slice(&good).unwrap();
    unknown["entry"]["data"]["user"]["unknown"] = serde_json::json!("value");
    let mut wrong_session: serde_json::Value = serde_json::from_slice(&good).unwrap();
    wrong_session["session_id"] = serde_json::json!(SessionId::new().unwrap());
    let mut gap: serde_json::Value = serde_json::from_slice(&good).unwrap();
    gap["position"] = serde_json::json!(2);
    let bad_cases = vec![
        good[..good.len() - 5].to_vec(),
        [good.clone(), b"{not json}\n".to_vec()].concat(),
        [good.clone(), good.clone()].concat(),
        [good.clone(), b"\n".to_vec()].concat(),
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec(&wrong_session).unwrap(),
        serde_json::to_vec(&gap).unwrap(),
        b"{\"schema_version\":1,\"schema_version\":1}".to_vec(),
    ];
    for bad in bad_cases {
        fs::write(f.history_path(), &bad).unwrap();
        assert!(f.session.read_history().is_err());
        assert!(f.session.open_journal().is_err());
        assert_eq!(fs::read(f.history_path()).unwrap(), bad);
    }
}
#[test]
fn unsupported_history_schema_has_original_position_and_is_unchanged() {
    let f = Fixture::new();
    let first = record_bytes(
        &f.session,
        vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())],
    );
    let second = HistoryRecord {
        schema_version: 999,
        session_id: f.session.id().clone(),
        position: 1,
        entry: input(TurnId::new().unwrap(), LoopId::new().unwrap()),
    };
    let mut bytes = first.clone();
    serde_json::to_writer(&mut bytes, &second).unwrap();
    fs::write(f.history_path(), &bytes).unwrap();
    assert!(
        matches!(f.session.read_history(), Err(StorageError::UnsupportedSchema { found: 999, position: Some(FilePosition { line: 2, byte_offset }), .. }) if byte_offset == first.len() as u64)
    );
    assert_eq!(fs::read(f.history_path()).unwrap(), bytes);
}
#[test]
fn missing_existing_history_is_not_recreated() {
    let f = Fixture::new();
    fs::remove_file(f.history_path()).unwrap();
    assert!(f.session.load().is_err());
    assert!(f.session.open_journal().is_err());
    assert!(!f.history_path().exists());
}
#[test]
fn symlink_hardlink_and_foreign_writable_paths_are_rejected() {
    let f = Fixture::new();
    let path = f.history_path();
    let target = f.temp.path().join("untouched");
    fs::write(&target, b"outside").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(matches!(
        f.session.open_journal(),
        Err(StorageError::UnsafePath { .. })
    ));
    assert_eq!(fs::read(&target).unwrap(), b"outside");
    fs::remove_file(&path).unwrap();
    fs::hard_link(&target, &path).unwrap();
    assert!(matches!(
        f.session.open_journal(),
        Err(StorageError::UnsafePath { .. })
    ));
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        f.session.open_journal(),
        Err(StorageError::UnsafePath { .. })
    ));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let session_path = f.session.canonical_path();
    let parked = session_path.with_extension("parked");
    fs::rename(&session_path, &parked).unwrap();
    symlink(&parked, &session_path).unwrap();
    assert!(matches!(
        f.session.load(),
        Err(StorageError::UnsafePath { .. })
    ));
}
#[test]
fn replaced_history_blocks_original_writer_without_touching_replacement() {
    let f = Fixture::new();
    let mut journal = f.session.open_journal().unwrap();
    journal
        .accept(vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())])
        .unwrap();
    fs::rename(f.history_path(), f.history_path().with_extension("old")).unwrap();
    fs::write(f.history_path(), b"replacement").unwrap();
    assert!(matches!(
        journal.persist_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(fs::read(f.history_path()).unwrap(), b"replacement");
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 1,
            persisted: Some(0)
        }
    );
}
#[test]
fn close_is_stable_and_explicit_delete_never_recurses() {
    let f = Fixture::new();
    assert!(matches!(
        f.agent.delete_empty(),
        Err(StorageError::DirectoryNotEmpty)
    ));
    assert!(f.session.delete_closed().is_err());
    let mut record = f.session.load().unwrap();
    record.data.closed = Some(SessionClosure {
        reason: "explicit close".into(),
    });
    f.session.save(&record).unwrap();
    let mut reopened = record.clone();
    reopened.data.closed = None;
    assert!(f.session.save(&reopened).is_err());
    let unexpected = f.session.canonical_path().join("artifact.txt");
    fs::write(&unexpected, "keep").unwrap();
    assert!(matches!(
        f.session.delete_closed(),
        Err(StorageError::DirectoryNotEmpty)
    ));
    assert_eq!(fs::read_to_string(&unexpected).unwrap(), "keep");
    fs::remove_file(unexpected).unwrap();
    f.session.delete_closed().unwrap();
    assert!(!f.session.canonical_path().exists());
    f.agent.delete_empty().unwrap();
    assert!(f.host.list_agents().unwrap().is_empty());
    assert!(f.root.canonical_path().join("host.json").exists());
}
#[test]
fn root_and_object_initialization_failure_release_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root_path = temp.path().join("host");
    fs::create_dir(&root_path).unwrap();
    fs::set_permissions(&root_path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        DataRoot::acquire(&root_path),
        Err(StorageError::UnsafePath { .. })
    ));
    fs::set_permissions(&root_path, fs::Permissions::from_mode(0o700)).unwrap();
    let root = DataRoot::acquire(&root_path).unwrap();
    fs::write(root_path.join("host.json"), b"broken").unwrap();
    assert!(root.load_host::<HostConfig>().is_err());
    drop(root);
    let new_root = DataRoot::acquire(&root_path).unwrap();
    assert!(new_root.load_host::<HostConfig>().is_err());
    assert_eq!(fs::read(root_path.join("host.json")).unwrap(), b"broken");
}
