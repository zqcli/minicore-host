//! Real retirement, direct-owner removal and persistent deletion boundaries.
#[allow(dead_code)]
#[path = "task079/support.rs"]
mod support;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_storage::DataRoot;
use support::*;
use tracing::instrument::WithSubscriber;

#[test]
fn synchronous_memory_retirement_has_one_actual_close_chain_without_an_executor() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (file, dispatch, guard) = logging();
    let (agent, session, operation) = tracing::dispatcher::with_default(&dispatch, || {
        let host = Host::memory(resources()).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let created = runtime
            .block_on(host.create_agent_async(&caller, request(StorageMode::Memory)))
            .unwrap();
        let agent = AgentRef {
            host_id: created.host_id,
            agent_id: created.agent_id.unwrap(),
        };
        let session = SessionRef {
            agent_id: agent.agent_id.clone(),
            session_id: created.session_id.unwrap(),
        };
        drop(runtime);
        assert!(tokio::runtime::Handle::try_current().is_err());
        let operation = OperationId::new().unwrap();
        host.session_close(
            &caller,
            &session,
            SessionCloseRequest {
                operation_id: operation.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap_err().kind(),
            Some(FaultKind::HostAgentNotFound)
        );
        assert!(
            host.session_close(
                &caller,
                &session,
                SessionCloseRequest {
                    operation_id: OperationId::new().unwrap()
                }
            )
            .is_err()
        );
        (agent, session, operation)
    });
    let rows = records(&file, guard);
    for (event, phase) in [
        ("agent.session_removed", "removed"),
        ("agent.retiring", "retiring"),
        ("agent.retired", "retired"),
        ("host.agent_removed", "removed"),
    ] {
        let matching: Vec<_> = rows.iter().filter(|row| row["event"] == event).collect();
        assert_eq!(matching.len(), 1, "{rows:?}");
        let row = matching[0];
        assert_eq!(row["host_id"], agent.host_id.as_str());
        assert_eq!(row["agent_id"], agent.agent_id.as_str());
        assert_eq!(row["attributes"]["mode"], "memory");
        assert_eq!(row["attributes"]["phase"], phase);
        if event == "host.agent_removed" {
            // The Host scans all actually retired Agents, not one caller's scope.
            assert!(row.get("operation_id").is_none());
        } else {
            assert_eq!(row["operation_id"], operation.as_str());
        }
        if event == "agent.session_removed" {
            assert_eq!(row["session_id"], session.session_id.as_str());
        }
        assert!(row["attributes"].get("persistence").is_none());
    }
}

struct PendingModelFixture {
    descriptor: minicore_runtime::model::ModelDescriptor,
}
impl minicore_runtime::model::Model for PendingModelFixture {
    fn descriptor(&self) -> &minicore_runtime::model::ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(
        &'a self,
        _: minicore_runtime::model::ModelRequest,
        _: minicore_runtime::model::ModelCallContext,
    ) -> minicore_runtime::model::ModelStartFuture<'a> {
        Box::pin(async {
            Ok(Box::pin(futures_util::stream::pending()) as minicore_runtime::model::ModelStream)
        })
    }
}
struct PendingResourcesFixture;
impl ResourceResolver for PendingResourcesFixture {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        resources().validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.validate(config)?;
            use minicore_runtime::{
                ExecutionConfig, LoopOptions,
                model::{ModelDescriptor, ReasoningPreference},
                prompt::DefaultPromptProvider,
                tools::ToolSet,
            };
            let model = std::sync::Arc::new(PendingModelFixture {
                descriptor: ModelDescriptor::new(
                    "fixture/model".parse().unwrap(),
                    128_000,
                    std::collections::BTreeSet::from([ReasoningPreference::Auto]),
                    true,
                )
                .unwrap(),
            });
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    model,
                    ReasoningPreference::Auto,
                    ToolSet::default(),
                    None,
                    std::sync::Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}

#[tokio::test]
async fn runtime_close_reaper_emits_actual_removed_owners_with_no_borrowed_operation() {
    let (file, dispatch, guard) = logging();
    let (agent, session) = async {
        let host = Host::memory(std::sync::Arc::new(PendingResourcesFixture)).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let mut create = request(StorageMode::Memory);
        create.initial_message = Some(input(false));
        let created = host.create_agent_async(&caller, create).await.unwrap();
        let agent = AgentRef {
            host_id: created.host_id,
            agent_id: created.agent_id.unwrap(),
        };
        let session = SessionRef {
            agent_id: agent.agent_id.clone(),
            session_id: created.session_id.unwrap(),
        };
        until(|| {
            host.turns(&caller, &session)
                .unwrap()
                .into_iter()
                .find(|turn| turn.state == TurnState::Running)
        })
        .await;
        // The real Runtime is still polling a model stream. Synchronous close
        // cannot retire that driver; the existing reaper must join it first.
        host.session_close(
            &caller,
            &session,
            SessionCloseRequest {
                operation_id: OperationId::new().unwrap(),
            },
        )
        .unwrap();
        until(|| host.agent_info(&caller, &agent).is_err().then_some(())).await;
        shutdown(&host, &caller).await;
        (agent, session)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    for event in [
        "agent.session_removed",
        "agent.retired",
        "host.agent_removed",
    ] {
        let matching: Vec<_> = rows.iter().filter(|row| row["event"] == event).collect();
        assert_eq!(matching.len(), 1, "{rows:?}");
        assert_eq!(matching[0]["agent_id"], agent.agent_id.as_str());
        assert_eq!(matching[0]["attributes"]["mode"], "memory");
        assert!(matching[0].get("turn_id").is_none());
    }
    let removed = rows
        .iter()
        .find(|row| row["event"] == "agent.session_removed")
        .unwrap();
    assert_eq!(removed["session_id"], session.session_id.as_str());
    assert!(removed.get("operation_id").is_none());
}

#[tokio::test]
async fn shutdown_distinguishes_memory_retirement_from_persistent_resource_release() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let (persistent, memory, operation) = async {
        let host = Host::open_persistent(directory.path(), settings(), resources())
            .await
            .unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let persistent = host
            .create_agent_async(&caller, request(StorageMode::Persistent))
            .await
            .unwrap();
        let memory = host
            .create_agent_async(&caller, request(StorageMode::Memory))
            .await
            .unwrap();
        let operation = shutdown(&host, &caller).await;
        let persistent_agent = AgentRef {
            host_id: persistent.host_id.clone(),
            agent_id: persistent.agent_id.clone().unwrap(),
        };
        assert_eq!(
            host.agent_info(&caller, &persistent_agent)
                .unwrap()
                .lifecycle,
            AgentLifecycle::Stopped
        );
        assert_eq!(
            host.session_state(
                &caller,
                &SessionRef {
                    agent_id: persistent.agent_id.clone().unwrap(),
                    session_id: persistent.session_id.clone().unwrap()
                }
            )
            .unwrap()
            .data
            .lifecycle,
            SessionLifecycle::Open
        );
        assert!(
            host.agent_info(
                &caller,
                &AgentRef {
                    host_id: memory.host_id.clone(),
                    agent_id: memory.agent_id.clone().unwrap()
                }
            )
            .is_err()
        );
        let root = DataRoot::acquire(directory.path()).unwrap();
        drop(root);
        (persistent, memory, operation)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    for (created, mode, event, phase) in [
        (
            &persistent,
            "persistent",
            "agent.resources_released",
            "stopped",
        ),
        (&memory, "memory", "agent.retired", "retired"),
    ] {
        let matching: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"] == event
                    && row["agent_id"] == created.agent_id.as_ref().unwrap().as_str()
            })
            .collect();
        assert_eq!(matching.len(), 1, "{rows:?}");
        assert_eq!(matching[0]["operation_id"], operation.as_str());
        assert_eq!(matching[0]["attributes"]["mode"], mode);
        assert_eq!(matching[0]["attributes"]["phase"], phase);
    }
    assert!(!rows.iter().any(|row| row["agent_id"]
        == persistent.agent_id.as_ref().unwrap().as_str()
        && matches!(
            row["event"].as_str(),
            Some("agent.retired" | "host.agent_removed")
        )));
    let phases: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["event"] == "agent.host_shutdown_started"
                && row["agent_id"] == persistent.agent_id.as_ref().unwrap().as_str()
        })
        .map(|row| row["attributes"]["phase"].as_str().unwrap())
        .collect();
    assert_eq!(phases, ["draining", "stopping"]);
}

#[tokio::test]
async fn actual_agent_directory_refusal_keeps_partial_delete_and_real_release_phase() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let (agent, operation, shutdown_operation, fault) = async {
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
        std::fs::write(
            path.join("independent-artifact"),
            "private retained artifact",
        )
        .unwrap();
        let operation = OperationId::new().unwrap();
        host.agent_delete(
            &caller,
            &agent,
            AgentDeleteRequest {
                operation_id: operation.clone(),
                scope: AgentDeleteScope::Sessions {
                    session_ids: vec![created.session_id.clone().unwrap()],
                    close: true,
                },
            },
        )
        .unwrap();
        let fault = until(|| {
            host.agent_info(&caller, &agent)
                .unwrap()
                .deletion
                .and_then(|deletion| match deletion.data {
                    DataDeletion::Failed { error } => Some(error),
                    _ => None,
                })
        })
        .await;
        assert_eq!(fault.kind(), Some(FaultKind::AgentDeleteFailed));
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().lifecycle,
            AgentLifecycle::Retiring
        );
        assert!(
            !path
                .join("sessions")
                .join(created.session_id.unwrap().as_str())
                .exists()
        );
        let shutdown_operation = shutdown(&host, &caller).await;
        assert_eq!(
            host.agent_info(&caller, &agent).unwrap().lifecycle,
            AgentLifecycle::Retiring
        );
        assert_eq!(
            std::fs::read_to_string(path.join("independent-artifact")).unwrap(),
            "private retained artifact"
        );
        (agent, operation, shutdown_operation, fault)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    let errors: Vec<_> = rows.iter().filter(|row| row["level"] == "ERROR").collect();
    assert_eq!(errors.len(), 1, "{rows:?}");
    assert_eq!(errors[0]["event"], "agent.resource_release_failed");
    assert_eq!(errors[0]["operation_id"], operation.as_str());
    assert_eq!(errors[0]["fault_code"], fault.code());
    assert_eq!(errors[0]["fault_type"], fault.type_name());
    assert_eq!(errors[0]["attributes"]["phase"], "agent_directory_delete");
    assert!(!rows.iter().any(|row| matches!(
        row["event"].as_str(),
        Some("agent.retired" | "host.agent_removed")
    )));
    let released = rows
        .iter()
        .find(|row| row["event"] == "agent.resources_released")
        .unwrap();
    assert_eq!(released["agent_id"], agent.agent_id.as_str());
    assert_eq!(released["operation_id"], shutdown_operation.as_str());
    assert_eq!(released["attributes"]["phase"], "retiring");
    assert!(
        !rows
            .iter()
            .any(|row| row.to_string().contains("private retained artifact"))
    );
}

#[tokio::test]
async fn closed_or_uninstalled_logging_does_not_retain_memory_owners() {
    for closed in [false, true] {
        let dispatch = if closed {
            let (_file, dispatch, guard) = logging();
            assert_eq!(
                guard.shutdown(std::time::Duration::from_secs(2)),
                minicore_host_diagnostics::ShutdownOutcome::Flushed
            );
            dispatch
        } else {
            tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default())
        };
        async {
            let host = Host::memory(resources()).unwrap();
            let connection = host.connect().unwrap();
            let caller = connection.caller();
            let created = host
                .create_agent_async(&caller, request(StorageMode::Memory))
                .await
                .unwrap();
            let agent = AgentRef {
                host_id: created.host_id,
                agent_id: created.agent_id.unwrap(),
            };
            host.session_close(
                &caller,
                &SessionRef {
                    agent_id: agent.agent_id.clone(),
                    session_id: created.session_id.unwrap(),
                },
                SessionCloseRequest {
                    operation_id: OperationId::new().unwrap(),
                },
            )
            .unwrap();
            until(|| host.agent_info(&caller, &agent).is_err().then_some(())).await;
            shutdown(&host, &caller).await;
        }
        .with_subscriber(dispatch)
        .await;
    }
}

#[tokio::test]
async fn successful_persistent_cascade_retires_once_without_reclassifying_removed_owner() {
    let (file, dispatch, guard) = logging();
    let directory = tempfile::tempdir().unwrap();
    let (agent, operation) = async {
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
        let operation = OperationId::new().unwrap();
        let receipt = host
            .agent_delete(
                &caller,
                &agent,
                AgentDeleteRequest {
                    operation_id: operation.clone(),
                    scope: AgentDeleteScope::Sessions {
                        session_ids: vec![created.session_id.unwrap()],
                        close: true,
                    },
                },
            )
            .unwrap();
        assert_eq!(receipt.result.lifecycle, AgentDeleteLifecycle::Retiring);
        until(|| host.agent_info(&caller, &agent).is_err().then_some(())).await;
        assert!(
            !directory
                .path()
                .join("agents")
                .join(agent.agent_id.as_str())
                .exists()
        );
        assert_eq!(
            host.receipt_get(&caller, &operation).unwrap(),
            Receipt::AgentDelete(receipt)
        );
        shutdown(&host, &caller).await;
        (agent, operation)
    }
    .with_subscriber(dispatch)
    .await;
    let rows = records(&file, guard);
    for event in ["agent.retiring", "agent.retired", "host.agent_removed"] {
        let matching: Vec<_> = rows.iter().filter(|row| row["event"] == event).collect();
        assert_eq!(matching.len(), 1, "{rows:?}");
        assert_eq!(matching[0]["agent_id"], agent.agent_id.as_str());
        assert_eq!(matching[0]["attributes"]["mode"], "persistent");
        if event != "host.agent_removed" {
            assert_eq!(matching[0]["operation_id"], operation.as_str());
        }
    }
    assert!(!rows.iter().any(|row| row["level"] == "ERROR"));
}
