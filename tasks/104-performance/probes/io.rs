//! Isolated fixtures exercising production public APIs, not live providers or
//! TUI behavior. Copy this unchanged to crates/host-core/examples/task104_io.rs
//! in each measured checkout, then build that example with --release --locked.
//! CLI: task104_io MODE SIZE REPETITIONS [first|middle|last|full]
//! Modes: metadata, journal, diff-lines, diff-long, edits.
//! SIZE is UTF-8 payload repetitions, diff line count, or edit-record count.
//! Setup, one warmup, and result assertions are excluded from each timed sample.
//! Emit raw samples; comparing different source sizes or builds is the runner's
//! responsibility. This fixture makes no speedup or production-load claim.

use std::{error::Error, hint::black_box, sync::Arc, time::Instant};

use minicore_host_protocol::{
    HostId, MessageId, MessageMode, MessageSource, SessionId, ToolCallId, ToolCallRef, TurnId,
};
use minicore_host_storage::{DataRoot, HistoryEntry, HostRecord, SessionJournal};
use minicore_host_workspace::{
    ChangeRef, ChangeScope, ChangesCursor, ChangesListRequest, DiffComparison, DiffCursor,
    DiffRequest, EditJournal, EditOutcome, EditRecord, FileSnapshot, QueryContext, diff_edit,
};
use minicore_runtime::{
    LoopId, UserInput,
    history::{UserHistory, UserMessageKind},
};
use serde_json::{Value, json};

type ProbeResult<T> = Result<T, Box<dyn Error>>;

fn elapsed(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).expect("sample fits u64 nanoseconds")
}

fn metadata(size: usize, repetitions: usize) -> ProbeResult<Value> {
    let temp = tempfile::tempdir()?;
    let root = DataRoot::acquire(temp.path().join("host"))?;
    let host = root.host(HostId::new()?);
    let text = "x中🙂".repeat(size);
    let mut nested = json!({"text": text});
    for _ in 0..6 {
        nested = json!({"nested": [nested]});
    }
    let exact: Value = serde_json::from_str("18446744073709551617")?;
    let record = HostRecord::new(
        host.id().clone(),
        json!({"payload":nested,"number":exact}),
        1,
    );
    host.save_host(&record)?;
    let file_bytes = std::fs::metadata(host.canonical_path().join("host.json"))?.len();
    let mut samples = Vec::with_capacity(repetitions);
    for sample in 0..=repetitions {
        let start = Instant::now();
        let loaded = black_box(root.load_host::<Value>()?).expect("published object");
        let nanos = elapsed(start);
        assert_eq!(loaded, record);
        if sample != 0 {
            samples.push(nanos);
        }
    }
    Ok(json!({"file_bytes":file_bytes,"samples_ns":samples}))
}

fn journal(size: usize, repetitions: usize) -> ProbeResult<Value> {
    let session = SessionId::new()?;
    let text = "x中🙂".repeat(size);
    let bytes = text.len();
    let entry = HistoryEntry::Input {
        received_at: minicore_host_protocol::Timestamp::now(),
        turn_id: TurnId::new()?,
        message_id: MessageId::new()?,
        source: MessageSource::User,
        mode: MessageMode::FollowUp,
        in_reply_to: None,
        user: UserHistory {
            loop_id: LoopId::new()?,
            kind: UserMessageKind::Prompt,
            input: UserInput::text(text)?,
        },
    };
    let mut samples = Vec::with_capacity(repetitions);
    for sample in 0..=repetitions {
        let mut journal = SessionJournal::memory(session.clone());
        let entries = vec![entry.clone()];
        let start = Instant::now();
        let range = black_box(journal.accept(entries)?);
        let nanos = elapsed(start);
        assert_eq!((range.start, range.end), (0, 1));
        assert_eq!(journal.positions().accepted, 1);
        if sample != 0 {
            samples.push(nanos);
        }
    }
    Ok(json!({"input_bytes":bytes,"samples_ns":samples}))
}

async fn diff(mode: &str, size: usize, repetitions: usize, position: &str) -> ProbeResult<Value> {
    let (before, after) = if mode == "diff-long" {
        let body = "x中🙂".repeat(size);
        (format!("old:{body}\n"), format!("new:{body}\n"))
    } else {
        let mut before = String::new();
        let mut after = String::new();
        for index in 0..size {
            if index % 12 == 0 {
                before.push_str(&format!("old {index}: x中🙂\n"));
                after.push_str(&format!("new {index}: x中🙂\n"));
            } else {
                let line = format!("same {index}: x中🙂\n");
                before.push_str(&line);
                after.push_str(&line);
            }
        }
        (before, after)
    };
    let source_bytes = before.len() + after.len();
    let tool_call = ToolCallRef {
        turn_id: TurnId::new()?,
        tool_call_id: ToolCallId("fixture-edit".into()),
    };
    let reference = ChangeRef::Tool {
        session_id: SessionId::new()?,
        index: 0,
        tool_call: tool_call.clone(),
    };
    let record = Arc::new(EditRecord {
        tool_call,
        path: "fixture.txt".into(),
        before: FileSnapshot::Content(before.into_bytes()),
        after: FileSnapshot::Content(after.into_bytes()),
        outcome: EditOutcome::Applied,
    });
    let mut request = DiffRequest {
        change_ref: reference,
        comparison: DiffComparison::ToolBeforeAfter,
        context_lines: 2,
        max_bytes: 128,
        cursor: None,
    };
    let first = diff_edit(record.clone(), request.clone(), QueryContext::default()).await?;
    let mut complete = request.clone();
    complete.max_bytes = usize::MAX;
    let full = diff_edit(record.clone(), complete.clone(), QueryContext::default()).await?;
    if position == "full" {
        request = complete;
    } else if position != "first" {
        let fingerprint = first
            .next_cursor
            .ok_or("fixture must span multiple pages")?
            .fingerprint;
        let hunk_index = if position == "last" {
            full.hunks.len() - 1
        } else {
            full.hunks.len() / 2
        };
        let hunk = &full.hunks[hunk_index];
        let line_index = if position == "last" {
            hunk.lines.len() - 1
        } else {
            hunk.lines.len() / 2
        };
        let text = &hunk.lines[line_index].text;
        let mut offset = if mode == "diff-long" {
            if position == "last" {
                text.len().saturating_sub(128)
            } else {
                text.len() / 2
            }
        } else {
            0
        };
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        request.cursor = Some(DiffCursor {
            fingerprint,
            hunk_index,
            line_index,
            line_byte_offset: offset,
        });
    }
    let expected = diff_edit(record.clone(), request.clone(), QueryContext::default()).await?;
    let output_bytes: usize = expected
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.text.len())
        .sum();
    let hunk_count = full.hunks.len();
    drop(full);
    let mut samples = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let record = record.clone();
        let request = request.clone();
        let start = Instant::now();
        let result = black_box(diff_edit(record, request, QueryContext::default()).await?);
        samples.push(elapsed(start));
        assert_eq!(result, expected);
    }
    Ok(
        json!({"source_bytes":source_bytes,"hunk_count":hunk_count,"output_bytes":output_bytes,"samples_ns":samples}),
    )
}

fn edits(size: usize, repetitions: usize) -> ProbeResult<Value> {
    let session = SessionId::new()?;
    let turn = TurnId::new()?;
    let mut journal = EditJournal::new(session.clone());
    for index in 0..size {
        journal.append(EditRecord {
            tool_call: ToolCallRef {
                turn_id: turn.clone(),
                tool_call_id: ToolCallId(format!("edit-{index}")),
            },
            path: format!("file-{index}"),
            before: FileSnapshot::Missing,
            after: FileSnapshot::Content(vec![1]),
            outcome: EditOutcome::Applied,
        })?;
    }
    let request = ChangesListRequest {
        scope: ChangeScope::Session,
        cursor: Some(ChangesCursor::Tool {
            session_id: session,
            turn_id: None,
            after_index: size - 2,
        }),
        limit: 1,
    };
    let expected = journal.list(request.clone())?;
    assert_eq!(expected.records.len(), 1);
    let mut samples = Vec::with_capacity(repetitions);
    for _ in 0..repetitions {
        let request = request.clone();
        let start = Instant::now();
        let result = black_box(journal.list(request)?);
        samples.push(elapsed(start));
        assert_eq!(result, expected);
    }
    Ok(json!({"record_count":size,"after_index":size-2,"returned_records":1,"samples_ns":samples}))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ProbeResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).ok_or("missing mode")?;
    let size: usize = args.get(2).ok_or("missing size")?.parse()?;
    let repetitions: usize = args.get(3).ok_or("missing repetitions")?.parse()?;
    let position = args.get(4).map_or("first", String::as_str);
    if size < 32 || repetitions == 0 || !["first", "middle", "last", "full"].contains(&position) {
        return Err("size must be >=32, repetitions positive, and page position valid".into());
    }
    let measured = match mode.as_str() {
        "metadata" => metadata(size, repetitions)?,
        "journal" => journal(size, repetitions)?,
        "diff-lines" | "diff-long" => diff(mode, size, repetitions, position).await?,
        "edits" => edits(size, repetitions)?,
        _ => return Err("unknown mode".into()),
    };
    println!(
        "{}",
        json!({"fixture":"task104_public_io_api","mode":mode,"size":size,"repetitions":repetitions,"position":position,"measurement":measured})
    );
    Ok(())
}
