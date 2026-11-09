mod support;
use minicore_host_local_tools::*;
use minicore_host_workspace::{EditOutcome, FileSnapshot, Workspace};
use minicore_runtime::{LoopOptions, LoopOutcome, ToolCallId, tools::ToolError};
use serde_json::json;
use std::{fs, sync::Arc};
use support::*;

#[tokio::test]
async fn real_runtime_reads_edits_and_writes_exact_captures_without_an_observer() {
    let (root, tools, records) = fixture().await;
    fs::write(root.path().join("unrelated"), "preexisting dirty data").unwrap();
    let model = ScriptedModel::calls(vec![
        (
            "write-1",
            "write",
            json!({"path":"new/deep.txt","content":"first\r\n中文\nlast\n"}),
        ),
        (
            "edit-1",
            "edit",
            json!({"path":"new/deep.txt","old_text":"中文","new_text":"第二行"}),
        ),
        (
            "read-1",
            "read",
            json!({"path":"new/deep.txt","offset":2,"limit":1}),
        ),
    ]);
    let runtime = start(
        model.clone(),
        &tools,
        &["read", "edit", "write"],
        LoopOptions::default_checked().unwrap(),
    );
    // No LoopEvent receiver or public observer is attached.
    let report = finish(runtime, &tools).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    let output = outputs(&report);
    assert_eq!(output[0]["status"], "applied");
    assert_eq!(output[1]["replacements"], 1);
    assert_eq!(output[2]["text"], "第二行\n");
    assert_eq!(output[2]["next_offset"], 3);
    assert_eq!(
        fs::read_to_string(root.path().join("new/deep.txt")).unwrap(),
        "first\r\n第二行\nlast\n"
    );
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0].0, "write-1");
    assert_eq!(edits[0].1.before, FileSnapshot::Missing);
    assert_eq!(
        edits[0].1.after,
        FileSnapshot::Content("first\r\n中文\nlast\n".as_bytes().to_vec())
    );
    assert_eq!(edits[1].1.before, edits[0].1.after);
    assert_eq!(edits[1].1.outcome, EditOutcome::Applied);
    assert!(edits.iter().all(|(_, edit)| edit.path == "new/deep.txt"));
    assert_eq!(model.requests.lock().unwrap()[0].tools().len(), 3);
}

#[tokio::test]
async fn explicit_read_range_has_no_old_line_or_byte_cap() {
    let (root, tools, _) = fixture().await;
    let long = format!("{}\n", "中".repeat(30_000));
    let text = long.clone() + &"line\n".repeat(2100);
    fs::write(root.path().join("large"), &text).unwrap();
    let result = invoke(
        &tools,
        "read",
        json!({"path":"large","offset":1,"limit":5000}),
    )
    .await
    .unwrap();
    assert_eq!(result["text"], text);
    assert_eq!(result["lines"], 2101);
    assert_eq!(result["eof"], true);
    assert_eq!(result["next_offset"], serde_json::Value::Null);
    tools.finish().await;
}

#[tokio::test]
async fn read_requires_range_and_rejects_non_utf8_without_lossy_success() {
    let (root, tools, _) = fixture().await;
    fs::write(root.path().join("binary"), [0xff, 0xfe]).unwrap();
    assert_eq!(
        invoke(&tools, "read", json!({"path":"binary"})).await,
        Err(ToolError::InvalidInvocation)
    );
    assert_eq!(
        invoke(&tools, "read", json!({"path":"binary","limit":0})).await,
        Err(ToolError::InvalidInvocation)
    );
    assert_eq!(
        invoke(&tools, "read", json!({"path":"binary","limit":1}))
            .await
            .unwrap()["status"],
        "failed"
    );
    tools.finish().await;
}

#[tokio::test]
async fn directory_listing_obeys_explicit_sorted_range() {
    let (root, tools, _) = fixture().await;
    for name in ["c", "a", "b"] {
        fs::write(root.path().join(name), "").unwrap();
    }
    let result = invoke(&tools, "read", json!({"path":".","offset":2,"limit":1}))
        .await
        .unwrap();
    assert_eq!(result["entries"], json!([{"name":"b","kind":"file"}]));
    assert_eq!(result["next_offset"], 3);
    tools.finish().await;
}

#[tokio::test]
async fn exact_edit_conflicts_are_not_success_and_do_not_normalize_newlines() {
    let (root, tools, records) = fixture().await;
    fs::write(root.path().join("file"), "x\r\nx\r\n").unwrap();
    for old in ["x", "x\nx\n", "absent"] {
        let output = invoke(
            &tools,
            "edit",
            json!({"path":"file","old_text":old,"new_text":"changed"}),
        )
        .await
        .unwrap();
        assert_eq!(output["status"], "failed");
        assert_eq!(output["error"]["code"], "conflict");
        assert_eq!(output["write_outcome"], "not_applied");
    }
    assert_eq!(fs::read(root.path().join("file")).unwrap(), b"x\r\nx\r\n");
    let output = invoke(
        &tools,
        "edit",
        json!({"path":"file","old_text":"x","new_text":"Y","replace_all":true}),
    )
    .await
    .unwrap();
    assert_eq!(output["replacements"], 2);
    {
        let edits = records.edits.lock().unwrap();
        assert_eq!(edits.len(), 4);
        assert!(
            edits[..3]
                .iter()
                .all(|(_, edit)| edit.outcome == EditOutcome::NotApplied
                    && edit.before == edit.after)
        );
    }
    tools.finish().await;
}

#[tokio::test]
async fn model_cannot_supply_owner_identity_and_unknown_tool_is_absent() {
    let (_, tools, _) = fixture().await;
    assert!(tools.tool("apply_patch").is_none());
    assert_eq!(
        invoke(
            &tools,
            "write",
            json!({"path":"file","content":"x","turn_id":"spoof"})
        )
        .await,
        Err(ToolError::InvalidInvocation)
    );
    assert_eq!(
        invoke(
            &tools,
            "bash",
            json!({"command":"true","session_id":"spoof"})
        )
        .await,
        Err(ToolError::InvalidInvocation)
    );
    tools.finish().await;
}

#[tokio::test]
async fn write_protects_directories_traversal_and_git_metadata() {
    let (root, tools, _) = fixture().await;
    fs::create_dir(root.path().join("directory")).unwrap();
    fs::write(root.path().join("directory/keep"), "kept").unwrap();
    for path in ["directory", "../escape", "/absolute", ".git/config", ""] {
        assert_eq!(
            invoke(&tools, "write", json!({"path":path,"content":"bad"}))
                .await
                .unwrap()["status"],
            "failed"
        );
    }
    assert_eq!(
        fs::read_to_string(root.path().join("directory/keep")).unwrap(),
        "kept"
    );
    tools.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_do_not_escape_reads_or_redirect_writes_and_permissions_survive() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (root, tools, _) = fixture().await;
    let outside = tempfile::TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink(outside.path(), root.path().join("outside")).unwrap();
    fs::write(root.path().join("target"), "old").unwrap();
    fs::set_permissions(
        root.path().join("target"),
        fs::Permissions::from_mode(0o754),
    )
    .unwrap();
    symlink("target", root.path().join("inside")).unwrap();
    assert_eq!(
        invoke(&tools, "read", json!({"path":"outside/secret","limit":1}))
            .await
            .unwrap()["status"],
        "failed"
    );
    assert_eq!(
        invoke(&tools, "read", json!({"path":"inside","limit":1}))
            .await
            .unwrap()["text"],
        "old"
    );
    for path in ["outside/new", "inside"] {
        assert_eq!(
            invoke(&tools, "write", json!({"path":path,"content":"bad"}))
                .await
                .unwrap()["status"],
            "failed"
        );
    }
    assert_eq!(
        invoke(&tools, "write", json!({"path":"target","content":"new"}))
            .await
            .unwrap()["status"],
        "applied"
    );
    assert_eq!(
        fs::metadata(root.path().join("target"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o754
    );
    assert!(!outside.path().join("new").exists());
    tools.finish().await;
}

struct RejectEdits;
impl LocalToolSink for RejectEdits {
    fn output(
        &self,
        _: &ToolCallId,
        _: OutputChannel,
        _: u64,
        _: &[u8],
    ) -> Result<(), RecordError> {
        Ok(())
    }
    fn edit(&self, _: &ToolCallId, _: CapturedEdit) -> Result<(), RecordError> {
        Err(RecordError)
    }
    fn command(&self, _: &ToolCallId, _: &CommandResult) -> Result<(), RecordError> {
        Ok(())
    }
}
#[tokio::test]
async fn failed_owner_record_is_internal_and_does_not_undo_or_retry_write() {
    let root = tempfile::TempDir::new().unwrap();
    let tools = LocalTools::new(
        Workspace::open(root.path()).await.unwrap(),
        Arc::new(RejectEdits),
    );
    assert_eq!(
        invoke(
            &tools,
            "write",
            json!({"path":"file","content":"applied once"})
        )
        .await,
        Err(ToolError::Internal)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        "applied once"
    );
    let finished = tools.finish().await;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].error, Some(ToolError::Internal));
    assert!(finished[0].cleanup_confirmed);
    assert_eq!(
        invoke(
            &tools,
            "write",
            json!({"path":"file","content":"not admitted"})
        )
        .await,
        Err(ToolError::Internal)
    );
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        "applied once"
    );
}

#[tokio::test]
async fn equal_upstream_call_ids_in_different_turn_bindings_remain_separate() {
    let (root, first, first_records) = fixture().await;
    let second_records = Arc::new(Recorder::default());
    let second = LocalTools::new(
        Workspace::open(root.path()).await.unwrap(),
        second_records.clone(),
    );
    invoke(
        &first,
        "write",
        json!({"path":"file","content":"first turn"}),
    )
    .await
    .unwrap();
    first.finish().await;
    invoke(
        &second,
        "write",
        json!({"path":"file","content":"second turn"}),
    )
    .await
    .unwrap();
    second.finish().await;
    let a = first_records.edits.lock().unwrap();
    let b = second_records.edits.lock().unwrap();
    assert_eq!(a[0].0, b[0].0);
    assert_eq!(a[0].1.after, b[0].1.before);
    assert_eq!(b[0].1.after, FileSnapshot::Content(b"second turn".to_vec()));
}
