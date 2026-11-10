use super::*;
use crate::observation::*;
use crate::{MessageSnapshot, TurnDetailsSnapshot, TurnListSnapshot, TurnResultSnapshot};

impl Host {
    pub fn answer_interaction(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: InteractionAnswerRequest<crate::InteractionAnswerData>,
    ) -> Result<InteractionAnswerReceipt, Fault> {
        self.session(caller, target)?
            .answer_interaction(caller, request)
    }

    pub fn turn_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
    ) -> Result<TurnDetailsSnapshot, Fault> {
        self.session(caller, target)?.turn_details(caller, turn_id)
    }

    pub fn message_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        message_id: &MessageId,
    ) -> Result<MessageSnapshot, Fault> {
        self.session(caller, target)?
            .message_snapshot(caller, message_id)
    }

    pub fn turn_list(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<TurnListSnapshot, Fault> {
        self.session(caller, target)?.turn_list(caller)
    }

    pub async fn turn_result_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        self.session(caller, target)?
            .turn_result_snapshot(caller, turn_id)
            .await
    }

    pub async fn wait_turn_snapshot(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        // A saved terminal result needs no execution subscription or waiter.
        if self
            .session(caller, target)?
            .turn_query_finished(caller, turn_id)?
            .unwrap_or(true)
        {
            return self.turn_result_snapshot(caller, target, turn_id).await;
        }
        // Subscribe before checking again, so completion cannot be missed.
        // The existing subscription holds only a Weak owner and light state.
        // Dropping this wait cannot retain content or cancel accepted work.
        let mut states = self.session_states(caller, target)?;
        loop {
            if self
                .session(caller, target)?
                .turn_query_finished(caller, turn_id)?
                .unwrap_or(true)
            {
                return self.turn_result_snapshot(caller, target, turn_id).await;
            }
            states.changed().await?;
        }
    }

    pub async fn history_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: &HistoryReadRequest,
    ) -> Result<SessionHistorySnapshot, Fault> {
        self.session(caller, target)?
            .history_read(caller, request)
            .await
    }
    pub(crate) fn publish_host_state_owned(&self) {
        let control = self.access.control_state_owned();
        let state = self.state.lock().expect("Host mutex poisoned");
        self.access
            .observation
            .publish(ObservationEvent::State(StateEvent::Host {
                host_id: self.id.clone(),
                host_incarnation: self.incarnation.clone(),
                data: HostStateData {
                    lifecycle: state.lifecycle,
                    shutdown_strategy: state.shutdown_strategy,
                    shutdown_fault: state.shutdown_fault.clone(),
                    control,
                },
            }));
    }
    /// Register first, then read each direct owner's baseline under that owner's
    /// own lock. New descendants are covered by the live scope throughout.
    pub fn events_subscribe(
        &self,
        caller: &CallerContext,
        request: SubscribeRequest,
    ) -> Result<EventSubscription, Fault> {
        let scope = request.scope.clone();
        let states = request.categories.contains(&EventCategory::State);
        let outputs = request.categories.contains(&EventCategory::Output);
        let mut subscription = self.access.read(caller, || {
            self.access
                .observation
                .register(caller, &self.access, request)
        })?;
        let sessions = match &scope {
            SubscriptionScope::Host { host_id } => {
                self.check_host(host_id)?;
                if states {
                    let info = self.info(caller)?;
                    subscription.result.initial_states.push(StateEvent::Host {
                        host_id: info.host_id,
                        host_incarnation: info.host_incarnation,
                        data: HostStateData {
                            lifecycle: info.lifecycle,
                            shutdown_strategy: info.shutdown_strategy,
                            shutdown_fault: info.shutdown_fault,
                            control: info.control,
                        },
                    });
                }
                let agents = self
                    .state
                    .lock()
                    .expect("Host mutex poisoned")
                    .agents
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                let mut sessions = Vec::new();
                for agent in agents {
                    sessions.extend(agent.list_sessions(caller)?);
                }
                sessions
            }
            SubscriptionScope::Agent { host_id, agent_id } => {
                self.check_host(host_id)?;
                self.agent(caller, agent_id)?.list_sessions(caller)?
            }
            SubscriptionScope::Session {
                agent_id,
                session_id,
            } => vec![self.session(
                caller,
                &SessionRef {
                    agent_id: agent_id.clone(),
                    session_id: session_id.clone(),
                },
            )?],
            SubscriptionScope::Turn { session_id, .. } => {
                // A TurnRef has its direct Session only. Resolve that Session
                // through each Agent, never through an auxiliary global index.
                let agents = self
                    .state
                    .lock()
                    .expect("Host mutex poisoned")
                    .agents
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                let mut found = None;
                for agent in agents {
                    match agent.session(session_id) {
                        Ok(session) => {
                            found = Some(session);
                            break;
                        }
                        Err(fault) if fault.kind() == Some(FaultKind::AgentSessionNotFound) => {}
                        Err(fault) => return Err(fault),
                    }
                }
                vec![found.ok_or_else(|| {
                    Fault::new(
                        FaultKind::AgentSessionNotFound,
                        "Turn's Session is not retained",
                    )
                })?]
            }
        };
        let turn = match &scope {
            SubscriptionScope::Turn { turn_id, .. } => Some(turn_id),
            _ => None,
        };
        for session in sessions {
            let (initial, positions) = descendant_baseline(
                &session,
                turn,
                states,
                outputs,
                matches!(
                    scope,
                    SubscriptionScope::Host { .. } | SubscriptionScope::Agent { .. }
                ),
            )?;
            subscription.result.initial_states.extend(initial);
            subscription.result.output_positions.extend(positions);
        }
        self.access.read(caller, || Ok(subscription))
    }
    pub fn events_unsubscribe(
        &self,
        caller: &CallerContext,
        request: &UnsubscribeRequest,
    ) -> Result<UnsubscribeResult, Fault> {
        self.access.read(caller, || {
            self.access
                .observation
                .unsubscribe(caller.connection_number(), &request.subscription_id)
        })
    }
    pub fn turn_output_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        turn_id: &TurnId,
        response_index: u64,
        request: &TextReadRequest,
    ) -> Result<ModelTextSnapshot, Fault> {
        let session = self.session(caller, target)?;
        let info = session.turn_info(caller, turn_id)?;
        let data = session.output_read(caller, turn_id, Some(response_index), None, request)?;
        Ok(ModelTextSnapshot {
            source: ModelOutputIdentity {
                turn: TurnIdentity {
                    session: SessionIdentity {
                        host_id: self.id.clone(),
                        host_incarnation: self.incarnation.clone(),
                        agent_id: target.agent_id.clone(),
                        session_id: target.session_id.clone(),
                    },
                    turn_id: turn_id.clone(),
                    load_id: Some(info.load_id),
                },
                response_index,
            },
            data,
        })
    }
    pub fn toolcall_output_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        tool: &ToolCallRef,
        request: &TextReadRequest,
    ) -> Result<ToolTextSnapshot, Fault> {
        let session = self.session(caller, target)?;
        let (source, _) = session.tool_get(caller, &tool.turn_id, &tool.tool_call_id)?;
        let data = session.output_read(
            caller,
            &tool.turn_id,
            None,
            Some(&tool.tool_call_id),
            request,
        )?;
        Ok(ToolTextSnapshot { source, data })
    }
    pub fn toolcall_get(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        tool: &ToolCallRef,
    ) -> Result<ToolCallSnapshot, Fault> {
        let (source, data) =
            self.session(caller, target)?
                .tool_get(caller, &tool.turn_id, &tool.tool_call_id)?;
        Ok(ToolCallSnapshot { source, data })
    }
}

// Broad live scopes may race a descendant leaving after enumeration. A direct
// Session/Turn target still reports its actual missing source, and every other
// failure (including access revocation) remains a failure.
fn descendant_baseline(
    session: &crate::session::SessionController,
    turn: Option<&TurnId>,
    states: bool,
    outputs: bool,
    broad: bool,
) -> Result<(Vec<StateEvent>, Vec<OutputPosition>), Fault> {
    match session.observation_baseline(turn, states, outputs) {
        Err(fault) if broad && fault.kind() == Some(FaultKind::AgentSessionNotFound) => {
            Ok((Vec::new(), Vec::new()))
        }
        result => result,
    }
}

#[cfg(test)]
mod baseline_race_tests {
    use super::*;
    struct UnusedResources;
    impl crate::ResourceResolver for UnusedResources {
        fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
            Ok(())
        }
        fn resolve<'a>(&'a self, _: &'a SessionConfig) -> crate::ResourceFuture<'a> {
            Box::pin(async {
                Err(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "no execution requested by baseline race test",
                ))
            })
        }
    }
    #[tokio::test]
    async fn enumerated_descendant_reclaimed_before_baseline_does_not_fail_a_broad_scope() {
        let host = Host::memory(Arc::new(UnusedResources)).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let receipt = host
            .create_agent(
                &caller,
                AgentCreate {
                    operation_id: OperationId::new().unwrap(),
                    name: "baseline fixture".into(),
                    mode: StorageMode::Memory,
                    config: AgentConfig::default(),
                    default_session: None,
                    initial_message: None,
                },
            )
            .unwrap();
        let target = SessionRef {
            agent_id: receipt.agent_id.unwrap(),
            session_id: receipt.session_id.unwrap(),
        };
        // This is exactly the retained Arc from a completed enumeration.
        let enumerated = host.session(&caller, &target).unwrap();
        let mut state = enumerated.subscribe_state(&caller).unwrap();
        host.close_session(&caller, &target).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while state.snapshot().is_ok() {
                if state.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let (states, positions) = descendant_baseline(&enumerated, None, true, true, true).unwrap();
        assert!(states.is_empty() && positions.is_empty());
        assert_eq!(
            descendant_baseline(&enumerated, None, true, true, false)
                .unwrap_err()
                .kind(),
            Some(FaultKind::AgentSessionNotFound)
        );
        assert!(
            host.events_subscribe(
                &caller,
                SubscribeRequest {
                    scope: SubscriptionScope::Session {
                        agent_id: target.agent_id,
                        session_id: target.session_id
                    },
                    categories: [EventCategory::State].into_iter().collect()
                }
            )
            .is_err()
        );
    }
}
