use minicore_host_config::resolve_config_paths;
use std::{ffi::OsString, path::PathBuf};

pub const HELP: &str = "MiniCore Host
Usage: minicore-host [--config-dir PATH] [--data-root PATH] [--stdio]
                     [--tls-cert PATH --tls-key PATH]
       minicore-host --help

--config-dir PATH JSON configuration directory (default: executable directory/config).
--data-root PATH  Exclusive persistent Host data directory (default: config-dir/data).
--stdio           Serve trusted newline-delimited JSON-RPC on stdin/stdout.
--tls-cert PATH   PEM certificate chain for the configured network.listen address.
--tls-key PATH    PEM private key for that certificate (required with --tls-cert).

At least --stdio or a configured network listener is required. WSS requires
network.access_key_env and both TLS files. Credential values are read only from
the configured environment sources. TLS paths are process inputs, not Host config.
host.json is required. agents/*.json and roles/*.json declare Agents and Roles.
Startup and host.reload read this directory; existing identities and history remain.

SIGINT/SIGTERM request graceful shutdown, without a drain deadline or automatic
force escalation. Explicit host.shutdown RPC may choose force. Stdio EOF closes
only that connection; accepted work and the Host continue until controlled shutdown.
All help and diagnostics go to stderr or the configured log file, never stdout.";

pub struct Options {
    pub data_root: PathBuf,
    pub config_dir: PathBuf,
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
                Some("--config-dir") => set_path(&mut config, &mut args)?,
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
        let paths = resolve_config_paths(config.as_deref(), root.as_deref())
            .map_err(|_| "cannot resolve configuration and data directories")?;
        Ok(Some(Self {
            data_root: paths.data_root,
            config_dir: paths.config_dir,
            stdio,
            tls,
        }))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Options>, &'static str> {
        Options::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn paths_use_shared_resolution_and_data_override() {
        let options = parse(&["--config-dir", "settings", "--stdio"])
            .unwrap()
            .unwrap();
        let expected = std::path::absolute("settings").unwrap();
        assert_eq!(options.config_dir, expected);
        assert_eq!(options.data_root, expected.join("data"));
        let options = parse(&["--config-dir", "settings", "--data-root", "state"])
            .unwrap()
            .unwrap();
        assert_eq!(options.data_root, std::path::absolute("state").unwrap());
        let options = parse(&[]).unwrap().unwrap();
        assert_eq!(
            options.config_dir,
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("config")
        );
    }

    #[test]
    fn old_and_ambiguous_inputs_are_rejected() {
        for args in [
            vec!["--config", "host.json"],
            vec!["--config-dir"],
            vec!["--config-dir", "a", "--config-dir", "b"],
            vec!["--tls-cert", "cert.pem"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
