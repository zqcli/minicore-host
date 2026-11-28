use crate::{
    AgentRecord, HostRecord, SCHEMA_VERSION, SessionJournal, SessionRecord, StorageError,
    filesystem::Directory, strict_json,
};
use minicore_host_protocol::{
    AgentId, ConfigRevision, HostId, PersistenceState, SessionId, StorageMode,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
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
            for name in ["agents", "sessions"] {
                if self.inner.directory.contains(name)?
                    && !self
                        .inner
                        .directory
                        .child(name, false)?
                        .entries()?
                        .is_empty()
                {
                    return Err(StorageError::InvalidObject {
                        reason: "stored children have no Host identity".into(),
                    });
                }
            }
            return Ok(None);
        }
        let record: HostRecord<C> = read_object(&self.inner.directory, "host.json")?;
        record.data.configuration.validate()?;
        Ok(Some(record))
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
        record.data.configuration.validate()?;
        Ok(record)
    }
    pub fn save_host<C: DeserializeOwned + Serialize>(
        &self,
        record: &HostRecord<C>,
    ) -> Result<ObjectSave, StorageError> {
        check_schema(record.schema_version)?;
        record.data.configuration.validate()?;
        if record.id != self.host_id {
            return Err(StorageError::OwnershipMismatch);
        }
        let dir = &self.root.inner.directory;
        if dir.contains("host.json")? {
            let old: HostRecord<C> = self.load_host()?;
            check_revision(old.data.config_revision, record.data.config_revision)?;
            if std::mem::discriminant(&old.data.configuration)
                != std::mem::discriminant(&record.data.configuration)
            {
                return Err(StorageError::InvalidObject {
                    reason: "Host configuration source cannot change".into(),
                });
            }
            if old.data.config_revision == record.data.config_revision {
                let before = serde_json::to_value(&old.data.configuration).map_err(|e| {
                    StorageError::InvalidObject {
                        reason: e.to_string(),
                    }
                })?;
                let after = serde_json::to_value(&record.data.configuration).map_err(|e| {
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
        self.root.inner.directory.child("agents", false)?.entries()?.into_iter().map(|name| {
            name.strip_suffix(".json").and_then(|stem| stem.parse().ok()).ok_or(StorageError::UnsafePath {
                reason: "Agent records must use <AgentId>.json; nested storage is unsupported",
            })
        }).collect()
    }
    /// Scan Session ownership once at startup. Direct Agents load their own
    /// records afterwards; no second persistent ownership index is maintained.
    pub fn discover_sessions(&self) -> Result<BTreeMap<AgentId, Vec<SessionId>>, StorageError> {
        let agents: BTreeSet<_> = self.list_agents()?.into_iter().collect();
        let mut grouped: BTreeMap<AgentId, Vec<SessionId>> = BTreeMap::new();
        if !self.root.inner.directory.contains("sessions")? {
            return Ok(grouped);
        }
        let directory = self.root.inner.directory.child("sessions", false)?;
        for id in list_ids::<SessionId>(&directory)? {
            let child = directory.child(id.as_str(), false)?;
            let value = strict_json::value(&child.read_metadata("session.json")?)
                .map_err(|reason| StorageError::InvalidObject { reason })?;
            let stored_id: SessionId = serde_json::from_value(
                value.get("id").cloned().unwrap_or_default(),
            )
            .map_err(|error| StorageError::InvalidObject {
                reason: error.to_string(),
            })?;
            let agent: AgentId =
                serde_json::from_value(value.get("agent_id").cloned().unwrap_or_default())
                    .map_err(|error| StorageError::InvalidObject {
                        reason: error.to_string(),
                    })?;
            if stored_id != id || !agents.contains(&agent) {
                return Err(StorageError::OwnershipMismatch);
            }
            grouped.entry(agent).or_default().push(id);
        }
        Ok(grouped)
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
        self.host.root.inner.directory.child("agents", create)
    }
    fn filename(&self) -> String {
        format!("{}.json", self.agent_id)
    }
    /// Read only this Agent's authoritative record. Initialization uses this
    /// boundary so a broken default Session cannot block healthy siblings. The
    /// owner initializes each child independently and keeps an unusable default
    /// unavailable; it must not silently choose another Session.
    pub fn load_record(&self) -> Result<AgentRecord, StorageError> {
        let record: AgentRecord = read_object(&self.directory(false)?, &self.filename())?;
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
        record.data.configuration.validate()?;
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
        let filename = self.filename();
        if dir.contains(&filename)? {
            let old: AgentRecord = read_object(&dir, &filename)?;
            self.validate(&old)?;
            if old.data.configuration.source() != record.data.configuration.source() {
                return Err(StorageError::InvalidObject {
                    reason: "Agent configuration source cannot change".into(),
                });
            }
            check_revision(old.data.config_revision, record.data.config_revision)?;
            if old.data.config_revision == record.data.config_revision
                && old.data.configuration != record.data.configuration
            {
                return Err(StorageError::InvalidObject {
                    reason: "configuration changed without a new revision".into(),
                });
            }
        }
        self.check_default(record)?;
        write_object(&dir, &filename, record)?;
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
    /// Explicit query for this Agent; startup uses one Host-wide discovery pass.
    pub fn list_sessions(&self) -> Result<Vec<SessionId>, StorageError> {
        self.load_record()?;
        Ok(self
            .host
            .discover_sessions()?
            .remove(&self.agent_id)
            .unwrap_or_default())
    }
    /// The direct owner has stopped the Agent and deleted its exact child scope.
    pub fn delete_empty(&self) -> Result<(), StorageError> {
        self.load()?;
        if !self.list_sessions()?.is_empty() {
            return Err(StorageError::DirectoryNotEmpty);
        }
        self.directory(false)?.remove_file(&self.filename())
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
            .join("sessions")
            .join(self.session_id.as_str())
    }
    pub(crate) fn directory(&self, create: bool) -> Result<Directory, StorageError> {
        if self.mode != StorageMode::Persistent {
            return Err(StorageError::InvalidStorageMode);
        }
        self.agent.load_record()?;
        self.agent
            .host
            .root
            .inner
            .directory
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
        if record.data.last_activity_at < record.data.created_at {
            return Err(StorageError::InvalidObject {
                reason: "Session activity precedes its creation".into(),
            });
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
    /// Initial admission reserves the Session ID with an exclusive directory
    /// creation. Even an empty existing directory is a conflict, not a retry.
    pub fn create(&self, record: &SessionRecord) -> Result<ObjectSave, StorageError> {
        self.validate(record)?;
        if self.mode == StorageMode::Memory {
            return Ok(ObjectSave {
                config_revision: record.data.config_revision,
                persistence: PersistenceState::NotApplicable,
            });
        }
        self.agent.load_record()?;
        let parent = self
            .agent
            .host
            .root
            .inner
            .directory
            .child("sessions", true)?;
        let dir = parent.create_child(self.session_id.as_str())?;
        let history = dir.open_file("history.jsonl", true, true)?;
        crate::filesystem::sync_file(
            &history,
            &dir.path.join("history.jsonl"),
            "sync initial history",
        )?;
        dir.sync()?;
        write_object(&dir, "session.json", record)?;
        Ok(ObjectSave {
            config_revision: record.data.config_revision,
            persistence: PersistenceState::Persisted,
        })
    }
    /// Update an existing object. Missing metadata is an error, never creation.
    pub fn save(&self, record: &SessionRecord) -> Result<ObjectSave, StorageError> {
        self.validate(record)?;
        if self.mode == StorageMode::Memory {
            return Ok(ObjectSave {
                config_revision: record.data.config_revision,
                persistence: PersistenceState::NotApplicable,
            });
        }
        let dir = self.directory(false)?;
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
            || old.data.created_at != record.data.created_at
        {
            return Err(StorageError::InvalidObject {
                reason: "Session creation provenance cannot change".into(),
            });
        }
        if record.data.last_activity_at < old.data.last_activity_at {
            return Err(StorageError::InvalidObject {
                reason: "Session activity cannot move backwards".into(),
            });
        }
        if old.data.closed.is_some() && record.data.closed != old.data.closed {
            return Err(StorageError::InvalidObject {
                reason: "a committed Session close cannot be reopened or rewritten".into(),
            });
        }
        write_object(&dir, "session.json", record)?;
        Ok(ObjectSave {
            config_revision: record.data.config_revision,
            persistence: PersistenceState::Persisted,
        })
    }
    /// Open the persistent history writer. Memory Sessions have no disk journal.
    pub fn open_journal(&self) -> Result<SessionJournal, StorageError> {
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
            let history = dir.open_history(true)?;
            let _history = crate::journal::lock_history(history, &dir.path)?;
            dir.remove_file("history.jsonl")?;
        }
        dir.remove_file("session.json")?;
        drop(dir);
        self.agent
            .host
            .root
            .inner
            .directory
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
