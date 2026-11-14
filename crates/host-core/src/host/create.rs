use super::*;
use crate::persistence::disk;
use futures_util::FutureExt;
use minicore_host_diagnostics::Context;
use minicore_host_storage::{AgentData, AgentRecord};
use std::panic::AssertUnwindSafe;
use tokio::sync::oneshot;
use tracing::{Instrument, instrument::WithSubscriber};

impl Host {
    /// Canonical asynchronous creation path for either storage mode. Once the
    /// owner accepts the request, dropping this wait does not stop the work.
    pub async fn create_agent_async(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: AgentCreate,
    ) -> Result<AgentCreateReceipt, Fault> {
        let context = Context {
            operation_id: Some(request.operation_id.clone()),
            message_id: request
                .initial_message
                .as_ref()
                .map(|input| input.message_id.clone()),
            in_reply_to: request
                .initial_message
                .as_ref()
                .and_then(|input| input.in_reply_to.clone()),
            ..self.diagnostic_context()
        };
        let mode = request.mode;
        let result = self
            .accept_agent_creation(Some(caller), request, context.clone(), None)
            .await;
        if let Err(fault) = &result {
            self.creation_failure_record(context, mode, "admission", fault, false)
                .emit();
        }
        result
    }
    pub(super) async fn create_configured_agent_owned(
        self: &Arc<Self>,
        definition: minicore_host_config::AgentDefinition,
        operation: OperationId,
    ) -> Result<AgentCreateReceipt, Fault> {
        let context = Context {
            operation_id: Some(operation.clone()),
            ..self.diagnostic_context()
        };
        let source = AgentConfigurationSource {
            key: definition.key,
            managed: true,
        };
        self.accept_agent_creation(
            None,
            AgentCreate {
                operation_id: operation,
                name: definition.name,
                mode: StorageMode::Persistent,
                config: definition.config,
                default_session: None,
                initial_message: None,
            },
            context,
            Some(source),
        )
        .await
    }
    async fn accept_agent_creation(
        self: &Arc<Self>,
        caller: Option<&CallerContext>,
        request: AgentCreate,
        context: Context,
        configuration_source: Option<AgentConfigurationSource>,
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
        // Create the short operation span before entering the admission lock.
        let mut span = Some(context.span());
        let subscriber = tracing::dispatcher::get_default(Clone::clone);
        let admission = || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            let directory_owned = caller.is_none() && configuration_source.is_some();
            if state.lifecycle != HostLifecycle::Running
                && !(directory_owned
                    && matches!(
                        state.lifecycle,
                        HostLifecycle::Starting | HostLifecycle::Draining | HostLifecycle::Stopping
                    ))
            {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host is not accepting new Agents",
                ));
            }
            state.creations.insert(id.clone(), initial);
            state.creating += 1;
            let owner = Arc::clone(self);
            let task_id = id.clone();
            let mode = request.mode;
            let task = runtime.spawn(
                async move {
                    let declaration_key = configuration_source
                        .as_ref()
                        .map(|source| source.key.clone());
                    let mut phase = "agent_creation";
                    let result = AssertUnwindSafe(owner.create_agent_work(
                        task_id.clone(),
                        request,
                        context.clone(),
                        configuration_source,
                        &mut phase,
                    ))
                    .catch_unwind()
                    .await;
                    let (receipt, diagnostic) = match result {
                        Ok(receipt) => receipt,
                        Err(_) => owner.fail_agent_creation(
                            &task_id,
                            mode,
                            phase,
                            context,
                            Fault::new(
                                FaultKind::HostInternalUnexpected,
                                "accepted Agent creation failed unexpectedly",
                            ),
                        ),
                    };
                    {
                        let mut state = owner.state.lock().expect("Host mutex poisoned");
                        // A failed first save may have written the source key.
                        // Retain that uncertainty in the accepted creation facts;
                        // another reload must not manufacture a second identity.
                        if let Some(key) = declaration_key
                            && receipt.agent_id.is_none()
                            && let CreateStep::Failed { error } = &receipt.result.steps.agent
                        {
                            state
                                .configuration_creation_failures
                                .insert(key, error.clone());
                        }
                        state.creations.insert(task_id.clone(), receipt.clone());
                    }
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
                    crate::diagnostics::emit(diagnostic);
                    let _ = sender.send(receipt);
                }
                .instrument(span.take().expect("accepted operation span"))
                .with_subscriber(subscriber),
            );
            state.create_tasks.insert(id, task);
            Ok(())
        };
        if let Some(caller) = caller {
            self.access.accept_business(caller, admission)?;
        } else {
            self.access.accept_internal_owned(admission)?;
        }
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "creation owner ended without a receipt",
            )
        })
    }
    fn fail_agent_creation(
        &self,
        id: &AgentId,
        mode: StorageMode,
        phase: &'static str,
        context: Context,
        error: Fault,
    ) -> (AgentCreateReceipt, Option<crate::diagnostics::Record>) {
        let mut receipt = self
            .state
            .lock()
            .expect("Host mutex poisoned")
            .creations
            .get(id)
            .expect("accepted creation registered")
            .clone();
        let diagnostic = self.creation_failure_record(
            Context {
                agent_id: receipt.agent_id.clone(),
                session_id: receipt.session_id.clone(),
                ..context
            },
            mode,
            phase,
            &error,
            phase == "default_session",
        );
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
        (receipt, Some(diagnostic))
    }
    async fn create_agent_work(
        self: &Arc<Self>,
        id: AgentId,
        request: AgentCreate,
        context: Context,
        configuration_source: Option<AgentConfigurationSource>,
        phase: &mut &'static str,
    ) -> (AgentCreateReceipt, Option<crate::diagnostics::Record>) {
        let operation = request.operation_id.clone();
        let mode = request.mode;
        let configuration_creation = configuration_source.is_some();
        let created = async {
            if !configuration_creation {
                self.access.ensure_execution_owned()?;
            }
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
                        configuration_source,
                        name: request.name.clone(),
                        mode: request.mode,
                        config: request.config.clone(),
                        config_revision: 0,
                        default_session_id: None,
                    },
                );
                let save_store = store.clone();
                let save_record = record.clone();
                *phase = "agent_metadata_save";
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
            self.agent_created_record(
                Context {
                    agent_id: Some(id.clone()),
                    ..context.clone()
                },
                mode,
            )
            .emit();
            if !self.access.execution_allowed_owned() {
                agent.begin_host_shutdown_owned(true, None);
            }
            *phase = "default_session";
            let result = agent
                .create_default_session_owned(
                    operation.clone(),
                    request.default_session.unwrap_or_default(),
                    request.initial_message,
                    configuration_creation,
                )
                .await;
            agent.finish_initial_creation();
            agent.try_retire_empty_owned(Some(&operation));
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
                (receipt, None)
            }
            Err(error) => self.fail_agent_creation(&id, mode, phase, context, error),
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
