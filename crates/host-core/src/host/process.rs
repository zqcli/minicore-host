use super::*;

/// Trusted application lifecycle capability, not a connection or a business caller.
#[derive(Clone)]
pub struct HostProcessControl {
    host: Weak<Host>,
}

/// Actual owner facts for process supervision, independent of client operation rights.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostProcessSnapshot {
    pub host_id: HostId,
    pub host_incarnation: HostIncarnation,
    pub lifecycle: HostLifecycle,
    pub shutdown_strategy: Option<HostShutdownStrategy>,
    pub shutdown_fault: Option<Fault>,
}

impl Host {
    /// Give the trusted embedding only lifecycle supervision, never a client slot.
    pub fn process_control(self: &Arc<Self>) -> HostProcessControl {
        HostProcessControl {
            host: Arc::downgrade(self),
        }
    }
}
impl HostProcessControl {
    fn owner(&self) -> Result<Arc<Host>, Fault> {
        self.host.upgrade().ok_or_else(|| {
            Fault::new(
                FaultKind::HostLifecycleUnavailable,
                "Host process owner is no longer available",
            )
        })
    }
    pub fn snapshot(&self) -> Result<HostProcessSnapshot, Fault> {
        let host = self.owner()?;
        let state = host.state.lock().expect("Host mutex poisoned");
        Ok(HostProcessSnapshot {
            host_id: host.id.clone(),
            host_incarnation: host.incarnation.clone(),
            lifecycle: state.lifecycle,
            shutdown_strategy: state.shutdown_strategy,
            shutdown_fault: state.shutdown_fault.clone(),
        })
    }
    /// Observe a changed snapshot; cancelling this wait changes no business work.
    pub async fn changed(
        &self,
        previous: &HostProcessSnapshot,
    ) -> Result<HostProcessSnapshot, Fault> {
        let host = self.owner()?;
        loop {
            let notified = host.process_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let current = self.snapshot()?;
            if current != *previous {
                return Ok(current);
            }
            notified.await;
        }
    }
    /// Accept an explicit application shutdown at the same Host admission boundary.
    pub fn shutdown(&self, request: HostShutdownRequest) -> Result<HostShutdownReceipt, Fault> {
        self.owner()?.shutdown_owned(None, request)
    }
}
