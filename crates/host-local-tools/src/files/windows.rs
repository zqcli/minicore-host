//! Local file replacement has the same real Windows directory flush requirement
//! as persistence, without making standalone tools depend on the storage owner.

use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_POINT: u32 = 0x0000_0400;

pub(super) fn sync_directory(path: &Path) -> io::Result<()> {
    // FlushFileBuffers requires write access, including for directories. Hold
    // the actual directory without delete sharing through validation and flush.
    let directory = open_directory(path, true)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(io::Error::other(
            "replacement parent is not a plain directory",
        ));
    }
    let canonical = fs::canonicalize(path)?;
    let canonical_text = canonical
        .to_str()
        .ok_or_else(|| io::Error::other("replacement parent is not UTF-8"))?;
    let volume = winsafe::GetVolumePathName(canonical_text).map_err(win_error)?;
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
    .map_err(win_error)?;
    let information = winapi_util::file::information(&directory)?;
    if filesystem != "NTFS"
        || !matches!(
            winsafe::GetDriveType(Some(&volume)),
            winsafe::co::DRIVE::FIXED | winsafe::co::DRIVE::REMOVABLE
        )
        || information.volume_serial_number() != u64::from(serial)
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows file replacement requires a checked local NTFS directory",
        ));
    }
    // Volume lookup is path-based; verify that it described the held directory.
    let current = open_directory(&canonical, false)?;
    let current = winapi_util::file::information(&current)?;
    if current.volume_serial_number() != information.volume_serial_number()
        || current.file_index() != information.file_index()
    {
        return Err(io::Error::other("replacement parent changed before sync"));
    }
    directory.sync_all()
}

fn open_directory(path: &Path, write: bool) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(write)
        .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE, without DELETE.
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
}

fn win_error(error: winsafe::co::ERROR) -> io::Error {
    io::Error::from_raw_os_error(error.raw() as i32)
}
