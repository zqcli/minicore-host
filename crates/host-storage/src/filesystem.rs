use crate::{StorageError, error::io_error};
use metadata::MetadataGates;
use rustix::fs::{self, AtFlags, Mode, OFlags};
use std::{
    fs::{DirBuilder, File},
    io::{Read, Write},
    os::unix::{
        ffi::OsStringExt,
        fs::{DirBuilderExt, MetadataExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct Directory {
    pub file: Arc<File>,
    pub path: PathBuf,
    parent: Option<Arc<Directory>>,
    metadata: Arc<MetadataGates>,
}

impl Directory {
    pub fn root(path: &Path) -> Result<Self, StorageError> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|e| io_error("create data root", path, e))?;
        let canonical =
            std::fs::canonicalize(path).map_err(|e| io_error("resolve data root", path, e))?;
        let file = File::from(
            fs::open(
                &canonical,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| io_error("open data root", &canonical, e))?,
        );
        check_directory(&file)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StorageError::RootInUse),
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(io_error("lock data root", &canonical, e));
            }
        }
        #[cfg(test)]
        crate::lease_tests::after_lock(&file);
        // Existence is not a durability proof, including after a previous failed
        // acquisition. Confirm every ancestor entry needed to locate this root.
        for parent in canonical.ancestors().skip(1) {
            let parent_file = File::open(parent)
                .map_err(|e| io_error("open root ancestor for sync", parent, e))?;
            sync_file(&parent_file, parent, "sync data root ancestor")?;
        }
        sync_file(&file, &canonical, "sync data root")?;
        Ok(Self {
            file: Arc::new(file),
            path: canonical,
            parent: None,
            metadata: Arc::default(),
        })
    }

    pub fn child(&self, name: &str, create: bool) -> Result<Self, StorageError> {
        component(name)?;
        let path = self.path.join(name);
        if create {
            match fs::mkdirat(&*self.file, name, Mode::from_bits_truncate(0o700)) {
                Ok(()) => {}
                Err(rustix::io::Errno::EXIST) => {}
                Err(e) => return Err(io_error("create child directory", &path, e)),
            }
            self.sync()?;
        }
        let file = File::from(
            fs::openat(
                &*self.file,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| path_error("open child directory", &path, e))?,
        );
        check_directory(&file)?;
        Ok(Self {
            file: Arc::new(file),
            path,
            parent: Some(Arc::new(self.clone())),
            metadata: self.metadata.clone(),
        })
    }

    pub fn contains(&self, name: &str) -> Result<bool, StorageError> {
        component(name)?;
        match fs::statat(&*self.file, name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => Ok(true),
            Err(rustix::io::Errno::NOENT) => Ok(false),
            Err(e) => Err(io_error("inspect storage entry", self.path.join(name), e)),
        }
    }

    pub fn open_file(
        &self,
        name: &str,
        write: bool,
        create_new: bool,
    ) -> Result<File, StorageError> {
        component(name)?;
        let mut flags = OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        flags |= if write { OFlags::RDWR } else { OFlags::RDONLY };
        if create_new {
            flags |= OFlags::CREATE | OFlags::EXCL;
        }
        let file = File::from(
            fs::openat(&*self.file, name, flags, Mode::from_bits_truncate(0o600))
                .map_err(|e| path_error("open storage file", &self.path.join(name), e))?,
        );
        check_file(&file)?;
        Ok(file)
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        let mut file = self.open_file(name, false, false)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| io_error("read storage file", self.path.join(name), e))?;
        Ok(bytes)
    }

    /// Capture one metadata file, excluding only this instance's atomic
    /// replacement of that exact path. Release before decoding or following any
    /// parent/default reference; no two metadata gates are held together.
    pub fn read_metadata(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        component(name)?;
        let gate = self.metadata.get(self.path.join(name))?;
        let _snapshot = gate.read()?;
        self.verify_attached()?;
        let mut file = self.open_file(name, false, false)?;
        #[cfg(test)]
        run_hook(&METADATA_READ_HOOK);
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| io_error("read object snapshot", self.path.join(name), e))?;
        // Internal publishers cannot invalidate this snapshot. Any unlink,
        // extra hardlink, unsafe permission or detached path here is external
        // or an uncoordinated deletion, not a reason to accept nlink == 0.
        check_file(&file)?;
        self.verify_file_identity(name, &file)?;
        self.verify_attached()?;
        Ok(bytes)
    }

    pub fn entries(&self) -> Result<Vec<String>, StorageError> {
        let reader = fs::Dir::read_from(&*self.file)
            .map_err(|e| io_error("read storage directory", &self.path, e))?;
        let mut entries = Vec::new();
        for entry in reader {
            let entry =
                entry.map_err(|e| io_error("read storage directory entry", &self.path, e))?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = std::ffi::OsString::from_vec(bytes.to_vec())
                .into_string()
                .map_err(|_| StorageError::UnsafePath {
                    reason: "non-UTF-8 storage entry",
                })?;
            component(&name)?;
            entries.push(name);
        }
        entries.sort();
        Ok(entries)
    }

    pub fn sync(&self) -> Result<(), StorageError> {
        sync_file(&self.file, &self.path, "sync storage directory")?;
        if let Some(parent) = &self.parent {
            parent.sync()?;
        }
        Ok(())
    }

    pub fn atomic_write(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        component(name)?;
        self.verify_attached()?;
        let nonce =
            minicore_host_protocol::OperationId::new().map_err(|_| StorageError::UnsafePath {
                reason: "secure temporary filename generation failed",
            })?;
        let temporary = format!(".{name}.{nonce}.tmp");
        let mut file = self.open_file(&temporary, true, true)?;
        let result = (|| {
            file.write_all(bytes).map_err(|e| {
                io_error("write object temporary file", self.path.join(&temporary), e)
            })?;
            file.sync_all().map_err(|e| {
                io_error("sync object temporary file", self.path.join(&temporary), e)
            })?;
            #[cfg(test)]
            run_hook(&METADATA_PUBLISH_HOOK);
            let gate = self.metadata.get(self.path.join(name))?;
            let _publication = gate.write()?;
            self.verify_attached()?;
            if self.contains(name)? {
                self.open_file(name, false, false)?;
            }
            fs::renameat(&*self.file, &temporary, &*self.file, name)
                .map_err(|e| io_error("publish object file", self.path.join(name), e))?;
            self.sync()?;
            self.verify_attached()?;
            self.verify_file_identity(name, &file)?;
            if file
                .metadata()
                .map_err(|e| io_error("inspect published object", self.path.join(name), e))?
                .len()
                != bytes.len() as u64
            {
                return Err(StorageError::UncertainWrite);
            }
            Ok(())
        })();
        // Only our private unpublished temporary is eligible for cleanup. Never
        // remove, truncate, replace, or repair an existing failed object file.
        if result.is_err() {
            let _ = fs::unlinkat(&*self.file, &temporary, AtFlags::empty());
        }
        result
    }

    pub fn remove_file(&self, name: &str) -> Result<(), StorageError> {
        self.open_file(name, false, false)?;
        fs::unlinkat(&*self.file, name, AtFlags::empty())
            .map_err(|e| io_error("delete owned file", self.path.join(name), e))?;
        self.sync()
    }

    pub fn remove_empty_child(&self, name: &str) -> Result<(), StorageError> {
        self.child(name, false)?;
        fs::unlinkat(&*self.file, name, AtFlags::REMOVEDIR).map_err(|e| {
            if e == rustix::io::Errno::NOTEMPTY {
                StorageError::DirectoryNotEmpty
            } else {
                io_error("delete empty owned directory", self.path.join(name), e)
            }
        })?;
        self.sync()
    }

    pub fn verify_attached(&self) -> Result<(), StorageError> {
        if let Some(parent) = &self.parent {
            parent.verify_attached()?;
            let name =
                self.path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or(StorageError::UnsafePath {
                        reason: "invalid stored directory name",
                    })?;
            self.verify_directory_identity(&parent.child(name, false)?)
        } else {
            let current = File::from(
                fs::open(
                    &self.path,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| path_error("verify data root identity", &self.path, e))?,
            );
            check_directory(&current)?;
            let a = self
                .file
                .metadata()
                .map_err(|e| io_error("inspect original root", &self.path, e))?;
            let b = current
                .metadata()
                .map_err(|e| io_error("inspect current root", &self.path, e))?;
            if a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(StorageError::UncertainWrite);
            }
            Ok(())
        }
    }

    pub fn verify_directory_identity(&self, other: &Self) -> Result<(), StorageError> {
        let a = self
            .file
            .metadata()
            .map_err(|e| io_error("inspect original directory", &self.path, e))?;
        let b = other
            .file
            .metadata()
            .map_err(|e| io_error("inspect current directory", &other.path, e))?;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return Err(StorageError::UncertainWrite);
        }
        Ok(())
    }

    pub fn verify_file_identity(&self, name: &str, file: &File) -> Result<(), StorageError> {
        let current = self.open_file(name, false, false)?;
        let a = file
            .metadata()
            .map_err(|e| io_error("inspect open history", self.path.join(name), e))?;
        let b = current
            .metadata()
            .map_err(|e| io_error("inspect history identity", self.path.join(name), e))?;
        if a.dev() != b.dev() || a.ino() != b.ino() {
            return Err(StorageError::UncertainWrite);
        }
        Ok(())
    }
}

fn component(name: &str) -> Result<(), StorageError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Err(StorageError::UnsafePath {
            reason: "invalid storage path component",
        })
    } else {
        Ok(())
    }
}
fn check_directory(file: &File) -> Result<(), StorageError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error("inspect directory", PathBuf::new(), e))?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err(StorageError::UnsafePath {
            reason: "directory is not owned by this user or is writable by other users",
        });
    }
    Ok(())
}
fn check_file(file: &File) -> Result<(), StorageError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error("inspect file", PathBuf::new(), e))?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
    {
        return Err(StorageError::UnsafePath {
            reason: "file is not a private owned regular file with one link",
        });
    }
    Ok(())
}
fn path_error(operation: &'static str, path: &Path, error: rustix::io::Errno) -> StorageError {
    if matches!(error, rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) {
        StorageError::UnsafePath {
            reason: "symlink or non-directory in storage path",
        }
    } else {
        io_error(operation, path, error)
    }
}

/// Explicit synchronization also reused for existing initial files after an
/// earlier failed creation. Test hooks are thread-local and never in production.
pub(crate) fn sync_file(
    file: &File,
    path: &Path,
    operation: &'static str,
) -> Result<(), StorageError> {
    #[cfg(test)]
    {
        SYNC_TRACE.with(|trace| trace.borrow_mut().push(path.to_owned()));
        let fail = SYNC_FAILURE.with(|failure| {
            if failure.borrow().as_deref() == Some(path) {
                failure.borrow_mut().take();
                true
            } else {
                false
            }
        });
        if fail {
            return Err(io_error(
                operation,
                path,
                std::io::Error::from_raw_os_error(5),
            ));
        }
    }
    file.sync_all().map_err(|e| io_error(operation, path, e))
}
#[cfg(test)]
thread_local! {
    static SYNC_FAILURE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static SYNC_TRACE: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    static METADATA_READ_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
    static METADATA_PUBLISH_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
type MetadataHook = std::thread::LocalKey<std::cell::RefCell<Option<Box<dyn FnOnce()>>>>;
#[cfg(test)]
fn run_hook(hook: &'static MetadataHook) {
    let callback = hook.with(|hook| hook.borrow_mut().take());
    if let Some(callback) = callback {
        callback();
    }
}
mod metadata;
#[cfg(test)]
mod tests;
