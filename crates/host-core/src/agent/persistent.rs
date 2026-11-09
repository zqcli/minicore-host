use super::*;
use crate::persistence::disk;

impl AgentOwner {
    pub(crate) async fn load_persistent(
        store: AgentStore,
        host_id: HostId,
        incarnation: HostIncarnation,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
    ) -> Result<Arc<Self>, Fault> {
        let source = store.clone();
        let record = disk(FaultKind::AgentConfigInvalid, move || source.load_record()).await?;
        let owner = Self::from_record(record, store.clone(), incarnation, access, resources);
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
                        Arc::clone(&owner.resources),
                        Arc::clone(&owner.enabled),
                    );
                    owner
                        .state
                        .lock()
                        .expect("Agent mutex poisoned")
                        .sessions
                        .insert(id, session);
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
    fn from_record(
        record: AgentRecord,
        store: AgentStore,
        incarnation: HostIncarnation,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: record.id.clone(),
            host_id: record.host_id.clone(),
            incarnation,
            access,
            resources,
            enabled: Arc::new(AtomicBool::new(true)),
            store: Some(store),
            state: Mutex::new(AgentState {
                info: AgentInfo {
                    id: record.id,
                    host_id: record.host_id,
                    name: record.data.name,
                    mode: record.data.mode,
                    lifecycle: AgentLifecycle::Active,
                    config_revision: record.data.config_revision,
                    config: record.data.config,
                    default_session_id: record.data.default_session_id,
                },
                sessions: BTreeMap::new(),
                load_failures: BTreeMap::new(),
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
