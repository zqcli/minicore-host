//! Explicit, prepared Host directory roots. Authorization stays with Host.
use minicore_host_protocol::{
    DirectoryCursor, DirectoryEntry, DirectoryListRequest, DirectoryListResult, DirectoryRoots,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BinaryHeap,
    fs, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, thiserror::Error)]
pub enum DirectoryError {
    #[error("the directory path or name is invalid")]
    InvalidPath,
    #[error("the directory is outside the configured roots")]
    OutsideRoots,
    #[error("the requested directory does not exist")]
    NotFound,
    #[error("the requested path is not a directory")]
    NotDirectory,
    #[error("an entry with that name already exists")]
    AlreadyExists,
    #[error("the requested page size must be positive")]
    InvalidRange,
    #[error("the continuation does not belong to this directory query")]
    InvalidCursor,
    #[error("the directory path cannot be represented as UTF-8")]
    NonUtf8Path,
    #[error("the directory filesystem operation failed")]
    Io(#[source] io::Error),
}
impl From<io::Error> for DirectoryError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::NotADirectory => Self::NotDirectory,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            _ => Self::Io(error),
        }
    }
}
type Result<T> = std::result::Result<T, DirectoryError>;

/// Canonical roots are fixed during configuration preparation. Neither HOME,
/// current directory nor a later change to a configured symlink adds authority.
#[derive(Debug)]
pub struct WorkspaceDirectories {
    roots: Vec<PathBuf>,
}
impl WorkspaceDirectories {
    pub fn prepare(paths: &[String]) -> Result<Self> {
        let mut roots = Vec::new();
        for path in paths {
            absolute(path)?;
            let root = fs::canonicalize(path)?;
            if !fs::metadata(&root)?.is_dir() {
                return Err(DirectoryError::NotDirectory);
            }
            display(&root)?;
            roots.push(root);
        }
        roots.sort();
        roots.dedup();
        Ok(Self { roots })
    }

    pub fn roots(&self) -> Result<DirectoryRoots> {
        Ok(DirectoryRoots {
            roots: self
                .roots
                .iter()
                .map(|path| {
                    let text = display(path)?;
                    Ok(DirectoryEntry {
                        name: match path.file_name() {
                            Some(name) => name.to_str().ok_or(DirectoryError::NonUtf8Path)?.into(),
                            None => text.clone(),
                        },
                        path: text,
                    })
                })
                .collect::<Result<_>>()?,
        })
    }

    pub fn list(&self, request: DirectoryListRequest) -> Result<DirectoryListResult> {
        if request.limit == 0 {
            return Err(DirectoryError::InvalidRange);
        }
        let path = self.directory(&request.path)?;
        let mut hash = Sha256::new();
        for value in self.roots.iter().chain(std::iter::once(&path)) {
            let text = display(value)?;
            hash.update(text.len().to_le_bytes());
            hash.update(text.as_bytes());
        }
        hash.update([u8::from(request.hidden)]);
        let scope = format!("{:x}", hash.finalize());
        if let Some(cursor) = &request.cursor
            && (cursor.scope != scope || single_component(&cursor.after_name).is_err())
        {
            return Err(DirectoryError::InvalidCursor);
        }
        let mut entries = BinaryHeap::new();
        let mut more = false;
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if !kind.is_dir() && !kind.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_str().ok_or(DirectoryError::NonUtf8Path)?;
            if (!request.hidden && name.starts_with('.'))
                || request
                    .cursor
                    .as_ref()
                    .is_some_and(|cursor| name <= cursor.after_name.as_str())
            {
                continue;
            }
            let target = match self.canonical(&entry.path()) {
                Ok(target) => target,
                Err(DirectoryError::OutsideRoots | DirectoryError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            if fs::metadata(&target)?.is_dir() {
                entries.push((name.to_owned(), display(&target)?));
                if entries.len() > request.limit {
                    entries.pop();
                    more = true;
                }
            }
        }
        let entries: Vec<_> = entries
            .into_sorted_vec()
            .into_iter()
            .map(|(name, path)| DirectoryEntry { name, path })
            .collect();
        let next_cursor = more.then(|| DirectoryCursor {
            scope,
            after_name: entries.last().expect("positive page size").name.clone(),
        });
        Ok(DirectoryListResult {
            path: display(&path)?,
            parent: path
                .parent()
                .filter(|parent| self.contains(parent))
                .map(display)
                .transpose()?,
            entries,
            next_cursor,
        })
    }

    /// Create exactly one child. Existing entries are never treated as success.
    /// The caller must own this blocking operation until it actually finishes.
    pub fn mkdir(&self, parent: &str, name: &str) -> Result<String> {
        valid_name(name)?;
        let parent = self.directory(parent)?;
        let path = parent.join(name);
        let result = display(&path)?;
        fs::create_dir(path)?;
        Ok(result)
    }

    fn contains(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }
    fn canonical(&self, path: &Path) -> Result<PathBuf> {
        let path = fs::canonicalize(path)?;
        if !self.contains(&path) {
            return Err(DirectoryError::OutsideRoots);
        }
        Ok(path)
    }
    fn directory(&self, path: &str) -> Result<PathBuf> {
        absolute(path)?;
        let path = self.canonical(Path::new(path))?;
        if !fs::metadata(&path)?.is_dir() {
            return Err(DirectoryError::NotDirectory);
        }
        Ok(path)
    }
}

fn absolute(path: &str) -> Result<()> {
    if path.contains('\0')
        || !Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|part| part == Component::ParentDir)
    {
        return Err(DirectoryError::InvalidPath);
    }
    Ok(())
}
fn single_component(name: &str) -> Result<()> {
    let mut parts = Path::new(name).components();
    if name.contains('\0')
        || !matches!(parts.next(), Some(Component::Normal(_)))
        || parts.next().is_some()
        || name.contains(std::path::MAIN_SEPARATOR)
    {
        return Err(DirectoryError::InvalidPath);
    }
    Ok(())
}
fn valid_name(name: &str) -> Result<()> {
    single_component(name)?;
    #[cfg(windows)]
    if !ordinary_windows_component(name) {
        return Err(DirectoryError::InvalidPath);
    }
    Ok(())
}
fn display(path: &Path) -> Result<String> {
    let path = path.to_str().ok_or(DirectoryError::NonUtf8Path)?;
    #[cfg(windows)]
    return Ok(strip_verbatim_prefix(path));
    #[cfg(not(windows))]
    Ok(path.to_owned())
}

#[cfg(any(windows, test))]
fn strip_verbatim_prefix(path: &str) -> String {
    let Some(tail) = path.strip_prefix(r"\\?\") else {
        return path.to_owned();
    };
    let safe_components = |value: &str| {
        value
            .trim_end_matches('\\')
            .split('\\')
            .all(ordinary_windows_component)
    };
    if let Some(unc) = tail.strip_prefix(r"UNC\") {
        if unc.trim_end_matches('\\').split('\\').count() >= 2 && safe_components(unc) {
            return format!(r"\\{unc}");
        }
    } else if tail.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && tail.as_bytes().get(1..3) == Some(b":\\")
        && (tail.len() == 3 || safe_components(&tail[3..]))
    {
        return tail.to_owned();
    }
    // Other namespaces or names needing verbatim interpretation must retain it.
    path.to_owned()
}

#[cfg(any(windows, test))]
fn ordinary_windows_component(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_lowercase();
    !name.is_empty()
        && !name.ends_with(['.', ' '])
        && !name.contains(['\\', '/', ':', '<', '>', '"', '|', '?', '*'])
        && !name.chars().any(|ch| ch.is_ascii_control())
        && !matches!(
            stem.as_str(),
            "con" | "nul" | "aux" | "prn" | "conin$" | "conout$"
        )
        && !["com", "lpt"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

#[cfg(test)]
mod tests;
