//! Narrow owner read hooks for the future workspace/changes transport adapter.
//! These do not register RPC routes or create an independent edit database.
use super::*;
use minicore_host_workspace::{ChangeRef, ChangesListRequest, ChangesListResult, EditRecord};

impl Host {
    /// List actual retained tool observations in one Session or Turn. Workspace
    /// Git changes are a different source and cannot be attributed by this hook.
    /// Observations survive unload, but not source reclamation or Host restart.
    /// After reopen, Session-wide coverage is unavailable; a retained current-
    /// incarnation Turn may still be queried explicitly.
    pub fn session_changes(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: ChangesListRequest,
    ) -> Result<ChangesListResult, Fault> {
        self.session(caller, target)?.changes_list(caller, request)
    }

    /// Clone one original before/after observation under its owner lock. A
    /// caller may run the existing workspace diff function after this returns.
    /// The returned value holds no Session/Runner or execution capability.
    pub fn session_change_record(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        reference: &ChangeRef,
    ) -> Result<Arc<EditRecord>, Fault> {
        self.session(caller, target)?
            .change_record(caller, reference)
    }
}
