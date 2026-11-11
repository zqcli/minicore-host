//! Caller-owned evidence of actual tool writes, separate from Git status.
use crate::{
    ChangeKind, ChangeOrigin, ChangeRecord, ChangeRef, ChangeScope, ChangesCursor,
    ChangesListRequest, ChangesListResult, EditOutcome, FileSnapshot, Result, WorkspaceError,
    workspace::{normalize, relative_path_text},
};
use minicore_host_protocol::{SessionId, ToolCallRef};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One retained tool observation. Its Session/Turn/ToolCall association comes
/// from the executing owner, never model-supplied attribution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRecord {
    pub tool_call: ToolCallRef,
    pub path: String,
    pub before: FileSnapshot,
    pub after: FileSnapshot,
    pub outcome: EditOutcome,
}

impl EditRecord {
    fn kind(&self) -> ChangeKind {
        match (&self.before, &self.after) {
            (FileSnapshot::Unknown, _) | (_, FileSnapshot::Unknown) => ChangeKind::Unknown,
            (FileSnapshot::Missing, FileSnapshot::Content(_)) => ChangeKind::Added,
            (FileSnapshot::Content(_), FileSnapshot::Missing) => ChangeKind::Deleted,
            (a, b) if a == b => ChangeKind::Unchanged,
            _ => ChangeKind::Modified,
        }
    }
}

/// An append-only, caller-owned Session journal. It has no I/O, tasks, locks,
/// retention quota, or implicit persistence. Records live with the caller's
/// retained execution facts; loading saved Session history does not rebuild this
/// journal. `from_records` accepts records explicitly supplied by that owner.
#[derive(Clone, Debug)]
pub struct EditJournal {
    session_id: SessionId,
    records: Vec<Arc<EditRecord>>,
}

impl EditJournal {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            records: Vec::new(),
        }
    }

    pub fn from_records(session_id: SessionId, records: Vec<Arc<EditRecord>>) -> Result<Self> {
        for record in &records {
            normalize(&record.path, false)?;
        }
        Ok(Self {
            session_id,
            records,
        })
    }

    pub fn records(&self) -> &[Arc<EditRecord>] {
        &self.records
    }

    /// Call after the tool owner has established the actual write outcome.
    /// A failed capture is `Unknown`, not `Missing`; a successful Git command
    /// alone is never evidence of a tool edit.
    pub fn append(&mut self, mut record: EditRecord) -> Result<ChangeRef> {
        record.path = relative_path_text(&normalize(&record.path, false)?)?;
        let reference = ChangeRef::Tool {
            session_id: self.session_id.clone(),
            index: self.records.len(),
            tool_call: record.tool_call.clone(),
        };
        self.records.push(Arc::new(record));
        Ok(reference)
    }

    /// Clone this record while holding an owner lock, then release the lock
    /// before invoking `diff_edit` or any other asynchronous work.
    pub fn get(&self, reference: &ChangeRef) -> Result<Arc<EditRecord>> {
        let ChangeRef::Tool {
            session_id,
            index,
            tool_call,
        } = reference
        else {
            return Err(WorkspaceError::ChangeNotFound);
        };
        if *session_id != self.session_id {
            return Err(WorkspaceError::ChangeNotFound);
        }
        self.records
            .get(*index)
            .filter(|record| record.tool_call == *tool_call)
            .cloned()
            .ok_or(WorkspaceError::ChangeNotFound)
    }

    pub fn list(&self, request: ChangesListRequest) -> Result<ChangesListResult> {
        if request.limit == 0 {
            return Err(WorkspaceError::InvalidRange);
        }
        let turn_id = match request.scope {
            ChangeScope::Session => None,
            ChangeScope::Turn { turn_id } => Some(turn_id),
            ChangeScope::Workspace => return Err(WorkspaceError::InvalidRange),
        };
        let start = match request.cursor {
            None => 0,
            Some(ChangesCursor::Tool {
                session_id,
                turn_id: cursor_turn,
                after_index,
            }) if session_id == self.session_id
                && cursor_turn == turn_id
                && after_index < self.records.len() =>
            {
                after_index + 1
            }
            _ => return Err(WorkspaceError::InvalidCursor),
        };
        let mut selected = self
            .records
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(_, record)| {
                turn_id
                    .as_ref()
                    .is_none_or(|turn| record.tool_call.turn_id == *turn)
            });
        let records: Vec<_> = selected
            .by_ref()
            .take(request.limit)
            .map(|(index, record)| ChangeRecord {
                change_ref: ChangeRef::Tool {
                    session_id: self.session_id.clone(),
                    index,
                    tool_call: record.tool_call.clone(),
                },
                path: record.path.clone(),
                kind: record.kind(),
                origin: ChangeOrigin::Tool,
                details_available: record.before != FileSnapshot::Unknown
                    && record.after != FileSnapshot::Unknown,
                edit_outcome: Some(record.outcome),
            })
            .collect();
        let next_cursor = selected.next().map(|_| {
            let ChangeRef::Tool { index, .. } = &records.last().expect("positive limit").change_ref
            else {
                unreachable!()
            };
            ChangesCursor::Tool {
                session_id: self.session_id.clone(),
                turn_id,
                after_index: *index,
            }
        });
        Ok(ChangesListResult {
            records,
            next_cursor,
        })
    }
}
