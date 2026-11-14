use super::*;
use crate::persistence::disk;

impl AgentOwner {
    pub(crate) async fn load_persistent(
        store: AgentStore,
        host_id: HostId,
        incarnation: HostIncarnation,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
        changed: Arc<tokio::sync::Notify>,
    ) -> Result<Arc<Self>, Fault> {
        let source = store.clone();
        let record = disk(FaultKind::AgentConfigInvalid, move || source.load_record()).await?;
        let owner = Self::from_record(record, store.clone(), incarnation, access, resources)
            .with_change_signal(changed);
        let source = store.clone();
        let ids = disk(FaultKind::HostStorageUnavailable, move || {
            source.list_sessions()
        })
        .await?;
        for id in ids {
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
                    configuration_source: record.data.configuration_source,
                    deletion: None,
                    id: record.id,
                    host_id: record.host_id,
                    name: record.data.name,
                    mode: record.data.mode,
                    lifecycle: AgentLifecycle::Active,
                    config_revision: record.data.config_revision,
                    config: record.data.config,
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
