use super::*;
use crate::persistence::disk;
use futures_util::FutureExt;
use minicore_host_storage::{AgentData, AgentRecord};
use std::panic::AssertUnwindSafe;
use tokio::sync::oneshot;

impl Host {
    /// Canonical asynchronous creation path for either storage mode. Once the
    /// owner accepts the request, dropping this wait does not stop the work.
    pub async fn create_agent_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: AgentCreate,
    ) -> Result<AgentCreateReceipt, Fault> {
        request
            .validate()
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        if request.mode == StorageMode::Persistent && self.store().is_none() {
            return Err(Fault::new(
                FaultKind::HostStorageUnavailable,
                "persistent Agents require a persistent Host data root",
            ));
        }
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "creation requires a live Tokio runtime",
            )
        })?;
        let resources = self.resources_owned()?;
        let config = SessionConfig::from_agent(&request.config, &SessionOverrides::default())
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        // This extension callback remains pure validation, not I/O. Isolate its
        // synchronous work so caller deadlines and other async work can run.
        // Authority is still checked only at the acceptance boundary below.
        runtime
            .spawn_blocking(move || resources.validate(&config))
            .await
            .map_err(|_| {
                Fault::new(
                    FaultKind::HostInternalUnexpected,
                    "Agent validation task terminated unexpectedly",
                )
            })??;
        let operation = request.operation_id.clone();
        let id = AgentId::new()
            .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))?;
        let initial = AgentCreateReceipt {
            kind: AgentCreateKind::AgentCreate,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id: operation.clone(),
            agent_id: None,
            session_id: None,
            message_id: request
                .initial_message
                .as_ref()
                .map(|value| value.message_id.clone()),
            result: AgentCreateResult::without_delivery(AgentCreateSteps {
                agent: CreateStep::Running,
                session: CreateStep::Pending,
                message: request
                    .initial_message
                    .as_ref()
                    .map(|_| CreateStep::Pending),
            }),
        };
        let (sender, receiver) = oneshot::channel();
        self.access.accept_business(caller, || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            if state.lifecycle != HostLifecycle::Running {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host is not accepting new Agents",
                ));
            }
            state.creations.insert(id.clone(), initial);
            state.creating += 1;
            let owner = Arc::clone(self);
            let task_id = id.clone();
            let task = runtime.spawn(async move {
                let result = AssertUnwindSafe(owner.create_agent_work(task_id.clone(), request))
                    .catch_unwind()
                    .await;
                let receipt = match result {
                    Ok(receipt) => receipt,
                    Err(_) => owner.fail_agent_creation(
                        &task_id,
                        Fault::new(
                            FaultKind::HostInternalUnexpected,
                            "accepted Agent creation failed unexpectedly",
                        ),
                    ),
                };
                owner
                    .state
                    .lock()
                    .expect("Host mutex poisoned")
                    .creations
                    .insert(task_id.clone(), receipt.clone());
                {
                    let mut state = owner.state.lock().expect("Host mutex poisoned");
                    state.create_tasks.remove(&task_id);
                    state.creating -= 1;
                }
                owner.changed.notify_waiters();
                owner.remove_retired_agents_owned();
                // Returning the final receipt also retires this owned creation
                // task. Its temporary root lease must not race an immediate
                // drop/reopen by a caller that has awaited the result.
                drop(owner);
                let _ = sender.send(receipt);
            });
            state.create_tasks.insert(id, task);
            Ok(())
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "creation owner ended without a receipt",
            )
        })
    }
    fn fail_agent_creation(&self, id: &AgentId, error: Fault) -> AgentCreateReceipt {
        let mut receipt = self
            .state
            .lock()
            .expect("Host mutex poisoned")
            .creations
            .get(id)
            .expect("accepted creation registered")
            .clone();
        let mut steps = receipt.result.steps.clone();
        if !steps.agent.succeeded() {
            steps.agent = CreateStep::Failed { error };
            steps.session = CreateStep::NotAttempted;
        } else if !steps.session.succeeded() {
            steps.session = CreateStep::Failed { error };
        } else if steps.message.is_some() {
            steps.message = Some(CreateStep::Failed { error });
        }
        if !steps.session.succeeded() && steps.message.is_some() {
            steps.message = Some(CreateStep::NotAttempted);
        }
        receipt.result = AgentCreateResult::without_delivery(steps);
        receipt
    }
    async fn create_agent_work(
        self: &Arc<Self>,
        id: AgentId,
        request: AgentCreate,
    ) -> AgentCreateReceipt {
        let operation = request.operation_id.clone();
        let created = async {
            self.access.ensure_execution_owned()?;
            let agent = if request.mode == StorageMode::Persistent {
                let store = self
                    .store()
                    .as_ref()
                    .expect("checked persistent Host")
                    .agent(id.clone(), request.mode);
                let record = AgentRecord::new(
                    id.clone(),
                    self.id.clone(),
                    AgentData {
                        name: request.name.clone(),
                        mode: request.mode,
                        config: request.config.clone(),
                        config_revision: 0,
                        default_session_id: None,
                    },
                );
                let save_store = store.clone();
                let save_record = record.clone();
                disk(FaultKind::HostStorageUnavailable, move || {
                    save_store.save(&save_record)
                })
                .await?;
                AgentOwner::from_record(
                    record,
                    store,
                    self.incarnation.clone(),
                    Arc::clone(&self.access),
                    self.resources_owned()?,
                )
            } else {
                AgentOwner::new(
                    id.clone(),
                    self.id.clone(),
                    self.incarnation.clone(),
                    request.name.clone(),
                    request.config.clone(),
                    Arc::clone(&self.access),
                    self.resources_owned()?,
                )
            };
            let agent = agent
                .with_change_signal(Arc::clone(&self.changed))
                .hold_root(
                    self.store()
                        .as_ref()
                        .map(|store| Arc::new(store.clone()) as Arc<dyn Send + Sync>),
                );
            agent.begin_initial_creation();
            {
                let mut state = self.state.lock().expect("Host mutex poisoned");
                state.agents.insert(id.clone(), Arc::clone(&agent));
                let receipt = state.creations.get_mut(&id).expect("registered creation");
                receipt.agent_id = Some(id.clone());
                receipt.result.steps.agent = CreateStep::Succeeded;
                receipt.result.steps.session = CreateStep::Running;
            }
            if !self.access.execution_allowed_owned() {
                agent.begin_host_shutdown_owned(true);
            }
            let result = agent
                .create_session_async_owned(
                    operation.clone(),
                    request.default_session.unwrap_or_default(),
                    request.initial_message,
                    true,
                )
                .await;
            agent.finish_initial_creation();
            agent.try_retire_empty_owned();
            result
        }
        .await;
        match created {
            Ok(session) => {
                let mut receipt = self
                    .state
                    .lock()
                    .expect("Host mutex poisoned")
                    .creations
                    .get(&id)
                    .expect("registered creation")
                    .clone();
                receipt.session_id = session.session_id;
                let steps = AgentCreateSteps {
                    agent: CreateStep::Succeeded,
                    session: session.result.steps.session,
                    message: session.result.steps.message,
                };
                receipt.result = if let Some(queue) = session.result.queue {
                    let mut facts = MessageAcceptedFacts::new(
                        queue,
                        session
                            .result
                            .controller_revision
                            .expect("accepted revision"),
                    );
                    facts.load_id = session.result.load_id;
                    facts.start_error = session.result.start_error;
                    AgentCreateResult::accepted(steps, facts).expect("accepted child receipt")
                } else {
                    AgentCreateResult::without_delivery(steps)
                };
                receipt
            }
            Err(error) => self.fail_agent_creation(&id, error),
        }
    }
    pub async fn create_session_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: SessionCreate,
    ) -> Result<SessionCreateReceipt, Fault> {
        self.check_host(&request.target.host_id)?;
        self.agent(caller, &request.target.agent_id)?
            .create_session_async(caller, request)
            .await
    }
    /// Reads a Session creation receipt from the Agent that actually owns it.
    pub fn session_creation_receipt(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        operation: &OperationId,
    ) -> Result<SessionCreateReceipt, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .creation_receipt(caller, operation)
    }
    /// All matching Session creation invocations, read from their direct Agent.
    pub fn session_creation_receipts(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
        operation: &OperationId,
    ) -> Result<Vec<SessionCreateReceipt>, Fault> {
        self.check_host(&target.host_id)?;
        self.agent(caller, &target.agent_id)?
            .creation_receipts(caller, operation)
    }
}
