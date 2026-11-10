//! Actual ordinary-tool workers remain owned beyond Runtime's ToolFuture.
use super::*;

/// A trusted tool bundle's execution owner, registered before Turn activation.
/// Called once, only after the public Turn has claimed its terminal outcome.
/// Close admission and join every actual worker, including processes and any
/// already-started file writes. Runtime join alone does not prove this.
pub trait TurnToolCleanup: Send + Sync + 'static {
    /// `Ok` confirms actual cleanup and carries any independent diagnostics.
    /// `Err` means cleanup was not confirmed: the Turn remains finalizing and
    /// its resources stay retained. Neither case permits repeating tool work.
    fn finish(&self) -> crate::OwnerFuture<'_, Vec<Fault>>;
}

pub(super) struct ToolCleanupWork {
    handles: Vec<Arc<dyn TurnToolCleanup>>,
    output_leases: Vec<Arc<AtomicBool>>,
    started: bool,
    pub(super) complete: bool,
    pub(super) fault: Option<Fault>,
}
impl ToolCleanupWork {
    pub(super) fn new(handles: Vec<Arc<dyn TurnToolCleanup>>) -> Self {
        Self {
            handles,
            output_leases: Vec::new(),
            started: false,
            complete: false,
            fault: None,
        }
    }
    pub(super) fn pending(&self) -> bool {
        !self.complete && !self.handles.is_empty()
    }
    pub(super) fn retain_output(&mut self, live: Arc<AtomicBool>) -> bool {
        if self.started || self.handles.is_empty() {
            return false;
        }
        self.output_leases.push(live);
        true
    }
}

impl SessionController {
    pub(super) async fn finish_tool_cleanup(&self, turn_id: &TurnId) -> bool {
        let handles = {
            let mut state = self.state.lock().expect("Session mutex poisoned");
            let Some(active) = state.active.as_mut().filter(|a| &a.info.id == turn_id) else {
                return false;
            };
            if active.tool_cleanup.started {
                return active.tool_cleanup.complete;
            }
            debug_assert_eq!(active.info.state, TurnState::Finalizing);
            active.tool_cleanup.started = true;
            active.tool_cleanup.handles.clone()
        };
        let mut issues = Vec::new();
        let mut unconfirmed = None;
        for handle in handles {
            let result = AssertUnwindSafe(async { handle.finish().await })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(Fault::new(
                        FaultKind::TurnInternalUnexpected,
                        "tool cleanup owner panicked before confirming actual completion",
                    ))
                });
            match result {
                Ok(faults) => issues.extend(faults),
                Err(fault) => {
                    unconfirmed.get_or_insert_with(|| fault.clone());
                    issues.push(fault);
                }
            }
        }
        let mut state = self.state.lock().expect("Session mutex poisoned");
        let Some(active) = state.active.as_mut().filter(|a| &a.info.id == turn_id) else {
            return false;
        };
        active.info.cleanup_faults.extend(issues);
        active.tool_cleanup.fault = unconfirmed;
        let complete = active.tool_cleanup.fault.is_none();
        active.tool_cleanup.complete = complete;
        let released = if complete {
            for lease in active.tool_cleanup.output_leases.drain(..) {
                lease.store(false, Ordering::SeqCst);
            }
            std::mem::take(&mut active.tool_cleanup.handles)
        } else {
            Vec::new()
        };
        let diagnostics = if complete {
            self.publish_cleaned_tool_results(&mut state, turn_id)
        } else {
            Vec::new()
        };
        if !complete && state.block_reason.is_none() {
            state.block_reason = state.active.as_ref().unwrap().tool_cleanup.fault.clone();
        }
        self.publish(&mut state);
        drop(state);
        drop(released);
        diagnostic::emit(diagnostics);
        complete
    }
}
