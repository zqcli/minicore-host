//! Host-owned publication of completely prepared configuration candidates.
use super::*;
use crate::configuration::{
    HostConfigSnapshot, HostReloadResult, ModelDescription, PublishedConfiguration,
};
use crate::persistence::disk;
use minicore_runtime::tools::ToolSpec;
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

    /// Publish a fully prepared candidate. The application prepares it outside
    /// all owner locks; no callback, provider request or tool executes here.
    /// Expected revision comes from config_get before preparation.
    ///
    /// Once accepted, the Host owns any necessary save independently of this
    /// waiter. It reserves the configuration version before I/O, so a rejected
    /// competitor never writes host.json or changes authentication.
    pub async fn reload_configuration(
        self: &Arc<Self>,
        caller: &CallerContext,
        expected_revision: ConfigRevision,
        candidate: PreparedHostConfiguration,
    ) -> Result<HostReloadResult, Fault> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "reload requires a live Tokio runtime",
            )
        })?;
        let (sender, receiver) = oneshot::channel();
        let store = self.store();
        // Keep rejected resources outside the authorization closure so their
        // destruction cannot run under the connection/owner locks.
        let mut candidate = Some(candidate);
        let restart = self.access.accept_business(caller, || {
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
            let fields = state
                .snapshot
                .config
                .restart_changes(&candidate.as_ref().expect("unclaimed candidate").config);
            if !fields.is_empty() {
                return Ok(Some(HostReloadResult::RestartRequired {
                    config_revision: state.snapshot.revision,
                    fields,
                }));
            }
            let revision = expected_revision.checked_add(1).ok_or_else(|| {
                Fault::new(
                    FaultKind::HostInternalUnexpected,
                    "Host configuration revision exhausted",
                )
            })?;
            state.saving = true;
            let candidate = candidate.take().expect("unclaimed candidate");
            let owner = Arc::clone(self);
            // The accepted task keeps the root/store and original candidate.
            runtime.spawn(async move {
                let result = owner
                    .publish_configuration(candidate, revision, store)
                    .await;
                let _ = sender.send(result);
            });
            Ok(None)
        })?;
        if let Some(result) = restart {
            return Ok(result);
        }
        receiver.await.map_err(|_| {
            Fault::new(
                FaultKind::HostInternalUnexpected,
                "Host configuration owner ended without a result",
            )
        })?
    }

    async fn publish_configuration(
        self: &Arc<Self>,
        candidate: PreparedHostConfiguration,
        revision: ConfigRevision,
        store: Option<HostStore>,
    ) -> Result<HostReloadResult, Fault> {
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
            let mut state = self
                .configuration
                .state
                .lock()
                .expect("Host configuration mutex poisoned");
            state.saving = false;
            // A failed save may have reached the filesystem before sync. Keep
            // its fact and prevent shutdown from claiming a confirmed save.
            state.save_fault = Some(fault.clone());
            state.failed_candidate = Some(candidate);
            drop(state);
            self.changed.notify_waiters();
            return Err(fault);
        }
        // This is the continuation of the accepted Host work, not a new request
        // or a forged internal connection. Disconnect cannot undo its save.
        let result = self.access.publish_network(None, candidate.access_key, || {
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
                resources: Some(candidate.resources),
            });
            let old = std::mem::replace(&mut state.snapshot, published);
            Ok(old)
        });
        // Dropping old provider/tool resources is deliberately outside all locks.
        drop(result?);
        self.configuration
            .state
            .lock()
            .expect("Host configuration mutex poisoned")
            .saving = false;
        self.changed.notify_waiters();
        Ok(HostReloadResult::Applied {
            config_revision: revision,
        })
    }
}
