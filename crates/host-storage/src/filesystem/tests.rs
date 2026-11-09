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
