use super::*;
use crate::backend::JournalHistory;

impl SessionController {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_record(
        record: SessionRecord,
        store: SessionStore,
        host_id: HostId,
        incarnation: HostIncarnation,
        access: Arc<ConnectionRegistry>,
        resources: Arc<dyn ResourceResolver>,
        enabled: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let backend: Arc<dyn HistoryBackend> = Arc::new(JournalHistory::new(store.clone()));
        let mut owner = Self::new(
            record.id.clone(),
            record.agent_id.clone(),
            host_id,
            incarnation,
            record.data.effective_config.clone(),
            access,
            resources,
            backend,
            Vec::new(),
            enabled,
        );
        let mutable = Arc::get_mut(&mut owner).expect("new unpublished Session");
        mutable.root_lease = Some(Arc::new(store.clone()));
        mutable.store = Some(store);
        mutable.provenance = Some(record.clone());
        let state = mutable.state.get_mut().expect("Session mutex poisoned");
        state.info.config_revision = record.data.config_revision;
        if record.data.closed.is_some() {
            state.lifecycle = SessionLifecycle::Closed;
        }
        mutable.snapshots.send_replace(Self::make_snapshot(
            state,
            &mutable.host_id,
            &mutable.incarnation,
            &mutable.agent_id,
            &mutable.id,
        ));
        owner
    }
}

impl SessionController {
    pub(crate) fn hold_root(mut self: Arc<Self>, lease: Option<Arc<dyn Send + Sync>>) -> Arc<Self> {
        Arc::get_mut(&mut self)
            .expect("unpublished Session")
            .root_lease = lease;
        self
    }
}
