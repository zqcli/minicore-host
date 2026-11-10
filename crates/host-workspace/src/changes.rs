use std::fs;

use minicore_host_protocol::{SessionId, ToolCallRef, TurnId};
use serde::{Deserialize, Serialize};

use crate::{
    QueryContext, Result, StatusEntry, StatusEntryKind, Workspace, WorkspaceError,
    context::blocking,
    git::Repository,
    workspace::{normalize, path_text, read_bytes},
};

/// Actual captured bytes. Unknown is never treated as an empty file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "bytes", rename_all = "snake_case")]
pub enum FileSnapshot {
    Missing,
    Content(Vec<u8>),
    Unknown,
}
impl FileSnapshot {
    pub(crate) fn bytes(&self) -> Result<&[u8]> {
        match self {
            Self::Missing => Ok(&[]),
            Self::Content(bytes) => Ok(bytes),
            Self::Unknown => Err(WorkspaceError::SnapshotUnavailable),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Conflict,
    Unchanged,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOrigin {
    Tool,
    WorkspaceUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeRef {
    Workspace {
        /// The original canonical-root scope, not authorization or a Session ID.
        workspace_scope: String,
        head_oid: Option<String>,
        entry: StatusEntry,
    },
    Tool {
        session_id: SessionId,
        index: usize,
        tool_call: ToolCallRef,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditOutcome {
    Applied,
    NotApplied,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub change_ref: ChangeRef,
    pub path: String,
    pub kind: ChangeKind,
    pub origin: ChangeOrigin,
    pub details_available: bool,
    /// Tool write outcome, not a Git staging/commit status.
    pub edit_outcome: Option<EditOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeScope {
    Workspace,
    Session,
    Turn { turn_id: TurnId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangesCursor {
    Workspace {
        scope: String,
        after_entry: StatusEntry,
    },
    Tool {
        session_id: SessionId,
        turn_id: Option<TurnId>,
        after_index: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangesListRequest {
    pub scope: ChangeScope,
    pub cursor: Option<ChangesCursor>,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangesListResult {
    pub records: Vec<ChangeRecord>,
    pub next_cursor: Option<ChangesCursor>,
}

impl Workspace {
    /// Current Git changes only. Session/Turn tool attribution requires the
    /// separate Session-owned edit journal, not a heuristic over Git status.
    pub async fn changes_list(
        &self,
        request: ChangesListRequest,
        context: QueryContext,
    ) -> Result<ChangesListResult> {
        if request.limit == 0 || request.scope != ChangeScope::Workspace {
            return Err(WorkspaceError::InvalidRange);
        }
        let scope = self.scope(&["changes"]);
        let after = match &request.cursor {
            None => None,
            Some(ChangesCursor::Workspace {
                scope: cursor_scope,
                after_entry,
            }) if *cursor_scope == scope => {
                normalize(&after_entry.path, false)?;
                Some(after_entry)
            }
            _ => return Err(WorkspaceError::InvalidCursor),
        };
        let status = self.status(context).await?;
        if !status.repo_available {
            return Err(WorkspaceError::NotRepository);
        }
        let mut entries = status
            .entries
            .into_iter()
            .filter(|entry| after.is_none_or(|after| entry > after));
        let records: Vec<_> = entries
            .by_ref()
            .take(request.limit)
            .map(|entry| ChangeRecord {
                path: entry.path.clone(),
                kind: git_kind(&entry),
                origin: ChangeOrigin::WorkspaceUnknown,
                details_available: entry.kind != StatusEntryKind::Unmerged && !entry.submodule,
                edit_outcome: None,
                change_ref: ChangeRef::Workspace {
                    workspace_scope: scope.clone(),
                    head_oid: status.head_oid.clone(),
                    entry,
                },
            })
            .collect();
        let next_cursor = entries.next().map(|_| ChangesCursor::Workspace {
            scope,
            after_entry: match &records.last().expect("positive limit").change_ref {
                ChangeRef::Workspace { entry, .. } => entry.clone(),
                _ => unreachable!(),
            },
        });
        Ok(ChangesListResult {
            records,
            next_cursor,
        })
    }

    /// Captures an actual regular file for the tool owner, before or after its
    /// own write. This neither performs a write nor inserts a journal record.
    pub async fn capture(&self, path: String, context: QueryContext) -> Result<FileSnapshot> {
        let workspace = self.clone();
        blocking(context, move |context| {
            let entry = workspace.entry_path(&path)?;
            match fs::symlink_metadata(entry) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(FileSnapshot::Missing)
                }
                Err(error) => Err(error.into()),
                Ok(_) => Ok(FileSnapshot::Content(read_bytes(
                    workspace.file(&path)?,
                    context,
                )?)),
            }
        })
        .await
    }

    pub(crate) async fn worktree_bytes(
        &self,
        path: String,
        context: QueryContext,
    ) -> Result<FileSnapshot> {
        let workspace = self.clone();
        blocking(context, move |context| {
            let path = workspace.entry_path(&path)?;
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(FileSnapshot::Missing);
                }
                Err(error) => return Err(error.into()),
            };
            if metadata.is_symlink() {
                let target = fs::read_link(path)?;
                return Ok(FileSnapshot::Content(
                    path_text(&target)?.as_bytes().to_vec(),
                ));
            }
            if !metadata.is_file() {
                return Err(WorkspaceError::UnsupportedChange);
            }
            Ok(FileSnapshot::Content(read_bytes(
                fs::File::open(path)?,
                context,
            )?))
        })
        .await
    }
}

pub(crate) fn git_kind(entry: &StatusEntry) -> ChangeKind {
    match entry.kind {
        StatusEntryKind::Untracked => ChangeKind::Added,
        StatusEntryKind::Renamed => ChangeKind::Renamed,
        StatusEntryKind::Unmerged => ChangeKind::Conflict,
        StatusEntryKind::Ordinary => {
            if entry.index_status.as_deref() == Some("D")
                || entry.worktree_status.as_deref() == Some("D")
            {
                ChangeKind::Deleted
            } else if entry.index_status.as_deref() == Some("A") {
                ChangeKind::Added
            } else {
                ChangeKind::Modified
            }
        }
    }
}

pub(crate) async fn repository(
    workspace: &Workspace,
    context: &QueryContext,
) -> Result<Repository> {
    Repository::discover(workspace, context)
        .await?
        .ok_or(WorkspaceError::NotRepository)
}
