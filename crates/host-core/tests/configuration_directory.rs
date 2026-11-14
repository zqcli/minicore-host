//! Actual directory files, native owners and persistence. The model is an
//! explicitly scripted fixture; no provider, UI or external service is exercised.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference as RuntimeReasoning,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct FixtureModel {
    descriptor: ModelDescriptor,
    calls: Arc<AtomicUsize>,
}
impl Model for FixtureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(Box::pin(stream::iter([
                Ok(ModelEvent::text_delta("directory fixture result")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
struct FixtureResources {
    model: Arc<dyn Model>,
}
impl ResourceResolver for FixtureResources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if config.model.as_str() == "test/model" {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "fixture model missing",
            ))
        }
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.validate(config)?;
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    RuntimeReasoning::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
struct FixturePreparer {
    resources: Arc<dyn ResourceResolver>,
    gate: Mutex<Option<Arc<PreparationGate>>>,
}
struct PreparationGate {
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}
impl HostConfigurationPreparer for FixturePreparer {
    fn prepare(&self, config: HostConfig) -> HostConfigurationFuture<'_> {
        let gate = self.gate.lock().unwrap().take();
        Box::pin(async move {
            if let Some(gate) = gate {
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
            let key = config
                .network
                .access_key_env
                .as_ref()
                .map(|name| format!("fixture-key-{name}"));
            PreparedHostConfiguration::new(config, self.resources.clone(), vec![], key)
        })
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
    preparer: Arc<FixturePreparer>,
    calls: Arc<AtomicUsize>,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config");
        let data = root.path().join("data");
        std::fs::create_dir_all(config.join("roles")).unwrap();
        std::fs::create_dir_all(config.join("agents")).unwrap();
        std::fs::write(
            config.join("roles/writer.json"),
            serde_json::to_vec(&Role {
                name: "Role display name".into(),
                description: "fixture".into(),
                instructions: "role text".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let resources = Arc::new(FixtureResources {
            model: Arc::new(FixtureModel {
                descriptor: ModelDescriptor::new(
                    "test/model".parse().unwrap(),
                    128_000,
                    BTreeSet::from([RuntimeReasoning::Auto]),
                    true,
                )
                .unwrap(),
                calls: calls.clone(),
            }),
        });
        let fixture = Self {
            _root: root,
            config,
            data,
            preparer: Arc::new(FixturePreparer {
                resources,
                gate: Mutex::new(None),
            }),
            calls,
        };
        fixture.host(&HostConfig::default());
        fixture
    }
    fn host(&self, config: &HostConfig) {
        std::fs::write(
            self.config.join("host.json"),
            serde_json::to_vec(config).unwrap(),
        )
        .unwrap();
    }
    fn agent(&self, file: &str, key: &str, name: &str, instructions: &str) {
        let defaults = SessionDefaults {
            model: "test/model".parse().unwrap(),
            ..SessionDefaults::default()
        };
        std::fs::write(self.config.join("agents").join(file), serde_json::to_vec(&json!({
            "key":key,"name":name,"role":"writer","instructions":instructions,"session_defaults":defaults,
        })).unwrap()).unwrap();
    }
    async fn open(&self) -> Arc<Host> {
        Host::open_directory_configured(&self.data, &self.config, |config| async move {
            self.preparer
                .prepare(config)
                .await
                .map(|prepared| prepared.with_preparer(self.preparer.clone()))
        })
        .await
        .unwrap()
    }
}
fn operation() -> OperationId {
    OperationId::new().unwrap()
}
fn target(host: &Host, caller: &CallerContext, key: &str) -> AgentRef {
    host.agents(caller)
        .unwrap()
        .into_iter()
        .find_map(|agent| {
            let target = AgentRef {
                host_id: agent.host_id,
                agent_id: agent.id,
            };
            (host
                .agent_info(caller, &target)
                .unwrap()
                .configuration_source
                .as_ref()
                .is_some_and(|source| source.key == key))
            .then_some(target)
        })
        .unwrap()
}
fn default_session(host: &Host, caller: &CallerContext, target: &AgentRef) -> SessionRef {
    SessionRef {
        agent_id: target.agent_id.clone(),
        session_id: host
            .agent_info(caller, target)
            .unwrap()
            .default_session_id
            .unwrap(),
    }
}
async fn terminal(
    host: &Host,
    caller: &CallerContext,
    operation: &OperationId,
) -> HostReloadResult {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Receipt::HostReload(receipt) = host.receipt_get(caller, operation).unwrap()
                && !matches!(
                    receipt.result,
                    HostReloadResult::Preparing { .. } | HostReloadResult::Applying { .. }
                )
            {
                return receipt.result;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
async fn reload(host: &Arc<Host>, caller: &CallerContext) -> HostReloadResult {
    let receipt = host
        .reload(
            caller,
            HostReloadRequest {
                operation_id: operation(),
                expected_revision: host.config_get(caller).unwrap().config_revision,
            },
        )
        .unwrap();
    terminal(host, caller, &receipt.operation_id).await
}
async fn stop(host: &Arc<Host>, caller: &CallerContext) {
    host.shutdown(
        caller,
        HostShutdownRequest {
            operation_id: operation(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while host.info(caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn directory_restart_preserves_identity_history_and_session_instructions() {
    let fixture = Fixture::new();
    fixture.agent(
        "one.json",
        "first",
        "same display name",
        "first instructions",
    );
    fixture.agent(
        "two.json",
        "second",
        "same display name",
        "second instructions",
    );
    std::fs::write(
        fixture.config.join("tui.json"),
        "invalid frontend JSON ignored by Host",
    )
    .unwrap();
    let host = fixture.open().await;
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let old_host = host.info(&caller).unwrap();
    let agent = target(&host, &caller, "first");
    let session = default_session(&host, &caller, &agent);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert_eq!(host.agents(&caller).unwrap().len(), 2);
    host.send(
        &caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "preserve this history".into(),
                in_reply_to: None,
            },
            session.clone(),
        ),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(turn) = host.turns(&caller, &session).unwrap().last() {
                let result = host.wait_turn(&caller, &session, &turn.id).await.unwrap();
                assert_eq!(result.info.persistence, PersistenceState::Persisted);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let history_path = fixture
        .data
        .join("agents")
        .join(agent.agent_id.as_str())
        .join("sessions")
        .join(session.session_id.as_str())
        .join("history.jsonl");
    stop(&host, &caller).await;
    drop(connection);
    drop(host);
    let history = std::fs::read(&history_path).unwrap();
    std::fs::rename(
        fixture.config.join("agents/one.json"),
        fixture.config.join("agents/renamed.json"),
    )
    .unwrap();
    fixture.agent(
        "renamed.json",
        "first",
        "renamed display name",
        "new instructions",
    );
    fixture.host(&HostConfig {
        name: "from files on restart".into(),
        logging: LogSettings {
            level: LogLevel::Debug,
            ..LogSettings::default()
        },
        ..HostConfig::default()
    });
    let host = fixture.open().await;
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(host.info(&caller).unwrap().host_id, old_host.host_id);
    assert_ne!(
        host.info(&caller).unwrap().host_incarnation,
        old_host.host_incarnation
    );
    assert_eq!(
        host.config_get(&caller).unwrap().config.name,
        "from files on restart"
    );
    assert_eq!(target(&host, &caller, "first"), agent);
    assert_eq!(default_session(&host, &caller, &agent), session);
    assert_eq!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .config
            .instructions,
        "new instructions"
    );
    assert_eq!(
        host.session_info(&caller, &session)
            .unwrap()
            .config
            .instructions,
        "first instructions"
    );
    let created = host
        .create_session_async(
            &caller,
            SessionCreate {
                operation_id: operation(),
                target: agent.clone(),
                mode: None,
                overrides: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let new_session = SessionRef {
        agent_id: agent.agent_id,
        session_id: created.session_id.unwrap(),
    };
    assert_eq!(
        host.session_info(&caller, &new_session)
            .unwrap()
            .config
            .instructions,
        "new instructions"
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    stop(&host, &caller).await;
    drop(connection);
    drop(host);
    assert_eq!(std::fs::read(&history_path).unwrap(), history);
}

#[tokio::test]
async fn managed_updates_delete_and_copy_obey_owner_source_boundaries() {
    let fixture = Fixture::new();
    fixture.agent("writer.json", "writer", "Writer", "file instructions");
    let host = fixture.open().await;
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = target(&host, &caller, "writer");
    let session = default_session(&host, &caller, &agent);
    assert_eq!(
        host.update_agent_async(
            &caller,
            &agent,
            0,
            &AgentConfigPatch {
                instructions: Some("RPC overwrite".into()),
                ..AgentConfigPatch::default()
            }
        )
        .await
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentConfigConflict)
    );
    assert_eq!(
        host.agent_delete(
            &caller,
            &agent,
            AgentDeleteRequest {
                operation_id: operation(),
                scope: AgentDeleteScope::default()
            }
        )
        .unwrap_err()
        .kind(),
        Some(FaultKind::AgentConfigConflict)
    );
    host.update_session_async(
        &caller,
        &session,
        0,
        &SessionConfigPatch {
            instructions: Some("Session override".into()),
            ..SessionConfigPatch::default()
        },
    )
    .await
    .unwrap();
    let copied = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: operation(),
                name: "dynamic copy".into(),
                mode: StorageMode::Persistent,
                config: host.agent_info(&caller, &agent).unwrap().config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let copy = AgentRef {
        host_id: agent.host_id.clone(),
        agent_id: copied.agent_id.unwrap(),
    };
    assert!(
        host.agent_info(&caller, &copy)
            .unwrap()
            .configuration_source
            .is_none()
    );
    std::fs::remove_file(fixture.config.join("agents/writer.json")).unwrap();
    let result = reload(&host, &caller).await;
    assert!(
        matches!(result, HostReloadResult::Applied { ref agents, .. } if matches!(&agents[0], AgentConfigurationResult::Unmanaged { .. }))
    );
    let detached = host.agent_info(&caller, &agent).unwrap();
    assert_eq!(
        detached.configuration_source,
        Some(AgentConfigurationSource {
            key: "writer".into(),
            managed: false
        })
    );
    host.update_agent_async(
        &caller,
        &agent,
        detached.config_revision,
        &AgentConfigPatch {
            instructions: Some("dynamic instructions".into()),
            ..AgentConfigPatch::default()
        },
    )
    .await
    .unwrap();
    stop(&host, &caller).await;
    drop(connection);
    drop(host);
    let host = fixture.open().await;
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    assert_eq!(
        host.agent_info(&caller, &agent)
            .unwrap()
            .configuration_source,
        Some(AgentConfigurationSource {
            key: "writer".into(),
            managed: false
        })
    );
    fixture.agent(
        "other-file.json",
        "writer",
        "returned Writer",
        "returned instructions",
    );
    assert!(matches!(
        reload(&host, &caller).await,
        HostReloadResult::Applied { .. }
    ));
    assert_eq!(target(&host, &caller, "writer"), agent);
    assert_eq!(default_session(&host, &caller, &agent), session);
    assert_eq!(
        host.session_info(&caller, &session)
            .unwrap()
            .config
            .instructions,
        "Session override"
    );
    assert!(
        host.agent_info(&caller, &copy)
            .unwrap()
            .configuration_source
            .is_none()
    );
    assert_eq!(host.agents(&caller).unwrap().len(), 2);
    std::fs::remove_file(fixture.config.join("agents/other-file.json")).unwrap();
    assert!(matches!(
        reload(&host, &caller).await,
        HostReloadResult::Applied { .. }
    ));
    host.agent_delete(
        &caller,
        &agent,
        AgentDeleteRequest {
            operation_id: operation(),
            scope: AgentDeleteScope::Sessions {
                session_ids: vec![session.session_id],
                close: true,
            },
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while host
            .agents(&caller)
            .unwrap()
            .iter()
            .any(|item| item.id == agent.agent_id)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(host.agents(&caller).unwrap().len(), 1);
    stop(&host, &caller).await;
}

#[tokio::test]
async fn complete_validation_and_restart_stop_before_any_agent_change() {
    let fixture = Fixture::new();
    fixture.agent("writer.json", "writer", "Before", "old");
    let host = fixture.open().await;
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let agent = target(&host, &caller, "writer");
    fixture.agent("writer.json", "writer", "After", "new");
    fixture.host(&HostConfig {
        name: "must wait".into(),
        ..HostConfig::default()
    });
    std::fs::write(fixture.config.join("agents/broken.json"), "{ broken").unwrap();
    assert!(matches!(
        reload(&host, &caller).await,
        HostReloadResult::Failed {
            config_revision: 0,
            ..
        }
    ));
    assert_eq!(host.agent_info(&caller, &agent).unwrap().name, "Before");
    std::fs::remove_file(fixture.config.join("agents/broken.json")).unwrap();
    let path = fixture.config.join("agents/writer.json");
    let valid = std::fs::read(&path).unwrap();
    let mut invalid: serde_json::Value = serde_json::from_slice(&valid).unwrap();
    invalid["session_defaults"]["model"] = json!("missing/model");
    std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    assert!(matches!(
        reload(&host, &caller).await,
        HostReloadResult::Failed {
            config_revision: 0,
            ..
        }
    ));
    std::fs::write(&path, valid).unwrap();
    fixture.host(&HostConfig {
        name: "must wait".into(),
        logging: LogSettings {
            level: LogLevel::Debug,
            ..LogSettings::default()
        },
        ..HostConfig::default()
    });
    assert!(matches!(
        reload(&host, &caller).await,
        HostReloadResult::RestartRequired {
            config_revision: 0,
            ..
        }
    ));
    assert_eq!(
        host.config_get(&caller).unwrap().config,
        HostConfig::default()
    );
    assert_eq!(host.agent_info(&caller, &agent).unwrap().name, "Before");
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    stop(&host, &caller).await;
}

#[tokio::test]
async fn real_agent_save_failure_reports_partial_host_and_sibling_commits() {
    let fixture = Fixture::new();
    fixture.agent("a.json", "a", "A", "old");
    fixture.agent("b.json", "b", "B", "old");
    let host = fixture.open().await;
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let a = target(&host, &caller, "a");
    let b = target(&host, &caller, "b");
    fixture.agent("a.json", "a", "A updated", "new");
    fixture.agent("b.json", "b", "B unsaved", "new");
    fixture.host(&HostConfig {
        name: "Host committed".into(),
        ..HostConfig::default()
    });
    let broken = fixture
        .data
        .join("agents")
        .join(b.agent_id.as_str())
        .join("agent.json");
    std::fs::rename(&broken, broken.with_extension("saved")).unwrap();
    std::fs::create_dir(&broken).unwrap();
    let result = reload(&host, &caller).await;
    let HostReloadResult::PartiallyApplied {
        config_revision,
        agents,
    } = result
    else {
        panic!("expected real partial result")
    };
    assert_eq!(config_revision, 1);
    assert!(matches!(&agents[0], AgentConfigurationResult::Updated { key, .. } if key == "a"));
    assert!(
        matches!(&agents[1], AgentConfigurationResult::Failed { key, agent_id: Some(id), .. } if key == "b" && id == &b.agent_id)
    );
    assert_eq!(
        host.config_get(&caller).unwrap().config.name,
        "Host committed"
    );
    assert_eq!(host.agent_info(&caller, &a).unwrap().name, "A updated");
    assert_eq!(host.agent_info(&caller, &b).unwrap().name, "B");
    // Repair only the deliberately injected test obstruction, then use the
    // explicit existing reconciliation entry for that exact pending save.
    std::fs::remove_dir(&broken).unwrap();
    std::fs::rename(broken.with_extension("saved"), &broken).unwrap();
    host.reconcile_agent_record(&caller, &b).await.unwrap();
    stop(&host, &caller).await;
}

#[tokio::test]
async fn key_rotation_does_not_cancel_new_configured_agent_creation() {
    let fixture = Fixture::new();
    fixture.host(&HostConfig {
        network: NetworkSettings {
            access_key_env: Some("OLD".into()),
            listen: None,
        },
        ..HostConfig::default()
    });
    let host = fixture.open().await;
    let old = host.connect_network("fixture-key-OLD").unwrap();
    let observer = host.connect().unwrap();
    fixture.agent(
        "new.json",
        "new",
        "Created after rotation",
        "new instructions",
    );
    fixture.host(&HostConfig {
        network: NetworkSettings {
            access_key_env: Some("NEW".into()),
            listen: None,
        },
        ..HostConfig::default()
    });
    let accepted = host
        .reload(
            &old.caller(),
            HostReloadRequest {
                operation_id: operation(),
                expected_revision: 0,
            },
        )
        .unwrap();
    let result = terminal(&host, &observer.caller(), &accepted.operation_id).await;
    let HostReloadResult::Applied { agents, .. } = result else {
        panic!("expected applied directory")
    };
    assert!(
        matches!(&agents[0], AgentConfigurationResult::Created { receipt, .. } if receipt.result.steps.session.succeeded() && receipt.message_id.is_none() && receipt.operation_id != accepted.operation_id)
    );
    assert!(host.info(&old.caller()).is_err());
    let new = host.connect_network("fixture-key-NEW").unwrap();
    let agent = target(&host, &new.caller(), "new");
    assert!(
        host.agent_info(&new.caller(), &agent)
            .unwrap()
            .configuration_source
            .unwrap()
            .managed
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    stop(&host, &new.caller()).await;
}

#[tokio::test]
async fn accepted_directory_work_finishes_existing_and_new_agents_during_shutdown() {
    for strategy in [HostShutdownStrategy::Graceful, HostShutdownStrategy::Force] {
        let fixture = Fixture::new();
        fixture.agent("existing.json", "existing", "Before", "old");
        let host = fixture.open().await;
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let existing = target(&host, &caller, "existing");
        fixture.agent("existing.json", "existing", "After", "new");
        fixture.agent("new.json", "new", "Created while stopping", "new");
        let gate = Arc::new(PreparationGate {
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        });
        *fixture.preparer.gate.lock().unwrap() = Some(gate.clone());
        let accepted = host
            .reload(
                &caller,
                HostReloadRequest {
                    operation_id: operation(),
                    expected_revision: 0,
                },
            )
            .unwrap();
        gate.entered.acquire().await.unwrap().forget();
        host.shutdown(
            &caller,
            HostShutdownRequest {
                operation_id: operation(),
                strategy,
            },
        )
        .unwrap();
        assert!(minicore_host_storage::DataRoot::acquire(&fixture.data).is_err());
        let ordinary = host
            .create_agent_async(
                &caller,
                AgentCreate {
                    operation_id: operation(),
                    name: "too late".into(),
                    mode: StorageMode::Memory,
                    config: host.agent_info(&caller, &existing).unwrap().config,
                    default_session: None,
                    initial_message: None,
                },
            )
            .await;
        assert_eq!(
            ordinary.unwrap_err().kind(),
            Some(FaultKind::HostLifecycleUnavailable)
        );
        gate.release.add_permits(1);
        let result = terminal(&host, &caller, &accepted.operation_id).await;
        let HostReloadResult::Applied { agents, .. } = result else {
            panic!("accepted directory work must finish through shutdown")
        };
        assert!(matches!(
            &agents[0],
            AgentConfigurationResult::Updated { .. }
        ));
        assert!(
            matches!(&agents[1], AgentConfigurationResult::Created { receipt, .. } if receipt.result.steps.session.succeeded())
        );
        tokio::time::timeout(Duration::from_secs(10), async {
            while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
        drop(connection);
        drop(host);
        let reopened = fixture.open().await;
        let connection = reopened.connect().unwrap();
        let caller = connection.caller();
        assert_eq!(target(&reopened, &caller, "existing"), existing);
        assert_eq!(
            reopened.agent_info(&caller, &existing).unwrap().name,
            "After"
        );
        let created = target(&reopened, &caller, "new");
        default_session(&reopened, &caller, &created);
        assert_eq!(reopened.agents(&caller).unwrap().len(), 2);
        stop(&reopened, &caller).await;
    }
}
