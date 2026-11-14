//! Real configuration owners and storage with an explicit nonexecuting resource
//! double. Only synthetic source names and private fixture keys are used.
use super::*;
use crate::Host;
use std::time::Duration;
use tokio::sync::Semaphore;

struct NoResources;
impl ResourceResolver for NoResources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async { Err(invalid("nonexecuting resource test double")) })
    }
}

fn config(models: &[&str], network: Option<&str>) -> HostConfig {
    HostConfig {
        models: models
            .iter()
            .enumerate()
            .map(|(index, name)| OpenAiModelSettings {
                api: OpenAiModelApi::ChatCompletions,
                model: format!("custom/provider/model{index}").parse().unwrap(),
                endpoint: "http://127.0.0.1:1/v1/chat/completions".into(),
                provider_model: "test-model".into(),
                api_key_env: (*name).into(),
                context_window: 1_000,
                supported_reasoning: vec![ReasoningPreference::Auto],
                supports_tools: true,
                request_timeout_ms: None,
            })
            .collect(),
        network: NetworkSettings {
            listen: None,
            access_key_env: network.map(str::to_owned),
        },
        ..HostConfig::default()
    }
}
fn prepared(config: HostConfig) -> PreparedHostConfiguration {
    let key = config
        .network
        .access_key_env
        .as_ref()
        .map(|_| "synthetic-network-key".into());
    PreparedHostConfiguration::new(config, Arc::new(NoResources), vec![], key).unwrap()
}
fn names(view: &ToolEnvironmentExclusions, expected: &[&str]) {
    let mut expected: Vec<String> = expected.iter().map(|name| (*name).into()).collect();
    expected.sort();
    assert_eq!(view.names(), expected);
}

#[tokio::test]
async fn published_names_are_cumulative_deduplicated_and_host_local() {
    let host = Host::memory_configured(prepared(config(
        &["TASK066_MODEL_A", "TASK066_MODEL_B", "TASK066_MODEL_A"],
        Some("TASK066_NETWORK_A"),
    )))
    .unwrap();
    let view = host.configuration.tool_environment.clone();
    let original = ["TASK066_MODEL_A", "TASK066_MODEL_B", "TASK066_NETWORK_A"];
    names(&view, &original);
    let other = Host::memory_configured(prepared(config(&["TASK066_OTHER"], None))).unwrap();
    let other_view = other.configuration.tool_environment.clone();
    let candidate = prepared(config(&["TASK066_MODEL_NEW"], None));
    names(&view, &original); // Construction alone cannot publish a candidate.
    let connection = host.connect().unwrap();
    assert_eq!(
        host.reload_configuration(&connection.caller(), 0, candidate)
            .await
            .unwrap(),
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    names(
        &view,
        &[
            "TASK066_MODEL_A",
            "TASK066_MODEL_B",
            "TASK066_NETWORK_A",
            "TASK066_MODEL_NEW",
        ],
    );
    names(&other_view, &["TASK066_OTHER"]);
    assert!(
        host.config_get(&connection.caller())
            .unwrap()
            .config
            .network
            .access_key_env
            .is_none()
    );
}

struct RejectPreparation {
    entered: Semaphore,
    release: Semaphore,
}
impl HostConfigurationPreparer for RejectPreparation {
    fn prepare(&self, _: HostConfig) -> HostConfigurationFuture<'_> {
        Box::pin(async {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            Err(invalid("explicit preparation rejection"))
        })
    }
}

#[tokio::test]
async fn preparation_failure_and_restart_candidate_do_not_publish_names() {
    let preparer = Arc::new(RejectPreparation {
        entered: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    let host = Host::memory_configured(
        prepared(config(&["TASK066_OLD"], None)).with_preparer(preparer.clone()),
    )
    .unwrap();
    let _configuration_directory = reload_tests::bind_directory(&host);
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let view = host.configuration.tool_environment.clone();
    let operation = OperationId::new().unwrap();
    reload_tests::write_candidate(
        &host,
        &config(&["TASK066_REJECTED"], Some("TASK066_REJECTED_NETWORK")),
    );
    host.reload(
        &caller,
        HostReloadRequest {
            operation_id: operation.clone(),
            expected_revision: 0,
        },
    )
    .unwrap();
    preparer.entered.acquire().await.unwrap().forget();
    names(&view, &["TASK066_OLD"]);
    preparer.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Receipt::HostReload(receipt) = host.receipt_get(&caller, &operation).unwrap()
                && matches!(receipt.result, HostReloadResult::Failed { .. })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    names(&view, &["TASK066_OLD"]);
    let other = Host::memory_configured(prepared(config(&["TASK066_OLD"], None))).unwrap();
    let control = other.connect().unwrap();
    let mut restart = config(&["TASK066_RESTART"], Some("TASK066_RESTART_NETWORK"));
    restart.logging.level = LogLevel::Trace;
    let result = other
        .reload_configuration(&control.caller(), 0, prepared(restart))
        .await
        .unwrap();
    assert!(matches!(result, HostReloadResult::RestartRequired { .. }));
    names(&view, &["TASK066_OLD"]);
    names(&other.configuration.tool_environment, &["TASK066_OLD"]);
    assert_eq!(host.config_get(&caller).unwrap().config_revision, 0);
}

#[tokio::test]
async fn save_in_progress_and_conflict_leave_policy_unchanged_until_publication() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open_configured(
        directory.path(),
        config(&["TASK066_OLD"], None),
        |value| async { Ok(prepared(value)) },
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let view = host.configuration.tool_environment.clone();
    let gate = tests::gate(&host);
    let pending = {
        let host = host.clone();
        let caller = caller.clone();
        tokio::spawn(async move {
            host.reload_configuration(&caller, 0, prepared(config(&["TASK066_PUBLISHED"], None)))
                .await
        })
    };
    gate.entered.acquire().await.unwrap().forget();
    names(&view, &["TASK066_OLD"]);
    let rejected = host
        .reload_configuration(&caller, 0, prepared(config(&["TASK066_CONFLICT"], None)))
        .await
        .unwrap_err();
    assert_eq!(rejected.kind(), Some(FaultKind::HostConfigConflict));
    names(&view, &["TASK066_OLD"]);
    gate.release.add_permits(1);
    assert_eq!(
        pending.await.unwrap().unwrap(),
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    names(&view, &["TASK066_OLD", "TASK066_PUBLISHED"]);
}

#[tokio::test]
async fn actual_save_failure_keeps_candidate_names_unpublished() {
    let directory = tempfile::tempdir().unwrap();
    let host = Host::open_configured(
        directory.path(),
        config(&["TASK066_OLD"], Some("TASK066_OLD_NETWORK")),
        |value| async { Ok(prepared(value)) },
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let network = host.connect_network("synthetic-network-key").unwrap();
    let view = host.configuration.tool_environment.clone();
    let path = directory.path().join("host.json");
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let error = host
        .reload_configuration(
            &caller,
            0,
            prepared(config(
                &["TASK066_UNSAVED"],
                Some("TASK066_UNSAVED_NETWORK"),
            )),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), Some(FaultKind::HostStorageUnavailable));
    names(&view, &["TASK066_OLD", "TASK066_OLD_NETWORK"]);
    assert_eq!(host.config_get(&caller).unwrap().config_revision, 0);
    assert!(host.info(&network.caller()).is_ok());
}
