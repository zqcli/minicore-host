use super::*;
use crate::persistence::disk;
use minicore_host_config::AgentDefinition;
use minicore_host_storage::AgentConfiguration;

pub(crate) struct PersistentAgent {
    pub record: AgentRecord,
    pub definition: Option<AgentDefinition>,
    pub sessions: Vec<SessionId>,
}

impl AgentOwner {
    pub(crate) async fn load_persistent(
        store: AgentStore,
        saved: PersistentAgent,
        incarnation: HostIncarnation,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
        changed: Arc<tokio::sync::Notify>,
    ) -> Result<Arc<Self>, Fault> {
        let mut record = saved.record;
        let values = match &record.data.configuration {
            AgentConfiguration::Inline { name, config } => (name.clone(), config.clone()),
            AgentConfiguration::File { key, .. } => {
                let definition = saved
                    .definition
                    .filter(|item| &item.key == key)
                    .ok_or_else(|| {
                        Fault::new(
                            FaultKind::AgentConfigInvalid,
                            format!("missing Agent declaration for key {key}"),
                        )
                    })?;
                let configuration = AgentConfiguration::from_values(
                    definition.name.clone(),
                    definition.config.clone(),
                    record.data.configuration.source(),
                )
                .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
                if record.data.configuration != configuration {
                    record.data.configuration = configuration;
                    record.data.config_revision =
                        record.data.config_revision.checked_add(1).ok_or_else(|| {
                            Fault::new(
                                FaultKind::AgentInternalUnexpected,
                                "configuration revision exhausted",
                            )
                        })?;
                    let source = store.clone();
                    let candidate = record.clone();
                    disk(FaultKind::HostStorageUnavailable, move || {
                        source.save(&candidate)
                    })
                    .await?;
                }
                (definition.name, definition.config)
            }
        };
        let host_id = record.host_id.clone();
        let owner = Self::from_record(
            record,
            values,
            store.clone(),
            incarnation,
            access,
            resources,
        )
        .with_change_signal(changed);
        for id in saved.sessions {
            let child_store =
                store
                    .session(id.clone(), StorageMode::Persistent)
                    .map_err(|error| {
                        Fault::new(FaultKind::SessionHistoryLoadFailed, error.to_string())
                    })?;
            let source = child_store.clone();
            let loaded = disk(FaultKind::SessionHistoryLoadFailed, move || source.load()).await;
            match loaded {
                Ok(record) => {
                    let session = SessionController::from_record(
                        record,
                        child_store,
                        host_id.clone(),
                        owner.incarnation.clone(),
                        Arc::clone(&owner.access),
                        owner.resources_owned()?,
                        Arc::clone(&owner.enabled),
                        true, // Old edit snapshots were not durable.
                    )
                    .notify_parent_on_change(Arc::clone(&owner.changed));
                    owner
                        .state
                        .lock()
                        .expect("Agent mutex poisoned")
                        .sessions
                        .insert(id, Arc::clone(&session));
                    session.publish_initial();
                }
                Err(fault) => {
                    owner
                        .state
                        .lock()
                        .expect("Agent mutex poisoned")
                        .load_failures
                        .insert(id, fault);
                }
            }
        }
        Ok(owner)
    }
    pub(crate) fn from_record(
        record: AgentRecord,
        values: (String, AgentConfig),
        store: AgentStore,
        incarnation: HostIncarnation,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
    ) -> Arc<Self> {
        let record_base = record.clone();
        let root_lease: Arc<dyn Send + Sync> = Arc::new(store.clone());
        Arc::new(Self {
            id: record.id.clone(),
            host_id: record.host_id.clone(),
            incarnation,
            access,
            resources: Mutex::new(Some(resources)),
            enabled: Arc::new(AtomicBool::new(true)),
            store: Mutex::new(Some(store)),
            root_lease: Mutex::new(Some(root_lease)),
            changed: Arc::new(tokio::sync::Notify::new()),
            state: Mutex::new(AgentState {
                info: AgentInfo {
                    configuration_source: record.data.configuration.source(),
                    deletion: None,
                    id: record.id,
                    host_id: record.host_id,
                    name: values.0,
                    mode: record.data.mode,
                    lifecycle: AgentLifecycle::Active,
                    config_revision: record.data.config_revision,
                    config: values.1,
                    default_session_id: record.data.default_session_id,
                    metadata_saving: false,
                    metadata_fault: None,
                },
                sessions: BTreeMap::new(),
                load_failures: BTreeMap::new(),
                creations: BTreeMap::new(),
                public_creations: Default::default(),
                controls: BTreeMap::new(),
                create_tasks: BTreeMap::new(),
                creating: 0,
                creation_committed: true,
                resources_released: false,
                host_shutdown_strategy: None,
                retirement_joining: false,
                executor: tokio::runtime::Handle::try_current().ok(),
                stop_task: None,
                record_base: Some(record_base),
                record_work: None,
                record_task: None,
                closing: Default::default(),
                close_tasks: BTreeMap::new(),
                close_running: Default::default(),
                deleting_sessions: Default::default(),
                delete_tasks: BTreeMap::new(),
                deleting: false,
            }),
        })
    }
    pub(crate) fn load_failures(
        &self,
        caller: &CallerContext,
    ) -> Result<Vec<SessionLoadFailure>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Agent mutex poisoned")
                .load_failures
                .iter()
                .map(|(id, fault)| SessionLoadFailure {
                    session_id: id.clone(),
                    fault: fault.clone(),
                })
                .collect())
        })
    }
}
