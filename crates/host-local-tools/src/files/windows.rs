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

pub(super) fn preserve_permissions(
    target: &Path,
    temporary: &tempfile::NamedTempFile,
) -> io::Result<()> {
    use windows_permissions::{
        constants::{SeObjectType, SecurityInformation},
        wrappers,
    };

    const READ_CONTROL: u32 = 0x0002_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    let source = OpenOptions::new()
        .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(target)?;
    let metadata = source.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(io::Error::other("permission source is not a plain file"));
    }
    let security = wrappers::GetSecurityInfo(
        &source,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl,
    )?;
    let source_dacl = dacl_text(&security)?;
    let (source_flags, source_aces) = dacl_parts(&source_dacl)?;
    let source_protected = source_flags.contains('P');
    // NamedTempFile's ordinary read/write handle does not include WRITE_DAC.
    // Reopen its empty file with that right and verify the actual file identity
    // before changing security. No body bytes have been written at this point.
    let mut destination = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(temporary.path())?;
    let expected = winapi_util::file::information(temporary.as_file())?;
    let actual = winapi_util::file::information(&destination)?;
    if expected.volume_serial_number() != actual.volume_serial_number()
        || expected.file_index() != actual.file_index()
    {
        return Err(io::Error::other(
            "temporary file changed before permission copy",
        ));
    }
    let destination_security = wrappers::GetSecurityInfo(
        &destination,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner,
    )?;
    if security.owner().is_none() || security.owner() != destination_security.owner() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic replacement cannot preserve a different file owner",
        ));
    }
    // SDDL's D:P flag is the documented SE_DACL_PROTECTED bit. The safe wrapper
    // does not expose GetSecurityDescriptorControl, so read that bit through its
    // OS-provided DACL-only SDDL conversion and preserve inheritance behavior.
    let protection = if source_protected {
        SecurityInformation::ProtectedDacl
    } else {
        SecurityInformation::UnprotectedDacl
    };
    wrappers::SetSecurityInfo(
        &mut destination,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl | protection,
        None,
        None,
        security.dacl(),
        None,
    )?;
    let copied = wrappers::GetSecurityInfo(
        &destination,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl,
    )?;
    let copied_dacl = dacl_text(&copied)?;
    let (copied_flags, copied_aces) = dacl_parts(&copied_dacl)?;
    if copied_flags.contains('P') != source_protected
        || copied_aces != source_aces
        || copied.dacl().is_none() != security.dacl().is_none()
    {
        // Re-evaluating unprotected inheritance can expose a changed parent
        // policy. Reject before body writes instead of publishing broader ACLs.
        return Err(io::Error::other(
            "replacement cannot preserve the source DACL",
        ));
    }
    temporary.as_file().set_permissions(metadata.permissions())
}

fn dacl_text(descriptor: &windows_permissions::SecurityDescriptor) -> io::Result<String> {
    windows_permissions::wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(
        descriptor,
        windows_permissions::constants::SecurityInformation::Dacl,
    )?
    .into_string()
    .map_err(|_| io::Error::other("DACL description is not Unicode"))
}

fn dacl_parts(text: &str) -> io::Result<(&str, &str)> {
    let text = text
        .strip_prefix("D:")
        .ok_or_else(|| io::Error::other("source file has no representable DACL"))?;
    Ok(text.split_at(text.find('(').unwrap_or(text.len())))
}

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
