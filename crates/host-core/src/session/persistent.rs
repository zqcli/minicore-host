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
        reopened: bool,
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
        *mutable
            .root_lease
            .get_mut()
            .expect("Session lease mutex poisoned") = Some(Arc::new(store.clone()));
        *mutable
            .store
            .get_mut()
            .expect("Session store mutex poisoned") = Some(store);
        let state = mutable.state.get_mut().expect("Session mutex poisoned");
        state.record_base = Some(record.clone());
        // Edit snapshots are retained observations, not independently durable
        // records. A reopened Session must not claim an empty complete list.
        state.edits_complete = !reopened;
        state.info.title = record.data.title.clone();
        state.info.config_revision = record.data.config_revision;
        state.info.agent_config_revision = record.data.agent_config_revision;
        if let Some(closed) = &record.data.closed {
            state.end_reason = Some(closed.reason.clone());
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
    pub(crate) fn storage_mode(&self) -> StorageMode {
        self.backend.mode()
    }
}

impl SessionController {
    pub(crate) fn notify_parent_on_change(mut self: Arc<Self>, changed: Arc<Notify>) -> Arc<Self> {
        Arc::get_mut(&mut self)
            .expect("unpublished Session")
            .parent_changed = Some(changed);
        self
    }
    pub(crate) fn hold_root(mut self: Arc<Self>, lease: Option<Arc<dyn Send + Sync>>) -> Arc<Self> {
        *Arc::get_mut(&mut self)
            .expect("unpublished Session")
            .root_lease
            .get_mut()
            .expect("Session lease mutex poisoned") = lease;
        self
    }
}
