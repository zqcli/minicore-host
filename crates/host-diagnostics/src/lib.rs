//! Process-owned, structured MiniCore diagnostics.
//!
//! The application installs one subscriber, retains its [`Guard`], and shuts it
//! down **after** business cleanup. Hosts only clone [`Health`], never the guard.
//! Diagnostics do not acknowledge business persistence or change a business
//! Fault. See the crate README for the intentionally small output contract.
//!
//! ```
//! use minicore_host_diagnostics::{Context, EventData, EventSpec, Scope};
//! use tracing::Level;
//!
//! const STARTED: EventSpec = EventSpec {
//!     level: Level::INFO,
//!     scope: Scope::Host,
//!     component: "host.owner",
//!     event: "host.started",
//!     message: "Host started",
//!     attributes: &["mode"],
//! };
//! // The owner supplies its real identity; no IDs are generated for logging.
//! let context = Context::default();
//! let work = context.instrument(async {
//!     STARTED.emit(|| EventData::default().attribute("mode", "memory"));
//! });
//! # drop(work);
//! ```

#![forbid(unsafe_code)]

mod backend;
mod record;
mod subscriber;

pub use backend::{Guard, Health, LoggingState, LoggingStatus, ShutdownOutcome};
pub use record::{Context, EventData, EventSpec, LogScalar, LogValue, Scope};

use std::{
    fmt,
    fs::{File, OpenOptions},
    io,
    path::PathBuf,
};
use tracing::{Dispatch, Level};
use tracing_subscriber::prelude::*;

/// Application-selected destination. Stdout is deliberately not an option.
#[derive(Debug)]
pub enum Output {
    Stderr,
    /// Append to exactly this path. The parent directory must already exist.
    /// The writer exclusively locks the actual file until its work has ended.
    File(PathBuf),
}

/// Startup settings, mapped by the application from its existing configuration.
/// This is not a new serializable configuration or environment-filter platform.
#[derive(Debug)]
pub struct Options {
    pub service: &'static str,
    pub service_version: Option<&'static str>,
    pub level: Level,
    pub output: Output,
}

impl Options {
    /// INFO-level JSONL to stderr. Environment variables do not override this.
    pub const fn new(service: &'static str) -> Self {
        Self {
            service,
            service_version: None,
            level: Level::INFO,
            output: Output::Stderr,
        }
    }
}

/// Startup errors are explicit and contain no path, credentials, or raw error
/// object. There is no alternate output or directory fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitError {
    EmptyService,
    OpenOutput(io::ErrorKind),
    LockOutput(io::ErrorKind),
    StartWorker(io::ErrorKind),
    SubscriberAlreadyInstalled,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyService => f.write_str("diagnostic service name is empty"),
            Self::OpenOutput(kind) => write!(f, "cannot open diagnostic output ({kind:?})"),
            Self::LockOutput(kind) => {
                write!(f, "cannot exclusively lock diagnostic output ({kind:?})")
            }
            Self::StartWorker(kind) => write!(f, "cannot start diagnostic worker ({kind:?})"),
            Self::SubscriberAlreadyInstalled => {
                f.write_str("a process tracing subscriber is already installed")
            }
        }
    }
}
impl std::error::Error for InitError {}

/// Prepare a subscriber without changing process-global tracing state.
///
/// Embedding applications can use the returned dispatch with their own tracing
/// lifecycle. Retain the guard until application exit, after all Hosts finish.
/// This function does not create directories, rotate files, or sync business
/// storage. Discarding the dispatch does not replace the explicit guard shutdown.
pub fn prepare(options: Options) -> Result<(Dispatch, Guard), InitError> {
    if options.service.trim().is_empty() {
        return Err(InitError::EmptyService);
    }
    match &options.output {
        Output::Stderr => prepare_writer(options, io::stderr(), |_| Ok(())),
        Output::File(path) => {
            let writer = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| InitError::OpenOutput(error.kind()))?;
            prepare_file(options, writer)
        }
    }
}

/// Install the process subscriber exactly once, from the application entrypoint.
/// An existing subscriber is an explicit error, never silently ignored.
pub fn install(options: Options) -> Result<Guard, InitError> {
    let (dispatch, guard) = prepare(options)?;
    tracing::dispatcher::set_global_default(dispatch)
        .map_err(|_| InitError::SubscriberAlreadyInstalled)?;
    Ok(guard)
}

fn prepare_file(options: Options, writer: File) -> Result<(Dispatch, Guard), InitError> {
    writer.try_lock().map_err(|error| {
        InitError::LockOutput(match error {
            std::fs::TryLockError::WouldBlock => io::ErrorKind::WouldBlock,
            std::fs::TryLockError::Error(error) => error.kind(),
        })
    })?;
    prepare_writer(options, writer, File::unlock)
}

fn prepare_writer<W: io::Write + Send + 'static>(
    options: Options,
    writer: W,
    release: fn(&W) -> io::Result<()>,
) -> Result<(Dispatch, Guard), InitError> {
    let (sender, guard) =
        backend::start(writer, release).map_err(|error| InitError::StartWorker(error.kind()))?;
    let layer = subscriber::DiagnosticsLayer::new(options, sender);
    Ok((
        Dispatch::new(tracing_subscriber::registry().with(layer)),
        guard,
    ))
}
