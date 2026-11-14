//! Directory input is parsed before mutation, then delegated to each existing
//! owner. Its aggregate result is deliberately not a cross-object transaction.
use super::*;
use crate::persistence::disk;
use minicore_host_config::{AgentDefinition, LoadedConfiguration, load_config_directory};
use minicore_host_storage::DataRoot;
use std::path::{Path, PathBuf};

pub(super) type ConfiguredOwners = BTreeMap<String, (Arc<AgentOwner>, AgentInfo)>;

pub(super) async fn read_directory(directory: PathBuf) -> Result<LoadedConfiguration, Fault> {
    tokio::task::spawn_blocking(move || load_config_directory(directory))
        .await
        .map_err(|_| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "configuration directory reader ended unexpectedly",
            )
        })?
}

pub(super) async fn validate_agents(
    candidate: &PreparedHostConfiguration,
    agents: &[AgentDefinition],
) -> Result<(), Fault> {
    let resources = candidate.resources.clone();
    let agents = agents.to_vec();
    tokio::task::spawn_blocking(move || {
        for agent in agents {
            let session = SessionConfig::from_agent(&agent.config, &SessionOverrides::default())
                .map_err(|error| {
                    Fault::new(
                        FaultKind::AgentConfigInvalid,
                        format!("Agent {}: {error}", agent.key),
                    )
                })?;
            resources.validate(&session)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| {
        Fault::new(
            FaultKind::HostInternalUnexpected,
            "Agent configuration validation ended unexpectedly",
        )
    })?
}

impl Host {
    /// Bind one directory for this Host lifetime. Saved records supply stable
    /// identity and history; the complete current directory supplies defaults.
    pub async fn open_directory_configured<F, Fut>(
        data_root: impl AsRef<Path>,
        config_dir: impl AsRef<Path>,
        prepare: F,
    ) -> Result<Arc<Self>, Fault>
    where
        F: FnOnce(HostConfig) -> Fut,
        Fut: std::future::Future<Output = Result<PreparedHostConfiguration, Fault>>,
    {
        let path = data_root.as_ref().to_path_buf();
        let root = disk(FaultKind::HostStorageUnavailable, move || {
            DataRoot::acquire(path)
        })
        .await?;
        let loaded = read_directory(config_dir.as_ref().to_path_buf()).await?;
        let candidate = prepare(loaded.host.clone()).await?;
        if candidate.config != loaded.host {
            return Err(crate::configuration::invalid(
                "resource preparation changed directory configuration values",
            ));
        }
        validate_agents(&candidate, &loaded.agents).await?;
        let reading = root.clone();
        let previous = disk(FaultKind::HostStorageUnavailable, move || {
            reading.load_host::<HostConfig>()
        })
        .await?;
        let record = match previous {
            Some(mut record) => {
                if record.data.config != loaded.host {
                    record.data.config_revision =
                        record.data.config_revision.checked_add(1).ok_or_else(|| {
                            Fault::new(
                                FaultKind::HostInternalUnexpected,
                                "configuration revision exhausted",
                            )
                        })?;
                    record.data.config = loaded.host;
                }
                record
            }
            None => HostRecord::new(
                HostId::new().map_err(|error| {
                    Fault::new(FaultKind::HostInternalUnexpected, error.to_string())
                })?,
                loaded.host,
                0,
            ),
        };
        let store = root.host(record.id.clone());
        let saving = store.clone();
        let saved = record.clone();
        disk(FaultKind::HostStorageUnavailable, move || {
            saving.save_host(&saved)
        })
        .await?;
        let owner = Self::construct(
            record.id,
            record.data.config_revision,
            candidate,
            Some(store),
            HostLifecycle::Starting,
        )?;
        owner
            .configuration_directory
            .set(loaded.directory)
            .expect("new Host directory binding");
        owner.load_persistent_agents_owned().await?;
        let existing = owner.configured_agents_owned()?;
        let results = owner
            .apply_directory_agents(loaded.agents, existing, &Self::operation_id()?)
            .await;
        for result in results {
            match result {
                AgentConfigurationResult::Failed { error, .. } => return Err(error),
                AgentConfigurationResult::Created { receipt, .. }
                    if !receipt.result.steps.agent.succeeded()
                        || !receipt.result.steps.session.succeeded() =>
                {
                    for step in [receipt.result.steps.agent, receipt.result.steps.session] {
                        if let CreateStep::Failed { error } = step {
                            return Err(error);
                        }
                    }
                    return Err(Fault::new(
                        FaultKind::HostStorageUnavailable,
                        "configured Agent creation did not complete",
                    ));
                }
                _ => {}
            }
        }
        owner.state.lock().expect("Host mutex poisoned").lifecycle = HostLifecycle::Running;
        owner
            .lifecycle_record("host.started", "running", StorageMode::Persistent, None)
            .emit();
        Ok(owner)
    }

    pub(super) fn configured_agents_owned(&self) -> Result<ConfiguredOwners, Fault> {
        let agents = {
            let state = self.state.lock().expect("Host mutex poisoned");
            if let Some(fault) = state.load_failures.values().next() {
                return Err(fault.clone());
            }
            if let Some(fault) = state.configuration_creation_failures.values().next() {
                return Err(fault.clone());
            }
            state.agents.values().cloned().collect::<Vec<_>>()
        };
        let mut keyed = BTreeMap::new();
        for agent in agents {
            let info = agent.configuration_info_owned();
            if let Some(source) = &info.configuration_source
                && keyed.insert(source.key.clone(), (agent, info)).is_some()
            {
                return Err(crate::configuration::invalid(
                    "multiple persistent Agents claim the same configuration key",
                ));
            }
        }
        Ok(keyed)
    }

    pub(super) async fn apply_directory_agents(
        self: &Arc<Self>,
        definitions: Vec<AgentDefinition>,
        mut existing: ConfiguredOwners,
        operation: &OperationId,
    ) -> Vec<AgentConfigurationResult> {
        let mut results = Vec::new();
        for definition in definitions {
            let key = definition.key.clone();
            if let Some((agent, original)) = existing.remove(&key) {
                let unchanged = original.name == definition.name
                    && original.config == definition.config
                    && original
                        .configuration_source
                        .as_ref()
                        .is_some_and(|source| source.managed);
                let result = agent
                    .apply_configuration_owned(
                        original.config_revision,
                        definition.name,
                        definition.config,
                        AgentConfigurationSource {
                            key: key.clone(),
                            managed: true,
                        },
                        operation,
                    )
                    .await;
                results.push(match result {
                    Ok(info) if unchanged => AgentConfigurationResult::Unchanged {
                        key,
                        agent_id: info.id,
                        config_revision: info.config_revision,
                    },
                    Ok(info) => AgentConfigurationResult::Updated {
                        key,
                        agent_id: info.id,
                        config_revision: info.config_revision,
                    },
                    Err(error) => AgentConfigurationResult::Failed {
                        key,
                        agent_id: Some(original.id),
                        error,
                    },
                });
            } else {
                // The containing reload keeps its public correlation. A child
                // creation has its own native operation, carried by the nested
                // receipt, so receipt.get(reload_id) remains unambiguous.
                let created = match Self::operation_id() {
                    Ok(creation) => {
                        self.create_configured_agent_owned(definition, creation)
                            .await
                    }
                    Err(error) => Err(error),
                };
                results.push(match created {
                    Ok(receipt) => AgentConfigurationResult::Created {
                        key,
                        receipt: Box::new(receipt),
                    },
                    Err(error) => AgentConfigurationResult::Failed {
                        key,
                        agent_id: None,
                        error,
                    },
                });
            }
        }
        for (key, (agent, original)) in existing {
            if !original
                .configuration_source
                .as_ref()
                .is_some_and(|source| source.managed)
            {
                continue;
            }
            let result = agent
                .apply_configuration_owned(
                    original.config_revision,
                    original.name,
                    original.config,
                    AgentConfigurationSource {
                        key: key.clone(),
                        managed: false,
                    },
                    operation,
                )
                .await;
            results.push(match result {
                Ok(info) => AgentConfigurationResult::Unmanaged {
                    key,
                    agent_id: info.id,
                    config_revision: info.config_revision,
                },
                Err(error) => AgentConfigurationResult::Failed {
                    key,
                    agent_id: Some(original.id),
                    error,
                },
            });
        }
        results
    }
}
