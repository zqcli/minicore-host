use super::*;
use crate::persistence::disk;
use minicore_host_storage::DataRoot;
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
        record.data.config.validate()?;
        let candidate = prepare(record.data.config.clone()).await?;
        if candidate.config != record.data.config {
            return Err(crate::configuration::invalid(
                "resource preparation changed saved configuration values",
            ));
        }
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
        let ids = disk(FaultKind::HostStorageUnavailable, move || {
            store.list_agents()
        })
        .await?;
        for id in ids {
            let store = owner
                .store()
                .as_ref()
                .expect("persistent Host")
                .agent(id.clone(), StorageMode::Persistent);
            let result = AgentOwner::load_persistent(
                store,
                owner.id.clone(),
                owner.incarnation.clone(),
                Arc::clone(&owner.access),
                owner.resources_owned()?,
                Arc::clone(&owner.changed),
            )
            .await;
            let mut state = owner.state.lock().expect("Host mutex poisoned");
            match result {
                Ok(agent) => {
                    state.agents.insert(id, agent);
                }
                Err(fault) => {
                    state.load_failures.insert(id, fault);
                }
            }
        }
        owner.state.lock().expect("Host mutex poisoned").lifecycle = HostLifecycle::Running;
        owner
            .lifecycle_record("host.started", "running", StorageMode::Persistent, None)
            .emit();
        Ok(owner)
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
