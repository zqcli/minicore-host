use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::{Algorithm, ChangeTag, TextDiff};

use crate::{
    ChangeOrigin, ChangeRef, EditRecord, FileSnapshot, QueryContext, Result, StatusEntryKind,
    Workspace, WorkspaceError, changes::repository, context::blocking,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffComparison {
    ToolBeforeAfter,
    HeadToIndex,
    IndexToWorktree,
    HeadToWorktree,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffRequest {
    pub change_ref: ChangeRef,
    pub comparison: DiffComparison,
    pub context_lines: usize,
    /// Maximum returned line-text bytes, excluding metadata. No Host maximum.
    pub max_bytes: usize,
    pub cursor: Option<DiffCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffCursor {
    pub fingerprint: String,
    pub hunk_index: usize,
    pub line_index: usize,
    pub line_byte_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_index: Option<usize>,
    pub new_index: Option<usize>,
    pub line_byte_offset: usize,
    pub text: String,
    pub line_complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffHunk {
    /// Original hunk coordinates. A page can carry only some lines/fragments.
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DiffResult {
    pub change_ref: ChangeRef,
    pub comparison: DiffComparison,
    pub origin: ChangeOrigin,
    pub hunks: Vec<DiffHunk>,
    pub binary: bool,
    /// The Git list observation changed before this live diff was read.
    pub stale: bool,
    pub versions_refreshed: bool,
    pub next_cursor: Option<DiffCursor>,
    pub complete: bool,
    pub truncated: bool,
}

impl Workspace {
    pub async fn changes_diff(
        &self,
        request: DiffRequest,
        context: QueryContext,
    ) -> Result<DiffResult> {
        validate_request(&request)?;
        let ChangeRef::Workspace {
            workspace_scope,
            head_oid: original_head,
            entry: original_entry,
        } = &request.change_ref
        else {
            return Err(WorkspaceError::InvalidRange);
        };
        if request.comparison == DiffComparison::ToolBeforeAfter {
            return Err(WorkspaceError::InvalidRange);
        }
        // Validate the list-time source before any Git/status/content reads.
        // Matching HEAD/status alone does not identify a workspace: two unborn
        // repositories can contain the same untracked path with different bytes.
        if *workspace_scope != self.scope(&["changes"]) {
            return Err(WorkspaceError::ChangeNotFound);
        }
        let repository = repository(self, &context).await?;
        let status = repository.status(&context).await?;
        let entry = status
            .entries
            .iter()
            .find(|entry| *entry == original_entry)
            .or_else(|| {
                status
                    .entries
                    .iter()
                    .find(|entry| entry.path == original_entry.path)
            })
            .ok_or(WorkspaceError::ChangeNotFound)?;
        if entry.kind == StatusEntryKind::Unmerged || entry.submodule {
            return Err(WorkspaceError::UnsupportedChange);
        }
        let stale = status.head_oid != *original_head || *entry != *original_entry;
        let head_path = entry.original_path.as_deref().unwrap_or(&entry.path);
        let (before, after) = match request.comparison {
            DiffComparison::HeadToIndex => (
                repository
                    .head_bytes(status.head_oid.as_deref(), head_path, &context)
                    .await?,
                repository.index_bytes(&entry.path, &context).await?,
            ),
            DiffComparison::IndexToWorktree => (
                repository.index_bytes(&entry.path, &context).await?,
                self.worktree_bytes(entry.path.clone(), context.clone())
                    .await?,
            ),
            DiffComparison::HeadToWorktree => (
                repository
                    .head_bytes(status.head_oid.as_deref(), head_path, &context)
                    .await?,
                self.worktree_bytes(entry.path.clone(), context.clone())
                    .await?,
            ),
            DiffComparison::ToolBeforeAfter => unreachable!(),
        };
        let scope = self.scope(&["diff", &entry.path]);
        blocking(context, move |context| {
            build_diff(request, &before, &after, &scope, stale, context)
        })
        .await
    }
}

/// Compute a diff from an actual retained tool observation. Obtain the record
/// with `EditJournal::get` and release the Session lock before awaiting this.
/// The caller must use the reference returned by that same journal lookup.
pub async fn diff_edit(
    record: Arc<EditRecord>,
    request: DiffRequest,
    context: QueryContext,
) -> Result<DiffResult> {
    validate_request(&request)?;
    let ChangeRef::Tool {
        session_id,
        index,
        tool_call,
    } = &request.change_ref
    else {
        return Err(WorkspaceError::InvalidRange);
    };
    if *tool_call != record.tool_call || request.comparison != DiffComparison::ToolBeforeAfter {
        return Err(WorkspaceError::ChangeNotFound);
    }
    let scope = format!(
        "{session_id}/{index}/{}/{}/{}",
        tool_call.turn_id, tool_call.tool_call_id.0, record.path
    );
    blocking(context, move |context| {
        build_diff(
            request,
            &record.before,
            &record.after,
            &scope,
            false,
            context,
        )
    })
    .await
}

fn validate_request(request: &DiffRequest) -> Result<()> {
    if request.max_bytes == 0 {
        Err(WorkspaceError::InvalidRange)
    } else {
        Ok(())
    }
}

fn build_diff(
    request: DiffRequest,
    before: &FileSnapshot,
    after: &FileSnapshot,
    scope: &str,
    stale: bool,
    context: &QueryContext,
) -> Result<DiffResult> {
    context.check()?;
    let old = before.bytes()?;
    let new = after.bytes()?;
    let mut hash = Sha256::new();
    for bytes in [scope.as_bytes(), old, new] {
        hash.update(bytes.len().to_le_bytes());
        hash.update(bytes);
    }
    hash.update(request.context_lines.to_le_bytes());
    hash.update([request.comparison as u8]);
    let fingerprint = format!("{:x}", hash.finalize());
    if request
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.fingerprint != fingerprint)
    {
        return Err(WorkspaceError::InvalidCursor);
    }
    let mut result = DiffResult {
        origin: if request.comparison == DiffComparison::ToolBeforeAfter {
            ChangeOrigin::Tool
        } else {
            ChangeOrigin::WorkspaceUnknown
        },
        comparison: request.comparison,
        versions_refreshed: request.comparison != DiffComparison::ToolBeforeAfter,
        change_ref: request.change_ref.clone(),
        hunks: Vec::new(),
        binary: false,
        stale,
        next_cursor: None,
        complete: true,
        truncated: false,
    };
    let (Ok(old), Ok(new)) = (std::str::from_utf8(old), std::str::from_utf8(new)) else {
        if request.cursor.is_some() {
            return Err(WorkspaceError::InvalidCursor);
        }
        result.binary = true;
        return Ok(result);
    };
    if old.contains('\0') || new.contains('\0') {
        if request.cursor.is_some() {
            return Err(WorkspaceError::InvalidCursor);
        }
        result.binary = true;
        return Ok(result);
    }
    let mut config = TextDiff::configure();
    config.algorithm(Algorithm::Patience);
    if let Some(deadline) = context.deadline {
        config.deadline(deadline);
    }
    let diff = config.diff_lines(old, new);
    context.check()?;
    // Context beyond the actual file is equivalent to all available lines,
    // and must not overflow Similar's grouping arithmetic.
    let available_context = diff
        .ops()
        .last()
        .map_or(0, |op| op.old_range().end.max(op.new_range().end));
    let groups = diff.grouped_ops(request.context_lines.min(available_context));
    let continued = request.cursor.is_some();
    let cursor = request.cursor.unwrap_or(DiffCursor {
        fingerprint: fingerprint.clone(),
        hunk_index: 0,
        line_index: 0,
        line_byte_offset: 0,
    });
    if groups.is_empty() {
        if continued {
            return Err(WorkspaceError::InvalidCursor);
        }
        return Ok(result);
    }
    if cursor.hunk_index >= groups.len() {
        return Err(WorkspaceError::InvalidCursor);
    }
    let mut remaining = request.max_bytes;
    let mut next = None;
    // Keep full logical hunk coordinates, but borrow source lines until the
    // requested page needs an owned fragment. Later hunk text is not copied.
    'pages: for (hi, group) in groups.iter().enumerate().skip(cursor.hunk_index) {
        context.check()?;
        let first_op = group.first().expect("nonempty group");
        let last_op = group.last().expect("nonempty group");
        let old_start = first_op.old_range().start;
        let new_start = first_op.new_range().start;
        let old_count = last_op.old_range().end - old_start;
        let new_count = last_op.new_range().end - new_start;
        let first = if hi == cursor.hunk_index {
            cursor.line_index
        } else {
            0
        };
        let mut page = DiffHunk {
            old_start: old_start + usize::from(old_count != 0),
            old_count,
            new_start: new_start + usize::from(new_count != 0),
            new_count,
            lines: Vec::new(),
        };
        let mut lines = group
            .iter()
            .flat_map(|op| diff.iter_changes(op))
            .enumerate()
            .skip(first)
            .peekable();
        if hi == cursor.hunk_index {
            let (_, line) = lines.peek().ok_or(WorkspaceError::InvalidCursor)?;
            let text = line.value();
            if cursor.line_byte_offset >= text.len()
                || !text.is_char_boundary(cursor.line_byte_offset)
            {
                return Err(WorkspaceError::InvalidCursor);
            }
        }
        for (li, line) in lines {
            context.check()?;
            let text = line.value();
            let start = if hi == cursor.hunk_index && li == cursor.line_index {
                cursor.line_byte_offset
            } else {
                0
            };
            let mut end = start + (text.len() - start).min(remaining);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            if end == start {
                if result.hunks.is_empty() && page.lines.is_empty() {
                    return Err(WorkspaceError::InvalidRange);
                }
                next = Some((hi, li, start));
                if !page.lines.is_empty() {
                    result.hunks.push(page);
                }
                break 'pages;
            }
            page.lines.push(DiffLine {
                kind: match line.tag() {
                    ChangeTag::Equal => DiffLineKind::Context,
                    ChangeTag::Delete => DiffLineKind::Removed,
                    ChangeTag::Insert => DiffLineKind::Added,
                },
                old_index: line.old_index().map(|index| index + 1),
                new_index: line.new_index().map(|index| index + 1),
                text: text[start..end].to_owned(),
                line_byte_offset: start,
                line_complete: end == text.len(),
            });
            remaining -= end - start;
            if end != text.len() {
                next = Some((hi, li, end));
                result.hunks.push(page);
                break 'pages;
            }
        }
        result.hunks.push(page);
    }
    result.next_cursor = next.map(|(hunk_index, line_index, line_byte_offset)| DiffCursor {
        fingerprint,
        hunk_index,
        line_index,
        line_byte_offset,
    });
    result.complete = result.next_cursor.is_none();
    result.truncated = !result.complete;
    Ok(result)
}
