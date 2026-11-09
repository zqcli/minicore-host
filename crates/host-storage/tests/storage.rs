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

fn tool_response(turn_id: TurnId, loop_id: LoopId) -> HistoryEntry {
    use minicore_runtime::{ToolCallId, model::ToolCall};
    let mut entry = response(turn_id, loop_id);
    if let HistoryEntry::Runtime {
        item: HistoryItem::Assistant(item),
        ..
    } = &mut entry
    {
        item.content = vec![AssistantPart::ToolCall(
            ToolCall::new(
                ToolCallId::new("native-call").unwrap(),
                "read_file".parse().unwrap(),
                serde_json::json!({"path":"file.txt"}),
                0,
            )
            .unwrap(),
        )];
        item.finish_reason = ModelFinishReason::ToolCalls;
    }
    entry
}
fn tool_result(turn_id: TurnId, loop_id: LoopId) -> HistoryEntry {
    use minicore_runtime::{
        ToolCallId,
        history::ToolResultHistory,
        tools::{ToolOutput, ToolResultOutcome},
    };
    HistoryEntry::Runtime {
        turn_id,
        response_index: 0,
        item: HistoryItem::ToolResult(ToolResultHistory {
            loop_id,
            request_index: 0,
            call_id: ToolCallId::new("native-call").unwrap(),
            tool_name: "read_file".parse().unwrap(),
            outcome: ToolResultOutcome::Success,
            output: ToolOutput::new("actual file result").unwrap(),
        }),
    }
}
#[test]
fn native_tool_history_and_unclosed_old_calls_are_valid_without_recovery() {
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let entries = vec![
        input(turn.clone(), loop_id),
        tool_response(turn.clone(), loop_id),
    ];
    let bytes = record_bytes(&f.session, entries.clone());
    fs::write(f.history_path(), &bytes).unwrap();
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.records.len(), 2);
    assert_eq!(fs::read(f.history_path()).unwrap(), bytes);
    let mut writer = f.session.open_journal().unwrap();
    writer.accept(vec![tool_result(turn, loop_id)]).unwrap();
    writer.persist_pending().unwrap();
    drop(writer);
    let full = f.session.read_history().unwrap();
    assert_eq!(full.runtime_history().len(), 3);
    assert!(matches!(
        full.runtime_history().last(),
        Some(HistoryItem::ToolResult(_))
    ));
    // Reopening only loads the records. No queue, task, tool executor, receipt,
    // invented result, or recovery operation is created by storage.
    let reopened = f.session.open_journal().unwrap();
    assert_eq!(
        reopened.positions(),
        SavePositions {
            accepted: 3,
            persisted: Some(3)
        }
    );
}
#[test]
fn orphan_mismatched_duplicate_tool_results_and_response_gaps_are_invalid() {
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let mut wrong = tool_result(turn.clone(), loop_id);
    if let HistoryEntry::Runtime {
        item: HistoryItem::ToolResult(result),
        ..
    } = &mut wrong
    {
        result.tool_name = "different_tool".parse().unwrap();
    }
    let mut gap = response(turn.clone(), loop_id);
    if let HistoryEntry::Runtime { response_index, .. } = &mut gap {
        *response_index = 3;
    }
    let cases = vec![
        vec![tool_result(turn.clone(), loop_id)],
        vec![tool_response(turn.clone(), loop_id), wrong],
        vec![
            tool_response(turn.clone(), loop_id),
            tool_result(turn.clone(), loop_id),
            tool_result(turn.clone(), loop_id),
        ],
        vec![gap],
    ];
    for entries in cases {
        let bytes = record_bytes(&f.session, entries);
        fs::write(f.history_path(), &bytes).unwrap();
        assert!(f.session.read_history().is_err());
        assert_eq!(fs::read(f.history_path()).unwrap(), bytes);
    }
}
#[test]
fn partial_model_fragments_are_distinct_and_never_projected_as_complete_assistant() {
    use minicore_runtime::{
        ToolCallId,
        model::{IncompleteToolCall, ModelPartial},
    };
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let partial = HistoryEntry::Partial {
        turn_id: turn.clone(),
        loop_id,
        response_index: 0,
        partial: ModelPartial {
            request_index: 0,
            content: vec![AssistantPart::Text("unfinished ".into())],
            incomplete_tool_calls: vec![IncompleteToolCall {
                part_index: 1,
                call_index: 0,
                tool_call_id: ToolCallId::new("incomplete-call").unwrap(),
                tool_name: "read_file".parse().unwrap(),
                arguments_text: "{\"path\":\"".into(),
            }],
            usage: None,
            provider_replay: None,
        },
    };
    let mut journal = f.session.open_journal().unwrap();
    journal
        .accept(vec![
            input(turn.clone(), loop_id),
            partial.clone(),
            HistoryEntry::TurnFinished {
                turn_id: turn.clone(),
                outcome: TurnOutcome::Cancelled,
                reason: Some("cancelled while receiving model output".into()),
            },
        ])
        .unwrap();
    journal.persist_pending().unwrap();
    drop(journal);
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.records[1].entry, partial);
    assert_eq!(loaded.runtime_history().len(), 1);
    assert!(matches!(loaded.runtime_history()[0], HistoryItem::User(_)));
    let complete_then_partial = record_bytes(&f.session, vec![response(turn, loop_id), partial]);
    fs::write(f.history_path(), &complete_then_partial).unwrap();
    assert!(f.session.read_history().is_err());
    assert_eq!(fs::read(f.history_path()).unwrap(), complete_then_partial);
}
#[test]
fn native_record_unknown_null_empty_input_and_content_normalization_are_rejected() {
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let input_bytes = record_bytes(&f.session, vec![input(turn.clone(), loop_id)]);
    let response_bytes = record_bytes(&f.session, vec![response(turn, loop_id)]);
    let mut unknown_null: serde_json::Value = serde_json::from_slice(&input_bytes).unwrap();
    unknown_null["entry"]["data"]["user"]["provider_replay"] = serde_json::Value::Null;
    let mut empty_input: serde_json::Value = serde_json::from_slice(&input_bytes).unwrap();
    empty_input["entry"]["data"]["user"]["input"]["text"] = serde_json::json!("");
    let mut normalized: serde_json::Value = serde_json::from_slice(&response_bytes).unwrap();
    normalized["entry"]["data"]["item"]["data"]["content"][0]["data"] =
        serde_json::json!("line\r\nchanged");
    for (case, value) in [unknown_null, empty_input, normalized]
        .into_iter()
        .enumerate()
    {
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(f.history_path(), &bytes).unwrap();
        assert!(
            f.session.read_history().is_err(),
            "invalid native payload case {case} was accepted"
        );
        assert_eq!(fs::read(f.history_path()).unwrap(), bytes);
    }
}
#[test]
fn moved_session_directory_blocks_original_writer() {
    let f = Fixture::new();
    let mut journal = f.session.open_journal().unwrap();
    journal
        .accept(vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())])
        .unwrap();
    let original = f.session.canonical_path();
    let moved = original.with_extension("moved");
    fs::rename(&original, &moved).unwrap();
    fs::create_dir(&original).unwrap();
    fs::write(original.join("history.jsonl"), b"replacement").unwrap();
    assert!(journal.persist_pending().is_err());
    assert_eq!(fs::read(moved.join("history.jsonl")).unwrap(), b"");
    assert_eq!(
        fs::read(original.join("history.jsonl")).unwrap(),
        b"replacement"
    );
}
#[test]
fn deletion_waits_for_existing_history_writer() {
    let f = Fixture::new();
    let journal = f.session.open_journal().unwrap();
    let mut record = f.session.load().unwrap();
    record.data.closed = Some(SessionClosure {
        reason: "explicit".into(),
    });
    f.session.save(&record).unwrap();
    assert!(matches!(
        f.session.delete_closed(),
        Err(StorageError::HistoryInUse)
    ));
    assert!(f.history_path().exists());
    drop(journal);
    f.session.delete_closed().unwrap();
}
#[test]
fn object_write_failure_probe() {
    let Ok(path) = std::env::var("MINICORE_STORAGE_FAILURE_PROBE") else {
        return;
    };
    let root = DataRoot::acquire(&path).unwrap();
    let mut record: HostRecord<HostConfig> = root.load_host().unwrap().unwrap();
    let host = root.host(record.id.clone());
    record.data.config.label = "large object under OS file-size limit".repeat(1024);
    record.data.config_revision += 1;
    assert!(matches!(
        host.save_host(&record),
        Err(StorageError::Io {
            operation: "write object temporary file",
            ..
        })
    ));
}
#[test]
fn atomic_object_save_keeps_original_on_real_os_write_failure() {
    let f = Fixture::new();
    let root_path = f.root.canonical_path().to_owned();
    let original = fs::read(root_path.join("host.json")).unwrap();
    drop(f.session);
    drop(f.agent);
    drop(f.host);
    drop(f.root);
    // SIGXFSZ is ignored only in this subprocess, so write(2) returns EFBIG.
    // This is a real kernel file write failure, not a production capacity rule.
    let status = Command::new("sh")
        .args(["-c", "trap '' 25; ulimit -f 1; exec \"$@\"", "storage-test"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "object_write_failure_probe", "--nocapture"])
        .env("MINICORE_STORAGE_FAILURE_PROBE", &root_path)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(fs::read(root_path.join("host.json")).unwrap(), original);
    assert!(fs::read_dir(&root_path).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
    DataRoot::acquire(&root_path).unwrap();
}
#[test]
fn journal_write_failure_probe() {
    let Ok(path) = std::env::var("MINICORE_STORAGE_JOURNAL_FAILURE_PROBE") else {
        return;
    };
    let root = DataRoot::acquire(path).unwrap();
    let host = root.host(root.load_host::<HostConfig>().unwrap().unwrap().id);
    let agent = host.agent(
        host.list_agents().unwrap().remove(0),
        StorageMode::Persistent,
    );
    let session = agent
        .session(
            agent.list_sessions().unwrap().remove(0),
            StorageMode::Persistent,
        )
        .unwrap();
    let mut journal = session.open_journal().unwrap();
    journal
        .accept(vec![HistoryEntry::Summary {
            summary: minicore_runtime::history::SummaryHistory {
                content: minicore_runtime::Text::new("large append".repeat(3000)),
            },
        }])
        .unwrap();
    assert!(matches!(
        journal.persist_pending(),
        Err(StorageError::Io {
            operation: "append history",
            ..
        })
    ));
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 1,
            persisted: Some(0)
        }
    );
    assert!(matches!(
        journal.reconcile_pending(),
        Err(StorageError::UncertainWrite)
    ));
}
#[test]
fn real_os_partial_history_write_remains_blocked_without_repair() {
    let f = Fixture::new();
    let root_path = f.root.canonical_path().to_owned();
    let history_path = f.history_path();
    drop(f.session);
    drop(f.agent);
    drop(f.host);
    drop(f.root);
    let status = Command::new("sh")
        .args(["-c", "trap '' 25; ulimit -f 4; exec \"$@\"", "storage-test"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "journal_write_failure_probe", "--nocapture"])
        .env("MINICORE_STORAGE_JOURNAL_FAILURE_PROBE", &root_path)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let partial = fs::read(&history_path).unwrap();
    assert!(!partial.is_empty());
    let root = DataRoot::acquire(root_path).unwrap();
    let host = root.host(root.load_host::<HostConfig>().unwrap().unwrap().id);
    let agent = host.agent(
        host.list_agents().unwrap().remove(0),
        StorageMode::Persistent,
    );
    let session = agent
        .session(
            agent.list_sessions().unwrap().remove(0),
            StorageMode::Persistent,
        )
        .unwrap();
    assert!(session.read_history().is_err());
    assert_eq!(fs::read(history_path).unwrap(), partial);
}

#[test]
fn same_revision_config_changes_and_creation_provenance_changes_are_rejected() {
    let f = Fixture::new();
    let path = f.session.canonical_path().join("session.json");
    let before = fs::read(&path).unwrap();
    let original = f.session.load().unwrap();
    let mut changed = original.clone();
    changed
        .data
        .effective_config
        .role
        .instructions
        .push_str("changed");
    assert!(f.session.save(&changed).is_err());
    changed = original.clone();
    changed.data.agent_config_revision += 1;
    assert!(f.session.save(&changed).is_err());
    changed = original;
    changed.data.config_revision = 0;
    assert!(f.session.save(&changed).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut host = f.host.load_host::<HostConfig>().unwrap();
    host.data.config.label = "changed without revision".into();
    assert!(f.host.save_host(&host).is_err());
    let mut agent = f.agent.load().unwrap();
    agent.data.config.role.instructions.push_str("changed");
    assert!(f.agent.save(&agent).is_err());
}

#[test]
fn child_listing_checks_direct_owner_records_before_enumeration() {
    let f = Fixture::new();
    assert!(matches!(
        f.root.host(HostId::new().unwrap()).list_agents(),
        Err(StorageError::OwnershipMismatch)
    ));
    let path = f
        .root
        .canonical_path()
        .join("agents")
        .join(f.agent.id().as_str())
        .join("agent.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["host_id"] = serde_json::json!(HostId::new().unwrap());
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(matches!(
        f.agent.list_sessions(),
        Err(StorageError::OwnershipMismatch)
    ));
}

#[test]
fn a_terminal_partial_prevents_further_records_for_that_loop_without_mutating_accepted_state() {
    use minicore_runtime::model::ModelPartial;
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let partial = |request_index, response_index| HistoryEntry::Partial {
        turn_id: turn.clone(),
        loop_id,
        response_index,
        partial: ModelPartial {
            request_index,
            content: vec![AssistantPart::Text("partial".into())],
            incomplete_tool_calls: Vec::new(),
            usage: None,
            provider_replay: None,
        },
    };
    let mut later_response = response(turn.clone(), loop_id);
    if let HistoryEntry::Runtime {
        response_index,
        item: HistoryItem::Assistant(item),
        ..
    } = &mut later_response
    {
        *response_index = 1;
        item.request_index = 1;
    }
    for entries in [
        vec![partial(0, 0), partial(1, 1)],
        vec![partial(0, 0), later_response],
    ] {
        let mut memory = SessionJournal::memory(f.session.id().clone());
        assert!(memory.accept(entries.clone()).is_err());
        assert_eq!(memory.positions().accepted, 0);
        // All speculative index changes from a rejected batch were discarded.
        memory
            .accept(vec![response(turn.clone(), loop_id)])
            .unwrap();
        let bad = record_bytes(&f.session, entries);
        fs::write(f.history_path(), &bad).unwrap();
        assert!(f.session.read_history().is_err());
        assert_eq!(fs::read(f.history_path()).unwrap(), bad);
    }
    let valid = record_bytes(
        &f.session,
        vec![response(turn.clone(), loop_id), partial(1, 1)],
    );
    fs::write(f.history_path(), &valid).unwrap();
    assert!(f.session.read_history().is_ok());
}
#[test]
fn exact_json_numbers_are_preserved_or_explicitly_rejected_by_native_codec() {
    let f = Fixture::new();
    let exact = "18446744073709551617";
    let config: serde_json::Value = serde_json::from_str(&format!("{{\"n\":{exact}}}")).unwrap();
    let host = HostRecord::new(f.host.id().clone(), config, 2);
    f.host.save_host(&host).unwrap();
    let loaded = f.root.load_host::<serde_json::Value>().unwrap().unwrap();
    assert_eq!(loaded.data.config["n"].to_string(), exact);
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let good = record_bytes(&f.session, vec![tool_response(turn, loop_id)]);
    let mut value: serde_json::Value = serde_json::from_slice(&good).unwrap();
    value["entry"]["data"]["item"]["data"]["content"][0]["data"]["arguments"]["n"] =
        serde_json::from_str(exact).unwrap();
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(f.history_path(), &bytes).unwrap();
    match f.session.read_history() {
        Ok(loaded) => {
            let HistoryEntry::Runtime {
                item: HistoryItem::Assistant(item),
                ..
            } = &loaded.records[0].entry
            else {
                panic!("wrong native item")
            };
            assert_eq!(
                item.content[0].as_tool_call().unwrap().arguments()["n"].to_string(),
                exact
            );
        }
        // serde's tagged-enum content buffer may reject u128 rather than retain
        // it. That is an explicit unsupported native encoding, never rounding.
        Err(StorageError::InvalidHistory {
            problem: HistoryProblem::InvalidRecord(reason),
            ..
        }) => assert!(
            reason.contains("u128"),
            "unexpected native failure: {reason}"
        ),
        other => panic!("unexpected numeric loading result: {other:?}"),
    }
    assert_eq!(fs::read(f.history_path()).unwrap(), bytes);
}
#[test]
fn failed_batch_validation_never_commits_a_finished_turn_or_native_normalization() {
    let mut journal = SessionJournal::memory(SessionId::new().unwrap());
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    assert!(
        journal
            .accept(vec![
                HistoryEntry::TurnFinished {
                    turn_id: turn.clone(),
                    outcome: TurnOutcome::Failed,
                    reason: None
                },
                input(turn.clone(), loop_id)
            ])
            .is_err()
    );
    let mut invalid = response(turn.clone(), loop_id);
    if let HistoryEntry::Runtime {
        item: HistoryItem::Assistant(item),
        ..
    } = &mut invalid
    {
        item.content = vec![AssistantPart::Text("changes\rcontent".into())];
    }
    assert!(journal.accept(vec![invalid]).is_err());
    assert_eq!(journal.positions().accepted, 0);
    journal
        .accept(vec![input(turn.clone(), loop_id), response(turn, loop_id)])
        .unwrap();
    assert_eq!(journal.positions().accepted, 2);
}

#[test]
fn broken_default_session_does_not_block_agent_record_or_healthy_sibling_initialization() {
    let f = Fixture::new();
    let healthy = f
        .agent
        .session(SessionId::new().unwrap(), StorageMode::Persistent)
        .unwrap();
    healthy
        .save(&SessionRecord::new(
            healthy.id().clone(),
            f.agent.id().clone(),
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
    let mut agent = f.agent.load_record().unwrap();
    agent.data.default_session_id = Some(f.session.id().clone());
    f.agent.save(&agent).unwrap();
    let metadata = f.session.canonical_path().join("session.json");
    let original = fs::read(&metadata).unwrap();
    fs::write(&metadata, b"broken Session metadata").unwrap();
    assert_eq!(f.agent.load_record().unwrap(), agent);
    assert!(f.agent.load().is_err());
    assert!(f.agent.list_sessions().unwrap().contains(healthy.id()));
    assert!(healthy.load().is_ok());
    assert!(f.session.load().is_err());
    // No replacement/default selection or original-file repair took place.
    assert_eq!(fs::read(&metadata).unwrap(), b"broken Session metadata");
    fs::write(&metadata, &original).unwrap();
    fs::remove_file(f.history_path()).unwrap();
    assert_eq!(f.agent.load_record().unwrap(), agent);
    assert!(f.agent.load().is_err());
    assert!(f.agent.list_sessions().unwrap().contains(healthy.id()));
    assert!(healthy.load().is_ok());
    assert!(!f.history_path().exists());
}

fn create_sibling_session(agent: &AgentStore) -> SessionStore {
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
    session
}

#[derive(Clone, Copy, Debug)]
enum UnsafeChildEntry {
    WritableDirectory,
    File,
    Symlink,
}

fn make_child_entry_unsafe(
    path: &std::path::Path,
    outside: &std::path::Path,
    kind: UnsafeChildEntry,
) {
    match kind {
        UnsafeChildEntry::WritableDirectory => {
            fs::set_permissions(path, fs::Permissions::from_mode(0o777)).unwrap()
        }
        UnsafeChildEntry::File => {
            fs::rename(path, outside).unwrap();
            fs::write(path, b"not a directory").unwrap();
        }
        UnsafeChildEntry::Symlink => {
            // The target is otherwise a fully valid object, so the subsequent
            // load must reject the link rather than succeed by following it.
            fs::rename(path, outside).unwrap();
            symlink(outside, path).unwrap();
        }
    }
}

fn assert_healthy_session_can_persist(session: &SessionStore) {
    assert!(session.load().is_ok());
    let mut journal = session.open_journal().unwrap();
    journal
        .accept(vec![input(TurnId::new().unwrap(), LoopId::new().unwrap())])
        .unwrap();
    assert_eq!(
        journal.persist_pending().unwrap().persistence,
        PersistenceState::Persisted
    );
    drop(journal);
    assert_eq!(session.read_history().unwrap().records.len(), 1);
}

#[test]
fn unsafe_child_session_entries_are_isolated_without_following_links() {
    for kind in [
        UnsafeChildEntry::WritableDirectory,
        UnsafeChildEntry::File,
        UnsafeChildEntry::Symlink,
    ] {
        let f = Fixture::new();
        let healthy = create_sibling_session(&f.agent);
        let record = f.session.load().unwrap();
        let path = f.session.canonical_path();
        let original = fs::read(path.join("session.json")).unwrap();
        let outside = f.temp.path().join("outside-session");
        make_child_entry_unsafe(&path, &outside, kind);
        let ids = f
            .agent
            .list_sessions()
            .expect("one unsafe Session entry must not prevent sibling enumeration");
        assert_eq!(ids.len(), 2, "{kind:?}");
        assert!(ids.contains(f.session.id()));
        assert!(ids.contains(healthy.id()));
        assert!(
            matches!(f.session.load(), Err(StorageError::UnsafePath { .. })),
            "{kind:?}"
        );
        assert!(
            matches!(
                f.session.save(&record),
                Err(StorageError::UnsafePath { .. })
            ),
            "{kind:?}"
        );
        assert!(
            matches!(
                f.session.open_journal(),
                Err(StorageError::UnsafePath { .. })
            ),
            "{kind:?}"
        );
        let preserved = if matches!(kind, UnsafeChildEntry::WritableDirectory) {
            &path
        } else {
            &outside
        };
        assert_eq!(fs::read(preserved.join("session.json")).unwrap(), original);
        assert_eq!(fs::read(preserved.join("history.jsonl")).unwrap(), b"");
        assert_healthy_session_can_persist(&healthy);
    }
}

#[test]
fn unsafe_child_agent_entries_are_isolated_without_following_links() {
    for kind in [
        UnsafeChildEntry::WritableDirectory,
        UnsafeChildEntry::File,
        UnsafeChildEntry::Symlink,
    ] {
        let f = Fixture::new();
        let healthy = f
            .host
            .agent(AgentId::new().unwrap(), StorageMode::Persistent);
        healthy
            .save(&AgentRecord::new(
                healthy.id().clone(),
                f.host.id().clone(),
                AgentData {
                    name: "healthy".into(),
                    mode: StorageMode::Persistent,
                    config: AgentConfig::default(),
                    config_revision: 1,
                    default_session_id: None,
                },
            ))
            .unwrap();
        let session = create_sibling_session(&healthy);
        let record = f.agent.load_record().unwrap();
        let path = f
            .root
            .canonical_path()
            .join("agents")
            .join(f.agent.id().as_str());
        let original = fs::read(path.join("agent.json")).unwrap();
        let outside = f.temp.path().join("outside-agent");
        make_child_entry_unsafe(&path, &outside, kind);
        let ids = f
            .host
            .list_agents()
            .expect("one unsafe Agent entry must not prevent sibling enumeration");
        assert_eq!(ids.len(), 2, "{kind:?}");
        assert!(ids.contains(f.agent.id()));
        assert!(ids.contains(healthy.id()));
        assert!(
            matches!(f.agent.load_record(), Err(StorageError::UnsafePath { .. })),
            "{kind:?}"
        );
        assert!(
            matches!(f.agent.save(&record), Err(StorageError::UnsafePath { .. })),
            "{kind:?}"
        );
        assert!(
            matches!(
                f.agent.list_sessions(),
                Err(StorageError::UnsafePath { .. })
            ),
            "{kind:?}"
        );
        let preserved = if matches!(kind, UnsafeChildEntry::WritableDirectory) {
            &path
        } else {
            &outside
        };
        assert_eq!(fs::read(preserved.join("agent.json")).unwrap(), original);
        assert!(healthy.load_record().is_ok());
        assert_eq!(healthy.list_sessions().unwrap(), vec![session.id().clone()]);
        assert_healthy_session_can_persist(&session);
    }
}

#[test]
fn unsafe_shared_collection_directories_still_fail_at_their_owner_boundary() {
    for kind in [
        UnsafeChildEntry::WritableDirectory,
        UnsafeChildEntry::File,
        UnsafeChildEntry::Symlink,
    ] {
        let f = Fixture::new();
        let sessions = f.session.canonical_path().parent().unwrap().to_owned();
        make_child_entry_unsafe(&sessions, &f.temp.path().join("outside-sessions"), kind);
        assert!(
            matches!(
                f.agent.list_sessions(),
                Err(StorageError::UnsafePath { .. })
            ),
            "{kind:?}"
        );
        let agents = f.root.canonical_path().join("agents");
        make_child_entry_unsafe(&agents, &f.temp.path().join("outside-agents"), kind);
        assert!(
            matches!(f.host.list_agents(), Err(StorageError::UnsafePath { .. })),
            "{kind:?}"
        );
    }
}

#[test]
fn concurrent_parent_child_metadata_saves_preserve_confirmed_records() {
    let f = Fixture::new();
    let mut agent_record = f.agent.load_record().unwrap();
    agent_record.data.default_session_id = Some(f.session.id().clone());
    f.agent.save(&agent_record).unwrap();
    let mut session_record = f.session.load().unwrap();
    let start = std::sync::Barrier::new(3);
    let rounds = 512;
    std::thread::scope(|scope| {
        let agent = scope.spawn(|| {
            start.wait();
            for revision in 2..=rounds + 1 {
                agent_record.data.config_revision = revision;
                agent_record.data.config.role.name = format!("Agent revision {revision}");
                assert_eq!(
                    f.agent.save(&agent_record).unwrap(),
                    ObjectSave {
                        config_revision: revision,
                        persistence: PersistenceState::Persisted,
                    }
                );
            }
        });
        let session = scope.spawn(|| {
            start.wait();
            for revision in 2..=rounds + 1 {
                session_record.data.config_revision = revision;
                session_record.data.effective_config.role.name =
                    format!("Session revision {revision}");
                assert_eq!(
                    f.session.save(&session_record).unwrap(),
                    ObjectSave {
                        config_revision: revision,
                        persistence: PersistenceState::Persisted,
                    }
                );
            }
        });
        start.wait();
        for _ in 0..rounds * 4 {
            f.agent.load().unwrap();
            f.session.load().unwrap();
        }
        agent.join().unwrap();
        session.join().unwrap();
    });
    assert_eq!(f.agent.load().unwrap(), agent_record);
    assert_eq!(f.session.load().unwrap(), session_record);
    assert_eq!(std::fs::read(f.history_path()).unwrap(), b"");

    let path = f.root.canonical_path().to_path_buf();
    let host_id = f.host.id().clone();
    let agent_id = f.agent.id().clone();
    let session_id = f.session.id().clone();
    drop(f.session);
    drop(f.agent);
    drop(f.host);
    drop(f.root);
    let root = DataRoot::acquire(&path).unwrap();
    let agent = root.host(host_id).agent(agent_id, StorageMode::Persistent);
    let session = agent.session(session_id, StorageMode::Persistent).unwrap();
    assert_eq!(agent.load().unwrap(), agent_record);
    assert_eq!(session.load().unwrap(), session_record);
}
