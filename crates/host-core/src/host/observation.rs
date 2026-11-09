use super::*;
use crate::observation::*;

impl Host {
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
            let (initial, positions) = session.observation_baseline(turn, states, outputs)?;
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
