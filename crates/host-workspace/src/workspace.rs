use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use minicore_host_protocol::WorkspaceRef;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::{QueryContext, Result, WorkspaceError, context::blocking};

/// One resolved binding. Clone it at the Session/Turn boundary; later config
/// changes do not mutate this root. Binding never changes process-global cwd.
/// Clones share mutation ordering; independently opened bindings do not.
#[derive(Clone, Debug)]
pub struct Workspace {
    pub(crate) root: Arc<PathBuf>,
    mutations: Arc<Mutex<()>>,
}

/// An exclusive mutation operation on one Workspace binding, not an owner-state
/// lock. Keep it with the actual worker from source capture through commit and
/// after capture, including if the caller stops waiting for that worker.
#[derive(Debug)]
#[must_use = "dropping the guard releases the workspace mutation operation"]
pub struct WorkspaceMutation {
    _guard: OwnedMutexGuard<()>,
}

impl Workspace {
    pub async fn bind(selection: Option<&WorkspaceRef>) -> Result<Self> {
        let root = selection.ok_or(WorkspaceError::NotConfigured)?.path.clone();
        Self::open(root).await
    }

    pub async fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(WorkspaceError::InvalidRoot);
        }
        blocking(QueryContext::default(), move |_| {
            let root = fs::canonicalize(root)?;
            if !root.is_dir() {
                return Err(WorkspaceError::NotDirectory);
            }
            path_text(&root)?;
            Ok(Self {
                root: Arc::new(root),
                mutations: Arc::new(Mutex::new(())),
            })
        })
        .await
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Begin a mutation shared by this binding's clones. The caller may cancel
    /// while waiting, but must retain the returned guard until its actual file
    /// operation and observation finish. Do not wait while holding owner state.
    /// This does not coordinate separately opened roots, shell commands, or
    /// external processes; owners that need shared ordering must reuse a binding.
    pub async fn begin_mutation(&self) -> WorkspaceMutation {
        WorkspaceMutation {
            _guard: Arc::clone(&self.mutations).lock_owned().await,
        }
    }

    pub(crate) fn resolve(&self, path: &str, allow_root: bool) -> Result<PathBuf> {
        let relative = normalize(path, allow_root)?;
        let resolved = fs::canonicalize(self.root.join(relative))?;
        if !resolved.starts_with(self.root()) {
            return Err(WorkspaceError::OutsideWorkspace);
        }
        Ok(resolved)
    }

    pub(crate) fn directory(&self, path: &str) -> Result<PathBuf> {
        let resolved = self.resolve(path, true)?;
        if !resolved.is_dir() {
            return Err(WorkspaceError::NotDirectory);
        }
        Ok(resolved)
    }

    pub(crate) fn file(&self, path: &str) -> Result<File> {
        let resolved = self.resolve(path, false)?;
        if !fs::metadata(&resolved)?.is_file() {
            return Err(WorkspaceError::NotFile);
        }
        let file = File::open(resolved)?;
        if !file.metadata()?.is_file() {
            return Err(WorkspaceError::NotFile);
        }
        Ok(file)
    }

    /// Checks all existing parents, including for a deleted or newly added
    /// path. The final symlink is left intact so Git can observe its link text.
    pub(crate) fn entry_path(&self, path: &str) -> Result<PathBuf> {
        let relative = normalize(path, false)?;
        let mut parent = relative.parent();
        while let Some(value) = parent {
            match self.resolve(path_text(value)?, true) {
                Ok(_) => return Ok(self.root.join(relative)),
                Err(WorkspaceError::NotFound) => parent = value.parent(),
                Err(error) => return Err(error),
            }
        }
        Err(WorkspaceError::NotFound)
    }

    pub(crate) fn scope(&self, parts: &[&str]) -> String {
        let mut hash = Sha256::new();
        for part in std::iter::once(self.root.to_str().expect("validated root"))
            .chain(parts.iter().copied())
        {
            hash.update(part.len().to_le_bytes());
            hash.update(part.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
}

pub(crate) fn normalize(path: &str, allow_root: bool) -> Result<PathBuf> {
    if path.contains('\0') {
        return Err(WorkspaceError::InvalidPath);
    }
    let mut result = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(value) => {
                #[cfg(windows)]
                if value
                    .to_str()
                    .is_none_or(|value| value.contains(':') || value.ends_with(['.', ' ']))
                {
                    return Err(WorkspaceError::InvalidPath);
                }
                result.push(value);
            }
            Component::CurDir => (),
            _ => return Err(WorkspaceError::InvalidPath),
        }
    }
    if result.as_os_str().is_empty() && !allow_root {
        return Err(WorkspaceError::InvalidPath);
    }
    Ok(result)
}

pub(crate) fn path_text(path: &Path) -> Result<&str> {
    path.to_str().ok_or(WorkspaceError::NonUtf8Path)
}

pub(crate) fn relative_path_text(path: &Path) -> Result<String> {
    let text = path_text(path)?;
    #[cfg(windows)]
    return Ok(text.replace('\\', "/"));
    #[cfg(not(windows))]
    Ok(text.to_owned())
}

pub(crate) fn read_bytes(mut file: impl Read, context: &QueryContext) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        context.check()?;
        let read = file.read(&mut chunk)?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}
