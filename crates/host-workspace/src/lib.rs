//! Workspace bindings, mutation coordination, and read-only filesystem/Git observations.
//!
//! Session owners resolve and clone a binding before I/O. This crate does not
//! authorize a caller, choose a Session, change process cwd, or attribute Git
//! changes to tools. See the crate README for the edit-journal handoff.

#![forbid(unsafe_code)]

mod context;
mod error;
mod workspace;

pub use context::QueryContext;
pub use error::{Result, WorkspaceError};
pub use workspace::{Workspace, WorkspaceMutation};

mod files;
pub use files::*;

mod git;
pub use git::{StatusEntry, StatusEntryKind, StatusResult};

mod changes;
pub use changes::*;

mod edits;
pub use edits::*;

mod diff;
pub use diff::*;
