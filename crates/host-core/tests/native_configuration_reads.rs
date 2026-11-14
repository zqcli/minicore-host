//! Supported native records retain explicit empty instruction fields. These
//! reads use the real strict storage decoder and never rewrite the source files.
use minicore_host_core::HostConfig;
use minicore_host_protocol::{
    AgentConfig, AgentId, HostId, SessionConfig, SessionId, StorageMode, Timestamp,
};
use minicore_host_runtime::{Text, history::SummaryHistory};
use minicore_host_storage::{
    AgentData, AgentRecord, DataRoot, HistoryEntry, HostRecord, SessionData, SessionRecord,
};
use serde_json::Value;
use std::path::Path;

fn rewrite(path: &Path, edit: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    edit(&mut value);
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn explicit_empty_native_instructions_and_pre_extension_records_remain_readable() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::acquire(directory.path().join("data")).unwrap();
    let host = root.host(HostId::new().unwrap());
    host.save_host(&HostRecord::new(
        host.id().clone(),
        HostConfig::default(),
        0,
    ))
    .unwrap();
    let agent = host.agent(AgentId::new().unwrap(), StorageMode::Persistent);
    agent
        .save(&AgentRecord::new(
            agent.id().clone(),
            host.id().clone(),
            AgentData {
                configuration_source: None,
                name: "Native record fixture".into(),
                mode: StorageMode::Persistent,
                config: AgentConfig::default(),
                config_revision: 0,
                default_session_id: None,
            },
        ))
        .unwrap();
    let session = agent
        .session(SessionId::new().unwrap(), StorageMode::Persistent)
        .unwrap();
    let created = Timestamp::now();
    session
        .save(&SessionRecord::new(
            session.id().clone(),
            agent.id().clone(),
            SessionData {
                created_at: created,
                last_activity_at: created,
                title: None,
                mode: StorageMode::Persistent,
                agent_config_revision: 0,
                config_revision: 0,
                effective_config: SessionConfig::default(),
                parent: None,
                closed: None,
            },
        ))
        .unwrap();
    let entry = HistoryEntry::Summary {
        summary: SummaryHistory {
            content: Text::new("saved native history"),
        },
    };
    let mut journal = session.open_journal().unwrap();
    journal.accept(vec![entry.clone()]).unwrap();
    journal.persist_pending().unwrap();
    drop(journal);

    let host_path = host.canonical_path().join("host.json");
    let agent_path = host
        .canonical_path()
        .join("agents")
        .join(agent.id().as_str())
        .join("agent.json");
    let session_path = session.canonical_path().join("session.json");
    let history_path = session.canonical_path().join("history.jsonl");
    // The supported record schema already wrote these two original fields as
    // explicit empty strings. New Agent/Session instructions were absent.
    rewrite(&host_path, |value| {
        value["data"]["config"]["framework_instructions"] = Value::String(String::new())
    });
    rewrite(&agent_path, |value| {
        value["data"]["config"]["role"]["instructions"] = Value::String(String::new());
        value["data"]["config"]
            .as_object_mut()
            .unwrap()
            .remove("instructions");
    });
    rewrite(&session_path, |value| {
        value["data"]["effective_config"]["role"]["instructions"] = Value::String(String::new());
        value["data"]["effective_config"]
            .as_object_mut()
            .unwrap()
            .remove("instructions");
    });
    let paths = [host_path, agent_path, session_path, history_path];
    let before: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();

    assert!(
        host.load_host::<HostConfig>()
            .unwrap()
            .data
            .config
            .framework_instructions
            .is_empty()
    );
    let loaded_agent = agent.load().unwrap();
    assert!(loaded_agent.data.config.role.instructions.is_empty());
    assert!(loaded_agent.data.config.instructions.is_empty());
    let loaded_session = session.load().unwrap();
    assert!(
        loaded_session
            .data
            .effective_config
            .role
            .instructions
            .is_empty()
    );
    assert!(loaded_session.data.effective_config.instructions.is_empty());
    let journal = session.open_journal().unwrap();
    assert_eq!(journal.loaded_records().len(), 1);
    assert_eq!(journal.loaded_records()[0].entry, entry);
    drop(journal);
    assert_eq!(session.read_history().unwrap().records.len(), 1);
    for (path, original) in paths.iter().zip(before) {
        assert_eq!(
            std::fs::read(path).unwrap(),
            original,
            "read rewrote {}",
            path.display()
        );
    }
}
