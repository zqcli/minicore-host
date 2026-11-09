use minicore_host_core::{LogLevel, LogOutput, LogSettings};
use minicore_host_diagnostics::{Context, EventData, EventSpec, Guard, Options, Output, Scope};
use minicore_host_protocol::Fault;
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tracing::Level;

static INSTALLED: AtomicBool = AtomicBool::new(false);

pub struct Diagnostics {
    guard: Guard,
}
impl Diagnostics {
    pub fn install(data_root: &Path, settings: LogSettings) -> Result<Self, &'static str> {
        validate_output(data_root, &settings.output)?;
        let output = match settings.output {
            LogOutput::Stderr => Output::Stderr,
            LogOutput::File { path } => Output::File(path.into()),
        };
        let options = Options {
            service: "minicore-host",
            service_version: Some(env!("CARGO_PKG_VERSION")),
            level: match settings.level {
                LogLevel::Error => Level::ERROR,
                LogLevel::Warn => Level::WARN,
                LogLevel::Info => Level::INFO,
                LogLevel::Debug => Level::DEBUG,
                LogLevel::Trace => Level::TRACE,
            },
            output,
        };
        let guard = minicore_host_diagnostics::install(options)
            .map_err(|_| "cannot initialize configured diagnostics")?;
        INSTALLED.store(true, Ordering::Relaxed);
        Ok(Self { guard })
    }
    pub fn finish(self) {
        let Self { guard } = self;
        // This bounds diagnostics only, never business draining or persistence.
        let _ = guard.shutdown(Duration::from_secs(1));
    }
}

fn validate_output(data_root: &Path, output: &LogOutput) -> Result<(), &'static str> {
    let LogOutput::File { path } = output else {
        return Ok(());
    };
    let root = std::fs::canonicalize(data_root).map_err(|_| "cannot resolve Host data root")?;
    let path = Path::new(path);
    let resolved = match std::fs::symlink_metadata(path) {
        Ok(_) => std::fs::canonicalize(path).map_err(|_| "cannot resolve diagnostic output")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or("cannot resolve diagnostic output parent")?;
            let name = path
                .file_name()
                .ok_or("cannot resolve diagnostic output file")?;
            std::fs::canonicalize(parent)
                .map_err(|_| "cannot resolve diagnostic output parent")?
                .join(name)
        }
        Err(_) => return Err("cannot inspect diagnostic output"),
    };
    // HostStore owns host.json and the agents subtree. A separate logs
    // directory remains valid; diagnostics must never append to those records.
    if resolved == root.join("host.json") || resolved.starts_with(root.join("agents")) {
        return Err("diagnostic output conflicts with Host business storage");
    }
    Ok(())
}

const fn event(
    level: Level,
    event: &'static str,
    message: &'static str,
    attributes: &'static [&'static str],
) -> EventSpec {
    EventSpec {
        level,
        scope: Scope::Host,
        component: "host.process",
        event,
        message,
        attributes,
    }
}
pub const STARTED: EventSpec = event(
    Level::INFO,
    "host.started",
    "Host process started",
    &["stdio", "wss", "listen_port"],
);
pub const STATE: EventSpec = event(
    Level::INFO,
    "host.shutdown_state",
    "Host shutdown state changed",
    &["lifecycle", "strategy"],
);
pub const SHUTDOWN_REQUESTED: EventSpec = event(
    Level::INFO,
    "host.shutdown_requested",
    "Process requested graceful Host shutdown",
    &["reason"],
);
pub const STOPPED: EventSpec = event(
    Level::INFO,
    "host.stopped",
    "Host execution, saves and cleanup stopped",
    &[],
);
pub const DISCONNECTED: EventSpec = event(
    Level::INFO,
    "transport.stdio_disconnected",
    "Stdio connection ended; Host ownership continues",
    &[],
);
pub const TRANSPORT_FAILED: EventSpec = event(
    Level::ERROR,
    "transport.failed",
    "Process transport failed",
    &["transport"],
);
pub const SHUTDOWN_BLOCKED: EventSpec = event(
    Level::ERROR,
    "host.shutdown_blocked",
    "Host shutdown remains blocked by owner fault",
    &[],
);
pub const STARTUP_FAILED: EventSpec = event(
    Level::ERROR,
    "host.startup_failed",
    "Host process startup failed",
    &[],
);
const PANICKED: EventSpec = event(
    Level::ERROR,
    "process.panicked",
    "Process task panicked; controlled completion is not established",
    &[],
);

pub fn fault(spec: EventSpec, context: Context, fault: &Fault) {
    spec.emit(|| EventData::default().context(context).fault(fault));
}
pub fn panic() {
    if INSTALLED.load(Ordering::Relaxed) {
        PANICKED.emit(EventData::default);
    } else {
        eprintln!("minicore-host: internal process failure");
    }
}
