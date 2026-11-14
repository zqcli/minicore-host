//! Host-owned publication of completely prepared configuration candidates.
use super::*;
use crate::configuration::{
    HostConfigSnapshot, HostReloadResult, ModelDescription, PublishedConfiguration,
};
use crate::persistence::disk;
use minicore_host_runtime::tools::ToolSpec;
use tokio::sync::oneshot;

impl Host {
    pub fn config_get(&self, caller: &CallerContext) -> Result<HostConfigSnapshot, Fault> {
        self.access.read(caller, || {
            let current = self.configuration.snapshot();
            Ok(HostConfigSnapshot {
                config: current.config.clone(),
                config_revision: current.revision,
            })
        })
    }
    /// Complete references, in lexical order. No credentials or clients.
    pub fn model_list(&self, caller: &CallerContext) -> Result<Vec<ModelDescription>, Fault> {
        self.access.read(caller, || {
            let current = self.configuration.snapshot();
            let mut models: Vec<_> = current
                .config
                .models
                .iter()
                .map(ModelDescription::from)
                .collect();
            models.sort_by(|a, b| a.model.cmp(&b.model));
            Ok(models)
        })
    }
    /// The actual available declarations, in lexical tool-name order.
    pub fn tool_list(&self, caller: &CallerContext) -> Result<Vec<ToolSpec>, Fault> {
        self.access
            .read(caller, || Ok(self.configuration.snapshot().tools.clone()))
    }

    /// Filter against the direct owner's committed selection. Missing selected
    /// resources remain an explicit fault rather than silently disappearing.
    pub fn tool_list_for_agent(
        &self,
        caller: &CallerContext,
        target: &AgentRef,
    ) -> Result<Vec<ToolSpec>, Fault> {
        let selected = self
            .agent_info(caller, target)?
            .config
            .session_defaults
            .tools;
        self.selected_tool_specs(caller, &selected)
    }
    pub fn tool_list_for_session(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<Vec<ToolSpec>, Fault> {
        let selected = self.session_info(caller, target)?.config.tools;
        self.selected_tool_specs(caller, &selected)
    }
    fn selected_tool_specs(
        &self,
        caller: &CallerContext,
        selected: &[ToolBinding],
    ) -> Result<Vec<ToolSpec>, Fault> {
        let available = self.tool_list(caller)?;
        let mut result = Vec::new();
        for selection in selected {
            let spec = available
                .iter()
                .find(|spec| spec.name().as_str() == selection.name)
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionResourceMissing,
                        format!("selected tool {} is not configured", selection.name),
                    )
                })?;
            result.push(spec.clone());
        }
        result.sort_by(|a, b| a.name().cmp(b.name()));
        Ok(result)
    }

    /// Accept a reread of the startup-bound directory. The accepted Host work
    /// owns parsing, preparation, per-owner saves and cleanup after disconnect.
    pub fn reload(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: HostReloadRequest,
    ) -> Result<HostReloadReceipt, Fault> {
        let runtime = reload_runtime()?;
        let directory = self.configuration_directory.get().cloned().ok_or_else(|| {
            Fault::new(
                FaultKind::HostMethodUnavailable,
                "Host has no startup-bound configuration directory",
            )
        })?;
        let preparer = self
            .configuration
            .state
            .lock()
            .expect("Host configuration mutex poisoned")
            .preparer
            .clone()
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::HostMethodUnavailable,
                    "Host has no configured application resource preparer",
                )
            })?;
        let (result, index) = self.claim_configuration(
            caller,
            request.expected_revision,
            None,
            Some(request.operation_id.clone()),
            true,
        )?;
        let receipt = self.reload_receipt(request.operation_id.clone(), result);
        let owner = Arc::clone(self);
        let store = self.store();
        runtime.spawn(async move {
            let work = async {
                let loaded = super::configuration_directory::read_directory(directory).await?;
                let prepared = preparer.prepare(loaded.host.clone()).await?;
                if prepared.config != loaded.host {
                    return Err(crate::configuration::invalid(
                        "resource preparation changed directory configuration values",
                    ));
                }
                super::configuration_directory::validate_agents(&prepared, &loaded.agents).await?;
                let existing = owner.configured_agents_owned()?;
                Ok((loaded, prepared, existing))
            }
            .await;
            drop(preparer);
            let (loaded, prepared, existing) = match work {
                Ok(work) => work,
                Err(fault) => {
                    owner.finish_configuration_failure(index.as_ref(), fault, None, false);
                    return;
                }
            };
            let fields = owner
                .configuration
                .snapshot()
                .config
                .restart_changes(&loaded.host);
            if !fields.is_empty() {
                drop(prepared);
                let mut state = owner
                    .configuration
                    .state
                    .lock()
                    .expect("Host configuration mutex poisoned");
                state.saving = false;
                update_reload(
                    &mut state,
                    index.as_ref(),
                    HostReloadResult::RestartRequired {
                        config_revision: request.expected_revision,
                        fields,
                    },
                );
                drop(state);
                owner.changed.notify_waiters();
                return;
            }
            let revision = request.expected_revision + 1;
            if owner
                .publish_configuration(prepared, revision, store, index.as_ref(), false)
                .await
                .is_err()
            {
                return;
            }
            let agents = owner
                .apply_directory_agents(loaded.agents, existing, &request.operation_id)
                .await;
            let result = if agents.iter().all(AgentConfigurationResult::succeeded) {
                HostReloadResult::Applied {
                    config_revision: revision,
                    agents,
                }
            } else {
                HostReloadResult::PartiallyApplied {
                    config_revision: revision,
                    agents,
                }
            };
            let mut state = owner
                .configuration
                .state
                .lock()
                .expect("Host configuration mutex poisoned");
            update_reload(&mut state, index.as_ref(), result);
            state.saving = false;
            drop(state);
            owner.changed.notify_waiters();
        });
        Ok(receipt)
    }

    /// Low-level embedding convenience for an already prepared candidate.
    /// Canonical clients use reload with their own operation_id. Both paths
    /// share the same reservation, necessary save and publication boundary.
    pub async fn reload_configuration(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected_revision: ConfigRevision,
        candidate: PreparedHostConfiguration,
    ) -> Result<HostReloadResult, Fault> {
        if self.configuration_directory.get().is_some() {
            return Err(Fault::new(
                FaultKind::HostMethodUnavailable,
                "directory-bound Hosts reload their files through host.reload",
            ));
        }
        let runtime = reload_runtime()?;
        let (result, _) = self.claim_configuration(
            caller,
            expected_revision,
            Some(&candidate.config),
            None,
            false,
        )?;
        if matches!(result, HostReloadResult::RestartRequired { .. }) {
            return Ok(result);
        }
        let (sender, receiver) = oneshot::channel();
        let owner = Arc::clone(self);
        let store = self.store();
        runtime.spawn(async move {
            let result = owner
                .publish_configuration(candidate, expected_revision + 1, store, None, true)
                .await;
            let _ = sender.send(result);
        });
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "Host configuration owner ended without a result",
            )
        })?
    }

    fn reload_receipt(
        &self,
        operation_id: OperationId,
        result: HostReloadResult,
    ) -> HostReloadReceipt {
        HostReloadReceipt {
            kind: HostReloadKind::HostReload,
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            operation_id,
            result,
        }
    }

    fn claim_configuration(
        &self,
        caller: &CallerContext,
        expected_revision: ConfigRevision,
        candidate: Option<&HostConfig>,
        operation: Option<OperationId>,
        preparing: bool,
    ) -> Result<(HostReloadResult, Option<ReloadIndex>), Fault> {
        self.access.accept_business(caller, || {
            let host = self.state.lock().expect("Host mutex poisoned");
            if host.lifecycle != HostLifecycle::Running {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host configuration cannot change during shutdown",
                ));
            }
            let mut state = self
                .configuration
                .state
                .lock()
                .expect("Host configuration mutex poisoned");
            if state.snapshot.revision != expected_revision
                || state.saving
                || state.save_fault.is_some()
            {
                return Err(Fault::new(
                    FaultKind::HostConfigConflict,
                    "Host configuration changed or has an unconfirmed save",
                ));
            }
            let fields = candidate
                .map(|candidate| state.snapshot.config.restart_changes(candidate))
                .unwrap_or_default();
            let result = if !fields.is_empty() {
                HostReloadResult::RestartRequired {
                    config_revision: expected_revision,
                    fields,
                }
            } else {
                expected_revision.checked_add(1).ok_or_else(|| {
                    Fault::new(
                        FaultKind::HostInternalUnexpected,
                        "Host configuration revision exhausted",
                    )
                })?;
                // The existing shutdown barrier covers preparation and cleanup
                // as well as I/O. No later writer may steal this reservation.
                state.saving = true;
                if preparing {
                    HostReloadResult::Preparing {
                        config_revision: expected_revision,
                    }
                } else {
                    HostReloadResult::Applying {
                        config_revision: expected_revision,
                    }
                }
            };
            let index = operation.map(|operation| {
                let records = state.reloads.entry(operation.clone()).or_default();
                let index = records.len();
                records.push(self.reload_receipt(operation.clone(), result.clone()));
                (operation, index)
            });
            Ok((result, index))
        })
    }

    fn finish_configuration_failure(
        &self,
        index: Option<&ReloadIndex>,
        fault: Fault,
        candidate: Option<PreparedHostConfiguration>,
        save_failed: bool,
    ) {
        let mut state = self
            .configuration
            .state
            .lock()
            .expect("Host configuration mutex poisoned");
        state.saving = false;
        if save_failed {
            // A failed save may have reached disk before sync. Keep its fact
            // and original candidate; do not claim a confirmed save or stopped.
            state.save_fault = Some(fault.clone());
            state.failed_candidate = candidate;
        }
        let result = HostReloadResult::Failed {
            config_revision: state.snapshot.revision,
            error: fault,
        };
        update_reload(&mut state, index, result);
        drop(state);
        self.changed.notify_waiters();
    }

    async fn publish_configuration(
        self: &Arc<Self>,
        mut candidate: PreparedHostConfiguration,
        revision: ConfigRevision,
        store: Option<HostStore>,
        index: Option<&ReloadIndex>,
        complete: bool,
    ) -> Result<HostReloadResult, Fault> {
        // Reload never replaces the startup assembler. Release any redundant
        // candidate handle outside owner locks and before work is settled.
        drop(candidate.preparer.take());
        {
            let mut state = self
                .configuration
                .state
                .lock()
                .expect("Host configuration mutex poisoned");
            debug_assert!(state.saving && state.snapshot.revision + 1 == revision);
            let current = state.snapshot.revision;
            update_reload(
                &mut state,
                index,
                HostReloadResult::Applying {
                    config_revision: current,
                },
            );
        }
        #[cfg(test)]
        {
            let gate = self.configuration.save_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.entered.add_permits(1);
                gate.release.acquire().await.unwrap().forget();
            }
        }
        let saved = if let Some(store) = store {
            let record = HostRecord::new(self.id.clone(), candidate.config.clone(), revision);
            disk(FaultKind::HostStorageUnavailable, move || {
                store.save_host(&record)
            })
            .await
            .map(|_| ())
        } else {
            Ok(())
        };
        if let Err(fault) = saved {
            self.finish_configuration_failure(index, fault.clone(), Some(candidate), true);
            return Err(fault);
        }
        // This is accepted Host work, not a new request or a forged internal
        // connection. Publication and key invalidation share the same boundary.
        let published = self.access.publish_network(None, candidate.access_key, || {
            let mut state = self
                .configuration
                .state
                .lock()
                .expect("Host configuration mutex poisoned");
            debug_assert!(state.saving && state.snapshot.revision + 1 == revision);
            let published = Arc::new(PublishedConfiguration {
                config: candidate.config,
                revision,
                tools: candidate.tools,
                directories: candidate.directories,
                resources: Some(candidate.resources),
            });
            // Only a successful publication tightens launch inheritance. Old
            // Turn bindings share this name-only policy, not the new resources.
            self.configuration
                .tool_environment
                .include(&published.config);
            let old = std::mem::replace(&mut state.snapshot, published);
            update_reload(
                &mut state,
                index,
                if complete {
                    HostReloadResult::Applied {
                        config_revision: revision,
                        agents: Vec::new(),
                    }
                } else {
                    HostReloadResult::Applying {
                        config_revision: revision,
                    }
                },
            );
            Ok(old)
        });
        match published {
            // Real resource cleanup is outside all locks and precedes the
            // shutdown barrier's completion, even after caller wait is gone.
            Ok(old) => drop(old),
            Err(fault) => {
                self.finish_configuration_failure(index, fault.clone(), None, true);
                return Err(fault);
            }
        }
        let result = HostReloadResult::Applied {
            config_revision: revision,
            agents: Vec::new(),
        };
        let mut state = self
            .configuration
            .state
            .lock()
            .expect("Host configuration mutex poisoned");
        if complete {
            state.saving = false;
        }
        drop(state);
        self.changed.notify_waiters();
        Ok(result)
    }
}

/// Locates one invocation within the configuration owner's existing facts.
/// Reusing an operation_id creates another entry, never an idempotent replay.
type ReloadIndex = (OperationId, usize);
fn update_reload(
    state: &mut crate::configuration::ConfigurationState,
    index: Option<&ReloadIndex>,
    result: HostReloadResult,
) {
    if let Some((operation, index)) = index {
        state.reloads.get_mut(operation).expect("accepted reload")[*index].result = result;
    }
}
fn reload_runtime() -> Result<tokio::runtime::Handle, Fault> {
    tokio::runtime::Handle::try_current().map_err(|_| {
        Fault::new(
            FaultKind::HostLifecycleUnavailable,
            "reload requires a live Tokio runtime",
        )
    })
}
