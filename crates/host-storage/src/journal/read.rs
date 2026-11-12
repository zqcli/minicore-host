//! Read and decode actual saved history without repairing or resuming execution.
use super::{
    FileStamp,
    validation::{Validation, Validator, invalid, validate_envelope},
};
use crate::{
    CompactionProjection, FilePosition, HISTORY_SCHEMA_VERSION, HistoryEntry, HistoryProblem,
    HistoryRecord, SCHEMA_VERSION, SessionStore, StorageError, file_handle::FileHandle,
    strict_json,
};
use minicore_host_protocol::SessionId;
use minicore_host_runtime::history::HistoryItem;
use std::io::Read;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedHistory {
    pub records: Vec<HistoryRecord>,
    pub next_position: u64,
}
impl LoadedHistory {
    /// Original native transcript, never the compressed model projection.
    pub fn runtime_history(&self) -> Vec<HistoryItem> {
        self.records
            .iter()
            .filter_map(|record| record.entry.runtime_item())
            .collect()
    }

    pub fn compaction_projection(&self) -> Option<CompactionProjection> {
        self.records.iter().rev().find_map(|record| {
            if let HistoryEntry::Compaction {
                source_position,
                covered_items,
                summary,
            } = &record.entry
            {
                Some(CompactionProjection::new(
                    *source_position,
                    *covered_items,
                    summary.clone(),
                ))
            } else {
                None
            }
        })
    }

    pub fn model_history(&self) -> Result<Vec<HistoryItem>, HistoryProblem> {
        let raw = self.runtime_history();
        match self.compaction_projection() {
            Some(projection) => projection.project(&raw),
            None => Ok(raw),
        }
    }
}

impl SessionStore {
    /// Internal full validation for load/resume, not public pagination. Never
    /// returns a prefix after an error and never writes or repairs the file.
    pub fn read_history(&self) -> Result<LoadedHistory, StorageError> {
        self.load()?;
        let dir = self.directory(false)?;
        let file = dir.open_history(false)?;
        let mut file = match FileHandle::shared(file) {
            Ok(file) => file,
            Err(std::fs::TryLockError::WouldBlock) => return Err(StorageError::HistoryInUse),
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(crate::error::io_error(
                    "lock history reader",
                    dir.path.join("history.jsonl"),
                    e,
                ));
            }
        };
        #[cfg(test)]
        crate::lease_tests::after_lock(&file);
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

pub(super) fn decode_history(
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
        if version != u64::from(SCHEMA_VERSION) && version != u64::from(HISTORY_SCHEMA_VERSION) {
            return Err(StorageError::UnsupportedSchema {
                found: version,
                supported: HISTORY_SCHEMA_VERSION,
                position: Some(position),
            });
        }
        let record: HistoryRecord = strict_json::decode(value)
            .map_err(|e| invalid(position, HistoryProblem::InvalidRecord(e)))?;
        validate_envelope(&record, session, next, position)?;
        validation
            .entry(&record.entry, record.position)
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
