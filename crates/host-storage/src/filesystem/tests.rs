use super::*;

fn fail(path: &Path) {
    SYNC_FAILURE.with(|failure| *failure.borrow_mut() = Some(path.to_owned()));
    SYNC_TRACE.with(|trace| trace.borrow_mut().clear());
}
fn traced(path: &Path) -> bool {
    SYNC_TRACE.with(|trace| trace.borrow().iter().any(|p| p == path))
}
fn clear_trace() {
    SYNC_TRACE.with(|trace| trace.borrow_mut().clear());
}

#[test]
fn root_retry_syncs_visible_ancestors_after_failed_creation_sync() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path().join("new-parent");
    let path = parent.join("host");
    fail(&parent);
    assert!(Directory::root(&path).is_err());
    assert!(path.exists());
    clear_trace();
    let _root = Directory::root(&path).unwrap();
    assert!(traced(&parent));
}
#[test]
fn child_retry_syncs_parent_after_mkdir_succeeded_but_sync_failed() {
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    fail(&root.path);
    assert!(root.child("agents", true).is_err());
    assert!(root.path.join("agents").exists());
    clear_trace();
    let child = root.child("agents", true).unwrap();
    assert!(traced(&root.path));
    clear_trace();
    child.atomic_write("object.json", b"{}").unwrap();
    assert!(traced(&child.path));
    assert!(traced(&root.path));
}
#[test]
fn initial_history_retry_syncs_existing_empty_file_before_publishing_session() {
    use crate::*;
    use minicore_host_protocol::*;
    let temp = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(temp.path().join("host")).unwrap();
    let host = root.host(HostId::new().unwrap());
    host.save_host(&HostRecord::new(
        host.id().clone(),
        serde_json::json!({}),
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
    let record = SessionRecord::new(
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
    );
    let history = session.canonical_path().join("history.jsonl");
    fail(&history);
    assert!(session.save(&record).is_err());
    assert_eq!(std::fs::read(&history).unwrap(), b"");
    assert!(!session.canonical_path().join("session.json").exists());
    clear_trace();
    session.save(&record).unwrap();
    assert!(traced(&history));
}

#[test]
fn partial_explicit_delete_can_continue_while_closed_ownership_record_remains() {
    use crate::*;
    use minicore_host_protocol::*;
    let temp = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(temp.path().join("host")).unwrap();
    let host = root.host(HostId::new().unwrap());
    host.save_host(&HostRecord::new(
        host.id().clone(),
        serde_json::json!({}),
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
                closed: Some(SessionClosure {
                    reason: "explicit".into(),
                }),
            },
        ))
        .unwrap();
    let path = session.canonical_path();
    fail(&path);
    assert!(session.delete_closed().is_err());
    assert!(!path.join("history.jsonl").exists());
    assert!(path.join("session.json").exists());
    session.delete_closed().unwrap();
    assert!(!path.exists());
}

#[test]
fn metadata_snapshot_excludes_only_its_same_path_publisher() {
    use std::{sync::mpsc, time::Duration};
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    root.atomic_write("object.json", b"old").unwrap();
    let (ready, opened) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let reader_dir = root.clone();
    let reader = std::thread::spawn(move || {
        METADATA_READ_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                ready.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
            }));
        });
        reader_dir.read_metadata("object.json")
    });
    opened.recv_timeout(Duration::from_secs(5)).unwrap();

    let gate = root.metadata.get(root.path.join("object.json")).unwrap();
    assert!(matches!(
        gate.try_write(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    let (ready, prepared) = mpsc::channel();
    let writer_dir = root.clone();
    let writer = std::thread::spawn(move || {
        METADATA_PUBLISH_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || ready.send(()).unwrap()));
        });
        writer_dir.atomic_write("object.json", b"new")
    });
    prepared.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        std::fs::read(root.path.join("object.json")).unwrap(),
        b"old"
    );
    // Real I/O on another path completes while the first snapshot is paused.
    // No sleeps, polling or a root-wide I/O gate hide a scheduling boundary.
    root.atomic_write("other.json", b"independent").unwrap();
    assert_eq!(root.read_metadata("other.json").unwrap(), b"independent");
    release.send(()).unwrap();
    assert_eq!(reader.join().unwrap().unwrap(), b"old");
    writer.join().unwrap().unwrap();
    assert_eq!(root.read_metadata("object.json").unwrap(), b"new");
}

#[test]
fn metadata_snapshots_reject_unknown_mutations_without_repair_or_retry() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for mutation in [
        "replace",
        "move_replace",
        "unlink",
        "hardlink",
        "chmod",
        "symlink",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = Directory::root(&temp.path().join("host")).unwrap();
        root.atomic_write("object.json", b"old").unwrap();
        let path = root.path.join("object.json");
        let original_path = path.clone();
        let saved = root.path.join("saved");
        let saved_path = saved.clone();
        METADATA_READ_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || match mutation {
                "replace" => {
                    std::fs::write(&saved, b"external").unwrap();
                    std::fs::rename(&saved, &path).unwrap();
                }
                "move_replace" => {
                    std::fs::rename(&path, &saved).unwrap();
                    std::fs::write(&path, b"external").unwrap();
                }
                "unlink" => std::fs::remove_file(&path).unwrap(),
                "hardlink" => std::fs::hard_link(&path, &saved).unwrap(),
                "chmod" => {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o777))
                        .unwrap();
                }
                "symlink" => {
                    std::fs::rename(&path, &saved).unwrap();
                    symlink(&saved, &path).unwrap();
                }
                _ => unreachable!(),
            }));
        });
        assert!(root.read_metadata("object.json").is_err(), "{mutation}");
        // Failure must preserve exactly what the external mutator left behind.
        match mutation {
            "replace" | "move_replace" => {
                assert_eq!(std::fs::read(&original_path).unwrap(), b"external");
            }
            "unlink" => assert!(!original_path.exists()),
            _ => assert_eq!(std::fs::read(&original_path).unwrap(), b"old"),
        }
        if matches!(mutation, "move_replace" | "hardlink" | "symlink") {
            assert_eq!(std::fs::read(saved_path).unwrap(), b"old");
        }
    }
}

#[test]
fn metadata_snapshot_rejects_detached_directory_and_history_stays_single_linked() {
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    let child = root.child("child", true).unwrap();
    child.atomic_write("object.json", b"old").unwrap();
    let path = child.path.clone();
    let detached = root.path.join("detached");
    let detached_path = detached.clone();
    METADATA_READ_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move || {
            std::fs::rename(&path, &detached).unwrap();
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
        }));
    });
    assert!(child.read_metadata("object.json").is_err());
    assert_eq!(
        std::fs::read(detached_path.join("object.json")).unwrap(),
        b"old"
    );

    let history = root.open_file("history.jsonl", true, true).unwrap();
    std::fs::remove_file(root.path.join("history.jsonl")).unwrap();
    assert!(matches!(
        check_file(&history),
        Err(StorageError::UnsafePath { .. })
    ));
    assert!(root.open_file("history.jsonl", false, false).is_err());
}

#[test]
fn failed_metadata_sync_releases_the_gate_but_does_not_confirm_readable_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    root.atomic_write("object.json", b"old").unwrap();
    fail(&root.path);
    assert!(matches!(
        root.atomic_write("object.json", b"new"),
        Err(StorageError::Io {
            operation: "sync storage directory",
            ..
        })
    ));
    // Reading the new bytes neither retries nor changes the failed save result.
    clear_trace();
    assert_eq!(root.read_metadata("object.json").unwrap(), b"new");
    assert!(!traced(&root.path));
}
