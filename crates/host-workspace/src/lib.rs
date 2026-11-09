//! Explicit workspace bindings and read-only filesystem/Git observations.
//!
//! Session owners resolve and clone a binding before I/O. This crate does not
//! authorize a caller, choose a Session, change process cwd, or attribute Git
//! changes to tools. See the crate README for the edit-journal handoff.

#![forbid(unsafe_code)]
