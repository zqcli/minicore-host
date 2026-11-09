use super::*;
use crate::persistence::disk;
use futures_util::FutureExt;
use minicore_host_storage::{AgentData, SessionData, SessionRecord};
use std::panic::AssertUnwindSafe;
use tokio::sync::oneshot;

struct PreparedSession {
    id: SessionId,
    mode: StorageMode,
    config: SessionConfig,
    agent_revision: ConfigRevision,
}
impl AgentOwner {
    pub(crate) fn begin_initial_creation(&self) {
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.creation_committed = false;
        if state.info.lifecycle == AgentLifecycle::Active {
            state.info.lifecycle = AgentLifecycle::Creating;
        }
    }
    pub(crate) fn finish_initial_creation(&self) {
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        state.creation_committed = true;
        if state.info.lifecycle == AgentLifecycle::Creating {
            state.info.lifecycle = AgentLifecycle::Active;
        }
        self.changed.notify_waiters();
    }
    pub(crate) fn creation_receipt(
        &self,
        caller: &CallerContext,
        operation: &OperationId,
    ) -> Result<SessionCreateReceipt, Fault> {
        let mut receipts = self.creation_receipts(caller, operation)?.into_iter();
        let receipt = receipts.next().ok_or_else(|| {
            Fault::new(
                FaultKind::AgentSessionNotFound,
                "no retained Session creation receipt for this operation",
            )
        })?;
        if receipts.next().is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "operation_id matches multiple invocations; use session_creation_receipts",
            ));
        }
        Ok(receipt)
    }
    pub(crate) fn creation_receipts(
        &self,
        caller: &CallerContext,
        operation: &OperationId,
    ) -> Result<Vec<SessionCreateReceipt>, Fault> {
        self.access.read(caller, || {
            Ok(self
                .state
                .lock()
                .expect("Agent mutex poisoned")
                .creations
                .values()
                .filter(|receipt| &receipt.operation_id == operation)
                .cloned()
                .collect())
        })
    }
    pub(crate) async fn create_session_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: SessionCreate,
    ) -> Result<SessionCreateReceipt, Fault> {
        let options = SessionOptions {
            mode: request.mode,
            overrides: request.overrides,
        };
        let receiver = self.access.accept_business(caller, || {
            self.accept_session_creation(
                request.operation_id,
                options,
                request.initial_message,
                false,
            )
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::AgentInternalUnexpected,
                "Session creation owner ended without a receipt",
            )
        })
    }
    pub(crate) async fn create_session_async_owned(
        self: &Arc<Self>,
        operation: OperationId,
        options: SessionOptions,
        message: Option<MessageInput>,
        make_default: bool,
    ) -> Result<SessionCreateReceipt, Fault> {
        let receiver = self.accept_session_creation(operation, options, message, make_default)?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::AgentInternalUnexpected,
                "Session creation owner ended without a receipt",
            )
        })
    }
    fn accept_session_creation(
        self: &Arc<Self>,
        operation: OperationId,
        options: SessionOptions,
        message: Option<MessageInput>,
        make_default: bool,
    ) -> Result<oneshot::Receiver<SessionCreateReceipt>, Fault> {
        self.access.ensure_execution_owned()?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "creation requires a live Tokio runtime",
            )
        })?;
        let id = SessionId::new()
            .map_err(|error| Fault::new(FaultKind::AgentInternalUnexpected, error.to_string()))?;
        let mut state = self.state.lock().expect("Agent mutex poisoned");
        if state.info.lifecycle != AgentLifecycle::Active
            && !(make_default && state.info.lifecycle == AgentLifecycle::Creating)
        {
            return Err(Fault::new(
                FaultKind::AgentLifecycleUnavailable,
                "Agent is not accepting Session creation",
            ));
        }
        let mode = state
            .info
            .mode
            .session_mode(options.mode)
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        let config =
            SessionConfig::from_agent(&state.info.config, &options.overrides.unwrap_or_default())
                .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        let prepared = PreparedSession {
            id: id.clone(),
            mode,
            config,
            agent_revision: state.info.config_revision,
        };
        let initial = SessionCreateReceipt {
            kind: SessionCreateKind::SessionCreate,
            host_id: self.host_id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: operation.clone(),
            agent_id: self.id.clone(),
            session_id: None,
            message_id: message.as_ref().map(|value| value.message_id.clone()),
            result: SessionCreateResult::without_delivery(SessionCreateSteps {
                session: CreateStep::Running,
                message: message.as_ref().map(|_| CreateStep::Pending),
            }),
        };
        state.creating += 1;
        state.executor = Some(runtime.clone());
        state.creations.insert(id.clone(), initial);
        let (sender, receiver) = oneshot::channel();
        let owner = Arc::clone(self);
        let task_id = id.clone();
        let task = runtime.spawn(async move {
            let result = AssertUnwindSafe(owner.create_session_work(
                &task_id,
                prepared,
                message,
                make_default,
            ))
            .catch_unwind()
            .await;
            let result = result.unwrap_or_else(|_| {
                Err(Fault::new(
                    FaultKind::AgentInternalUnexpected,
                    "accepted Session creation failed unexpectedly",
                ))
            });
            let mut state = owner.state.lock().expect("Agent mutex poisoned");
            let receipt = state
                .creations
                .get_mut(&task_id)
                .expect("accepted creation registered");
            if let Err(error) = result {
                let mut steps = receipt.result.steps.clone();
                if !steps.session.succeeded() {
                    steps.session = CreateStep::Failed { error };
                    if steps.message.is_some() {
                        steps.message = Some(CreateStep::NotAttempted);
                    }
                } else if steps.message.is_some() {
                    steps.message = Some(CreateStep::Failed { error });
                }
                receipt.result = SessionCreateResult::without_delivery(steps);
            }
            let receipt = receipt.clone();
            state.creating -= 1;
            state.create_tasks.remove(&task_id);
            owner.changed.notify_waiters();
            drop(state);
            drop(owner);
            let _ = sender.send(receipt);
        });
        state.create_tasks.insert(id, task);
        Ok(receiver)
    }
    async fn create_session_work(
        self: &Arc<Self>,
        work_id: &SessionId,
        prepared: PreparedSession,
        message: Option<MessageInput>,
        make_default: bool,
    ) -> Result<(), Fault> {
        self.access.ensure_execution_owned()?;
        self.resources_owned()?.validate(&prepared.config)?;
        self.access.ensure_execution_owned()?;
        let session = if prepared.mode == StorageMode::Persistent {
            let store = self
                .store()
                .as_ref()
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::HostStorageUnavailable,
                        "Agent has no persistent storage",
                    )
                })?
                .session(prepared.id.clone(), prepared.mode)
                .map_err(|error| {
                    Fault::new(FaultKind::SessionHistoryWriteFailed, error.to_string())
                })?;
            let record = SessionRecord::new(
                prepared.id.clone(),
                self.id.clone(),
                SessionData {
                    mode: prepared.mode,
                    agent_config_revision: prepared.agent_revision,
                    config_revision: 0,
                    effective_config: prepared.config,
                    parent: None,
                    closed: None,
                },
            );
            let save_store = store.clone();
            let save_record = record.clone();
            disk(FaultKind::SessionHistoryWriteFailed, move || {
                save_store.save(&save_record)
            })
            .await?;
            SessionController::from_record(
                record,
                store,
                self.host_id.clone(),
                self.incarnation.clone(),
                Arc::clone(&self.access),
                self.resources_owned()?,
                Arc::clone(&self.enabled),
            )
        } else {
            SessionController::new(
                prepared.id.clone(),
                self.id.clone(),
                self.host_id.clone(),
                self.incarnation.clone(),
                prepared.config,
                Arc::clone(&self.access),
                self.resources_owned()?,
                Arc::new(MemoryHistory),
                Vec::new(),
                Arc::clone(&self.enabled),
            )
        };
        let session = session
            .hold_root(self.root_lease())
            .notify_parent_on_change(Arc::clone(&self.changed));
        session.set_source_revision(prepared.agent_revision);
        if !self.access.execution_allowed_owned() {
            session.begin_host_shutdown_owned(true);
        }
        // Default publication is ordered after Session files, and before exposing
        // the default Session. A failed phase retains actual disk artifacts; it
        // does not publish a usable reference or invent a rollback.
        if make_default && let Some(store) = self.store() {
            let info = self
                .state
                .lock()
                .expect("Agent mutex poisoned")
                .info
                .clone();
            let record = AgentRecord::new(
                self.id.clone(),
                self.host_id.clone(),
                AgentData {
                    name: info.name,
                    mode: info.mode,
                    config: info.config,
                    config_revision: info.config_revision,
                    default_session_id: (prepared.mode == StorageMode::Persistent)
                        .then(|| prepared.id.clone()),
                },
            );
            let store = store.clone();
            let saved = record.clone();
            disk(FaultKind::HostStorageUnavailable, move || {
                store.save(&saved)
            })
            .await?;
            self.state.lock().expect("Agent mutex poisoned").record_base = Some(record);
        }
        {
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            // A stop accepted after creation does not erase the accepted object.
            // It is published disabled, and initial input is then truthfully refused.
            state
                .sessions
                .insert(prepared.id.clone(), Arc::clone(&session));
            if make_default {
                state.info.default_session_id = Some(prepared.id.clone());
            }
            let receipt = state
                .creations
                .get_mut(work_id)
                .expect("registered creation");
            receipt.session_id = Some(prepared.id);
            receipt.result.steps.session = CreateStep::Succeeded;
            if receipt.result.steps.message.is_some() {
                receipt.result.steps.message = Some(CreateStep::Running);
            }
        }
        if let Some(message) = message {
            let delivered = session.send_owned(message);
            let mut state = self.state.lock().expect("Agent mutex poisoned");
            let receipt = state
                .creations
                .get_mut(work_id)
                .expect("registered creation");
            receipt.result = match delivered {
                Ok(accepted) => SessionCreateResult::accepted(
                    SessionCreateSteps {
                        session: CreateStep::Succeeded,
                        message: Some(CreateStep::Succeeded),
                    },
                    accepted.result,
                )
                .expect("accepted creation"),
                Err(error) => SessionCreateResult::without_delivery(SessionCreateSteps {
                    session: CreateStep::Succeeded,
                    message: Some(CreateStep::Failed { error }),
                }),
            };
        }
        self.changed.notify_waiters();
        Ok(())
    }
}
