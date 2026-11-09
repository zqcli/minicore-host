use std::{collections::BTreeMap, fs, sync::Arc};

use minicore_host_protocol::{SessionId, ToolCallId, ToolCallRef, TurnId};
use minicore_host_workspace::*;
use tempfile::TempDir;

fn tool(turn_id: &TurnId, name: &str) -> ToolCallRef {
    ToolCallRef {
        turn_id: turn_id.clone(),
        tool_call_id: ToolCallId(name.into()),
    }
}
fn list(scope: ChangeScope, limit: usize) -> ChangesListRequest {
    ChangesListRequest {
        scope,
        limit,
        cursor: None,
    }
}
fn request(reference: ChangeRef, bytes: usize) -> DiffRequest {
    DiffRequest {
        change_ref: reference,
        comparison: DiffComparison::ToolBeforeAfter,
        context_lines: 1,
        max_bytes: bytes,
        cursor: None,
    }
}

#[tokio::test]
async fn retained_tool_edits_use_real_snapshots_not_later_filesystem_or_git_state() {
    // This test deliberately observes a directory with no repository ancestors.
    let root = TempDir::new_in("/tmp").unwrap();
    fs::write(root.path().join("file"), "before\n").unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let before = ws
        .capture("file".into(), QueryContext::default())
        .await
        .unwrap();
    fs::write(root.path().join("file"), "after\n").unwrap();
    let after = ws
        .capture("file".into(), QueryContext::default())
        .await
        .unwrap();
    let session = SessionId::new().unwrap();
    let turn = TurnId::new().unwrap();
    let mut journal = EditJournal::new(session.clone());
    let reference = journal
        .append(EditRecord {
            tool_call: tool(&turn, "write_1"),
            path: "file".into(),
            before,
            after,
            outcome: EditOutcome::Applied,
        })
        .unwrap();
    fs::write(root.path().join("file"), "unrelated later content\n").unwrap();
    let record = journal.get(&reference).unwrap();
    let result = diff_edit(
        record,
        request(reference.clone(), 1000),
        QueryContext::default(),
    )
    .await
    .unwrap();
    let lines: String = result
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.text.as_str())
        .collect();
    assert_eq!(lines, "before\nafter\n");
    assert_eq!(result.origin, ChangeOrigin::Tool);
    assert!(!result.versions_refreshed);
    assert!(
        !ws.status(QueryContext::default())
            .await
            .unwrap()
            .repo_available
    );
    let records = journal
        .list(list(ChangeScope::Turn { turn_id: turn }, 1))
        .unwrap()
        .records;
    assert_eq!(records[0].origin, ChangeOrigin::Tool);
    assert_eq!(records[0].edit_outcome, Some(EditOutcome::Applied));
    assert!(records[0].details_available);
    let bytes = serde_json::to_vec(journal.records()).unwrap();
    let restored =
        EditJournal::from_records(session, serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(
        restored.get(&reference).unwrap(),
        journal.get(&reference).unwrap()
    );
}

#[tokio::test]
async fn utf8_diff_fragments_reconstruct_lines_across_multiple_hunks() {
    let session = SessionId::new().unwrap();
    let turn = TurnId::new().unwrap();
    let mut journal = EditJournal::new(session);
    let old = "one\nold中🙂\nkeep1\nkeep2\nkeep3\nkeep4\nlast\r\n";
    let new = "one\nnew🙂中\nkeep1\nkeep2\nkeep3\nkeep4\n尾巴";
    let reference = journal
        .append(EditRecord {
            tool_call: tool(&turn, "edit"),
            path: "file".into(),
            before: FileSnapshot::Content(old.as_bytes().to_vec()),
            after: FileSnapshot::Content(new.as_bytes().to_vec()),
            outcome: EditOutcome::Applied,
        })
        .unwrap();
    let record = journal.get(&reference).unwrap();
    let full = diff_edit(
        record.clone(),
        request(reference.clone(), usize::MAX),
        QueryContext::default(),
    )
    .await
    .unwrap();
    assert_eq!(full.hunks.len(), 2);
    let mut all_context = request(reference.clone(), usize::MAX);
    all_context.context_lines = usize::MAX;
    assert_eq!(
        diff_edit(record.clone(), all_context, QueryContext::default())
            .await
            .unwrap()
            .hunks
            .len(),
        1
    );
    let mut pages = Vec::new();
    let mut next = request(reference.clone(), 4);
    loop {
        let page = diff_edit(record.clone(), next.clone(), QueryContext::default())
            .await
            .unwrap();
        let bytes: usize = page
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .map(|line| line.text.len())
            .sum();
        assert!((1..=4).contains(&bytes));
        next.cursor = page.next_cursor.clone();
        pages.push(page);
        if next.cursor.is_none() {
            break;
        }
    }
    assert!(pages.len() > 10);
    let mut reconstructed: BTreeMap<(usize, usize, Option<usize>, Option<usize>), String> =
        BTreeMap::new();
    for page in &pages {
        for hunk in &page.hunks {
            for line in &hunk.lines {
                let key = (
                    hunk.old_start,
                    hunk.new_start,
                    line.old_index,
                    line.new_index,
                );
                let text = reconstructed.entry(key).or_default();
                assert_eq!(text.len(), line.line_byte_offset);
                text.push_str(&line.text);
            }
        }
    }
    for hunk in &full.hunks {
        for line in &hunk.lines {
            assert_eq!(
                reconstructed[&(
                    hunk.old_start,
                    hunk.new_start,
                    line.old_index,
                    line.new_index
                )],
                line.text
            );
        }
    }
    let mut changed_context = request(reference.clone(), 4);
    changed_context.context_lines = 2;
    changed_context.cursor = pages[0].next_cursor.clone();
    assert!(matches!(
        diff_edit(record.clone(), changed_context, QueryContext::default()).await,
        Err(WorkspaceError::InvalidCursor)
    ));
    let mut wrong_offset = request(reference, 4);
    wrong_offset.cursor = pages.iter().find_map(|page| page.next_cursor.clone());
    wrong_offset.cursor.as_mut().unwrap().line_byte_offset = usize::MAX;
    assert!(matches!(
        diff_edit(record, wrong_offset, QueryContext::default()).await,
        Err(WorkspaceError::InvalidCursor)
    ));
}

#[tokio::test]
async fn unknown_failed_and_binary_edit_evidence_is_not_fabricated() {
    let session = SessionId::new().unwrap();
    let turn = TurnId::new().unwrap();
    let mut journal = EditJournal::new(session);
    let unknown = journal
        .append(EditRecord {
            tool_call: tool(&turn, "unknown"),
            path: "file".into(),
            before: FileSnapshot::Unknown,
            after: FileSnapshot::Content(b"after".to_vec()),
            outcome: EditOutcome::Unknown,
        })
        .unwrap();
    let no_write = journal
        .append(EditRecord {
            tool_call: tool(&turn, "failed"),
            path: "file".into(),
            before: FileSnapshot::Missing,
            after: FileSnapshot::Missing,
            outcome: EditOutcome::NotApplied,
        })
        .unwrap();
    let binary = journal
        .append(EditRecord {
            tool_call: tool(&turn, "binary"),
            path: "binary".into(),
            before: FileSnapshot::Missing,
            after: FileSnapshot::Content(vec![0, 255]),
            outcome: EditOutcome::Applied,
        })
        .unwrap();
    let list = journal.list(list(ChangeScope::Session, 100)).unwrap();
    assert!(!list.records[0].details_available);
    assert_eq!(list.records[0].kind, ChangeKind::Unknown);
    assert_eq!(list.records[1].kind, ChangeKind::Unchanged);
    assert_eq!(list.records[1].edit_outcome, Some(EditOutcome::NotApplied));
    assert!(matches!(
        diff_edit(
            journal.get(&unknown).unwrap(),
            request(unknown, 20),
            QueryContext::default()
        )
        .await,
        Err(WorkspaceError::SnapshotUnavailable)
    ));
    let empty = diff_edit(
        journal.get(&no_write).unwrap(),
        request(no_write, 20),
        QueryContext::default(),
    )
    .await
    .unwrap();
    assert!(empty.hunks.is_empty() && empty.complete);
    let binary = diff_edit(
        journal.get(&binary).unwrap(),
        request(binary, 20),
        QueryContext::default(),
    )
    .await
    .unwrap();
    assert!(binary.binary && binary.hunks.is_empty());
}

#[test]
fn journal_pages_filter_actual_turns_and_reject_wrong_session_or_reference() {
    let session = SessionId::new().unwrap();
    let turn = TurnId::new().unwrap();
    let other_turn = TurnId::new().unwrap();
    let mut journal = EditJournal::new(session);
    let mut references = Vec::new();
    for (index, turn) in [&turn, &other_turn, &turn].iter().enumerate() {
        references.push(
            journal
                .append(EditRecord {
                    tool_call: tool(turn, &format!("call{index}")),
                    path: "same-file".into(),
                    before: FileSnapshot::Missing,
                    after: FileSnapshot::Content(vec![index as u8]),
                    outcome: EditOutcome::Applied,
                })
                .unwrap(),
        );
    }
    let scope = ChangeScope::Turn {
        turn_id: turn.clone(),
    };
    let first = journal.list(list(scope.clone(), 1)).unwrap();
    assert_eq!(first.records[0].change_ref, references[0]);
    let second = journal
        .list(ChangesListRequest {
            scope,
            cursor: first.next_cursor.clone(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(second.records[0].change_ref, references[2]);
    assert!(second.next_cursor.is_none());
    assert!(matches!(
        journal.list(ChangesListRequest {
            scope: ChangeScope::Turn {
                turn_id: other_turn
            },
            cursor: first.next_cursor,
            limit: 1
        }),
        Err(WorkspaceError::InvalidCursor)
    ));
    let ChangeRef::Tool { session_id, .. } = &mut references[0] else {
        unreachable!()
    };
    *session_id = SessionId::new().unwrap();
    assert!(matches!(
        journal.get(&references[0]),
        Err(WorkspaceError::ChangeNotFound)
    ));
    assert!(matches!(
        journal.append(EditRecord {
            tool_call: tool(&turn, "escape"),
            path: "../outside".into(),
            before: FileSnapshot::Unknown,
            after: FileSnapshot::Unknown,
            outcome: EditOutcome::Unknown
        }),
        Err(WorkspaceError::InvalidPath)
    ));
}

#[tokio::test]
async fn capture_distinguishes_actual_missing_from_unreadable_and_tool_id_mismatch() {
    let root = TempDir::new().unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    assert_eq!(
        ws.capture("missing/sub/file".into(), QueryContext::default())
            .await
            .unwrap(),
        FileSnapshot::Missing
    );
    fs::create_dir(root.path().join("directory")).unwrap();
    assert!(matches!(
        ws.capture("directory".into(), QueryContext::default())
            .await,
        Err(WorkspaceError::NotFile)
    ));
    let turn = TurnId::new().unwrap();
    let record = Arc::new(EditRecord {
        tool_call: tool(&turn, "actual"),
        path: "file".into(),
        before: FileSnapshot::Missing,
        after: FileSnapshot::Content(b"new".to_vec()),
        outcome: EditOutcome::Applied,
    });
    let reference = ChangeRef::Tool {
        session_id: SessionId::new().unwrap(),
        index: 0,
        tool_call: tool(&turn, "wrong"),
    };
    assert!(matches!(
        diff_edit(record, request(reference, 4), QueryContext::default()).await,
        Err(WorkspaceError::ChangeNotFound)
    ));
}
