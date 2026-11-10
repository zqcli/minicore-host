use super::*;
use crate::agent::shutdown::creation_save_fault;
use minicore_host_diagnostics::Context;
use tracing::{Instrument, instrument::WithSubscriber};

impl Host {
    /// Accept a controlled shutdown without waiting for model/tool completion,
    /// persistence, or cleanup. The Host owns that work after this call returns.
    pub fn shutdown(
        self: &Arc<Self>,
        caller: &CallerContext,
        request: HostShutdownRequest,
    ) -> Result<HostShutdownReceipt, Fault> {
        self.shutdown_owned(Some(caller), request)
    }

    pub(super) fn shutdown_owned(
        self: &Arc<Self>,
        caller: Option<&CallerContext>,
        request: HostShutdownRequest,
    ) -> Result<HostShutdownReceipt, Fault> {
        let executor = tokio::runtime::Handle::try_current().ok();
        let mode = if self.store().is_some() {
            StorageMode::Persistent
        } else {
            StorageMode::Memory
        };
        let context = Context {
            operation_id: Some(request.operation_id.clone()),
            ..self.diagnostic_context()
        };
        let mut span = Some(context.span());
        let accept = || {
            let mut state = self.state.lock().expect("Host mutex poisoned");
            if state.lifecycle == HostLifecycle::Starting {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "Host initialization is still running",
                ));
            }
            let previous = (state.lifecycle, state.shutdown_strategy);
            let first = state.shutdown_strategy.is_none();
            if first && executor.is_none() {
                return Err(Fault::new(
                    FaultKind::HostLifecycleUnavailable,
                    "shutdown requires a live Tokio runtime",
                ));
            }
            let strategy = if state.shutdown_strategy == Some(HostShutdownStrategy::Force)
                || request.strategy == HostShutdownStrategy::Force
            {
                HostShutdownStrategy::Force
            } else {
                HostShutdownStrategy::Graceful
            };
            state.shutdown_strategy = Some(strategy);
            if state.lifecycle != HostLifecycle::Stopped {
                state.lifecycle = if strategy == HostShutdownStrategy::Force
                    || state.lifecycle == HostLifecycle::Stopping
                {
                    HostLifecycle::Stopping
                } else {
                    HostLifecycle::Draining
                };
                self.access.set_host_lifecycle_owned(state.lifecycle);
            }
            if first {
                let owner = Arc::clone(self);
                let operation = request.operation_id.clone();
                state.shutdown_task = Some(
                    executor.expect("checked executor").spawn(
                        async move {
                            owner.run_shutdown(mode, operation).await;
                        }
                        .instrument(span.take().expect("accepted operation span"))
                        .with_current_subscriber(),
                    ),
                );
            }
            let receipt = HostShutdownReceipt {
                kind: HostShutdownKind::HostShutdown,
                host_id: self.id.clone(),
                host_incarnation: self.incarnation.clone(),
                operation_id: request.operation_id,
                result: HostShutdownResult {
                    strategy,
                    lifecycle: state.lifecycle,
                },
            };
            state
                .controls
                .entry(receipt.operation_id.clone())
                .or_default()
                .push(Receipt::HostShutdown(receipt.clone()));
            let diagnostic = (previous != (state.lifecycle, state.shutdown_strategy)).then(|| {
                self.lifecycle_record(
                    "host.shutdown_accepted",
                    match state.lifecycle {
                        HostLifecycle::Draining => "draining",
                        HostLifecycle::Stopping => "stopping",
                        HostLifecycle::Stopped => "stopped",
                        _ => unreachable!("accepted shutdown"),
                    },
                    mode,
                    Some(receipt.operation_id.clone()),
                )
            });
            Ok((
                (
                    state.lifecycle,
                    state.agents.values().cloned().collect::<Vec<_>>(),
                ),
                receipt,
                diagnostic,
            ))
        };
        let (children, receipt, diagnostic) = match caller {
            Some(caller) => self.access.accept(caller, accept)?,
            None => self.access.accept_internal_owned(accept)?,
        };
        crate::diagnostics::emit(diagnostic);
        let stopping = matches!(children.0, HostLifecycle::Stopping | HostLifecycle::Stopped);
        if children.0 != HostLifecycle::Stopped {
            for child in children.1 {
                child.begin_host_shutdown_owned(stopping);
            }
        }
        self.publish_host_state_owned();
        self.changed.notify_waiters();
        self.process_changed.notify_waiters();
        Ok(receipt)
    }

    async fn run_shutdown(self: &Arc<Self>, mode: StorageMode, operation: OperationId) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (lifecycle, children, creating, creation_fault) = {
                let state = self.state.lock().expect("Host mutex poisoned");
                (
                    state.lifecycle,
                    state.agents.values().cloned().collect::<Vec<_>>(),
                    state.creating + state.deleting,
                    state.creations.values().find_map(|receipt| {
                        creation_save_fault(&receipt.result.steps.agent)
                            .or_else(|| creation_save_fault(&receipt.result.steps.session))
                    }),
                )
            };
            let stopping = lifecycle == HostLifecycle::Stopping;
            let (configuration_saving, configuration_fault) = self.configuration.status();
            let mut drained = creating == 0 && !configuration_saving;
            let mut quiescent = creating == 0 && !configuration_saving;
            let mut fault = creation_fault.or(configuration_fault);
            for child in &children {
                child.begin_host_shutdown_owned(stopping);
                let status = child.shutdown_status_owned();
                drained &= status.drained;
                quiescent &= status.quiescent;
                if fault.is_none() {
                    fault = status.fault;
                }
            }
            {
                let mut state = self.state.lock().expect("Host mutex poisoned");
                if state.shutdown_fault != fault {
                    state.shutdown_fault = fault.clone();
                    self.process_changed.notify_waiters();
                }
                if lifecycle == HostLifecycle::Draining
                    && drained
                    && fault.is_none()
                    && state.lifecycle == HostLifecycle::Draining
                    && state.creating == 0
                {
                    state.lifecycle = HostLifecycle::Stopping;
                    self.access
                        .set_host_lifecycle_owned(HostLifecycle::Stopping);
                    self.changed.notify_waiters();
                    self.process_changed.notify_waiters();
                    drop(state);
                    self.lifecycle_record(
                        "host.stopping",
                        "stopping",
                        mode,
                        Some(operation.clone()),
                    )
                    .emit();
                    self.publish_host_state_owned();
                    continue;
                }
            }
            if stopping && quiescent && fault.is_none() {
                // A normal-close reaper may already own a child retirement.
                // Join that real cleanup before final shutdown resource release.
                let watcher = self
                    .state
                    .lock()
                    .expect("Host mutex poisoned")
                    .retirement_watch
                    .take();
                if let Some(watcher) = watcher
                    && watcher.await.is_err()
                {
                    self.state
                        .lock()
                        .expect("Host mutex poisoned")
                        .shutdown_fault = Some(Fault::new(
                        FaultKind::HostInternalUnexpected,
                        "memory retirement owner did not join cleanly",
                    ));
                    self.publish_host_state_owned();
                    self.process_changed.notify_waiters();
                    return;
                }
                let mut failure = None;
                for child in children {
                    if let Err(error) = child.release_host_resources_owned().await {
                        failure = Some(error);
                        break;
                    }
                }
                if let Some(error) = failure {
                    self.state
                        .lock()
                        .expect("Host mutex poisoned")
                        .shutdown_fault = Some(error);
                    self.process_changed.notify_waiters();
                } else {
                    self.remove_retired_agents_owned();
                    // Every actual writer and child resource is now retired.
                    // Retaining this public Host handle must not retain the lock.
                    self.configuration.retire();
                    drop(self.store.lock().expect("Host store mutex poisoned").take());
                    drop(
                        self.resources
                            .lock()
                            .expect("Host resources mutex poisoned")
                            .take(),
                    );
                    let mut state = self.state.lock().expect("Host mutex poisoned");
                    state.lifecycle = HostLifecycle::Stopped;
                    state.shutdown_fault = None;
                    self.access.set_host_lifecycle_owned(HostLifecycle::Stopped);
                    state.shutdown_task.take();
                    self.changed.notify_waiters();
                    self.process_changed.notify_waiters();
                    drop(state);
                    self.lifecycle_record("host.stopped", "stopped", mode, Some(operation.clone()))
                        .emit();
                    self.publish_host_state_owned();
                    return;
                }
            }
            self.publish_host_state_owned();
            notified.await;
        }
    }
}
