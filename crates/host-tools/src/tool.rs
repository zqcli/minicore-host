use crate::*;
use minicore_host_protocol::*;
use minicore_host_runtime::tools::{
    Tool, ToolContext, ToolError, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput,
    ToolSpec,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct AgentTool {
    spec: ToolSpec,
    port: Arc<dyn TurnPort>,
    scopes: Arc<ScopeManager>,
}
impl AgentTool {
    /// Binding is fallible in the backend before Session claims input. This
    /// constructor does not activate a candidate Turn or acquire a connection.
    pub fn new(port: Arc<dyn TurnPort>, scopes: Arc<ScopeManager>) -> Self {
        Self {
            spec: agent_tool_spec(),
            port,
            scopes,
        }
    }
    async fn dispatch(
        &self,
        action: AgentAction,
        origin: ToolCallRef,
        context: ToolContext,
    ) -> Result<Value, ToolFailure> {
        self.port.check_active()?;
        match action {
            AgentAction::Create(action) => self
                .create(action, origin, &context)
                .await
                .map_err(Into::into),
            AgentAction::Send(request) => {
                let input = MessageInput {
                    message_id: MessageId::new().map_err(id_fault)?,
                    mode: request.mode,
                    text: request.text,
                    in_reply_to: request.in_reply_to,
                };
                let accepted = match request.target {
                    MessageTarget::Session {
                        agent_id,
                        session_id,
                    } => {
                        self.scopes
                            .send(
                                Arc::clone(&self.port),
                                DispatchContext::new(origin, &context),
                                SessionRef {
                                    agent_id,
                                    session_id,
                                },
                                input,
                                request.delegate,
                                false,
                            )
                            .await?
                    }
                    target => self.port.send(SendMessage::new(input, target)).await?,
                };
                Ok(encoded(accepted)?)
            }
            AgentAction::Inspect(request) => Ok(self.port.inspect(request).await?),
            AgentAction::Wait(request) => self.wait(request, context).await,
            AgentAction::Update(UpdateAction::Agent { target, changes }) => {
                let snapshot = self.port.agent_snapshot(&target)?;
                Ok(self
                    .port
                    .update_agent(target, snapshot.apply(&changes)?)
                    .await?)
            }
            AgentAction::Update(UpdateAction::Session { target, changes }) => {
                let snapshot = self.port.session_snapshot(&target)?;
                Ok(self
                    .port
                    .update_session(target, snapshot.apply(&changes)?)
                    .await?)
            }
            AgentAction::Control(ControlAction::CancelTurn {
                target,
                turn_id,
                keep_inbox,
            }) => Ok(encoded(
                self.port
                    .cancel_turn(
                        target,
                        TurnCancelRequest {
                            operation_id: OperationId::new().map_err(id_fault)?,
                            turn_id,
                            keep_inbox,
                        },
                    )
                    .await?,
            )?),
            AgentAction::Control(ControlAction::CancelSession { target, keep_inbox }) => {
                Ok(encoded(
                    self.port
                        .cancel_session(
                            target,
                            SessionCancelRequest {
                                operation_id: OperationId::new().map_err(id_fault)?,
                                keep_inbox,
                            },
                        )
                        .await?,
                )?)
            }
            AgentAction::Control(ControlAction::CloseSession { target }) => Ok(encoded(
                self.port
                    .close_session(
                        target,
                        SessionCloseRequest {
                            operation_id: OperationId::new().map_err(id_fault)?,
                        },
                    )
                    .await?,
            )?),
            AgentAction::Control(ControlAction::CloseScope {}) => {
                Ok(encoded(self.scopes.close_scope(self.port.as_ref())?)?)
            }
        }
    }
    async fn create(
        &self,
        action: CreateAction,
        origin: ToolCallRef,
        context: &ToolContext,
    ) -> Result<Value, Fault> {
        match action {
            CreateAction::Agent {
                target,
                name,
                mode,
                from_agent_id,
                settings,
                text,
            } => {
                if target.host_id != self.port.source().host_id {
                    return Err(Fault::new(
                        FaultKind::HostOwnershipMismatch,
                        "target is not this Host",
                    ));
                }
                let source = match from_agent_id {
                    Some(agent_id) => {
                        self.port
                            .agent_snapshot(&AgentRef {
                                host_id: target.host_id,
                                agent_id,
                            })?
                            .config
                    }
                    None => AgentConfig::default(),
                };
                let config = agent_config_from_settings(&source, &settings.unwrap_or_default())?;
                let initial = initial(text)?;
                let request = AgentCreate {
                    operation_id: OperationId::new().map_err(id_fault)?,
                    name,
                    mode,
                    config,
                    default_session: None,
                    initial_message: None,
                };
                let CreatedForDelegation {
                    mut receipt,
                    cleanup,
                } = if initial.is_some() {
                    self.port.create_agent_for_delegation(request).await?
                } else {
                    CreatedForDelegation {
                        receipt: self.port.create_agent(request).await?,
                        cleanup: None,
                    }
                };
                if let Some(input) = initial {
                    receipt.message_id = Some(input.message_id.clone());
                    let mut steps = receipt.result.steps.clone();
                    if steps.session.succeeded() {
                        let child = SessionRef {
                            agent_id: receipt.agent_id.clone().ok_or_else(missing_created_id)?,
                            session_id: receipt
                                .session_id
                                .clone()
                                .ok_or_else(missing_created_id)?,
                        };
                        match self
                            .scopes
                            .send(
                                Arc::clone(&self.port),
                                DispatchContext::new(origin, context),
                                child,
                                input,
                                true,
                                mode == StorageMode::Memory,
                            )
                            .await
                        {
                            Ok(accepted) => {
                                steps.message = Some(CreateStep::Succeeded);
                                receipt.result =
                                    AgentCreateResult::accepted(steps, accepted.result)
                                        .map_err(internal)?;
                            }
                            Err(error) => {
                                let error = cleanup_rejected(cleanup, error).await;
                                steps.message = Some(CreateStep::Failed { error });
                                receipt.result = AgentCreateResult::without_delivery(steps);
                            }
                        }
                    } else {
                        steps.message = Some(CreateStep::NotAttempted);
                        receipt.result = AgentCreateResult::without_delivery(steps);
                    }
                }
                encoded(receipt)
            }
            CreateAction::Session {
                target,
                mode,
                settings,
                text,
            } => {
                let initial = initial(text)?;
                let request = SessionCreate {
                    operation_id: OperationId::new().map_err(id_fault)?,
                    target,
                    mode,
                    overrides: settings.map(SessionOverrides),
                    initial_message: None,
                };
                let CreatedForDelegation {
                    mut receipt,
                    cleanup,
                } = if initial.is_some() {
                    self.port.create_session_for_delegation(request).await?
                } else {
                    CreatedForDelegation {
                        receipt: self.port.create_session(request).await?,
                        cleanup: None,
                    }
                };
                if let Some(input) = initial {
                    receipt.message_id = Some(input.message_id.clone());
                    let mut steps = receipt.result.steps.clone();
                    if steps.session.succeeded() {
                        let child = SessionRef {
                            agent_id: receipt.agent_id.clone(),
                            session_id: receipt
                                .session_id
                                .clone()
                                .ok_or_else(missing_created_id)?,
                        };
                        let temporary = cleanup.is_some();
                        match self
                            .scopes
                            .send(
                                Arc::clone(&self.port),
                                DispatchContext::new(origin, context),
                                child,
                                input,
                                true,
                                temporary,
                            )
                            .await
                        {
                            Ok(accepted) => {
                                steps.message = Some(CreateStep::Succeeded);
                                receipt.result =
                                    SessionCreateResult::accepted(steps, accepted.result)
                                        .map_err(internal)?;
                            }
                            Err(error) => {
                                let error = cleanup_rejected(cleanup, error).await;
                                steps.message = Some(CreateStep::Failed { error });
                                receipt.result = SessionCreateResult::without_delivery(steps);
                            }
                        }
                    } else {
                        steps.message = Some(CreateStep::NotAttempted);
                        receipt.result = SessionCreateResult::without_delivery(steps);
                    }
                }
                encoded(receipt)
            }
        }
    }
    async fn wait(&self, request: WaitAction, context: ToolContext) -> Result<Value, ToolFailure> {
        let business_deadline = request
            .timeout_ms
            .map(|ms| {
                Instant::now()
                    .checked_add(Duration::from_millis(ms))
                    .ok_or(ToolFailure::Invalid)
            })
            .transpose()?;
        let deadline =
            business_deadline.map_or(context.deadline, |business| business.min(context.deadline));
        tokio::select! {
            biased;
            _=context.cancellation.cancelled()=>Err(ToolFailure::Cancelled),
            _=tokio::time::sleep_until(deadline.into())=>{
                if business_deadline.is_some_and(|business|business<=context.deadline) { Ok(json!({"status":"timed_out"})) }
                else { Err(ToolFailure::TimedOut) }
            },
            result=self.port.wait_input()=>{result?;Ok(json!({"status":"input_available"}))}
        }
    }
}
impl Tool for AgentTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, invocation: ToolInvocation, context: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            if invocation.tool_name() != self.spec.name() {
                return Err(ToolError::InvalidInvocation);
            }
            if context.cancellation.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            if Instant::now() >= context.deadline {
                return Err(ToolError::TimedOut);
            }
            let action = AgentAction::parse(invocation.arguments().clone())
                .map_err(|_| ToolError::InvalidInvocation)?;
            let origin = ToolCallRef {
                turn_id: self.port.source().turn.turn_id.clone(),
                tool_call_id: minicore_host_protocol::ToolCallId(
                    invocation.tool_call_id().as_str().to_owned(),
                ),
            };
            let startup = matches!(
                &action,
                AgentAction::Create(
                    CreateAction::Agent { text: Some(_), .. }
                        | CreateAction::Session { text: Some(_), .. }
                ) | AgentAction::Send(_)
            );
            let result = if startup {
                let owned = self.clone();
                tokio::spawn(async move { owned.dispatch(action, origin, context).await })
                    .await
                    .map_err(|_| ToolError::Internal)?
            } else {
                self.dispatch(action, origin, context).await
            };
            let value = match result {
                Ok(value) => value,
                Err(ToolFailure::Fault(error)) => json!({"error":error}),
                Err(ToolFailure::Cancelled) => return Err(ToolError::Cancelled),
                Err(ToolFailure::TimedOut) => return Err(ToolError::TimedOut),
                Err(ToolFailure::Invalid) => return Err(ToolError::InvalidInvocation),
            };
            let output =
                ToolOutput::new(serde_json::to_string(&value).map_err(|_| ToolError::Internal)?)
                    .map_err(|_| ToolError::Internal)?;
            Ok(ToolExecutionOutcome::Completed(output))
        })
    }
}
enum ToolFailure {
    Fault(Fault),
    Cancelled,
    TimedOut,
    Invalid,
}
impl From<Fault> for ToolFailure {
    fn from(error: Fault) -> Self {
        Self::Fault(error)
    }
}
fn initial(text: Option<String>) -> Result<Option<MessageInput>, Fault> {
    text.map(|text| {
        Ok(MessageInput {
            message_id: MessageId::new().map_err(id_fault)?,
            mode: MessageMode::FollowUp,
            text,
            in_reply_to: None,
        })
    })
    .transpose()
}
fn encoded(value: impl Serialize) -> Result<Value, Fault> {
    serde_json::to_value(value).map_err(internal)
}
fn internal(error: impl std::fmt::Display) -> Fault {
    Fault::new(FaultKind::HostInternalUnexpected, error.to_string())
}
fn id_fault(error: IdGenerationError) -> Fault {
    internal(error)
}
fn missing_created_id() -> Fault {
    internal("successful creation did not return the actual object ID")
}

async fn cleanup_rejected(cleanup: Option<Arc<dyn CreationCleanup>>, error: Fault) -> Fault {
    if let Some(cleanup) = cleanup {
        let result = match OperationId::new() {
            Ok(operation) => cleanup.close(operation).await,
            Err(error) => Err(id_fault(error)),
        };
        if let Err(cleanup_error) = result {
            return internal(format!(
                "task was not accepted ({error}); temporary cleanup failed ({cleanup_error})"
            ));
        }
    }
    error
}
