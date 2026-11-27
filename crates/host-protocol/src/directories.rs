//! Host directory selection and opt-in file references for an editor.
use crate::{OperationId, SessionRef};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_file: bool,
    pub name: String,
    /// Canonical absolute path in the Host platform's syntax.
    pub path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryRoots {
    pub roots: Vec<DirectoryEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryCursor {
    pub scope: String,
    pub after_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryListRequest {
    pub path: String,
    #[serde(default)]
    pub hidden: bool,
    /// Opt in to regular files as well as directories, or resolve a file path.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_files: bool,
    /// With include_files, a caller may also browse its selected Session workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_session: Option<SessionRef>,
    pub cursor: Option<DirectoryCursor>,
    /// Caller-requested page size, not an application capacity limit.
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryListResult {
    /// A direct file resolution has no entries, parent or continuation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_file: bool,
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<DirectoryEntry>,
    pub next_cursor: Option<DirectoryCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryMkdirRequest {
    pub operation_id: OperationId,
    pub parent: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryMkdirResult {
    Creating {},
    Created { path: String },
    Failed { error: crate::Fault },
}

crate::control_receipt!(
    DirectoryMkdirReceipt,
    DirectoryMkdirKind,
    WorkspaceMkdir,
    "workspace.mkdir",
    DirectoryMkdirResult
);
