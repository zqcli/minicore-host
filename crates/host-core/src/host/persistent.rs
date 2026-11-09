use super::*;
use crate::persistence::disk;
use minicore_host_storage::DataRoot;
use std::path::Path;

impl Host {
    /// Acquire the real directory before reading or constructing any executable
    /// owner. A saved Host always uses its saved settings, not the supplied defaults.
    /// Runtime resources are explicitly assembled by the embedding application.
    pub async fn open_persistent(
        path: impl AsRef<Path>,
        initial_settings: HostSettings,
        resources: Arc<dyn ResourceResolver>,
    ) -> Result<Arc<Self>, Fault> {
        let path = path.as_ref().to_path_buf();
        let (store, settings) = disk(FaultKind::HostStorageUnavailable, move || {
            let root = DataRoot::acquire(path)?;
            let record = match root.load_host::<HostSettings>()? {
                Some(record) => record,
                None => {
                    initial_settings.validate().map_err(|_| {
                        minicore_host_storage::StorageError::InvalidObject {
                            reason: "Host name must not be blank".into(),
                        }
                    })?;
                    let id = HostId::new().map_err(|error| {
                        minicore_host_storage::StorageError::InvalidObject {
                            reason: error.to_string(),
                        }
                    })?;
                    let record = HostRecord::new(id.clone(), initial_settings, 0);
                    root.host(id).save_host(&record)?;
                    record
                }
            };
            Ok((root.host(record.id.clone()), record))
        })
        .await?;
        settings.data.config.validate()?;
        let incarnation = HostIncarnation::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        let access = ConnectionRegistry::new(settings.id.clone(), incarnation.clone());
        let owner = Arc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            id: settings.id.clone(),
            incarnation,
            access,
            resources: Mutex::new(Some(resources)),
            store: Mutex::new(Some(store.clone())),
            changed: Arc::new(tokio::sync::Notify::new()),
            settings: Some(settings),
            state: Mutex::new(HostState {
                lifecycle: HostLifecycle::Starting,
                shutdown_strategy: None,
                shutdown_fault: None,
                shutdown_task: None,
                creating: 0,
                retirement_watch: None,
                retirement_epoch: 0,
                agents: BTreeMap::new(),
                creations: BTreeMap::new(),
                controls: BTreeMap::new(),
                load_failures: BTreeMap::new(),
                create_tasks: BTreeMap::new(),
            }),
        });
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
        Ok(owner)
    }

    pub fn settings(&self, caller: &CallerContext) -> Result<Option<HostSettings>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .settings
                .as_ref()
                .map(|record| record.data.config.clone()))
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
