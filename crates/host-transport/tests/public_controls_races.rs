//! Deterministic owner-boundary races, using real Runtime and explicit gates.
mod common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::Semaphore;

fn update(revision: u64, role: &str) -> SessionUpdateRequest {
    SessionUpdateRequest {
        operation_id: OperationId::new().unwrap(),
        expected_config_revision: revision,
        changes: SessionUpdateChanges {
            title: NullablePatch::Unchanged,
            config: SessionConfigPatch {
                role: Some(Role {
                    name: role.into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        },
    }
}
fn agent_update(revision: u64, name: &str) -> AgentUpdateRequest {
    AgentUpdateRequest {
        operation_id: OperationId::new().unwrap(),
        expected_config_revision: revision,
        changes: AgentUpdateChanges {
            name: Some(name.into()),
            config: Default::default(),
        },
    }
}
fn arm(resources: &FixtureResources) -> Arc<ValidationGate> {
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((
        resources.validate_calls.load(Ordering::SeqCst) + 1,
        gate.clone(),
    ));
    gate
}
async fn entered(semaphore: &Semaphore) {
    tokio::time::timeout(Duration::from_secs(5), semaphore.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn message_revision_does_not_conflict_but_real_config_revision_does_for_both_owners() {
    let (host, resources) = fixture();
    let client = HostClient::in_process(host).unwrap();
    let session = create(&client).await;
    session.pause(control(), options(2000)).await.unwrap();
    let gate = arm(&resources);
    let child = session.clone();
    let task = tokio::spawn(async move { child.update(update(0, "first"), options(10000)).await });
    entered(&gate.entered).await;
    session
        .send(input("ordinary revision"), options(2000))
        .await
        .unwrap();
    gate.release();
    assert_eq!(
        task.await.unwrap().unwrap().result,
        MetadataUpdateResult::Committed { config_revision: 1 }
    );
    let gate = arm(&resources);
    let child = session.clone();
    let task = tokio::spawn(async move { child.update(update(1, "loser"), options(10000)).await });
    entered(&gate.entered).await;
    session
        .update(update(1, "winner"), options(2000))
        .await
        .unwrap();
    gate.release();
    assert!(
        matches!(task.await.unwrap().unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::SessionConfigConflict))
    );
    assert_eq!(
        session.info(options(2000)).await.unwrap().config.role.name,
        "winner"
    );
    let agent = AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: session.target().agent_id.clone(),
    };
    let gate = arm(&resources);
    let child = client.clone();
    let target = agent.clone();
    let task = tokio::spawn(async move {
        child
            .update_agent(target, agent_update(0, "loser"), options(10000))
            .await
    });
    entered(&gate.entered).await;
    client
        .update_agent(agent.clone(), agent_update(0, "winner"), options(2000))
        .await
        .unwrap();
    gate.release();
    assert!(
        matches!(task.await.unwrap().unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::AgentConfigConflict))
    );
    assert_eq!(
        client.agent(agent, options(2000)).await.unwrap().name,
        "winner"
    );
    client.close();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_external_authority_is_rechecked_after_preparation_at_each_owner() {
    for agent_control in [false, true] {
        let (host, resources) = fixture();
        let client = HostClient::in_process(host.clone()).unwrap();
        let session = create(&client).await;
        let target = session.target().clone();
        let agent = AgentRef {
            host_id: client.initial_info().host_id.clone(),
            agent_id: target.agent_id.clone(),
        };
        let gate = arm(&resources);
        let child = client.clone();
        let child_session = session.clone();
        let child_agent = agent.clone();
        let task = tokio::spawn(async move {
            if agent_control {
                child
                    .update_agent(child_agent, agent_update(0, "stale"), options(10000))
                    .await
                    .map(|_| ())
            } else {
                child_session
                    .update(update(0, "stale"), options(10000))
                    .await
                    .map(|_| ())
            }
        });
        entered(&gate.entered).await;
        client.close();
        let replacement = HostClient::in_process(host).unwrap();
        assert!(replacement.initial_info().can_operate);
        gate.release();
        assert!(
            matches!(task.await.unwrap().unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::HostAccessInvalid))
        );
        assert_eq!(
            replacement
                .agent(agent, options(2000))
                .await
                .unwrap()
                .config_revision,
            0
        );
        assert_eq!(
            replacement
                .session(target)
                .info(options(2000))
                .await
                .unwrap()
                .config_revision,
            0
        );
        replacement.close();
    }
}
struct PrepareGate {
    inner: Arc<FixtureResources>,
    entered: Semaphore,
    release: Semaphore,
    first: AtomicBool,
}
impl ResourceResolver for PrepareGate {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            if self.first.swap(false, Ordering::SeqCst) {
                self.entered.add_permits(1);
                self.release.acquire().await.unwrap().forget();
            }
            self.inner.resolve(config).await
        })
    }
}
#[tokio::test]
async fn input_remove_wins_before_binding_and_loses_after_binding_without_clearing_later_input() {
    let (_, resources) = fixture();
    let gate = Arc::new(PrepareGate {
        inner: resources.clone(),
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
        first: AtomicBool::new(true),
    });
    let host = Host::memory(gate.clone()).unwrap();
    let client = HostClient::in_process(host).unwrap();
    let session = create(&client).await;
    let first = input("remove during preparation");
    let first_id = first.message_id.clone();
    session.send(first, options(2000)).await.unwrap();
    entered(&gate.entered).await;
    let listed = session.inputs(options(2000)).await.unwrap();
    assert_eq!(listed.next_turn[0].input.message_id, first_id);
    session
        .remove_input(
            InputRemoveRequest {
                operation_id: OperationId::new().unwrap(),
                message_id: first_id.clone(),
                expected_controller_revision: Some(listed.controller_revision),
            },
            options(2000),
        )
        .await
        .unwrap();
    let later = input("later binds");
    let later_id = later.message_id.clone();
    session.send(later, options(2000)).await.unwrap();
    gate.release.add_permits(1);
    entered(&resources.model.started).await;
    let turn = running(&session).await;
    assert!(
        session
            .inputs(options(2000))
            .await
            .unwrap()
            .next_turn
            .is_empty()
    );
    let err = session
        .remove_input(
            InputRemoveRequest {
                operation_id: OperationId::new().unwrap(),
                message_id: later_id.clone(),
                expected_controller_revision: None,
            },
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err,CallError::Business(f) if f.kind()==Some(FaultKind::SessionInputAlreadyBound))
    );
    let future = input("unrelated next turn");
    let future_id = future.message_id.clone();
    session.send(future, options(2000)).await.unwrap();
    let missing = session
        .remove_input(
            InputRemoveRequest {
                operation_id: OperationId::new().unwrap(),
                message_id: first_id,
                expected_controller_revision: None,
            },
            options(2000),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(missing,CallError::Business(f) if f.kind()==Some(FaultKind::SessionInputNotFound))
    );
    assert_eq!(
        session.inputs(options(2000)).await.unwrap().next_turn[0]
            .input
            .message_id,
        future_id
    );
    // next-step binding uses the same owner lock, not a second input service.
    let mut steer = input("remove before continuation");
    steer.mode = MessageMode::Steer;
    let steer_id = steer.message_id.clone();
    session.send(steer, options(2000)).await.unwrap();
    session
        .remove_input(
            InputRemoveRequest {
                operation_id: OperationId::new().unwrap(),
                message_id: steer_id,
                expected_controller_revision: None,
            },
            options(2000),
        )
        .await
        .unwrap();
    session.pause(control(), options(2000)).await.unwrap();
    resources.model.finish.add_permits(1);
    session.wait_turn(turn, options(3000)).await.unwrap();
    assert_eq!(
        session.inputs(options(2000)).await.unwrap().next_turn[0]
            .input
            .message_id,
        future_id
    );
    client.close();
}
struct VersionResources {
    old: Arc<FixtureResources>,
    new: Arc<FixtureResources>,
    resolved: Mutex<Vec<SessionConfig>>,
}
impl ResourceResolver for VersionResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        self.old.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.resolved.lock().unwrap().push(config.clone());
            if config.role.name == "new binding" {
                self.new.resolve(config).await
            } else {
                self.old.resolve(config).await
            }
        })
    }
}
#[tokio::test]
async fn active_turn_and_continuation_keep_actual_old_resources_next_turn_uses_committed_config() {
    let (_, old) = fixture();
    let (_, new) = fixture();
    let resources = Arc::new(VersionResources {
        old: old.clone(),
        new: new.clone(),
        resolved: Mutex::new(Vec::new()),
    });
    let host = Host::memory(resources.clone()).unwrap();
    let client = HostClient::in_process(host).unwrap();
    let session = create(&client).await;
    session.send(input("first"), options(2000)).await.unwrap();
    entered(&old.model.started).await;
    let turn = running(&session).await;
    session
        .update(update(0, "new binding"), options(2000))
        .await
        .unwrap();
    let mut steer = input("same public Turn");
    steer.mode = MessageMode::Steer;
    let steer_id = steer.message_id.clone();
    session.send(steer, options(2000)).await.unwrap();
    session
        .send(input("next public Turn"), options(2000))
        .await
        .unwrap();
    old.model.finish.add_permits(1);
    entered(&old.model.started).await;
    assert_eq!(running(&session).await, turn);
    assert_eq!(new.model.calls.load(Ordering::SeqCst), 0);
    let bound = session.turn(turn.clone(), options(2000)).await.unwrap();
    assert_eq!(bound.data.state.config_revision, 0);
    assert!(
        matches!(session.remove_input(InputRemoveRequest{operation_id:OperationId::new().unwrap(),message_id:steer_id,expected_controller_revision:None},options(2000)).await.unwrap_err(),CallError::Business(f) if f.kind()==Some(FaultKind::SessionInputAlreadyBound))
    );
    old.model.finish.add_permits(1);
    session.wait_turn(turn, options(3000)).await.unwrap();
    entered(&new.model.started).await;
    let next = running(&session).await;
    let bound = session.turn(next.clone(), options(2000)).await.unwrap();
    assert_eq!(bound.data.state.config_revision, 1);
    assert_eq!(old.model.calls.load(Ordering::SeqCst), 2);
    assert_eq!(new.model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        resources
            .resolved
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.role.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Assistant", "new binding"]
    );
    new.model.finish.add_permits(1);
    session.wait_turn(next, options(3000)).await.unwrap();
    client.close();
}
