//! Scheduling gates around real Host acceptance, storage, publication and exit.
use super::*;
use crate::{CallerContext, Host};
use std::time::Duration;
use tokio::sync::Semaphore;
pub(crate) struct SaveGate {
    pub(crate) entered: Semaphore,
    pub(crate) release: Semaphore,
}
struct UnusedResources;
impl ResourceResolver for UnusedResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "fixture has no model",
            ))
        })
    }
}
pub(super) fn config(name: &str) -> HostConfig {
    HostConfig {
        name: name.into(),
        network: NetworkSettings {
            listen: None,
            access_key_env: Some("TASK023_PRIVATE_FIXTURE_KEY".into()),
        },
        ..Default::default()
    }
}
pub(super) fn prepared(config: HostConfig, key: &str) -> PreparedHostConfiguration {
    PreparedHostConfiguration::new(config, Arc::new(UnusedResources), vec![], Some(key.into()))
        .unwrap()
}
pub(super) fn gate(host: &Host) -> Arc<SaveGate> {
    let gate = Arc::new(SaveGate {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    *host.configuration.save_gate.lock().unwrap() = Some(gate.clone());
    gate
}
pub(super) async fn stopped(host: &Host, caller: &CallerContext) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = host.info(caller).unwrap();
            if info.lifecycle == HostLifecycle::Stopped {
                return;
            }
            assert!(info.shutdown_fault.is_none(), "{info:?}");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_configuration_save_survives_waiter_drop_disconnect_and_force_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open_configured(directory.path(), config("old"), |config| async move {
        Ok(prepared(config, "old-key"))
    })
    .await
    .unwrap();
    let original = host.connect().unwrap();
    let old_network = host.connect_network("old-key").unwrap();
    let observer = host.connect().unwrap();
    let gate = gate(&host);
    let task = {
        let host = host.clone();
        let caller = original.caller();
        tokio::spawn(async move {
            host.reload_configuration(&caller, 0, prepared(config("new"), "new-key"))
                .await
        })
    };
    gate.entered.acquire().await.unwrap().forget();
    // Real accepted owner work continues after both the wait and caller end.
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    original.close();
    let operator = host.connect().unwrap();
    let caller = operator.caller();
    assert!(operator.initial_info().can_operate);
    assert_eq!(host.config_get(&caller).unwrap().config.name, "old");
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("host.json")).unwrap())
            .unwrap();
    assert_eq!(stored["data"]["config"]["name"], "old");
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    assert_eq!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Stopping
    );
    assert!(host.info(&old_network.caller()).is_ok());
    gate.release.add_permits(1);
    stopped(&host, &caller).await;
    assert_eq!(
        host.config_get(&observer.caller()).unwrap().config_revision,
        1
    );
    assert_eq!(host.config_get(&caller).unwrap().config.name, "new");
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.path().join("host.json")).unwrap())
            .unwrap();
    assert_eq!(stored["data"]["config"]["name"], "new");
    assert_eq!(stored["data"]["config_revision"], 1);
    assert!(host.info(&old_network.caller()).is_err());
    assert!(host.info(&caller).is_ok());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn competing_prepared_reloads_cannot_overwrite_the_reserved_version_or_key() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open_configured(directory.path(), config("old"), |config| async move {
        Ok(prepared(config, "old-key"))
    })
    .await
    .unwrap();
    let operator = host.connect().unwrap();
    let network = host.connect_network("old-key").unwrap();
    let gate = gate(&host);
    let first = {
        let host = host.clone();
        let caller = operator.caller();
        tokio::spawn(async move {
            host.reload_configuration(&caller, 0, prepared(config("first"), "first-key"))
                .await
        })
    };
    gate.entered.acquire().await.unwrap().forget();
    let error = host
        .reload_configuration(
            &operator.caller(),
            0,
            prepared(config("second"), "second-key"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Some(FaultKind::HostConfigConflict));
    assert_eq!(
        host.config_get(&operator.caller()).unwrap().config.name,
        "old"
    );
    assert!(host.info(&network.caller()).is_ok());
    gate.release.add_permits(1);
    first.await.unwrap().unwrap();
    assert_eq!(
        host.config_get(&operator.caller()).unwrap().config.name,
        "first"
    );
    assert!(host.connect_network("first-key").is_ok());
    assert!(host.connect_network("second-key").is_err());
}
#[tokio::test]
async fn actual_host_record_failure_keeps_live_configuration_and_key_and_blocks_false_shutdown() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open_configured(directory.path(), config("old"), |config| async move {
        Ok(prepared(config, "old-key"))
    })
    .await
    .unwrap();
    let operator = host.connect().unwrap();
    let caller = operator.caller();
    let network = host.connect_network("old-key").unwrap();
    let path = directory.path().join("host.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let error = host
        .reload_configuration(&caller, 0, prepared(config("new"), "new-key"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Some(FaultKind::HostStorageUnavailable));
    assert_eq!(host.config_get(&caller).unwrap().config_revision, 0);
    assert_eq!(host.config_get(&caller).unwrap().config.name, "old");
    assert!(host.info(&network.caller()).is_ok());
    assert!(host.connect_network("new-key").is_err());
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = host.info(&caller).unwrap();
            if info.shutdown_fault.is_some() {
                assert_eq!(info.lifecycle, HostLifecycle::Stopping);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

struct GatedPreparationResources {
    gate: Arc<SaveGate>,
    execution: minicore_runtime::ExecutionConfig,
}
impl ResourceResolver for GatedPreparationResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.gate.entered.add_permits(1);
            self.gate.release.acquire().await.unwrap().forget();
            Ok(BoundResources {
                execution: self.execution.clone(),
                options: minicore_runtime::LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
#[tokio::test]
async fn turn_preparation_from_an_old_host_revision_never_consumes_the_input() {
    use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
    use minicore_runtime::{
        ExecutionConfig,
        model::{ModelDescriptor, ReasoningPreference as RuntimeReasoning},
        prompt::DefaultPromptProvider,
        tools::ToolSet,
    };
    let gate = Arc::new(SaveGate {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let descriptor = ModelDescriptor::new(
        "default".parse().unwrap(),
        100_000,
        std::collections::BTreeSet::from([RuntimeReasoning::Auto]),
        true,
    )
    .unwrap();
    // Real provider client; a correct rejected preparation never calls it.
    let model = OpenAiChatModel::new(
        OpenAiChatConfig::new(
            "http://127.0.0.1:1/v1/chat/completions",
            "fixture",
            descriptor,
        ),
        SecretApiKey::new("fixture-only-key").unwrap(),
    )
    .unwrap();
    let execution = ExecutionConfig::new(
        Arc::new(model),
        RuntimeReasoning::Auto,
        ToolSet::default(),
        None,
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap();
    let candidate = PreparedHostConfiguration::new(
        config("old"),
        Arc::new(GatedPreparationResources {
            gate: gate.clone(),
            execution,
        }),
        vec![],
        Some("old-key".into()),
    )
    .unwrap();
    let host = Host::memory_configured(candidate).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let created = host
        .create_agent_async(
            &caller,
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "gate fixture".into(),
                mode: StorageMode::Memory,
                config: AgentConfig::default(),
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    let id = MessageId::new().unwrap();
    host.send(
        &caller,
        SendMessage::new(
            MessageInput {
                message_id: id.clone(),
                mode: MessageMode::FollowUp,
                text: "do not consume".into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
    gate.entered.acquire().await.unwrap().forget();
    host.reload_configuration(&caller, 0, prepared(config("new"), "old-key"))
        .await
        .unwrap();
    gate.release.add_permits(1);
    let mut states = host.session_states(&caller, &target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = states.snapshot().unwrap();
            if state.data.block_reason.is_some() {
                assert_eq!(state.data.next_turn[0].message_id, id);
                assert!(!state.data.preparing);
                break;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    assert!(host.history(&caller, &target).unwrap().is_empty());
}
