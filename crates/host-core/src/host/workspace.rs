//! Explicit Session targets and actual committed workspace/edit sources.
use super::*;
use minicore_host_workspace::{
    ChangeRef, ChangeScope, ChangesListRequest, ChangesListResult, DiffRequest, DiffResult,
    FilesRequest, FilesResult, QueryContext, ReadRequest, ReadResult, SearchRequest, SearchResult,
    StatusResult, Workspace, WorkspaceError, diff_edit,
};
use serde::{Deserialize, Serialize};

/// The source Session is fixed before I/O. Workspace/Git queries also identify
/// the committed configuration that selected the root. Retained tool edits do
/// not borrow the current configuration or acquire a fabricated Load/Turn ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceSnapshot<T> {
    #[serde(flatten)]
    pub source: SessionIdentity,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config_revision: Option<ConfigRevision>,
    pub data: T,
}

impl Host {
    fn workspace_source(&self, target: &SessionRef) -> SessionIdentity {
        SessionIdentity {
            host_id: self.id.clone(),
            host_incarnation: self.incarnation.clone(),
            agent_id: target.agent_id.clone(),
            session_id: target.session_id.clone(),
        }
    }

    async fn workspace_binding(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        context: &QueryContext,
    ) -> Result<(Workspace, ConfigRevision), Fault> {
        // Resolve only the requested direct owner. `info` clones the committed
        // configuration under its own lock. No owner/authority lock survives
        // canonicalization or filesystem/Git I/O, and no active Turn is read.
        let info = self.session(caller, target)?.info(caller)?;
        context.check().map_err(workspace_fault)?;
        let workspace = Workspace::bind(info.config.workspace.as_ref())
            .await
            .map_err(workspace_fault)?;
        context.check().map_err(workspace_fault)?;
        Ok((workspace, info.config_revision))
    }

    fn workspace_result<T>(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        config_revision: Option<ConfigRevision>,
        data: T,
    ) -> Result<WorkspaceSnapshot<T>, Fault> {
        self.access.read(caller, || {
            Ok(WorkspaceSnapshot {
                source: self.workspace_source(target),
                config_revision,
                data,
            })
        })
    }

    pub async fn workspace_files(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: FilesRequest,
        context: QueryContext,
    ) -> Result<WorkspaceSnapshot<FilesResult>, Fault> {
        let (workspace, revision) = self.workspace_binding(caller, target, &context).await?;
        let data = workspace
            .files(request, context)
            .await
            .map_err(workspace_fault)?;
        self.workspace_result(caller, target, Some(revision), data)
    }

    pub async fn workspace_read(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: ReadRequest,
        context: QueryContext,
    ) -> Result<WorkspaceSnapshot<ReadResult>, Fault> {
        let (workspace, revision) = self.workspace_binding(caller, target, &context).await?;
        let data = workspace
            .read(request, context)
            .await
            .map_err(workspace_fault)?;
        self.workspace_result(caller, target, Some(revision), data)
    }

    pub async fn workspace_search(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: SearchRequest,
        context: QueryContext,
    ) -> Result<WorkspaceSnapshot<SearchResult>, Fault> {
        let (workspace, revision) = self.workspace_binding(caller, target, &context).await?;
        let data = workspace
            .search(request, context)
            .await
            .map_err(workspace_fault)?;
        self.workspace_result(caller, target, Some(revision), data)
    }

    pub async fn workspace_status(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        context: QueryContext,
    ) -> Result<WorkspaceSnapshot<StatusResult>, Fault> {
        let (workspace, revision) = self.workspace_binding(caller, target, &context).await?;
        let data = workspace.status(context).await.map_err(workspace_fault)?;
        self.workspace_result(caller, target, Some(revision), data)
    }

    pub async fn changes_list(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: ChangesListRequest,
        context: QueryContext,
    ) -> Result<WorkspaceSnapshot<ChangesListResult>, Fault> {
        if request.scope == ChangeScope::Workspace {
            let (workspace, revision) = self.workspace_binding(caller, target, &context).await?;
            let data = workspace
                .changes_list(request, context)
                .await
                .map_err(workspace_fault)?;
            self.workspace_result(caller, target, Some(revision), data)
        } else {
            // The Session owns coverage and retained Turn checks. A restarted
            // persistent Session must not present missing observations as empty.
            let data = self.session_changes(caller, target, request)?;
            context.check().map_err(workspace_fault)?;
            self.workspace_result(caller, target, None, data)
        }
    }

    pub async fn changes_diff(
        &self,
        caller: &CallerContext,
        target: &SessionRef,
        request: DiffRequest,
        context: QueryContext,
    ) -> Result<WorkspaceSnapshot<DiffResult>, Fault> {
        match &request.change_ref {
            ChangeRef::Workspace { .. } => {
                let (workspace, revision) =
                    self.workspace_binding(caller, target, &context).await?;
                let data = workspace
                    .changes_diff(request, context)
                    .await
                    .map_err(workspace_fault)?;
                self.workspace_result(caller, target, Some(revision), data)
            }
            ChangeRef::Tool { .. } => {
                // This is an Arc to the actual original record, not a Session
                // or Runtime lease. Never read a current file to fill a gap.
                let record = self.session_change_record(caller, target, &request.change_ref)?;
                let data = diff_edit(record, request, context)
                    .await
                    .map_err(workspace_fault)?;
                self.workspace_result(caller, target, None, data)
            }
        }
    }
}

fn workspace_fault(error: WorkspaceError) -> Fault {
    use WorkspaceError::*;
    let kind = match &error {
        NotConfigured | InvalidRoot => FaultKind::SessionWorkspaceUnavailable,
        InvalidPath | OutsideWorkspace | NotFile | NotDirectory => {
            FaultKind::SessionWorkspacePathInvalid
        }
        NotFound => FaultKind::SessionWorkspacePathNotFound,
        InvalidRange => FaultKind::SessionWorkspaceRangeInvalid,
        InvalidCursor => FaultKind::SessionWorkspaceCursorInvalid,
        Io(_) | Changed | GitUnavailable(_) | GitFailed { .. } | GitOutput(_) => {
            FaultKind::SessionWorkspaceReadFailed
        }
        Binary | NonUtf8Path | UnsupportedFilter | UnsupportedChange => {
            FaultKind::SessionWorkspaceUnsupported
        }
        NotRepository => FaultKind::SessionWorkspaceNotRepository,
        ChangeNotFound => FaultKind::SessionChangeNotFound,
        SnapshotUnavailable => FaultKind::SessionChangeSourceUnavailable,
        Cancelled => FaultKind::SessionWorkspaceQueryCancelled,
        Deadline => FaultKind::SessionWorkspaceQueryDeadline,
        WorkerFailed => FaultKind::SessionInternalUnexpected,
    };
    // WorkspaceError's display is deliberately sanitized and does not include
    // raw filesystem/Git output, absolute paths, or underlying I/O sources.
    Fault::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_snapshots_and_io_diagnostics_keep_explicit_safe_faults() {
        let missing = workspace_fault(WorkspaceError::SnapshotUnavailable);
        assert_eq!(
            missing.kind(),
            Some(FaultKind::SessionChangeSourceUnavailable)
        );
        assert_eq!(missing.code(), 3810);
        let fault = workspace_fault(WorkspaceError::Io(std::io::Error::other(
            "private absolute path and raw diagnostic",
        )));
        assert_eq!(fault.kind(), Some(FaultKind::SessionWorkspaceReadFailed));
        assert_eq!(fault.message(), "the filesystem operation failed");
        assert!(!serde_json::to_string(&fault).unwrap().contains("private"));
        assert_eq!(
            serde_json::from_str::<Fault>(&serde_json::to_string(&missing).unwrap()).unwrap(),
            missing
        );
    }
}
