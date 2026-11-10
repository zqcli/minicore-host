//! The only production bridge: owner-minted capabilities, never connections.
use crate::*;
use minicore_host_core as core;
use minicore_host_protocol::*;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

pub struct CoreTurnPort {
    capability: core::TurnCapability,
    source: ToolSource,
}
impl CoreTurnPort {
    pub fn prepared(binding: core::PreparedTurnBinding) -> Arc<Self> {
        Arc::new(Self {
            source: binding.source().clone(),
            capability: binding.capability(),
        })
    }
    fn check_host(&self, host: &HostId) -> Result<(), Fault> {
        if host == &self.source.host_id {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::HostOwnershipMismatch,
                "target is not this bound Host",
            ))
        }
    }
    async fn inspect_actual(&self, request: InspectAction) -> Result<Value, Fault> {
        request.validate()?;
        match request.target {
            InspectTarget::Agent { host_id, agent_id } => {
                self.check_host(&host_id)?;
                let target = AgentRef { host_id, agent_id };
                let summary = self.capability.agent_summary(&target)?;
                let mut data = encoded(&summary)?;
                remove(&mut data, &["id", "host_id"]);
                if request.include_config {
                    data["config"] = encoded(self.capability.agent_snapshot(&target)?.config)?;
                }
                Ok(
                    json!({"host_id":self.source.host_id,"host_incarnation":self.source.host_incarnation,"agent_id":summary.id,"data":data}),
                )
            }
            InspectTarget::Session {
                agent_id,
                session_id,
            } => {
                let target = SessionRef {
                    agent_id,
                    session_id,
                };
                let state = self.capability.session_state(&target)?;
                let info = self.capability.session_snapshot(&target)?;
                let mut result = encoded(state)?;
                result["data"]["title"] = encoded(info.title)?;
                result["data"]["config_revision"] = encoded(info.config_revision)?;
                result["data"]["agent_config_revision"] = encoded(info.agent_config_revision)?;
                if request.include_config {
                    result["data"]["config"] = encoded(info.config)?;
                }
                if request.include_output {
                    let page = request.history_page.unwrap_or(HistoryReadRequest {
                        cursor: None,
                        limit: u64::MAX,
                    });
                    let history = self.capability.history_read(&target, &page).await?;
                    result["data"]["history"] = encoded(history.data)?;
                }
                Ok(result)
            }
            InspectTarget::Turn {
                agent_id,
                session_id,
                turn_id,
            } => {
                let target = SessionRef {
                    agent_id,
                    session_id,
                };
                let data = if request.include_output && request.output_range.is_none() {
                    // Full result reads share the public held/saved owner path.
                    // Saved-only results have no current execution metadata.
                    let result = self
                        .capability
                        .turn_result_snapshot(&target, &turn_id)
                        .await?;
                    let mut data = match result.data.info {
                        Some(info) => encoded(info)?,
                        None => json!({}),
                    };
                    if let Some(load_id) = result.source.load_id {
                        data["load_id"] = encoded(load_id)?;
                    }
                    if let Some(terminal) = result.data.saved_terminal {
                        data["saved_terminal"] = encoded(terminal)?;
                    }
                    data["history"] = encoded(result.data.history)?;
                    // Keep the tool's flat partial shape and public response index.
                    data["partials"]=json!(result.data.partials.into_iter().map(|value| json!({"response_index":value.response_index,"content":value.partial.content,"incomplete_tool_calls":value.partial.incomplete_tool_calls,"usage":value.partial.usage})).collect::<Vec<_>>());
                    data
                } else {
                    let info = self.capability.turn_info(&target, &turn_id)?;
                    let mut data = encoded(&info)?;
                    // Lightweight/current-range reads retain current execution facts.
                    remove(&mut data, &["id", "session_id", "config"]);
                    if let Some(range) = request.output_range {
                        let output = self.capability.turn_output_read(
                            &target,
                            &turn_id,
                            request.response_index.expect("validated model range"),
                            &range,
                        )?;
                        data["output"] = encoded(output.data)?;
                        data["response_index"] = encoded(output.source.response_index)?;
                    }
                    data
                };
                Ok(
                    json!({"host_id":self.source.host_id,"host_incarnation":self.source.host_incarnation,"agent_id":target.agent_id,"session_id":target.session_id,"turn_id":turn_id,"data":data}),
                )
            }
            InspectTarget::Toolcall {
                agent_id,
                session_id,
                turn_id,
                tool_call_id,
            } => {
                let target = SessionRef {
                    agent_id,
                    session_id,
                };
                let tool = ToolCallRef {
                    turn_id,
                    tool_call_id,
                };
                let mut result = encoded(self.capability.toolcall_get(&target, &tool)?)?;
                if !request.include_output {
                    remove(&mut result["data"], &["arguments"]);
                }
                if (!request.include_output || request.output_range.is_some())
                    && let Some(actual) = result["data"]["result"].as_object_mut()
                {
                    actual.remove("output");
                }
                if let Some(range) = request.output_range {
                    let output = self
                        .capability
                        .toolcall_output_read(&target, &tool, &range)?;
                    result["data"]["output"] = encoded(output.data)?;
                }
                Ok(result)
            }
            InspectTarget::Message {
                agent_id,
                session_id,
                message_id,
            } => {
                let target = SessionRef {
                    agent_id,
                    session_id,
                };
                let record = self.capability.message(&target, &message_id)?;
                let mut data = encoded(record)?;
                remove(&mut data["input"], &["message_id"]);
                if !request.include_output {
                    remove(&mut data["input"], &["text"]);
                }
                Ok(
                    json!({"host_id":self.source.host_id,"host_incarnation":self.source.host_incarnation,"agent_id":target.agent_id,"session_id":target.session_id,"message_id":message_id,"data":data}),
                )
            }
            InspectTarget::Receipt {
                host_id,
                operation_id,
            } => {
                self.check_host(&host_id)?;
                encoded(self.capability.receipt(&operation_id)?)
            }
        }
    }
}
impl TurnPort for CoreTurnPort {
    fn source(&self) -> &ToolSource {
        &self.source
    }
    fn check_active(&self) -> Result<(), Fault> {
        self.capability.check_active()
    }
    fn agent_snapshot(&self, target: &AgentRef) -> Result<AgentConfigSnapshot, Fault> {
        let snapshot = self.capability.agent_snapshot(target)?;
        Ok(AgentConfigSnapshot {
            name: snapshot.name,
            config: snapshot.config,
            config_revision: snapshot.config_revision,
        })
    }
    fn session_snapshot(&self, target: &SessionRef) -> Result<SessionConfigSnapshot, Fault> {
        let snapshot = self.capability.session_snapshot(target)?;
        Ok(SessionConfigSnapshot {
            title: snapshot.title,
            mode: snapshot.mode,
            config: snapshot.config,
            config_revision: snapshot.config_revision,
        })
    }
    fn create_agent(&self, request: AgentCreate) -> PortFuture<'_, AgentCreateReceipt> {
        Box::pin(self.capability.create_agent(request))
    }
    fn create_session(&self, request: SessionCreate) -> PortFuture<'_, SessionCreateReceipt> {
        Box::pin(self.capability.create_session(request))
    }
    fn create_agent_for_delegation(
        &self,
        request: AgentCreate,
    ) -> PortFuture<'_, CreatedForDelegation<AgentCreateReceipt>> {
        Box::pin(async move {
            let (receipt, cleanup) = self.capability.create_agent_for_delegation(request).await?;
            Ok(CreatedForDelegation {
                receipt,
                cleanup: cleanup.map(|permit| {
                    Arc::new(CoreCreationCleanup(permit)) as Arc<dyn CreationCleanup>
                }),
            })
        })
    }
    fn create_session_for_delegation(
        &self,
        request: SessionCreate,
    ) -> PortFuture<'_, CreatedForDelegation<SessionCreateReceipt>> {
        Box::pin(async move {
            let (receipt, cleanup) = self
                .capability
                .create_session_for_delegation(request)
                .await?;
            Ok(CreatedForDelegation {
                receipt,
                cleanup: cleanup.map(|permit| {
                    Arc::new(CoreCreationCleanup(permit)) as Arc<dyn CreationCleanup>
                }),
            })
        })
    }
    fn send(&self, request: SendMessage) -> PortFuture<'_, MessageAccepted> {
        Box::pin(async move { self.capability.send(request) })
    }
    fn send_delegated<'a>(
        &'a self,
        registration: &'a DelegationRegistration,
        input: MessageInput,
    ) -> PortFuture<'a, MessageAccepted> {
        Box::pin(async move { self.capability.send_delegated(registration, input) })
    }
    fn inspect(&self, request: InspectAction) -> PortFuture<'_, Value> {
        Box::pin(self.inspect_actual(request))
    }
    fn update_agent(
        &self,
        target: AgentRef,
        candidate: AgentUpdateCandidate,
    ) -> PortFuture<'_, Value> {
        Box::pin(async move {
            encoded(
                self.capability
                    .update_agent(
                        &target,
                        candidate.expected_revision,
                        candidate.config,
                        candidate.name,
                    )
                    .await?,
            )
        })
    }
    fn update_session(
        &self,
        target: SessionRef,
        candidate: SessionUpdateCandidate,
    ) -> PortFuture<'_, Value> {
        Box::pin(async move {
            encoded(
                self.capability
                    .update_session(
                        &target,
                        candidate.expected_revision,
                        candidate.config,
                        candidate.title,
                    )
                    .await?,
            )
        })
    }
    fn cancel_turn(
        &self,
        target: SessionRef,
        request: TurnCancelRequest,
    ) -> PortFuture<'_, TurnCancelReceipt> {
        Box::pin(async move { self.capability.cancel_turn(&target, request) })
    }
    fn cancel_session(
        &self,
        target: SessionRef,
        request: SessionCancelRequest,
    ) -> PortFuture<'_, SessionCancelReceipt> {
        Box::pin(async move { self.capability.cancel_session(&target, request) })
    }
    fn close_session(
        &self,
        target: SessionRef,
        request: SessionCloseRequest,
    ) -> PortFuture<'_, SessionCloseReceipt> {
        Box::pin(async move { self.capability.close_session(&target, request) })
    }
    fn wait_input(&self) -> PortFuture<'_, ()> {
        Box::pin(self.capability.wait_input())
    }
    fn prepare_delegation(
        &self,
        registration: DelegationRegistration,
        sink: Arc<dyn DelegationSink>,
    ) -> PortFuture<'_, Arc<dyn DelegationPort>> {
        Box::pin(async move {
            Ok(Arc::new(CoreDelegationPort(
                self.capability
                    .prepare_delegation(registration, Arc::new(CoreSink(sink)))?,
            )) as Arc<dyn DelegationPort>)
        })
    }
    fn prepare_scope(&self, hooks: Arc<dyn ScopeHooks>) -> Result<Arc<dyn ScopePort>, Fault> {
        Ok(Arc::new(CoreScopePort(self.capability.prepare_scope(
            Arc::new(CoreHooks(Arc::downgrade(&hooks))),
        )?)))
    }
    fn record_delegation(&self, origin: &ToolCallRef, registration: &DelegationRegistration) {
        self.capability.record_delegation(origin, registration);
    }
}
struct CoreCreationCleanup(core::CreationCleanupPermit);
impl CreationCleanup for CoreCreationCleanup {
    fn close(&self, operation: OperationId) -> PortFuture<'_, ()> {
        Box::pin(self.0.close(operation))
    }
}
struct CoreDelegationPort(core::DelegationPermit);
impl DelegationPort for CoreDelegationPort {
    fn notify(&self, input: MessageInput) -> PortFuture<'_, MessageAccepted> {
        Box::pin(self.0.notify(input))
    }
    fn release(&self, disposition: DelegationDisposition) -> PortFuture<'_, ()> {
        Box::pin(self.0.release(disposition))
    }
    fn close(&self, operation: OperationId) -> PortFuture<'_, ()> {
        Box::pin(self.0.close(operation))
    }
}
struct CoreScopePort(core::ScopePermit);
impl ScopePort for CoreScopePort {
    fn seal(&self) -> Result<(), Fault> {
        self.0.seal()
    }
    fn close_root(&self, operation: OperationId) -> PortFuture<'_, ()> {
        Box::pin(self.0.close_root(operation))
    }
}
// ScopeManager owns live scopes until root_closed. A weak hook avoids the
// Scope -> permit -> owner scope -> hook -> Scope reference cycle.
struct CoreHooks(Weak<dyn ScopeHooks>);
impl core::ScopeHooks for CoreHooks {
    fn seal(&self) {
        if let Some(scope) = self.0.upgrade() {
            scope.closing().cancel();
        }
    }
    fn close_members(&self) -> core::OwnerFuture<'_, ()> {
        Box::pin(async move {
            let scope = self.0.upgrade().ok_or_else(|| {
                Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "delegation scope owner is no longer available",
                )
            })?;
            scope.close_members().await
        })
    }
    fn root_closed(&self) {
        if let Some(scope) = self.0.upgrade() {
            scope.root_closed();
        }
    }
}
struct CoreSink(Arc<dyn DelegationSink>);
impl core::DelegationSink for CoreSink {
    fn handoff(&self, result: core::DelegationCompletion) -> Result<(), Fault> {
        self.0.handoff(result)
    }
}
fn encoded(value: impl Serialize) -> Result<Value, Fault> {
    serde_json::to_value(value)
        .map_err(|error| Fault::new(FaultKind::HostInternalUnexpected, error.to_string()))
}
fn remove(value: &mut Value, keys: &[&str]) {
    if let Some(map) = value.as_object_mut() {
        for key in keys {
            map.remove(*key);
        }
    }
}
