//! Windows storage uses NTFS handles with real directory flushes. Directory
//! handles deny delete sharing, so a checked component cannot be replaced while
//! a descendant operation still owns it. The root additionally denies write
//! sharing, making its handle the exclusive root lease without a lock sidecar.

use crate::{StorageError, error::io_error, file_handle::FileHandle};
use metadata::MetadataGates;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use windows_permissions::{
    LocalBox, Sid,
    constants::{AceType, SeObjectType, SecurityInformation},
    wrappers,
};

const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_POINT: u32 = 0x0000_0400;
const SHARE_READ: u32 = 1;
const SHARE_WRITE: u32 = 2;
const SHARE_DELETE: u32 = 4;

#[derive(Clone)]
pub(crate) struct Directory {
    pub file: Arc<FileHandle>,
    pub path: PathBuf,
    parent: Option<Arc<Directory>>,
    metadata: Arc<MetadataGates>,
    // All canonical ancestors remain attached as long as a root descendant is
    // live. Unlike Unix openat, the safe std Windows API opens child paths.
    _ancestors: Arc<Vec<File>>,
}

impl Directory {
    pub fn root(path: &Path) -> Result<Self, StorageError> {
        // The OS-created current user's profile is an explicit pre-existing
        // namespace boundary. Resolve and pin it before creating any data-root
        // ancestors; an environment variable or our own mkdir is not evidence
        // that this boundary already exists.
        let profile_path = winsafe::SHGetKnownFolderPath(
            &winsafe::co::KNOWNFOLDERID::Profile,
            winsafe::co::KF::DEFAULT,
            None,
        )
        .map_err(|e| {
            io_error(
                "locate current user profile",
                path,
                std::io::Error::other(e.to_string()),
            )
        })?;
        let profile_path = fs::canonicalize(profile_path)
            .map_err(|e| io_error("resolve current user profile", path, e))?;
        let profile = open_directory(&profile_path, false, false)
            .map_err(|e| io_error("pin current user profile", &profile_path, e))?;
        check_directory(&profile)?;
        fs::create_dir_all(path).map_err(|e| io_error("create data root", path, e))?;
        let canonical =
            fs::canonicalize(path).map_err(|e| io_error("resolve data root", path, e))?;
        let file = open_directory(&canonical, true, true).map_err(|e| {
            if matches!(e.raw_os_error(), Some(32 | 33)) {
                StorageError::RootInUse
            } else {
                io_error("open and lease data root", &canonical, e)
            }
        })?;
        check_directory(&file)?;
        check_ntfs(&canonical, &file)?;
        let under_profile = canonical.starts_with(&profile_path);
        if canonical == profile_path && identity(&file)? != identity(&profile)? {
            return Err(StorageError::UncertainWrite);
        }
        let mut ancestors = Vec::new();
        for parent in canonical.ancestors().skip(1) {
            let needs_sync = !under_profile || parent.starts_with(&profile_path);
            let handle = open_directory(parent, needs_sync, false)
                .map_err(|e| io_error("open root ancestor", parent, e))?;
            check_directory_kind(&handle)?;
            if needs_sync {
                sync_file(&handle, parent, "sync data root ancestor")?;
            }
            if parent == profile_path && identity(&handle)? != identity(&profile)? {
                return Err(StorageError::UncertainWrite);
            }
            ancestors.push(handle);
        }
        if under_profile {
            ancestors.push(profile);
        }
        sync_file(&file, &canonical, "sync data root")?;
        let root = Self {
            file: Arc::new(FileHandle::unlocked(file)),
            path: canonical,
            parent: None,
            metadata: Arc::default(),
            _ancestors: Arc::new(ancestors),
        };
        root.verify_attached()?;
        Ok(root)
    }

    pub fn child(&self, name: &str, create: bool) -> Result<Self, StorageError> {
        component(name)?;
        let path = self.path.join(name);
        if create {
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_error("create child directory", &path, e)),
            }
            self.sync()?;
        }
        let file = open_directory(&path, true, false)
            .map_err(|e| io_error("open child directory", &path, e))?;
        check_directory(&file)?;
        Ok(Self {
            file: Arc::new(FileHandle::unlocked(file)),
            path,
            parent: Some(Arc::new(self.clone())),
            metadata: self.metadata.clone(),
            _ancestors: self._ancestors.clone(),
        })
    }

    pub fn contains(&self, name: &str) -> Result<bool, StorageError> {
        component(name)?;
        match fs::symlink_metadata(self.path.join(name)) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
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
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .create_new(create_new)
            .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
            .open(self.path.join(name))
            .map_err(|e| io_error("open storage file", self.path.join(name), e))?;
        check_file(&file)?;
        Ok(file)
    }

    pub fn open_history(&self, write: bool) -> Result<File, StorageError> {
        // Sharing is checked by Windows against existing write handles and
        // writable file mappings, before we acquire the byte-range lock. Keep
        // delete sharing so a changed filename remains detectable by identity.
        // Metadata-only opens use open_file's ordinary sharing instead.
        let path = self.path.join("history.jsonl");
        let file = OpenOptions::new()
            .read(true)
            .write(write)
            .share_mode(SHARE_READ | SHARE_DELETE)
            .custom_flags(OPEN_REPARSE_POINT | BACKUP_SEMANTICS)
            .open(&path)
            .map_err(|e| {
                if matches!(e.raw_os_error(), Some(32 | 33)) {
                    StorageError::HistoryInUse
                } else {
                    io_error("open history lease", &path, e)
                }
            })?;
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

    pub fn read_metadata(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        component(name)?;
        let gate = self.metadata.get(self.path.join(name))?;
        let _snapshot = gate.read()?;
        self.verify_attached()?;
        let mut file = self.open_file(name, false, false)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| io_error("read object snapshot", self.path.join(name), e))?;
        check_file(&file)?;
        self.verify_file_identity(name, &file)?;
        self.verify_attached()?;
        Ok(bytes)
    }

    pub fn entries(&self) -> Result<Vec<String>, StorageError> {
        let mut entries = Vec::new();
        let reader = fs::read_dir(&self.path)
            .map_err(|e| io_error("read storage directory", &self.path, e))?;
        for entry in reader {
            let entry =
                entry.map_err(|e| io_error("read storage directory entry", &self.path, e))?;
            let name = entry
                .file_name()
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
            let gate = self.metadata.get(self.path.join(name))?;
            let _publication = gate.write()?;
            self.verify_attached()?;
            if self.contains(name)? {
                self.open_file(name, false, false)?;
            }
            fs::rename(self.path.join(&temporary), self.path.join(name))
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
        if result.is_err() {
            let _ = fs::remove_file(self.path.join(&temporary));
        }
        result
    }

    pub fn remove_file(&self, name: &str) -> Result<(), StorageError> {
        self.open_file(name, false, false)?;
        fs::remove_file(self.path.join(name))
            .map_err(|e| io_error("delete owned file", self.path.join(name), e))?;
        self.sync()
    }

    pub fn remove_empty_child(&self, name: &str) -> Result<(), StorageError> {
        // Release this temporary no-delete handle before the legitimate delete.
        // A live journal/descendant retains its own handle and still prevents it.
        self.child(name, false)?;
        fs::remove_dir(self.path.join(name)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::DirectoryNotEmpty {
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
        }
        let current = open_directory(&self.path, false, false)
            .map_err(|e| io_error("verify directory identity", &self.path, e))?;
        check_directory(&current)?;
        if identity(&self.file)? != identity(&current)? {
            return Err(StorageError::UncertainWrite);
        }
        Ok(())
    }

    pub fn verify_directory_identity(&self, other: &Self) -> Result<(), StorageError> {
        if identity(&self.file)? != identity(&other.file)? {
            return Err(StorageError::UncertainWrite);
        }
        Ok(())
    }

    pub fn verify_file_identity(&self, name: &str, file: &File) -> Result<(), StorageError> {
        let current = self.open_file(name, false, false)?;
        if identity(file)? != identity(&current)? {
            return Err(StorageError::UncertainWrite);
        }
        Ok(())
    }
}

fn open_directory(path: &Path, write: bool, exclusive: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(write)
        .share_mode(if exclusive {
            SHARE_READ
        } else {
            SHARE_READ | SHARE_WRITE
        })
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
}

fn component(name: &str) -> Result<(), StorageError> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|tail| {
                matches!(
                    tail,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
        || reserved
    {
        return Err(StorageError::UnsafePath {
            reason: "invalid storage path component",
        });
    }
    Ok(())
}

fn check_directory_kind(file: &File) -> Result<(), StorageError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error("inspect directory", PathBuf::new(), e))?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(StorageError::UnsafePath {
            reason: "reparse point or non-directory in storage path",
        });
    }
    Ok(())
}

fn check_directory(file: &File) -> Result<(), StorageError> {
    check_directory_kind(file)?;
    check_permissions(file)
}

fn check_file(file: &File) -> Result<(), StorageError> {
    let metadata = file
        .metadata()
        .map_err(|e| io_error("inspect file", PathBuf::new(), e))?;
    let information = winapi_util::file::information(file)
        .map_err(|e| io_error("inspect file links", PathBuf::new(), e))?;
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || information.number_of_links() != 1
    {
        return Err(StorageError::UnsafePath {
            reason: "file is not a private owned regular file with one link",
        });
    }
    check_permissions(file)
}

pub(crate) fn identity(file: &File) -> Result<(u64, u64), StorageError> {
    let information = winapi_util::file::information(file)
        .map_err(|e| io_error("inspect storage identity", PathBuf::new(), e))?;
    Ok((information.volume_serial_number(), information.file_index()))
}

fn check_ntfs(path: &Path, file: &File) -> Result<(), StorageError> {
    let path = path.to_str().ok_or(StorageError::UnsafePath {
        reason: "non-UTF-8 data root",
    })?;
    let volume = winsafe::GetVolumePathName(path)
        .map_err(|e| io_error("locate storage volume", path, win_error(e)))?;
    if !matches!(
        winsafe::GetDriveType(Some(&volume)),
        winsafe::co::DRIVE::FIXED | winsafe::co::DRIVE::REMOVABLE
    ) {
        return Err(StorageError::UnsafePath {
            reason: "Windows storage requires a local NTFS volume",
        });
    }
    let mut serial = 0;
    let mut filesystem = String::new();
    winsafe::GetVolumeInformation(
        Some(&volume),
        None,
        Some(&mut serial),
        None,
        None,
        Some(&mut filesystem),
    )
    .map_err(|e| io_error("inspect storage volume", path, win_error(e)))?;
    if filesystem != "NTFS" || identity(file)?.0 != u64::from(serial) {
        return Err(StorageError::UnsafePath {
            reason: "Windows storage requires the checked NTFS volume",
        });
    }
    Ok(())
}

fn win_error(error: winsafe::co::ERROR) -> std::io::Error {
    std::io::Error::from_raw_os_error(error.raw() as i32)
}

fn check_permissions(file: &File) -> Result<(), StorageError> {
    let token = winsafe::HACCESSTOKEN::GetCurrentProcessToken();
    let information = token
        .GetTokenInformation(winsafe::co::TOKEN_INFORMATION_CLASS::User)
        .map_err(|e| io_error("inspect storage user", PathBuf::new(), win_error(e)))?;
    let winsafe::TokenInfo::User(user) = information else {
        return Err(StorageError::UnsafePath {
            reason: "current token has no user identity",
        });
    };
    let user = user.User.Sid().ok_or(StorageError::UnsafePath {
        reason: "current token has no user SID",
    })?;
    let current: LocalBox<Sid> = user
        .to_string()
        .parse()
        .map_err(|e| io_error("decode storage user SID", PathBuf::new(), e))?;
    let system: LocalBox<Sid> = "S-1-5-18"
        .parse()
        .map_err(|e| io_error("decode system SID", PathBuf::new(), e))?;
    let administrators: LocalBox<Sid> = "S-1-5-32-544"
        .parse()
        .map_err(|e| io_error("decode administrator SID", PathBuf::new(), e))?;
    let trusted = |sid: &Sid| sid == &*current || sid == &*system || sid == &*administrators;
    let descriptor = wrappers::GetSecurityInfo(
        file,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl,
    )
    .map_err(|e| io_error("inspect storage permissions", PathBuf::new(), e))?;
    if !descriptor.owner().is_some_and(trusted) {
        return Err(StorageError::UnsafePath {
            reason: "storage owner is not the current user or a system administrator",
        });
    }
    let dacl = descriptor.dacl().ok_or(StorageError::UnsafePath {
        reason: "storage has an unrestricted DACL",
    })?;
    // Includes data/child creation, attribute writes, deletion, ACL/owner changes
    // and generic write/all. A deny ACE never makes an unsafe allow acceptable:
    // this conservative check does not try to emulate Windows access evaluation.
    const MUTATION_RIGHTS: u32 = 0x500d_0156;
    for index in 0..dacl.len() {
        let ace = dacl.get_ace(index).ok_or(StorageError::UnsafePath {
            reason: "invalid storage DACL entry",
        })?;
        if ace.flags().bits() & 0x08 != 0 {
            continue; // INHERIT_ONLY does not grant access to this object.
        }
        match ace.ace_type() {
            AceType::ACCESS_DENIED_ACE_TYPE
            | AceType::ACCESS_DENIED_CALLBACK_ACE_TYPE
            | AceType::ACCESS_DENIED_CALLBACK_OBJECT_ACE_TYPE
            | AceType::ACCESS_DENIED_OBJECT_ACE_TYPE => continue,
            AceType::ACCESS_ALLOWED_ACE_TYPE
            | AceType::ACCESS_ALLOWED_CALLBACK_ACE_TYPE
            | AceType::ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE
            | AceType::ACCESS_ALLOWED_OBJECT_ACE_TYPE => {
                if ace.mask().bits() & MUTATION_RIGHTS != 0 && !ace.sid().is_some_and(trusted) {
                    return Err(StorageError::UnsafePath {
                        reason: "storage is writable by other users",
                    });
                }
            }
            _ => {
                return Err(StorageError::UnsafePath {
                    reason: "unsupported storage DACL entry",
                });
            }
        }
    }
    Ok(())
}

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
                std::io::Error::other("injected directory sync failure"),
            ));
        }
    }
    // FlushFileBuffers requires write access. Every directory handle used here
    // was opened read/write, and root acquisition verified NTFS rather than
    // accepting another filesystem's successful no-op directory flush.
    file.sync_all().map_err(|e| io_error(operation, path, e))
}

#[cfg(test)]
thread_local! {
    static SYNC_FAILURE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
    static SYNC_TRACE: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[path = "filesystem/metadata.rs"]
mod metadata;
#[cfg(test)]
#[path = "filesystem/windows_tests.rs"]
mod tests;
