use crate::{
    FilePosition, HistoryEntry, HistoryProblem, HistoryRecord, SCHEMA_VERSION, SaveRange,
    SessionStore, StorageError, filesystem::Directory, strict_json,
};
use minicore_host_protocol::{PersistenceState, SessionId, TurnId, TurnOutcome};
use minicore_runtime::{
    LoopId,
    history::HistoryItem,
    model::{AssistantPart, ModelResponse},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
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
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedHistory {
    pub records: Vec<HistoryRecord>,
    pub next_position: u64,
}
impl LoadedHistory {
    pub fn runtime_history(&self) -> Vec<HistoryItem> {
        self.records
            .iter()
            .filter_map(|record| record.entry.runtime_item())
            .collect()
    }
}

struct Pending {
    range: SaveRange,
    bytes: Vec<u8>,
}
struct Persistent {
    // The owner handle retains the root inode lock until pending I/O is gone.
    _owner: SessionStore,
    dir: Directory,
    file: File,
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
        let mut file = dir.open_file("history.jsonl", true, false)?;
        lock_history(&file, &dir.path)?;
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
                .entry(&record.entry)
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
impl SessionStore {
    /// Internal full validation for load/resume, not public pagination. Never
    /// returns a prefix after an error and never writes or repairs the file.
    pub fn read_history(&self) -> Result<LoadedHistory, StorageError> {
        self.load()?;
        let dir = self.directory(false)?;
        let mut file = dir.open_file("history.jsonl", false, false)?;
        match file.try_lock_shared() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StorageError::HistoryInUse),
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(crate::error::io_error(
                    "lock history reader",
                    dir.path.join("history.jsonl"),
                    e,
                ));
            }
        }
        let before = FileStamp::read(&file, &dir.path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| {
            crate::error::io_error("read history", dir.path.join("history.jsonl"), e)
        })?;
        let (loaded, _) = decode_history(&bytes, self.id())?;
        if before != FileStamp::read(&file, &dir.path)? {
            return Err(StorageError::UncertainWrite);
        }
        Ok(loaded)
    }
}

pub(crate) fn lock_history(file: &File, directory: &Path) -> Result<(), StorageError> {
    match file.try_lock() {
        Ok(()) => Ok(()),
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
    mtime: (i64, i64),
    ctime: (i64, i64),
}
impl FileStamp {
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
}

fn decode_history(
    bytes: &[u8],
    session: &SessionId,
) -> Result<(LoadedHistory, Validation), StorageError> {
    let mut records = Vec::new();
    let baseline = Validation::default();
    let mut validation = Validator::new(&baseline);
    let mut offset = 0_u64;
    let mut next = 0_u64;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let position = FilePosition {
            line: next.checked_add(1).ok_or(StorageError::PositionOverflow)?,
            byte_offset: offset,
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            return Err(invalid(position, HistoryProblem::BlankRecord));
        }
        let value =
            strict_json::value(line).map_err(|e| invalid(position, HistoryProblem::Json(e)))?;
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                invalid(
                    position,
                    HistoryProblem::InvalidRecord("missing or invalid schema_version".into()),
                )
            })?;
        if version != u64::from(SCHEMA_VERSION) {
            return Err(StorageError::UnsupportedSchema {
                found: version,
                supported: SCHEMA_VERSION,
                position: Some(position),
            });
        }
        let record: HistoryRecord = strict_json::decode(value)
            .map_err(|e| invalid(position, HistoryProblem::InvalidRecord(e)))?;
        validate_envelope(&record, session, next, position)?;
        validation
            .entry(&record.entry)
            .map_err(|e| invalid(position, e))?;
        records.push(record);
        next = next.checked_add(1).ok_or(StorageError::PositionOverflow)?;
        offset = offset
            .checked_add(line.len() as u64)
            .ok_or(StorageError::PositionOverflow)?;
    }
    Ok((
        LoadedHistory {
            records,
            next_position: next,
        },
        validation.finish(),
    ))
}
fn invalid(position: FilePosition, problem: HistoryProblem) -> StorageError {
    StorageError::InvalidHistory { position, problem }
}
fn validate_envelope(
    record: &HistoryRecord,
    session: &SessionId,
    expected: u64,
    location: FilePosition,
) -> Result<(), StorageError> {
    if record.schema_version != SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema {
            found: u64::from(record.schema_version),
            supported: SCHEMA_VERSION,
            position: Some(location),
        });
    }
    if &record.session_id != session {
        return Err(invalid(location, HistoryProblem::WrongSession));
    }
    if record.position < expected {
        return Err(invalid(
            location,
            HistoryProblem::DuplicatePosition {
                expected,
                found: record.position,
            },
        ));
    }
    if record.position > expected {
        return Err(invalid(
            location,
            HistoryProblem::UnexpectedPosition {
                expected,
                found: record.position,
            },
        ));
    }
    Ok(())
}

#[derive(Default)]
struct Validation {
    loops: BTreeMap<LoopId, TurnId>,
    response_next: BTreeMap<TurnId, u32>,
    requests: BTreeSet<(LoopId, u32)>,
    calls: BTreeMap<(TurnId, String), SavedCall>,
    finished: BTreeSet<TurnId>,
    partial_turns: BTreeSet<TurnId>,
    partial_loops: BTreeSet<LoopId>,
}
#[derive(Clone)]
struct SavedCall {
    loop_id: LoopId,
    request_index: u32,
    response_index: u32,
    name: String,
    has_result: bool,
}
impl Validation {
    fn merge(&mut self, delta: Self) {
        self.loops.extend(delta.loops);
        self.response_next.extend(delta.response_next);
        self.requests.extend(delta.requests);
        self.calls.extend(delta.calls);
        self.finished.extend(delta.finished);
        self.partial_turns.extend(delta.partial_turns);
        self.partial_loops.extend(delta.partial_loops);
    }
}
// An overlay owns only this batch's index changes. Failed validation drops the
// overlay without copying or mutating any accumulated historical index.
struct Validator<'a> {
    base: &'a Validation,
    delta: Validation,
}
impl<'a> Validator<'a> {
    fn new(base: &'a Validation) -> Self {
        Self {
            base,
            delta: Validation::default(),
        }
    }
    fn finish(self) -> Validation {
        self.delta
    }

    fn entry(&mut self, entry: &HistoryEntry) -> Result<(), HistoryProblem> {
        let bad = |reason: &str| HistoryProblem::InvalidRecord(reason.to_owned());
        if entry
            .turn_id()
            .is_some_and(|id| self.delta.finished.contains(id) || self.base.finished.contains(id))
        {
            return Err(bad("record follows an already finished Turn"));
        }
        match entry {
            HistoryEntry::Input { turn_id, user, .. } => {
                if user.input.as_text().is_empty() {
                    return Err(bad("empty consumed input"));
                }
                self.loop_owner(user.loop_id, turn_id)?;
            }
            HistoryEntry::Runtime {
                turn_id,
                response_index,
                item,
            } => match item {
                HistoryItem::User(_) => {
                    return Err(bad(
                        "Runtime UserHistory must be an Input with consumed-message provenance",
                    ));
                }
                HistoryItem::Summary(_) => {}
                HistoryItem::Assistant(item) => {
                    ModelResponse::new_with_provider_replay(
                        item.content.clone(),
                        item.finish_reason,
                        item.usage,
                        item.provider_replay.clone(),
                    )
                    .map_err(|e| bad(&e.to_string()))?;
                    self.response(turn_id, item.loop_id, item.request_index, *response_index)?;
                    for call in item.content.iter().filter_map(AssistantPart::as_tool_call) {
                        let key = (turn_id.clone(), call.tool_call_id().as_str().to_owned());
                        if self.delta.calls.contains_key(&key) || self.base.calls.contains_key(&key)
                        {
                            return Err(bad("duplicate ToolCall identity within one Turn"));
                        }
                        self.delta.calls.insert(
                            key,
                            SavedCall {
                                loop_id: item.loop_id,
                                request_index: item.request_index,
                                response_index: *response_index,
                                name: call.name().as_str().to_owned(),
                                has_result: false,
                            },
                        );
                    }
                }
                HistoryItem::ToolResult(item) => {
                    self.loop_owner(item.loop_id, turn_id)?;
                    let key = (turn_id.clone(), item.call_id.as_str().to_owned());
                    if !self.delta.calls.contains_key(&key) {
                        let old =
                            self.base.calls.get(&key).ok_or_else(|| {
                                bad("ToolResult has no preceding matching ToolCall")
                            })?;
                        self.delta.calls.insert(key.clone(), old.clone());
                    }
                    let saved = self
                        .delta
                        .calls
                        .get_mut(&key)
                        .expect("call exists in the batch overlay");
                    if saved.loop_id != item.loop_id
                        || saved.request_index != item.request_index
                        || saved.response_index != *response_index
                        || saved.name != item.tool_name.as_str()
                    {
                        return Err(bad(
                            "ToolResult association conflicts with its original ToolCall",
                        ));
                    }
                    if saved.has_result {
                        return Err(bad("duplicate ToolResult"));
                    }
                    saved.has_result = true;
                }
            },
            HistoryEntry::Partial {
                turn_id,
                loop_id,
                response_index,
                partial,
            } => {
                self.response(turn_id, *loop_id, partial.request_index, *response_index)?;
                if partial.content.is_empty()
                    && partial.incomplete_tool_calls.is_empty()
                    && partial.usage.is_none()
                    && partial.provider_replay.is_none()
                {
                    return Err(bad("empty model partial"));
                }
                let mut ids = BTreeSet::new();
                let mut indices = BTreeSet::new();
                for part in &partial.content {
                    part.validate().map_err(|e| bad(&e.to_string()))?;
                    if let Some(call) = part.as_tool_call()
                        && (!ids.insert(call.tool_call_id().as_str())
                            || !indices.insert(call.call_index()))
                    {
                        return Err(bad("duplicate call within partial"));
                    }
                }
                let total = partial
                    .content
                    .len()
                    .checked_add(partial.incomplete_tool_calls.len())
                    .ok_or_else(|| bad("partial index overflow"))?;
                let mut slots = BTreeSet::new();
                for call in &partial.incomplete_tool_calls {
                    if call.part_index >= total
                        || !slots.insert(call.part_index)
                        || !ids.insert(call.tool_call_id.as_str())
                        || !indices.insert(call.call_index)
                    {
                        return Err(bad(
                            "invalid or duplicate incomplete call index or identity",
                        ));
                    }
                }
                if indices
                    .iter()
                    .enumerate()
                    .any(|(position, index)| u32::try_from(position).ok() != Some(*index))
                {
                    return Err(bad("non-contiguous partial call indices"));
                }
                let incomplete: BTreeMap<_, _> = partial
                    .incomplete_tool_calls
                    .iter()
                    .map(|c| (c.part_index, c))
                    .collect();
                let mut complete = partial.content.iter();
                let mut next_call = 0;
                for slot in 0..total {
                    let call_index = if let Some(call) = incomplete.get(&slot) {
                        Some(call.call_index)
                    } else {
                        complete
                            .next()
                            .and_then(AssistantPart::as_tool_call)
                            .map(|call| call.call_index())
                    };
                    if let Some(index) = call_index {
                        if index != next_call {
                            return Err(bad(
                                "partial call order conflicts with original content positions",
                            ));
                        }
                        next_call += 1;
                    }
                }
                self.delta.partial_turns.insert(turn_id.clone());
                self.delta.partial_loops.insert(*loop_id);
            }
            HistoryEntry::TurnFinished {
                turn_id, outcome, ..
            } => {
                if *outcome == TurnOutcome::Completed
                    && (self.delta.partial_turns.contains(turn_id)
                        || self.base.partial_turns.contains(turn_id))
                {
                    return Err(bad(
                        "completed Turn contains an unfinished terminal model response",
                    ));
                }
                self.delta.finished.insert(turn_id.clone());
            }
            HistoryEntry::Summary { .. } => {}
        }
        Ok(())
    }
    fn loop_owner(&mut self, loop_id: LoopId, turn: &TurnId) -> Result<(), HistoryProblem> {
        if self.delta.partial_loops.contains(&loop_id) || self.base.partial_loops.contains(&loop_id)
        {
            return Err(HistoryProblem::InvalidRecord(
                "record follows this Loop's terminal partial".into(),
            ));
        }
        match self
            .delta
            .loops
            .get(&loop_id)
            .or_else(|| self.base.loops.get(&loop_id))
        {
            Some(owner) if owner != turn => Err(HistoryProblem::InvalidRecord(
                "Loop belongs to another Turn".into(),
            )),
            Some(_) => Ok(()),
            None => {
                self.delta.loops.insert(loop_id, turn.clone());
                Ok(())
            }
        }
    }
    fn response(
        &mut self,
        turn: &TurnId,
        loop_id: LoopId,
        request: u32,
        response: u32,
    ) -> Result<(), HistoryProblem> {
        self.loop_owner(loop_id, turn)?;
        let next = self
            .delta
            .response_next
            .get(turn)
            .or_else(|| self.base.response_next.get(turn))
            .copied()
            .unwrap_or_default();
        if response != next {
            return Err(HistoryProblem::InvalidRecord(
                "response_index is not the next contiguous Turn response".into(),
            ));
        }
        let key = (loop_id, request);
        if self.delta.requests.contains(&key) || self.base.requests.contains(&key) {
            return Err(HistoryProblem::InvalidRecord(
                "duplicate Runtime logical response".into(),
            ));
        }
        self.delta.requests.insert(key);
        self.delta.response_next.insert(
            turn.clone(),
            next.checked_add(1)
                .ok_or_else(|| HistoryProblem::InvalidRecord("response index overflow".into()))?,
        );
        Ok(())
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
            std::fs::rename(
                &backend.dir.path,
                backend.dir.path.with_extension("detached"),
            )
            .unwrap();
            std::fs::create_dir(&backend.dir.path).unwrap();
            std::fs::write(backend.dir.path.join("history.jsonl"), b"replacement").unwrap();
        }
        _ => {}
    }
    Ok(())
}
