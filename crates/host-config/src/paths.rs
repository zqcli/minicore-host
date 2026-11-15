//! Startup path selection. Explicit configuration and data overrides are independent.
use crate::invalid;
use minicore_host_protocol::Fault;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub data_root: PathBuf,
}

/// Resolve explicit paths against the startup working directory. Omitted paths
/// use the user's ~/.minicore root, independently of the other override.
pub fn resolve_config_paths(
    config_override: Option<&Path>,
    data_override: Option<&Path>,
) -> Result<ConfigPaths, Fault> {
    let defaults = if config_override.is_none() || data_override.is_none() {
        Some(default_root()?)
    } else {
        None
    };
    let config_dir = match config_override {
        Some(path) => absolute(path, "configuration directory")?,
        None => defaults.as_ref().expect("default root").join("config"),
    };
    let data_root = match data_override {
        Some(path) => absolute(path, "data root")?,
        None => defaults.as_ref().expect("default root").join("data"),
    };
    Ok(ConfigPaths {
        config_dir,
        data_root,
    })
}

pub(crate) fn default_root() -> Result<PathBuf, Fault> {
    let home = std::env::home_dir()
        .ok_or_else(|| invalid("cannot determine the user's home directory"))?;
    if !home.is_absolute() {
        return Err(invalid("user home must be an absolute directory"));
    }
    Ok(home.join(".minicore"))
}

fn absolute(path: &Path, label: &str) -> Result<PathBuf, Fault> {
    if path.as_os_str().is_empty() {
        return Err(invalid(format!("{label} must not be empty")));
    }
    std::path::absolute(path).map_err(|error| {
        invalid(format!(
            "cannot resolve {label} {}: {error}",
            path.display()
        ))
    })
}
