use minicore_host_core::HostConfig;
use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "MiniCore Host
Usage: minicore-host --data-root PATH [--config PATH] [--stdio]
                     [--tls-cert PATH --tls-key PATH]
       minicore-host --help

--data-root PATH  Exclusive persistent Host data directory (required).
--config PATH     Initial HostConfig JSON for a new root. Saved config wins on reopen.
--stdio           Serve trusted newline-delimited JSON-RPC on stdin/stdout.
--tls-cert PATH   PEM certificate chain for the configured network.listen address.
--tls-key PATH    PEM private key for that certificate (required with --tls-cert).

At least --stdio or a configured network listener is required. WSS requires
network.access_key_env and both TLS files. Credential values are read only from
the configured environment sources. TLS paths are process inputs, not Host config.

SIGINT/SIGTERM request graceful shutdown, without a drain deadline or automatic
force escalation. Explicit host.shutdown RPC may choose force. Stdio EOF closes
only that connection; accepted work and the Host continue until controlled shutdown.
All help and diagnostics go to stderr or the configured log file, never stdout.";

pub struct Options {
    pub data_root: PathBuf,
    pub config: Option<PathBuf>,
    pub stdio: bool,
    pub tls: Option<(PathBuf, PathBuf)>,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<Self>, &'static str> {
        let mut args = args.into_iter().peekable();
        if args
            .peek()
            .is_some_and(|arg| arg == "--help" || arg == "-h")
        {
            args.next();
            if args.next().is_some() {
                return Err("--help must be used alone");
            }
            return Ok(None);
        }
        let mut root = None;
        let mut config = None;
        let mut stdio = false;
        let mut cert = None;
        let mut key = None;
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--data-root") => set_path(&mut root, &mut args)?,
                Some("--config") => set_path(&mut config, &mut args)?,
                Some("--tls-cert") => set_path(&mut cert, &mut args)?,
                Some("--tls-key") => set_path(&mut key, &mut args)?,
                Some("--stdio") if !stdio => stdio = true,
                _ => return Err("unknown or repeated option; use --help"),
            }
        }
        let tls = match (cert, key) {
            (Some(cert), Some(key)) => Some((cert, key)),
            (None, None) => None,
            _ => return Err("--tls-cert and --tls-key must be supplied together"),
        };
        Ok(Some(Self {
            data_root: root.ok_or("--data-root is required; use --help")?,
            config,
            stdio,
            tls,
        }))
    }

    pub fn initial_config(&self) -> Result<HostConfig, &'static str> {
        match &self.config {
            None => Ok(HostConfig::default()),
            Some(path) => {
                let bytes = std::fs::read(path).map_err(|_| "cannot read initial Host config")?;
                serde_json::from_slice(&bytes).map_err(|_| "invalid initial HostConfig JSON")
            }
        }
    }
}
fn set_path(
    target: &mut Option<PathBuf>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<(), &'static str> {
    if target.is_some() {
        return Err("repeated path option; use --help");
    }
    let value = args.next().ok_or("path option requires a value")?;
    if value.is_empty() || value.to_str().is_some_and(|value| value.starts_with("--")) {
        return Err("path option requires a nonempty path value");
    }
    *target = Some(value.into());
    Ok(())
}
