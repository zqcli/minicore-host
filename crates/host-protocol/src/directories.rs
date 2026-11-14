//! Host directory selection before an Agent or Session exists.
use crate::OperationId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
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
    pub cursor: Option<DirectoryCursor>,
    /// Caller-requested page size, not an application capacity limit.
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryListResult {
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
