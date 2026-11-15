use super::*;
use crate::agent::PersistentAgent;
use crate::persistence::disk;
use minicore_host_config::AgentDefinition;
use minicore_host_storage::{AgentConfiguration, AgentRecord, DataRoot, HostConfiguration};

pub(super) struct StoredAgent {
    pub id: AgentId,
    pub record: Result<AgentRecord, Fault>,
    pub sessions: Vec<SessionId>,
}

/// Discover physical records before startup publication. Session ownership is
/// read once and remains a direct-Agent responsibility after this handoff.
pub(super) async fn read_agents(store: HostStore) -> Result<Vec<StoredAgent>, Fault> {
    disk(FaultKind::HostStorageUnavailable, move || {
        let ids = store.list_agents()?;
        let mut sessions = store.discover_sessions()?;
        Ok(ids
            .into_iter()
            .map(|id| {
                let record = store
                    .agent(id.clone(), StorageMode::Persistent)
                    .load_record()
                    .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()));
                StoredAgent {
                    sessions: sessions.remove(&id).unwrap_or_default(),
                    id,
                    record,
                }
            })
            .collect())
    })
    .await
}

pub(super) fn validate_sources(
    saved: &[StoredAgent],
    definitions: Option<&[AgentDefinition]>,
) -> Result<(), Fault> {
    let mut keys = std::collections::BTreeSet::new();
    for agent in saved {
        let record = match &agent.record {
            Ok(record) => record,
            Err(fault) if definitions.is_some() => return Err(fault.clone()),
            Err(_) => continue,
        };
        if let AgentConfiguration::File { key, .. } = &record.data.configuration {
            if !keys.insert(key) {
                return Err(crate::configuration::invalid(
                    "multiple Agents claim the same declaration key",
                ));
            }
            if !definitions.is_some_and(|items| items.iter().any(|item| &item.key == key)) {
                return Err(crate::configuration::invalid(format!(
                    "missing Agent declaration for key {key}"
                )));
            }
        }
    }
    Ok(())
}
use std::path::Path;

impl Host {
    /// Embedding convenience. It uses the same complete HostConfig record as
    /// the production constructor; no legacy JSON shape is read or migrated.
    pub async fn open_persistent(
        path: impl AsRef<Path>,
        initial_settings: HostSettings,
        resources: Arc<dyn ResourceResolver>,
    ) -> Result<Arc<Self>, Fault> {
        Self::open_configured(path, initial_settings.into(), move |config| async move {
            PreparedHostConfiguration::new(config, resources, Vec::new(), None)
        })
        .await
    }

    /// Read the single saved configuration before preparing private resources.
    /// The preparation closure runs outside locks, without any executable owner
    /// or connection. Saved values win over initial defaults on reopen.
    pub async fn open_configured<F, Fut>(
        path: impl AsRef<Path>,
        initial: HostConfig,
        prepare: F,
    ) -> Result<Arc<Self>, Fault>
    where
        F: FnOnce(HostConfig) -> Fut,
        Fut: std::future::Future<Output = Result<PreparedHostConfiguration, Fault>>,
    {
        let path = path.as_ref().to_path_buf();
        let (store, record, fresh) = disk(FaultKind::HostStorageUnavailable, move || {
            let root = DataRoot::acquire(path)?;
            let (record, fresh) = match root.load_host::<HostConfig>()? {
                Some(record) => (record, false),
                None => {
                    let id = HostId::new().map_err(|error| {
                        minicore_host_storage::StorageError::InvalidObject {
                            reason: error.to_string(),
                        }
                    })?;
                    (HostRecord::new(id, initial, 0), true)
                }
            };
            Ok((root.host(record.id.clone()), record, fresh))
        })
        .await?;
        let HostConfiguration::Inline { config } = &record.data.configuration else {
            return Err(crate::configuration::invalid(
                "file-managed Host requires its configuration directory",
            ));
        };
        config.validate()?;
        let candidate = prepare(config.clone()).await?;
        if candidate.config != *config {
            return Err(crate::configuration::invalid(
                "resource preparation changed saved configuration values",
            ));
        }
        let agents = if fresh {
            Vec::new()
        } else {
            read_agents(store.clone()).await?
        };
        validate_sources(&agents, None)?;
        if fresh {
            let store = store.clone();
            let record = record.clone();
            disk(FaultKind::HostStorageUnavailable, move || {
                store.save_host(&record)
            })
            .await?;
        }
        let owner = Self::construct(
            record.id,
            record.data.config_revision,
            candidate,
            Some(store.clone()),
            HostLifecycle::Starting,
        )?;
        owner.load_persistent_agents_owned(agents, &[]).await?;
        owner.state.lock().expect("Host mutex poisoned").lifecycle = HostLifecycle::Running;
        owner
            .lifecycle_record("host.started", "running", StorageMode::Persistent, None)
            .emit();
        Ok(owner)
    }
    pub(super) async fn load_persistent_agents_owned(
        self: &Arc<Self>,
        saved: Vec<StoredAgent>,
        definitions: &[AgentDefinition],
    ) -> Result<(), Fault> {
        for saved in saved {
            let id = saved.id;
            let result = match saved.record {
                Err(fault) => Err(fault),
                Ok(record) => {
                    let definition = record.data.configuration.source().and_then(|source| {
                        definitions
                            .iter()
                            .find(|item| item.key == source.key)
                            .cloned()
                    });
                    let store = self
                        .store()
                        .expect("persistent Host")
                        .agent(id.clone(), StorageMode::Persistent);
                    AgentOwner::load_persistent(
                        store,
                        PersistentAgent {
                            record,
                            definition,
                            sessions: saved.sessions,
                        },
                        self.incarnation.clone(),
                        Arc::clone(&self.access),
                        self.resources_owned()?,
                        Arc::clone(&self.changed),
                    )
                    .await
                }
            };
            let mut state = self.state.lock().expect("Host mutex poisoned");
            match result {
                Ok(agent) => {
                    state.agents.insert(id, agent);
                }
                Err(fault) => {
                    state.load_failures.insert(id, fault);
                }
            }
        }
        Ok(())
    }
    /// Name-only embedding convenience, derived from the same committed config.
    pub fn settings(&self, caller: &CallerContext) -> Result<Option<HostSettings>, Fault> {
        self.access.read(caller, || {
            Ok(self.store().map(|_| HostSettings {
                name: self.configuration.snapshot().config.name.clone(),
            }))
        })
    }
    pub fn agent_load_failures(
        &self,
        caller: &CallerContext,
    ) -> Result<Vec<AgentLoadFailure>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Host mutex poisoned")
                .load_failures
                .iter()
                .map(|(id, fault)| AgentLoadFailure {
                    agent_id: id.clone(),
                    fault: fault.clone(),
                })
                .collect())
        })
    }
    pub fn session_load_failures(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<Vec<SessionLoadFailure>, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?.load_failures(caller)
    }
}
