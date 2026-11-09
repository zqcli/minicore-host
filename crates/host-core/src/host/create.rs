use super::*;
use crate::persistence::disk;
use futures_util::FutureExt;
use minicore_host_storage::{AgentData, AgentRecord};
use std::panic::AssertUnwindSafe;
use tokio::sync::oneshot;

impl Host {
    /// Canonical asynchronous creation path for either storage mode. Once the
    /// first poll accepts the request, dropping this wait does not stop the work.
    pub async fn create_agent_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: AgentCreate,
    ) -> Result<AgentCreateReceipt, Fault> {
        request
            .validate()
            .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?;
        if request.mode == StorageMode::Persistent && self.store.is_none() {
            return Err(Fault::new(
                FaultKind::HostStorageUnavailable,
                "persistent Agents require a persistent Host data root",
            ));
        }
        self.resources.validate(
            &SessionConfig::from_agent(&request.config, &SessionOverrides::default())
                .map_err(|error| Fault::new(FaultKind::AgentConfigInvalid, error.to_string()))?,
        )?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "creation requires a live Tokio runtime",
            )
        })?;
        let operation = request.operation_id.clone();
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
        self.access.accept(caller, || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            if state.lifecycle != HostLifecycle::Running {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host is not accepting new Agents",
                ));
            }
            state.creations.insert(operation.clone(), initial);
            let owner = Arc::clone(self);
            let task_operation = operation.clone();
            let task = runtime.spawn(async move {
                let result = AssertUnwindSafe(owner.create_agent_work(request))
                    .catch_unwind()
                    .await;
                let receipt = match result {
                    Ok(receipt) => receipt,
                    Err(_) => owner.fail_agent_creation(
                        &task_operation,
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
                    .insert(task_operation.clone(), receipt.clone());
                owner
                    .state
                    .lock()
                    .expect("Host mutex poisoned")
                    .create_tasks
                    .remove(&task_operation);
                let _ = sender.send(receipt);
            });
            state.create_tasks.insert(operation, task);
            Ok(())
        })?;
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "creation owner ended without a receipt",
            )
        })
    }
    fn fail_agent_creation(&self, operation: &OperationId, error: Fault) -> AgentCreateReceipt {
        let mut receipt = self
            .state
            .lock()
            .expect("Host mutex poisoned")
            .creations
            .get(operation)
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
    async fn create_agent_work(self: &Arc<Self>, request: AgentCreate) -> AgentCreateReceipt {
        let operation = request.operation_id.clone();
        let created = async {
            let id = AgentId::new().map_err(|error| {
                Fault::new(FaultKind::HostInternalUnexpected, error.to_string())
            })?;
            let agent = if request.mode == StorageMode::Persistent {
                let store = self
                    .store
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
                    Arc::clone(&self.resources),
                )
            } else {
                AgentOwner::new(
                    id.clone(),
                    self.id.clone(),
                    self.incarnation.clone(),
                    request.name.clone(),
                    request.config.clone(),
                    Arc::clone(&self.access),
                    Arc::clone(&self.resources),
                )
            };
            agent.begin_initial_creation();
            {
                let mut state = self.state.lock().expect("Host mutex poisoned");
                state.agents.insert(id.clone(), Arc::clone(&agent));
                let receipt = state
                    .creations
                    .get_mut(&operation)
                    .expect("registered creation");
                receipt.agent_id = Some(id);
                receipt.result.steps.agent = CreateStep::Succeeded;
                receipt.result.steps.session = CreateStep::Running;
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
                    .get(&operation)
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
            Err(error) => self.fail_agent_creation(&operation, error),
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
}
