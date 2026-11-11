#[cfg(unix)]
use std::os::fd::{AsFd, BorrowedFd};
use std::{
    fs::{File, TryLockError},
    ops::{Deref, DerefMut},
};

/// One owned descriptor and, when acquired, its advisory lease. Root directory
/// clones share this handle through Arc, so only the final actual owner unlocks.
/// A forked child can inherit the description until exec without owning storage
/// work; close alone would incorrectly let that child extend the lease.
pub(crate) struct FileHandle {
    file: File,
    leased: bool,
}
impl FileHandle {
    pub(crate) fn unlocked(file: File) -> Self {
        Self {
            file,
            leased: false,
        }
    }
    pub(crate) fn exclusive(file: File) -> Result<Self, TryLockError> {
        file.try_lock()?;
        Ok(Self { file, leased: true })
    }
    pub(crate) fn shared(file: File) -> Result<Self, TryLockError> {
        file.try_lock_shared()?;
        Ok(Self { file, leased: true })
    }
}
impl Drop for FileHandle {
    fn drop(&mut self) {
        if self.leased {
            // This is resource cleanup, never a save/sync confirmation. Closing
            // follows even on an unlock error; any next acquisition must still
            // independently obtain the OS lock rather than presume success.
            let _ = self.file.unlock();
        }
    }
}
impl Deref for FileHandle {
    type Target = File;
    fn deref(&self) -> &File {
        &self.file
    }
}
impl DerefMut for FileHandle {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.file
    }
}
#[cfg(unix)]
impl AsFd for FileHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}
