use super::*;
use crate::SessionContextSnapshot;

impl Host {
    /// Read context usage, compaction and execution facts through the actual
    /// Agent and Session owners. Cold, never-loaded history remains unknown.
    pub async fn session_context(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
    ) -> Result<SessionContextSnapshot, Fault> {
        self.session(caller, target)?.context(caller).await
    }
}
