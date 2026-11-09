use crate::{
    AgentRecord, HostRecord, SCHEMA_VERSION, SessionJournal, SessionRecord, StorageError,
    filesystem::Directory, strict_json,
};
use minicore_host_protocol::{
    AgentId, ConfigRevision, HostId, PersistenceState, SessionId, StorageMode,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

struct Root {
    directory: Directory,
}
/// Exclusive ownership of an actual directory inode. Every child storage handle
/// retains this guard; dropping a caller cannot release an accepted save's lock.
#[derive(Clone)]
pub struct DataRoot {
    inner: Arc<Root>,
}
impl DataRoot {
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Ok(Self {
            inner: Arc::new(Root {
                directory: Directory::root(path.as_ref())?,
            }),
        })
    }
    pub fn load_host<C: DeserializeOwned + Serialize>(
        &self,
    ) -> Result<Option<HostRecord<C>>, StorageError> {
        if !self.inner.directory.contains("host.json")? {
            return Ok(None);
        }
        read_object(&self.inner.directory, "host.json").map(Some)
    }
    pub fn canonical_path(&self) -> &Path {
        &self.inner.directory.path
    }
    pub fn host(&self, host_id: HostId) -> HostStore {
        HostStore {
            root: self.clone(),
            host_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectSave {
    pub config_revision: ConfigRevision,
    pub persistence: PersistenceState,
}

#[derive(Clone)]
pub struct HostStore {
    root: DataRoot,
    host_id: HostId,
}
impl HostStore {
    pub fn id(&self) -> &HostId {
        &self.host_id
    }
    pub fn canonical_path(&self) -> &Path {
        self.root.canonical_path()
    }
    pub fn load_host<C: DeserializeOwned + Serialize>(
        &self,
    ) -> Result<HostRecord<C>, StorageError> {
        let record: HostRecord<C> = read_object(&self.root.inner.directory, "host.json")?;
        if record.id != self.host_id {
            return Err(StorageError::OwnershipMismatch);
        }
        Ok(record)
    }
    pub fn save_host<C: DeserializeOwned + Serialize>(
        &self,
        record: &HostRecord<C>,
    ) -> Result<ObjectSave, StorageError> {
        check_schema(record.schema_version)?;
        if record.id != self.host_id {
            return Err(StorageError::OwnershipMismatch);
        }
        let dir = &self.root.inner.directory;
        if dir.contains("host.json")? {
            let old: HostRecord<C> = self.load_host()?;
            check_revision(old.data.config_revision, record.data.config_revision)?;
            if old.data.config_revision == record.data.config_revision {
                let before = serde_json::to_value(&old.data.config).map_err(|e| {
                    StorageError::InvalidObject {
                        reason: e.to_string(),
                    }
                })?;
                let after = serde_json::to_value(&record.data.config).map_err(|e| {
                    StorageError::InvalidObject {
                        reason: e.to_string(),
                    }
                })?;
                if before != after {
                    return Err(StorageError::InvalidObject {
                        reason: "configuration changed without a new revision".into(),
                    });
                }
            }
        }
        write_object(dir, "host.json", record)?;
        Ok(ObjectSave {
            config_revision: record.data.config_revision,
            persistence: PersistenceState::Persisted,
        })
    }
    pub fn agent(&self, agent_id: AgentId, mode: StorageMode) -> AgentStore {
        AgentStore {
            host: self.clone(),
            agent_id,
            mode,
        }
    }
    /// Discover canonical direct-child IDs without opening their entries. Each
    /// Agent's load checks its directory safety and authoritative ownership; one
    /// unsafe child must not prevent other Agents from being initialized.
    pub fn list_agents(&self) -> Result<Vec<AgentId>, StorageError> {
        self.check_host()?;
        if !self.root.inner.directory.contains("agents")? {
            return Ok(Vec::new());
        }
        list_ids(&self.root.inner.directory.child("agents", false)?)
    }
    fn check_host(&self) -> Result<(), StorageError> {
        // Only identity is needed here. The Host remains the owner of its actual
        // typed configuration and must load/validate that config during startup.
        let record: HostRecord<serde_json::Value> = self.load_host()?;
        check_schema(record.schema_version)
    }
}

#[derive(Clone)]
pub struct AgentStore {
    host: HostStore,
    agent_id: AgentId,
    mode: StorageMode,
}
impl AgentStore {
    pub fn id(&self) -> &AgentId {
        &self.agent_id
    }
    pub fn mode(&self) -> StorageMode {
        self.mode
    }
    fn directory(&self, create: bool) -> Result<Directory, StorageError> {
        if self.mode != StorageMode::Persistent {
            return Err(StorageError::InvalidStorageMode);
        }
        self.host.check_host()?;
        self.host
            .root
            .inner
            .directory
            .child("agents", create)?
            .child(self.agent_id.as_str(), create)
    }
    /// Read only this Agent's authoritative record. Initialization uses this
    /// boundary so a broken default Session cannot block healthy siblings. The
    /// owner initializes each child independently and keeps an unusable default
    /// unavailable; it must not silently choose another Session.
    pub fn load_record(&self) -> Result<AgentRecord, StorageError> {
        let record: AgentRecord = read_object(&self.directory(false)?, "agent.json")?;
        self.validate(&record)?;
        Ok(record)
    }
    /// Strict convenience read including the saved default reference.
    pub fn load(&self) -> Result<AgentRecord, StorageError> {
        let record = self.load_record()?;
        self.check_default(&record)?;
        Ok(record)
    }
    fn validate(&self, record: &AgentRecord) -> Result<(), StorageError> {
        check_schema(record.schema_version)?;
        if record.id != self.agent_id || record.host_id != self.host.host_id {
            return Err(StorageError::OwnershipMismatch);
        }
        if record.data.mode != self.mode {
            return Err(StorageError::InvalidStorageMode);
        }
        record
            .data
            .config
            .validate()
            .map_err(|e| StorageError::InvalidObject {
                reason: e.to_string(),
            })?;
        Ok(())
    }
    fn check_default(&self, record: &AgentRecord) -> Result<(), StorageError> {
        if let Some(id) = &record.data.default_session_id {
            let session = self
                .session(id.clone(), StorageMode::Persistent)?
                .load_without_agent()?;
            if session.data.closed.is_some() {
                return Err(StorageError::InvalidObject {
                    reason: "default Session is closed".into(),
                });
            }
        }
        Ok(())
    }
    pub fn save(&self, record: &AgentRecord) -> Result<ObjectSave, StorageError> {
        self.validate(record)?;
        if self.mode == StorageMode::Memory {
            return Ok(ObjectSave {
                config_revision: record.data.config_revision,
                persistence: PersistenceState::NotApplicable,
            });
        }
        let dir = self.directory(true)?;
        if dir.contains("agent.json")? {
            let old: AgentRecord = read_object(&dir, "agent.json")?;
            self.validate(&old)?;
            check_revision(old.data.config_revision, record.data.config_revision)?;
            if old.data.config_revision == record.data.config_revision
                && (old.data.config != record.data.config || old.data.name != record.data.name)
            {
                return Err(StorageError::InvalidObject {
                    reason: "configuration changed without a new revision".into(),
                });
            }
        }
        self.check_default(record)?;
        write_object(&dir, "agent.json", record)?;
        Ok(ObjectSave {
            config_revision: record.data.config_revision,
            persistence: PersistenceState::Persisted,
        })
    }
    pub fn session(
        &self,
        session_id: SessionId,
        mode: StorageMode,
    ) -> Result<SessionStore, StorageError> {
        if self.mode == StorageMode::Memory && mode == StorageMode::Persistent {
            return Err(StorageError::InvalidStorageMode);
        }
        Ok(SessionStore {
            agent: self.clone(),
            session_id,
            mode,
        })
    }
    /// Discover canonical direct-child IDs, not a claim that their objects are
    /// usable. Session loads perform all path/ownership checks independently.
    pub fn list_sessions(&self) -> Result<Vec<SessionId>, StorageError> {
        self.load_record()?;
        let dir = self.directory(false)?;
        if !dir.contains("sessions")? {
            return Ok(Vec::new());
        }
        list_ids(&dir.child("sessions", false)?)
    }
    /// The caller is the direct owner and must first stop the Agent. This only
    /// deletes its own empty hierarchy; it never recursively removes Sessions.
    pub fn delete_empty(&self) -> Result<(), StorageError> {
        self.load()?;
        let dir = self.directory(false)?;
        let entries = dir.entries()?;
        if entries.iter().any(|s| s != "agent.json" && s != "sessions") {
            return Err(StorageError::DirectoryNotEmpty);
        }
        if dir.contains("sessions")? {
            if !dir.child("sessions", false)?.entries()?.is_empty() {
                return Err(StorageError::DirectoryNotEmpty);
            }
            dir.remove_empty_child("sessions")?;
        }
        dir.remove_file("agent.json")?;
        self.host
            .root
            .inner
            .directory
            .child("agents", false)?
            .remove_empty_child(self.agent_id.as_str())
    }
}

#[derive(Clone)]
pub struct SessionStore {
    agent: AgentStore,
    session_id: SessionId,
    mode: StorageMode,
}
impl SessionStore {
    pub fn id(&self) -> &SessionId {
        &self.session_id
    }
    pub fn mode(&self) -> StorageMode {
        self.mode
    }
    pub fn canonical_path(&self) -> PathBuf {
        self.agent
            .host
            .canonical_path()
            .join("agents")
            .join(self.agent.agent_id.as_str())
            .join("sessions")
            .join(self.session_id.as_str())
    }
    pub(crate) fn directory(&self, create: bool) -> Result<Directory, StorageError> {
        if self.mode != StorageMode::Persistent {
            return Err(StorageError::InvalidStorageMode);
        }
        let agent = self.agent.directory(false)?;
        // Avoid following the Agent default reference back into this Session.
        let record: AgentRecord = read_object(&agent, "agent.json")?;
        self.agent.validate(&record)?;
        agent
            .child("sessions", create)?
            .child(self.session_id.as_str(), create)
    }
    fn validate(&self, record: &SessionRecord) -> Result<(), StorageError> {
        check_schema(record.schema_version)?;
        if record.id != self.session_id || record.agent_id != self.agent.agent_id {
            return Err(StorageError::OwnershipMismatch);
        }
        if record.data.mode != self.mode {
            return Err(StorageError::InvalidStorageMode);
        }
        if record
            .data
            .title
            .as_ref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(StorageError::InvalidObject {
                reason: "Session title must not be blank".into(),
            });
        }
        record
            .data
            .effective_config
            .validate()
            .map_err(|e| StorageError::InvalidObject {
                reason: e.to_string(),
            })?;
        Ok(())
    }
    fn load_without_agent(&self) -> Result<SessionRecord, StorageError> {
        let dir = self.directory(false)?;
        let record: SessionRecord = read_object(&dir, "session.json")?;
        self.validate(&record)?;
        // Missing histories are not silently recreated after an object exists.
        dir.open_file("history.jsonl", false, false)?;
        Ok(record)
    }
    pub fn load(&self) -> Result<SessionRecord, StorageError> {
        self.load_without_agent()
    }
    pub fn save(&self, record: &SessionRecord) -> Result<ObjectSave, StorageError> {
        self.validate(record)?;
        if self.mode == StorageMode::Memory {
            return Ok(ObjectSave {
                config_revision: record.data.config_revision,
                persistence: PersistenceState::NotApplicable,
            });
        }
        let dir = self.directory(true)?;
        if dir.contains("session.json")? {
            let old = self.load()?;
            check_revision(old.data.config_revision, record.data.config_revision)?;
            if old.data.config_revision == record.data.config_revision
                && (old.data.effective_config != record.data.effective_config
                    || old.data.title != record.data.title)
            {
                return Err(StorageError::InvalidObject {
                    reason: "configuration changed without a new revision".into(),
                });
            }
            if old.data.agent_config_revision != record.data.agent_config_revision
                || old.data.parent != record.data.parent
            {
                return Err(StorageError::InvalidObject {
                    reason: "Session creation provenance cannot change".into(),
                });
            }
            if old.data.closed.is_some() && record.data.closed != old.data.closed {
                return Err(StorageError::InvalidObject {
                    reason: "a committed Session close cannot be reopened or rewritten".into(),
                });
            }
        } else {
            // Initial publication is deliberately not a multi-file transaction.
            // The empty history is confirmed before the Session record. A later
            // error reports the actual partial creation, never fake rollback.
            let history = if dir.contains("history.jsonl")? {
                if !dir.read("history.jsonl")?.is_empty() {
                    return Err(StorageError::UncertainWrite);
                }
                dir.open_file("history.jsonl", true, false)?
            } else {
                dir.open_file("history.jsonl", true, true)?
            };
            crate::filesystem::sync_file(
                &history,
                &dir.path.join("history.jsonl"),
                "sync initial history",
            )?;
            dir.sync()?;
        }
        write_object(&dir, "session.json", record)?;
        Ok(ObjectSave {
            config_revision: record.data.config_revision,
            persistence: PersistenceState::Persisted,
        })
    }
    pub fn open_journal(&self) -> Result<SessionJournal, StorageError> {
        if self.mode == StorageMode::Memory {
            return Ok(SessionJournal::memory(self.session_id.clone()));
        }
        self.load()?;
        SessionJournal::persistent(self.clone(), self.directory(false)?)
    }
    /// Explicit deletion only. No stop, close, unload, or Drop calls this method.
    /// Existing journal writers hold a file lock, so deletion fails until their
    /// save workers have actually released the file and root handles.
    pub fn delete_closed(&self) -> Result<(), StorageError> {
        let dir = self.directory(false)?;
        let record: SessionRecord = read_object(&dir, "session.json")?;
        self.validate(&record)?;
        if record.data.closed.is_none() {
            return Err(StorageError::InvalidObject {
                reason: "Session is not closed".into(),
            });
        }
        if dir
            .entries()?
            .iter()
            .any(|s| s != "session.json" && s != "history.jsonl")
        {
            return Err(StorageError::DirectoryNotEmpty);
        }
        if dir.contains("history.jsonl")? {
            let history = dir.open_file("history.jsonl", true, false)?;
            crate::journal::lock_history(&history, &dir.path)?;
            dir.remove_file("history.jsonl")?;
        }
        dir.remove_file("session.json")?;
        self.agent
            .directory(false)?
            .child("sessions", false)?
            .remove_empty_child(self.session_id.as_str())
    }
}

fn list_ids<I: std::str::FromStr>(dir: &Directory) -> Result<Vec<I>, StorageError> {
    dir.entries()?
        .into_iter()
        .map(|name| {
            let id = name.parse().map_err(|_| StorageError::UnsafePath {
                reason: "object directory name is not a canonical ID",
            })?;
            // Never open or follow a child while enumerating its owner. A
            // canonical ID names the fault target even when its entry is a
            // symlink, non-directory or unsafe directory. Actual object reads
            // and writes still require Directory::child's no-follow checks.
            Ok(id)
        })
        .collect()
}
fn check_revision(old: ConfigRevision, new: ConfigRevision) -> Result<(), StorageError> {
    if new < old {
        return Err(StorageError::InvalidObject {
            reason: "configuration revision would move backwards".into(),
        });
    }
    Ok(())
}
pub(crate) fn check_schema(version: u32) -> Result<(), StorageError> {
    if version != SCHEMA_VERSION {
        Err(StorageError::UnsupportedSchema {
            found: u64::from(version),
            supported: SCHEMA_VERSION,
            position: None,
        })
    } else {
        Ok(())
    }
}
fn read_object<T: DeserializeOwned + Serialize>(
    dir: &Directory,
    name: &str,
) -> Result<T, StorageError> {
    let value = strict_json::value(&dir.read_metadata(name)?)
        .map_err(|reason| StorageError::InvalidObject { reason })?;
    let version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| StorageError::InvalidObject {
            reason: "missing or invalid schema_version".into(),
        })?;
    if version != u64::from(SCHEMA_VERSION) {
        return Err(StorageError::UnsupportedSchema {
            found: version,
            supported: SCHEMA_VERSION,
            position: None,
        });
    }
    strict_json::decode(value).map_err(|reason| StorageError::InvalidObject { reason })
}
fn write_object<T: Serialize + DeserializeOwned>(
    dir: &Directory,
    name: &str,
    record: &T,
) -> Result<(), StorageError> {
    let bytes = serde_json::to_vec(record).map_err(|e| StorageError::InvalidObject {
        reason: e.to_string(),
    })?;
    let value =
        strict_json::value(&bytes).map_err(|reason| StorageError::InvalidObject { reason })?;
    let _: T =
        strict_json::decode(value).map_err(|reason| StorageError::InvalidObject { reason })?;
    dir.atomic_write(name, &bytes)
}
