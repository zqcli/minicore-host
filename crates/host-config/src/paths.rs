//! Startup path selection. No legacy environment or home-directory fallback.
use crate::invalid;
use minicore_host_protocol::Fault;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub data_root: PathBuf,
}

/// Resolve explicit paths against the startup working directory. The default
/// configuration is next to the executable; data defaults to its `data` child.
/// This does not require either directory to exist and does not create files.
pub fn resolve_config_paths(
    config_override: Option<&Path>,
    data_override: Option<&Path>,
) -> Result<ConfigPaths, Fault> {
    let config_dir = match config_override {
        Some(path) => absolute(path, "configuration directory")?,
        None => {
            let executable = std::env::current_exe()
                .map_err(|error| invalid(format!("cannot locate executable: {error}")))?;
            executable
                .parent()
                .ok_or_else(|| invalid("executable has no parent directory"))?
                .join("config")
        }
    };
    let data_root = match data_override {
        Some(path) => absolute(path, "data root")?,
        None => config_dir.join("data"),
    };
    Ok(ConfigPaths {
        config_dir,
        data_root,
    })
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
