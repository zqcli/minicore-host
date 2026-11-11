//! Actual file-byte inspection for synchronized test scenarios, not a save
//! confirmation or the production history-reader path.

use std::{io, path::Path};

#[cfg(unix)]
pub fn read_history_bytes(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    std::fs::read(path)
}

#[cfg(windows)]
pub fn read_history_bytes(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    use winsafe::{HFILE, co};
    // A read-only mapping can inspect actual stored bytes while LockFileEx
    // protects the writer's range. It neither releases that writer nor obtains
    // write access. Tests must arrange their own stable observation boundary.
    let path = path
        .as_ref()
        .to_str()
        .ok_or_else(|| io::Error::other("non-UTF-8 history fixture path"))?;
    let error = |e: co::ERROR| io::Error::from_raw_os_error(e.raw() as i32);
    let (file, _) = HFILE::CreateFile(
        path,
        co::GENERIC::READ,
        Some(co::FILE_SHARE::READ | co::FILE_SHARE::WRITE | co::FILE_SHARE::DELETE),
        None,
        co::DISPOSITION::OPEN_EXISTING,
        co::FILE_ATTRIBUTE::NORMAL,
        None,
        None,
        None,
    )
    .map_err(error)?;
    let length = usize::try_from(file.GetFileSizeEx().map_err(error)?)
        .map_err(|_| io::Error::other("history fixture too large to map"))?;
    if length == 0 {
        return Ok(Vec::new());
    }
    let mapping = file
        .CreateFileMapping(None, co::PAGE::READONLY, None, None, None)
        .map_err(error)?;
    let view = mapping
        .MapViewOfFile(co::FILE_MAP::READ, 0, None)
        .map_err(error)?;
    Ok(view.as_slice(length).to_vec())
}
