//! Accepted history writes and original-range persistence confirmation.
mod read;
mod validation;

pub use read::LoadedHistory;
use read::decode_history;
use validation::{Validation, Validator, invalid, validate_envelope};

use crate::{
    FilePosition, HistoryEntry, HistoryProblem, HistoryRecord, SaveRange, SessionStore,
    StorageError, file_handle::FileHandle, filesystem::Directory, strict_json,
};
use minicore_host_protocol::{PersistenceState, SessionId};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SavePositions {
    /// Exclusive end of records this writer has accepted, including failed saves.
    pub accepted: u64,
    /// None for memory. Never advanced by readable bytes or an unconfirmed write.
    pub persisted: Option<u64>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SaveConfirmation {
    pub range: SaveRange,
    pub persistence: PersistenceState,
}

struct Pending {
    range: SaveRange,
    bytes: Vec<u8>,
}
struct Persistent {
    // Drop the history lease before its directory and final root owner. All
    // accepted I/O must have finished before the owning worker drops this value.
    file: FileHandle,
    dir: Directory,
    _owner: SessionStore,
    stamp: FileStamp,
    prefix_hash: Sha256,
    ends_with_newline: bool,
}
impl Persistent {
    fn checked_stamp(&self, expected_len: u64) -> Result<FileStamp, StorageError> {
        self.dir.verify_attached()?;
        self.dir
            .verify_directory_identity(&self._owner.directory(false)?)?;
        self.dir.verify_file_identity("history.jsonl", &self.file)?;
        let stamp = FileStamp::read(&self.file, &self.dir.path)?;
        if stamp.len != expected_len {
            return Err(StorageError::UncertainWrite);
        }
        Ok(stamp)
    }
}
enum Backend {
    Memory,
    Persistent(Box<Persistent>),
}

/// One Session's single-owner incremental writer, not a business owner.
/// The caller keeps accepted work alive and invokes its synchronous methods in a
/// blocking worker, outside Host/Agent/Session state locks.
pub struct SessionJournal {
    session_id: SessionId,
    backend: Backend,
    positions: SavePositions,
    validation: Validation,
    loaded: LoadedHistory,
    last_memory_range: SaveRange,
    pending: Option<Pending>,
    failed: bool,
    #[cfg(test)]
    failure: Option<TestFailure>,
}
impl SessionJournal {
    pub fn memory(session_id: SessionId) -> Self {
        Self {
            session_id,
            backend: Backend::Memory,
            positions: SavePositions {
                accepted: 0,
                persisted: None,
            },
            validation: Validation::default(),
            loaded: LoadedHistory {
                records: Vec::new(),
                next_position: 0,
            },
            last_memory_range: SaveRange::default(),
            pending: None,
            failed: false,
            #[cfg(test)]
            failure: None,
        }
    }
    pub(crate) fn persistent(owner: SessionStore, dir: Directory) -> Result<Self, StorageError> {
        let file = dir.open_file("history.jsonl", true, false)?;
        let mut file = lock_history(file, &dir.path)?;
        let before = FileStamp::read(&file, &dir.path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| {
            crate::error::io_error("load history", dir.path.join("history.jsonl"), e)
        })?;
        let (loaded, validation) = decode_history(&bytes, owner.id())?;
        if before != FileStamp::read(&file, &dir.path)? {
            return Err(StorageError::UncertainWrite);
        }
        // A reload proves readable content, not the old operation's confirmation.
        // Establish this writer's baseline with a fresh explicit sync, without
        // reconstructing any previous Turn, queue, pending save, or tool work.
        file.sync_all().map_err(|e| {
            crate::error::io_error(
                "sync loaded history baseline",
                dir.path.join("history.jsonl"),
                e,
            )
        })?;
        dir.sync()?;
        let mut hash = Sha256::new();
        hash.update(&bytes);
        let session_id = owner.id().clone();
        Ok(Self {
            session_id,
            backend: Backend::Persistent(Box::new(Persistent {
                _owner: owner,
                dir,
                file,
                stamp: before,
                prefix_hash: hash,
                ends_with_newline: bytes.last() == Some(&b'\n'),
            })),
            positions: SavePositions {
                accepted: loaded.next_position,
                persisted: Some(loaded.next_position),
            },
            validation,
            loaded,
            last_memory_range: SaveRange::default(),
            pending: None,
            failed: false,
            #[cfg(test)]
            failure: None,
        })
    }
    pub fn positions(&self) -> SavePositions {
        self.positions
    }
    /// Borrow the full validated baseline loaded on open, not later appends.
    pub fn loaded_records(&self) -> &[HistoryRecord] {
        &self.loaded.records
    }
    /// Move the initial context to its Session owner exactly once.
    pub fn take_loaded_history(&mut self) -> LoadedHistory {
        std::mem::replace(
            &mut self.loaded,
            LoadedHistory {
                records: Vec::new(),
                next_position: self.positions.accepted,
            },
        )
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.failed
    }
    pub fn pending_range(&self) -> Option<SaveRange> {
        self.pending.as_ref().map(|p| p.range)
    }
    pub fn accept(&mut self, entries: Vec<HistoryEntry>) -> Result<SaveRange, StorageError> {
        let start = self.positions.accepted;
        let records = entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| {
                let offset = u64::try_from(index).map_err(|_| StorageError::PositionOverflow)?;
                let position = start
                    .checked_add(offset)
                    .ok_or(StorageError::PositionOverflow)?;
                Ok(HistoryRecord::new(self.session_id.clone(), position, entry))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        self.accept_batch(records)
    }
    /// Accept exact caller-supplied positions. Repeating an already accepted
    /// range is an error, not idempotent replay or a second successful append.
    pub fn accept_batch(&mut self, records: Vec<HistoryRecord>) -> Result<SaveRange, StorageError> {
        if self.failed {
            return Err(StorageError::ReconciliationRequired);
        }
        if self.pending.is_some() {
            return Err(StorageError::PendingBatch);
        }
        let start = self.positions.accepted;
        let mut next = start;
        let mut validation = Validator::new(&self.validation);
        let mut bytes = Vec::new();
        if let Backend::Persistent(backend) = &self.backend
            && backend.stamp.len != 0
            && !backend.ends_with_newline
            && !records.is_empty()
        {
            bytes.push(b'\n');
        }
        for record in &records {
            let location = FilePosition {
                line: next.checked_add(1).ok_or(StorageError::PositionOverflow)?,
                byte_offset: 0,
            };
            validate_envelope(record, &self.session_id, next, location)?;
            validation
                .entry(&record.entry, record.position)
                .map_err(|problem| StorageError::InvalidHistory {
                    position: location,
                    problem,
                })?;
            let encoded = serde_json::to_vec(record)
                .map_err(|e| invalid(location, HistoryProblem::Json(e.to_string())))?;
            // Public native structs can be constructed without their wire
            // validators. Check the exact encoding before accepting ownership.
            let value = strict_json::value(&encoded)
                .map_err(|e| invalid(location, HistoryProblem::Json(e)))?;
            let _: HistoryRecord = strict_json::decode(value)
                .map_err(|e| invalid(location, HistoryProblem::InvalidRecord(e)))?;
            bytes.extend_from_slice(&encoded);
            bytes.push(b'\n');
            next = next.checked_add(1).ok_or(StorageError::PositionOverflow)?;
        }
        let range = SaveRange { start, end: next };
        self.validation.merge(validation.finish());
        self.positions.accepted = next;
        self.last_memory_range = range;
        if matches!(self.backend, Backend::Persistent(_)) && !range.is_empty() {
            self.pending = Some(Pending { range, bytes });
        }
        Ok(range)
    }
    pub fn persist_pending(&mut self) -> Result<SaveConfirmation, StorageError> {
        if self.failed {
            return Err(StorageError::ReconciliationRequired);
        }
        self.persist_original()
    }
    fn persist_original(&mut self) -> Result<SaveConfirmation, StorageError> {
        let current = self.positions.accepted;
        if matches!(self.backend, Backend::Memory) {
            return Ok(SaveConfirmation {
                range: self.last_memory_range,
                persistence: PersistenceState::NotApplicable,
            });
        }
        let Some(pending) = self.pending.as_ref() else {
            return Ok(SaveConfirmation {
                range: SaveRange {
                    start: current,
                    end: current,
                },
                persistence: if matches!(self.backend, Backend::Memory) {
                    PersistenceState::NotApplicable
                } else {
                    PersistenceState::Persisted
                },
            });
        };
        let Backend::Persistent(backend) = &mut self.backend else {
            unreachable!("only persistent journals hold pending bytes")
        };
        let result = (|| {
            backend
                .dir
                .verify_directory_identity(&backend._owner.directory(false)?)?;
            backend
                .dir
                .verify_file_identity("history.jsonl", &backend.file)?;
            if FileStamp::read(&backend.file, &backend.dir.path)? != backend.stamp {
                return Err(StorageError::UncertainWrite);
            }
            backend
                .file
                .seek(SeekFrom::Start(backend.stamp.len))
                .map_err(|e| {
                    crate::error::io_error(
                        "seek history append",
                        backend.dir.path.join("history.jsonl"),
                        e,
                    )
                })?;
            #[cfg(test)]
            if let Some(TestFailure::WritePrefix(count)) = self.failure {
                self.failure = None;
                backend
                    .file
                    .write_all(&pending.bytes[..count.min(pending.bytes.len())])
                    .map_err(|e| {
                        crate::error::io_error(
                            "injected prefix write",
                            backend.dir.path.join("history.jsonl"),
                            e,
                        )
                    })?;
                return Err(crate::error::io_error(
                    "injected history write failure",
                    backend.dir.path.join("history.jsonl"),
                    std::io::Error::from_raw_os_error(28),
                ));
            }
            backend.file.write_all(&pending.bytes).map_err(|e| {
                crate::error::io_error("append history", backend.dir.path.join("history.jsonl"), e)
            })?;
            #[cfg(test)]
            if matches!(self.failure, Some(TestFailure::FileSync)) {
                self.failure = None;
                return Err(crate::error::io_error(
                    "injected history sync failure",
                    backend.dir.path.join("history.jsonl"),
                    std::io::Error::from_raw_os_error(5),
                ));
            }
            backend.file.sync_all().map_err(|e| {
                crate::error::io_error("sync history", backend.dir.path.join("history.jsonl"), e)
            })?;
            #[cfg(test)]
            if matches!(self.failure, Some(TestFailure::DirectorySync)) {
                self.failure = None;
                return Err(crate::error::io_error(
                    "injected history directory sync failure",
                    backend.dir.path.join("history.jsonl"),
                    std::io::Error::from_raw_os_error(5),
                ));
            }
            #[cfg(test)]
            test_after_sync(&mut self.failure, backend)?;
            backend.dir.sync()?;
            let expected_len = backend
                .stamp
                .len
                .checked_add(pending.bytes.len() as u64)
                .ok_or(StorageError::PositionOverflow)?;
            backend.checked_stamp(expected_len)
        })();
        match result {
            Ok(stamp) => self.confirm(stamp),
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }
    /// Reconcile only the pending bytes owned by this running writer. If the
    /// complete original suffix exists, sync it without appending. If none of it
    /// exists and the original prefix matches, continue that same write. Partial,
    /// modified, truncated, replaced, or extra bytes remain blocked and unchanged.
    pub fn reconcile_pending(&mut self) -> Result<SaveConfirmation, StorageError> {
        let Some(pending) = self.pending.as_ref() else {
            return self.persist_pending();
        };
        let Backend::Persistent(backend) = &mut self.backend else {
            unreachable!()
        };
        let checked = (|| {
            backend
                .dir
                .verify_directory_identity(&backend._owner.directory(false)?)?;
            backend
                .dir
                .verify_file_identity("history.jsonl", &backend.file)?;
            let stamp = FileStamp::read(&backend.file, &backend.dir.path)?;
            let expected_end = backend
                .stamp
                .len
                .checked_add(
                    u64::try_from(pending.bytes.len())
                        .map_err(|_| StorageError::PositionOverflow)?,
                )
                .ok_or(StorageError::PositionOverflow)?;
            if stamp.len != backend.stamp.len && stamp.len != expected_end {
                return Err(StorageError::UncertainWrite);
            }
            backend.file.seek(SeekFrom::Start(0)).map_err(|e| {
                crate::error::io_error(
                    "seek original history range",
                    backend.dir.path.join("history.jsonl"),
                    e,
                )
            })?;
            let mut hash = Sha256::new();
            let mut remaining = backend.stamp.len;
            let mut buffer = [0_u8; 8192];
            while remaining != 0 {
                let length = usize::try_from(remaining.min(buffer.len() as u64))
                    .map_err(|_| StorageError::PositionOverflow)?;
                backend
                    .file
                    .read_exact(&mut buffer[..length])
                    .map_err(|e| {
                        crate::error::io_error(
                            "verify original history prefix",
                            backend.dir.path.join("history.jsonl"),
                            e,
                        )
                    })?;
                hash.update(&buffer[..length]);
                remaining -= length as u64;
            }
            if hash.finalize() != backend.prefix_hash.clone().finalize() {
                return Err(StorageError::UncertainWrite);
            }
            let complete = stamp.len == expected_end;
            if complete {
                let mut tail = vec![0; pending.bytes.len()];
                backend.file.read_exact(&mut tail).map_err(|e| {
                    crate::error::io_error(
                        "verify pending history range",
                        backend.dir.path.join("history.jsonl"),
                        e,
                    )
                })?;
                if tail != pending.bytes {
                    return Err(StorageError::UncertainWrite);
                }
            }
            if stamp != FileStamp::read(&backend.file, &backend.dir.path)? {
                return Err(StorageError::UncertainWrite);
            }
            Ok((complete, stamp))
        })();
        let (complete, stamp) = match checked {
            Ok(v) => v,
            Err(e) => {
                self.failed = true;
                return Err(e);
            }
        };
        if !complete {
            backend.stamp = stamp;
            self.failed = false;
            return self.persist_original();
        }
        let sync_result = (|| {
            backend.file.sync_all().map_err(|e| {
                crate::error::io_error(
                    "sync original pending range",
                    backend.dir.path.join("history.jsonl"),
                    e,
                )
            })?;
            #[cfg(test)]
            test_after_sync(&mut self.failure, backend)?;
            backend.dir.sync()?;
            let expected_len = backend
                .stamp
                .len
                .checked_add(pending.bytes.len() as u64)
                .ok_or(StorageError::PositionOverflow)?;
            backend.checked_stamp(expected_len)
        })();
        match sync_result {
            Ok(stamp) => self.confirm(stamp),
            Err(e) => {
                self.failed = true;
                Err(e)
            }
        }
    }
    fn confirm(&mut self, stamp: FileStamp) -> Result<SaveConfirmation, StorageError> {
        let pending = self
            .pending
            .take()
            .expect("confirmation has a pending original range");
        let Backend::Persistent(backend) = &mut self.backend else {
            unreachable!()
        };
        backend.prefix_hash.update(&pending.bytes);
        backend.stamp = stamp;
        backend.ends_with_newline = true;
        self.positions.persisted = Some(pending.range.end);
        self.failed = false;
        Ok(SaveConfirmation {
            range: pending.range,
            persistence: PersistenceState::Persisted,
        })
    }
}

pub(crate) fn lock_history(file: File, directory: &Path) -> Result<FileHandle, StorageError> {
    match FileHandle::exclusive(file) {
        Ok(file) => {
            #[cfg(test)]
            crate::lease_tests::after_lock(&file);
            Ok(file)
        }
        Err(std::fs::TryLockError::WouldBlock) => Err(StorageError::HistoryInUse),
        Err(std::fs::TryLockError::Error(e)) => Err(crate::error::io_error(
            "lock history writer",
            directory.join("history.jsonl"),
            e,
        )),
    }
}
#[derive(Clone, Copy, Eq, PartialEq)]
struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    #[cfg(unix)]
    mtime: (i64, i64),
    #[cfg(unix)]
    ctime: (i64, i64),
    #[cfg(windows)]
    write_time: u64,
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    attributes: u32,
    #[cfg(windows)]
    content_hash: [u8; 32],
}
impl FileStamp {
    #[cfg(unix)]
    fn read(file: &File, dir: &Path) -> Result<Self, StorageError> {
        let m = file.metadata().map_err(|e| {
            crate::error::io_error("inspect history range", dir.join("history.jsonl"), e)
        })?;
        Ok(Self {
            dev: m.dev(),
            ino: m.ino(),
            len: m.len(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        })
    }

    #[cfg(windows)]
    fn read(file: &File, dir: &Path) -> Result<Self, StorageError> {
        use std::os::windows::fs::{FileExt, MetadataExt};
        let inspect = || {
            file.metadata().map_err(|e| {
                crate::error::io_error("inspect history range", dir.join("history.jsonl"), e)
            })
        };
        let before = inspect()?;
        let (dev, ino) = crate::filesystem::identity(file)?;
        // Stable Rust does not expose Windows ChangeTime. Compare actual bytes
        // as well as identity/length/timestamps instead of silently omitting the
        // Unix stamp's protection against an in-place rewrite of equal length.
        // Windows seek_read changes the shared cursor, unlike Unix read_at.
        // Restore it even when verification fails; the journal owns all I/O on
        // this handle, so no other worker can race the temporary position.
        let mut cursor = file;
        let position = cursor.stream_position().map_err(|e| {
            crate::error::io_error("capture history cursor", dir.join("history.jsonl"), e)
        })?;
        let hashed = (|| {
            let mut hash = Sha256::new();
            let mut offset = 0u64;
            let mut buffer = [0u8; 8192];
            while offset < before.len() {
                let count = usize::try_from((before.len() - offset).min(buffer.len() as u64))
                    .expect("bounded by buffer length");
                let read = file.seek_read(&mut buffer[..count], offset).map_err(|e| {
                    crate::error::io_error("verify history bytes", dir.join("history.jsonl"), e)
                })?;
                if read == 0 {
                    return Err(StorageError::UncertainWrite);
                }
                hash.update(&buffer[..read]);
                offset += read as u64;
            }
            Ok(hash.finalize().into())
        })();
        cursor.seek(SeekFrom::Start(position)).map_err(|e| {
            crate::error::io_error("restore history cursor", dir.join("history.jsonl"), e)
        })?;
        let content_hash = hashed?;
        let after = inspect()?;
        if before.len() != after.len()
            || before.last_write_time() != after.last_write_time()
            || before.creation_time() != after.creation_time()
            || before.file_attributes() != after.file_attributes()
        {
            return Err(StorageError::UncertainWrite);
        }
        Ok(Self {
            dev,
            ino,
            len: after.len(),
            write_time: after.last_write_time(),
            creation_time: after.creation_time(),
            attributes: after.file_attributes(),
            content_hash,
        })
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum TestFailure {
    WritePrefix(usize),
    FileSync,
    DirectorySync,
    ReplaceFileAfterSync,
    ReplaceDirectoryAfterSync,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
fn test_after_sync(
    failure: &mut Option<TestFailure>,
    backend: &Persistent,
) -> Result<(), StorageError> {
    match failure {
        Some(TestFailure::ReplaceFileAfterSync) => {
            *failure = None;
            let path = backend.dir.path.join("history.jsonl");
            std::fs::rename(&path, path.with_extension("detached")).unwrap();
            std::fs::write(&path, b"replacement").unwrap();
        }
        Some(TestFailure::ReplaceDirectoryAfterSync) => {
            *failure = None;
            let moved = std::fs::rename(
                &backend.dir.path,
                backend.dir.path.with_extension("detached"),
            );
            #[cfg(windows)]
            {
                let error = moved.expect_err("owned directory must reject replacement");
                assert!(matches!(error.raw_os_error(), Some(5 | 32)));
            }
            #[cfg(unix)]
            {
                moved.unwrap();
                std::fs::create_dir(&backend.dir.path).unwrap();
                std::fs::write(backend.dir.path.join("history.jsonl"), b"replacement").unwrap();
            }
        }
        _ => {}
    }
    Ok(())
}
