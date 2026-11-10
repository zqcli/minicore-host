//! Recovered TASK082 failing probes from the reviewer's retained conversation.
//! The two reentry test functions and subscriber checks preserve the original probes.
//! Old file hashes and raw logs were lost; this is not a byte-identity claim.
#[allow(dead_code)]
#[path = "task079/support.rs"]
mod support;

use minicore_host_core::*;
use minicore_host_protocol::*;
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use support::*;
use tracing::{
    Dispatch, Event, Metadata, Subscriber,
    instrument::WithSubscriber,
    span::{Attributes, Id, Record},
};

#[derive(Clone)]
struct QueryTarget {
    host: Weak<Host>,
    caller: CallerContext,
    agent: Option<AgentRef>,
}

#[derive(Default)]
struct ProbeState {
    target: Mutex<Option<QueryTarget>>,
    active: AtomicBool,
    next_id: AtomicU64,
    callbacks: Mutex<Vec<&'static str>>,
    blocked: Mutex<Vec<&'static str>>,
}

impl ProbeState {
    fn arm(&self, host: &Arc<Host>, caller: &CallerContext, agent: Option<AgentRef>) {
        *self.target.lock().unwrap() = Some(QueryTarget {
            host: Arc::downgrade(host),
            caller: caller.clone(),
            agent,
        });
        self.active.store(true, Ordering::SeqCst);
    }

    fn check(&self, hook: &'static str) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        let Some(target) = self.target.lock().unwrap().clone() else {
            return;
        };
        let Some(host) = target.host.upgrade() else {
            return;
        };
        self.callbacks.lock().unwrap().push(hook);
        let (tx, rx) = mpsc::channel();
        // Bound the probe rather than deadlocking the test process. A direct
        // callback reentry would wait forever if the emitting thread owns the
        // registry/owner lock. This thread performs the identical public query.
        std::thread::spawn(move || {
            let info = host.info(&target.caller).map(|_| ());
            let agent = target
                .agent
                .map(|agent| host.agent_info(&target.caller, &agent).map(|_| ()));
            let _ = tx.send((info, agent));
        });
        if rx.recv_timeout(Duration::from_millis(500)).is_err() {
            self.blocked.lock().unwrap().push(hook);
        }
    }

    fn assert_unlocked(&self) {
        self.active.store(false, Ordering::SeqCst);
        assert!(
            !self.callbacks.lock().unwrap().is_empty(),
            "probe did not run"
        );
        assert!(
            self.blocked.lock().unwrap().is_empty(),
            "subscriber callback retained an outer registry or owner guard: {:?}",
            self.blocked.lock().unwrap()
        );
    }
}

struct ReentrySubscriber(Arc<ProbeState>);
impl Subscriber for ReentrySubscriber {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        self.0.check("new_span");
        Id::from_u64(self.0.next_id.fetch_add(1, Ordering::SeqCst) + 1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {
        self.0.check("record");
    }
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, _: &Event<'_>) {
        self.0.check("event");
    }
    fn enter(&self, _: &Id) {
        self.0.check("enter");
    }
    fn exit(&self, _: &Id) {
        self.0.check("exit");
    }
    fn try_close(&self, _: Id) -> bool {
        self.0.check("try_close");
        true
    }
}

fn reentry() -> (Arc<ProbeState>, Dispatch) {
    let state = Arc::new(ProbeState::default());
    let dispatch = Dispatch::new(ReentrySubscriber(state.clone()));
    (state, dispatch)
}

fn session_request(agent: AgentRef) -> SessionCreate {
    SessionCreate {
        target: agent,
        operation_id: OperationId::new().unwrap(),
        mode: None,
        overrides: None,
        initial_message: None,
    }
}

async fn memory_agent(host: &Arc<Host>, caller: &CallerContext) -> AgentRef {
    let receipt = host
        .create_agent_async(caller, request(StorageMode::Memory))
        .await
        .unwrap();
    AgentRef {
        host_id: receipt.host_id,
        agent_id: receipt.agent_id.unwrap(),
    }
}

#[tokio::test]
async fn rejected_session_creation_drops_span_outside_registry() {
    let host = Host::memory(resources()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = memory_agent(&host, &caller).await;
    host.agent_stop(
        &caller,
        &agent,
        AgentStopRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(
        host.agent_info(&caller, &agent).unwrap().lifecycle,
        AgentLifecycle::Stopped
    );
    let (probe, dispatch) = reentry();
    probe.arm(&host, &caller, Some(agent.clone()));
    let error = host
        .create_session_async(&caller, session_request(agent))
        .with_subscriber(dispatch)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Some(FaultKind::AgentLifecycleUnavailable));
    probe.assert_unlocked();
}

#[tokio::test]
async fn repeated_host_shutdown_drops_unused_span_outside_registry() {
    let host = Host::memory(resources()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    shutdown(&host, &caller).await;
    let (probe, dispatch) = reentry();
    probe.arm(&host, &caller, None);
    tracing::dispatcher::with_default(&dispatch, || {
        host.shutdown(
            &caller,
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
        )
        .unwrap();
    });
    probe.assert_unlocked();
}

#[tokio::test]
async fn memory_metadata_commits_emit_after_registry_and_agent_unlock() {
    let host = Host::memory(resources()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = memory_agent(&host, &caller).await;
    let (probe, dispatch) = reentry();
    probe.arm(&host, &caller, Some(agent.clone()));
    async {
        host.agent_update(
            &caller,
            &agent,
            AgentUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: AgentUpdateChanges {
                    name: Some("private memory metadata name".into()),
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
        host.agent_set_default_session(
            &caller,
            &agent,
            AgentSetDefaultSessionRequest {
                operation_id: OperationId::new().unwrap(),
                session_id: None,
            },
        )
        .unwrap();
        host.update_agent(
            &caller,
            &agent,
            1,
            &AgentConfigPatch {
                role: Some(Role {
                    name: "private role metadata".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        host.set_default_session(&caller, &agent, None).unwrap();
    }
    .with_subscriber(dispatch)
    .await;
    probe.assert_unlocked();
}

#[tokio::test]
async fn persistent_metadata_failure_and_reconcile_callbacks_leave_owner_guards() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open_persistent(directory.path(), settings(), resources())
        .await
        .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let created = host
        .create_agent_async(&caller, request(StorageMode::Persistent))
        .await
        .unwrap();
    let agent = AgentRef {
        host_id: created.host_id,
        agent_id: created.agent_id.unwrap(),
    };
    let path = directory
        .path()
        .join("agents")
        .join(agent.agent_id.as_str());
    std::fs::rename(path.join("agent.json"), path.join("original-agent.json")).unwrap();
    std::fs::create_dir(path.join("agent.json")).unwrap();
    let (probe, dispatch) = reentry();
    probe.arm(&host, &caller, Some(agent.clone()));
    async {
        host.agent_update(
            &caller,
            &agent,
            AgentUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: AgentUpdateChanges {
                    name: Some("private persistent metadata name".into()),
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
        until(|| host.agent_info(&caller, &agent).unwrap().metadata_fault).await;
        std::fs::remove_dir(path.join("agent.json")).unwrap();
        std::fs::rename(path.join("original-agent.json"), path.join("agent.json")).unwrap();
        host.reconcile_agent_record(&caller, &agent).await.unwrap();
    }
    .with_subscriber(dispatch)
    .await;
    probe.assert_unlocked();
    shutdown(&host, &caller).await;
}

#[tokio::test]
async fn memory_retirement_and_persistent_shutdown_callbacks_leave_all_owner_guards() {
    for mode in [StorageMode::Memory, StorageMode::Persistent] {
        let directory = tempfile::tempdir().unwrap();
        let host = if mode == StorageMode::Memory {
            Host::memory(resources()).unwrap()
        } else {
            Host::open_persistent(directory.path(), settings(), resources())
                .await
                .unwrap()
        };
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let created = host
            .create_agent_async(&caller, request(mode))
            .await
            .unwrap();
        let agent = AgentRef {
            host_id: created.host_id,
            agent_id: created.agent_id.unwrap(),
        };
        let (probe, dispatch) = reentry();
        probe.arm(&host, &caller, Some(agent.clone()));
        async {
            if mode == StorageMode::Memory {
                host.session_close(
                    &caller,
                    &SessionRef {
                        agent_id: agent.agent_id,
                        session_id: created.session_id.unwrap(),
                    },
                    SessionCloseRequest {
                        operation_id: OperationId::new().unwrap(),
                    },
                )
                .unwrap();
            }
            shutdown(&host, &caller).await;
        }
        .with_subscriber(dispatch)
        .await;
        probe.assert_unlocked();
    }
}
