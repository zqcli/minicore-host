mod support;
use minicore_host_local_tools::*;
use minicore_host_runtime::{ExecutionOptions, ToolCallId, tools::ToolError};
use minicore_host_workspace::{EditOutcome, FileSnapshot, Workspace};
use serde_json::json;
use std::{fs, sync::Arc, time::Duration};
use support::*;

#[tokio::test]
async fn real_runtime_reads_edits_and_writes_exact_captures_without_an_observer() {
    let (root, tools, records) = fixture().await;
    fs::write(root.path().join("unrelated"), "preexisting dirty data").unwrap();
    let calls = vec![
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
    ];
    // No public observer is attached to the actual ordered tool batch.
    let report = run_calls(&tools, calls, ExecutionOptions::default()).await;
    tools.finish().await;
    assert_eq!(report.stop, None);
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
    assert!(tools.tool("unknown-local-tool").is_none());
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

#[cfg(any(windows, target_os = "macos"))]
#[tokio::test]
async fn git_metadata_case_aliases_are_not_writable() {
    let (root, tools, _) = fixture().await;
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::write(root.path().join(".git/config"), "retained").unwrap();
    for path in [".GIT/config", ".Git/new"] {
        let result = invoke(&tools, "write", json!({"path":path,"content":"bad"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "failed");
    }
    assert_eq!(
        fs::read(root.path().join(".git/config")).unwrap(),
        b"retained"
    );
    assert!(!root.path().join(".git/new").exists());
    tools.finish().await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_aliases_and_junctions_cannot_redirect_file_operations() {
    let (root, tools, _) = fixture().await;
    let outside = tempfile::TempDir::new().unwrap();
    fs::write(root.path().join("file"), "retained").unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    let junction = root.path().join("outside");
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    for path in ["file:stream", "file.", "file ", "outside/new"] {
        let result = invoke(&tools, "write", json!({"path":path,"content":"bad"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "failed", "{path}: {result}");
    }
    for path in ["file:stream", "file.", "file ", "outside/secret"] {
        let result = invoke(&tools, "read", json!({"path":path,"limit":1}))
            .await
            .unwrap();
        assert_eq!(result["status"], "failed", "{path}: {result}");
    }
    assert_eq!(fs::read(root.path().join("file")).unwrap(), b"retained");
    assert_eq!(fs::read(outside.path().join("secret")).unwrap(), b"secret");
    assert!(!outside.path().join("new").exists());
    assert!(!root.path().join("file:stream").exists());
    fs::remove_dir(junction).unwrap();
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
        Arc::new(InheritCommandEnvironment),
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
        Arc::new(InheritCommandEnvironment),
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

#[tokio::test]
async fn separate_turn_tools_share_mutation_order_and_disjoint_edit_snapshots() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::open(root.path()).await.unwrap();
    let original = "LEFT_OLD RIGHT_OLD";
    let expected = "LEFT_NEW RIGHT_NEW";
    fs::write(root.path().join("file"), original).unwrap();
    let first_records = Arc::new(Recorder::default());
    let second_records = Arc::new(Recorder::default());
    let first = LocalTools::new(
        workspace.clone(),
        first_records.clone(),
        Arc::new(InheritCommandEnvironment),
    );
    let second = LocalTools::new(
        workspace.clone(),
        second_records.clone(),
        Arc::new(InheritCommandEnvironment),
    );

    // An explicit operation gate, not a large-file timing race: both accepted
    // workers must wait before capturing any source through either Turn binding.
    let gate = workspace.begin_mutation().await;
    let mut left = Box::pin(invoke(
        &first,
        "edit",
        json!({"path":"file","old_text":"LEFT_OLD","new_text":"LEFT_NEW"}),
    ));
    let mut right = Box::pin(invoke(
        &second,
        "edit",
        json!({"path":"file","old_text":"RIGHT_OLD","new_text":"RIGHT_NEW"}),
    ));
    assert!(futures_util::poll!(left.as_mut()).is_pending());
    assert!(futures_util::poll!(right.as_mut()).is_pending());
    let mut finished = Box::pin(async { tokio::join!(first.finish(), second.finish()) });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut finished)
            .await
            .is_err(),
        "shared Workspace mutations must remain pending while the gate is held"
    );
    assert_eq!(
        fs::read(root.path().join("file")).unwrap(),
        original.as_bytes()
    );
    assert!(first_records.edits.lock().unwrap().is_empty());
    assert!(second_records.edits.lock().unwrap().is_empty());

    drop(gate);
    let (left, right) = tokio::join!(left, right);
    assert_eq!(left.unwrap()["status"], "applied");
    assert_eq!(right.unwrap()["status"], "applied");
    let (first_finished, second_finished) = finished.await;
    for completions in [first_finished, second_finished] {
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].error, None);
        assert!(completions[0].cleanup_confirmed);
    }
    assert_eq!(
        fs::read(root.path().join("file")).unwrap(),
        expected.as_bytes()
    );

    let first_edits = first_records.edits.lock().unwrap();
    let second_edits = second_records.edits.lock().unwrap();
    assert_eq!(first_edits.len(), 1);
    assert_eq!(second_edits.len(), 1);
    let left = &first_edits[0].1;
    let right = &second_edits[0].1;
    for (edit, old, new) in [
        (left, "LEFT_OLD", "LEFT_NEW"),
        (right, "RIGHT_OLD", "RIGHT_NEW"),
    ] {
        assert_eq!(edit.outcome, EditOutcome::Applied);
        let FileSnapshot::Content(before) = &edit.before else {
            panic!("the existing file must have an actual before capture");
        };
        assert_eq!(
            edit.after,
            FileSnapshot::Content(
                String::from_utf8(before.clone())
                    .unwrap()
                    .replace(old, new)
                    .into_bytes()
            )
        );
    }
    let initial = FileSnapshot::Content(original.as_bytes().to_vec());
    let (earlier, later) = if left.before == initial {
        (left, right)
    } else {
        (right, left)
    };
    assert_eq!(earlier.before, initial);
    assert_eq!(earlier.after, later.before);
    assert_eq!(
        later.after,
        FileSnapshot::Content(expected.as_bytes().to_vec())
    );
}

#[tokio::test]
async fn rejected_before_worker_start_reports_actual_cleanup_without_waiting_for_turn_end() {
    let (_root, tools, records) = fixture().await;
    for (id, arguments, cancelled) in [
        (
            "invalid",
            serde_json::json!({"missing":"read fields"}),
            false,
        ),
        (
            "cancelled",
            serde_json::json!({"path":"file", "limit":1}),
            true,
        ),
    ] {
        let context = context();
        if cancelled {
            context.cancellation.cancel();
        }
        let result = tools
            .tool("read")
            .unwrap()
            .execute(
                minicore_host_runtime::tools::ToolInvocation::new(
                    id.parse().unwrap(),
                    "read".parse().unwrap(),
                    arguments,
                )
                .unwrap(),
                context,
            )
            .await;
        assert_eq!(
            result.unwrap_err(),
            if cancelled {
                ToolError::Cancelled
            } else {
                ToolError::InvalidInvocation
            }
        );
        assert_eq!(
            records.completions.lock().unwrap().last(),
            Some(&(id.to_owned(), true))
        );
    }
    assert!(
        tools.finish().await.is_empty(),
        "neither rejected invocation started a worker"
    );
    assert_eq!(records.completions.lock().unwrap().len(), 2);
}
