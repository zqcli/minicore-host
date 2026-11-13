//! Real Runtime/file execution with explicit standalone record sinks. No live
//! provider, Session adapter, or TUI is impersonated by these fixtures.
mod support;

use minicore_host_local_tools::*;
use minicore_host_runtime::{
    ExecutionOptions, ToolCallId,
    tools::{ToolError, ToolInvocation},
};
use minicore_host_workspace::{EditOutcome, FileSnapshot, Workspace};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use support::*;

fn arguments(path: &str, patch: &str) -> Value {
    json!({"path":path,"patch":patch})
}

fn invocation(id: &str, patch: &str) -> ToolInvocation {
    ToolInvocation::new(
        id.parse().unwrap(),
        "apply_patch".parse().unwrap(),
        arguments("file", patch),
    )
    .unwrap()
}

#[tokio::test]
async fn real_runtime_applies_unified_and_codex_patches_with_exact_edit_records() {
    let (root, tools, records) = fixture().await;
    fs::write(root.path().join("file"), "first\nold\nlast\n").unwrap();
    fs::write(root.path().join("unrelated"), "preexisting changes").unwrap();
    let unified = "diff --git a/file b/file\nindex 1111111..2222222 100644\n--- a/file\n+++ b/file\n@@ -2 +2 @@ section name\n-old\n+中间\n";
    let codex = "*** Begin Patch\n*** Update File: file\n@@\n-中间\n+changed\n*** End Patch\n";
    for (id, patch) in [("unified", unified), ("codex", codex)] {
        let report = run_calls(
            &tools,
            vec![(id, "apply_patch", arguments("file", patch))],
            ExecutionOptions::default(),
        )
        .await;
        assert_eq!(report.stop, None);
        let output = outputs(&report);
        assert_eq!(output[0]["status"], "applied");
        assert_eq!(output[0]["path"], "file");
    }
    let completions = tools.finish().await;
    assert_eq!(completions.len(), 2);
    assert!(
        completions
            .iter()
            .all(|item| item.cleanup_confirmed && item.error.is_none())
    );
    assert_eq!(
        fs::read(root.path().join("file")).unwrap(),
        b"first\nchanged\nlast\n"
    );
    assert_eq!(
        fs::read(root.path().join("unrelated")).unwrap(),
        b"preexisting changes"
    );
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0].0, "unified");
    assert_eq!(
        edits[0].1.before,
        FileSnapshot::Content(b"first\nold\nlast\n".to_vec())
    );
    assert_eq!(
        edits[0].1.after,
        FileSnapshot::Content("first\n中间\nlast\n".as_bytes().to_vec())
    );
    assert_eq!(edits[1].0, "codex");
    assert_eq!(edits[1].1.before, edits[0].1.after);
    assert_eq!(
        edits[1].1.after,
        FileSnapshot::Content(b"first\nchanged\nlast\n".to_vec())
    );
    assert!(
        edits
            .iter()
            .all(|(_, edit)| edit.path == "file" && edit.outcome == EditOutcome::Applied)
    );
}

#[tokio::test]
async fn patch_schema_requires_business_arguments_and_rejects_owner_spoofing() {
    let (root, tools, records) = fixture().await;
    fs::write(root.path().join("file"), "old\n").unwrap();
    let spec = tools.tool("apply_patch").unwrap().spec().clone();
    assert_eq!(spec.input_schema()["required"], json!(["path", "patch"]));
    assert_eq!(spec.input_schema()["additionalProperties"], false);
    for input in [
        json!({"patch":"@@ -1 +1 @@\n-old\n+new\n"}),
        json!({"path":"file"}),
        json!({"path":"file","patch":""}),
        json!({"path":"file","patch":"bad\0patch"}),
        json!({"path":"file","patch":12}),
        json!({"path":"file","patch":"@@ -1 +1 @@\n-old\n+new\n","session_id":"spoof"}),
    ] {
        assert_eq!(
            invoke(&tools, "apply_patch", input).await,
            Err(ToolError::InvalidInvocation)
        );
    }
    assert_eq!(fs::read(root.path().join("file")).unwrap(), b"old\n");
    assert!(records.edits.lock().unwrap().is_empty());
    tools.finish().await;
}

#[tokio::test]
async fn unified_newline_markers_preserve_exact_whitespace_and_source_endings() {
    let (root, tools, records) = fixture().await;
    let cases = [
        (
            "  keep\r\n\told  \r\ntail\r\n",
            "@@ -2 +2 @@\n-\told  \n+\tnew  \n",
            "  keep\r\n\tnew  \r\ntail\r\n",
        ),
        (
            "one\r\ntwo\nthree\r\n",
            "@@ -2 +2 @@\n-two\n+TWO\n",
            "one\r\nTWO\r\nthree\r\n",
        ),
        (
            "keep\r\nold",
            "@@ -2 +2 @@\r\n-old\r\n\\ No newline at end of file\r\n+new\r\n",
            "keep\r\nnew\r\n",
        ),
        (
            "keep\r\nold\r\n",
            "@@ -2 +2 @@\n-old\n+new\n\\ No newline at end of file\n",
            "keep\r\nnew",
        ),
        (
            "keep\nlast",
            "@@ -2 +2 @@\n last\n\\ No newline at end of file\n",
            "keep\nlast",
        ),
        (
            "keep\n\nold\n",
            "@@ -1,3 +1,3 @@\n keep\n\n-old\n+new\n\n",
            "keep\n\nnew\n",
        ),
    ];
    for (source, patch, expected) in cases {
        fs::write(root.path().join("file"), source).unwrap();
        let result = invoke(&tools, "apply_patch", arguments("file", patch))
            .await
            .unwrap();
        assert_eq!(result["status"], "applied", "{result}");
        assert_eq!(
            fs::read(root.path().join("file")).unwrap(),
            expected.as_bytes()
        );
        let edits = records.edits.lock().unwrap();
        let edit = &edits.last().unwrap().1;
        assert_eq!(
            edit.before,
            FileSnapshot::Content(source.as_bytes().to_vec())
        );
        assert_eq!(
            edit.after,
            FileSnapshot::Content(expected.as_bytes().to_vec())
        );
    }
    tools.finish().await;
}

#[tokio::test]
async fn codex_anchors_ranges_blank_context_and_eof_keep_reference_semantics() {
    let (root, tools, _) = fixture().await;
    let cases = [
        (
            "header\r\nanchor\r\nold\r\n\r\nfinal",
            "@@ anchor\n anchor\n-old\n+new\n@@ -4,2 +4,2 @@\n \n-final\n+tail\n*** End of File\n",
            "header\r\nanchor\r\nnew\r\n\r\ntail",
        ),
        (
            "outer\ninner\nold\n",
            "@@ outer\n@@ inner\n-old\n+new\n",
            "outer\ninner\nnew\n",
        ),
        (
            "same\nnoise\nsame\nold\n",
            "@@\n same\n-old\n+new\n",
            "same\nnoise\nsame\nnew\n",
        ),
        ("old", "@@\n old\n+new\n*** End of File\n", "old\nnew"),
        ("anchor\n", "@@ anchor\n+new\n", "anchor\nnew\n"),
    ];
    for (source, body, expected) in cases {
        fs::write(root.path().join("file"), source).unwrap();
        let patch = format!("*** Begin Patch\n*** Update File: file\n{body}*** End Patch\n");
        let result = invoke(&tools, "apply_patch", arguments("file", &patch))
            .await
            .unwrap();
        assert_eq!(result["status"], "applied", "{result}");
        assert_eq!(
            fs::read(root.path().join("file")).unwrap(),
            expected.as_bytes()
        );
    }
    tools.finish().await;
}

#[tokio::test]
async fn invalid_shapes_and_ambiguous_matches_never_modify_any_file() {
    let (root, tools, records) = fixture().await;
    let source = "a\nb\nsame\nsame\n";
    fs::write(root.path().join("file"), source).unwrap();
    for patch in [
        "*** Begin Patch\n*** Add File: file\n+a\n*** End Patch\n",
        "*** Begin Patch\n*** Delete File: file\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n*** Move to: other\n@@\n-a\n+A\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@\n-a\n+A\n*** Update File: other\n@@\n-b\n+B\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: other\n@@\n-a\n+A\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@\n-same\n+changed\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@ same\n+new\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@ a\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@ a\n*** End of File\n*** End Patch\n",
        "*** Begin Patch\n*** Update File: file\n@@ -1 +1 @@\n-a\n+A\n@@ -1 +1 @@\n-a\n+A2\n*** End Patch\n",
        "--- a/file\n+++ b/other\n@@ -1 +1 @@\n-a\n+A\n",
        "--- /dev/null\n+++ b/file\n@@ -0,0 +1 @@\n+a\n",
        "--- a/file\n+++ /dev/null\n@@ -1 +0,0 @@\n-a\n",
        "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-a\n+A\n--- a/other\n+++ b/other\n@@ -1 +1 @@\n-b\n+B\n",
        "diff --git a/file b/file\nGIT binary patch\nliteral 4\n",
        "@@ -0 +1 @@\n-a\n+A\n",
        "@@ -1 +1 @@bad suffix\n-a\n+A\n",
        "@@ -x +1 @@\n-a\n+A\n",
        "@@ -184467440737095516160,0 +1,0 @@\n+x\n",
        "@@ -1,2 +1 @@\n-a\n+A\n",
        "@@ -2 +2 @@\n-b\n+B\n@@ -1 +1 @@\n-a\n+A\n",
        "@@ -1,2 +1,2 @@\n a\n b\n@@ -2 +2 @@\n-b\n+B\n",
        "@@ -1 +1 @@\n-a\n+A\ntrailing junk\n",
        "@@ -1 +1 @@\n-a\n\\ No newline at end of file\n\\ No newline at end of file\n+A\n",
    ] {
        assert_eq!(
            invoke(&tools, "apply_patch", arguments("file", patch)).await,
            Err(ToolError::InvalidInvocation),
            "{patch}"
        );
        assert_eq!(
            fs::read(root.path().join("file")).unwrap(),
            source.as_bytes()
        );
        assert!(!root.path().join("other").exists());
    }
    tools.finish().await;
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 25);
    assert!(
        edits
            .iter()
            .all(|(_, edit)| edit.outcome == EditOutcome::NotApplied && edit.before == edit.after)
    );
}

#[tokio::test]
async fn exact_position_whitespace_and_later_hunk_conflicts_do_not_partially_commit() {
    let (root, tools, records) = fixture().await;
    let source = "a\nb\ntarget\n";
    fs::write(root.path().join("file"), source).unwrap();
    for patch in [
        "@@ -1 +1 @@\n-target\n+changed\n",
        "@@ -1 +1 @@\n-a \n+A\n",
        "@@ -1 +1 @@\n-a\n+A\n@@ -2 +2 @@\n-absent\n+B\n",
        "@@ -1 +2 @@\n-a\n+A\n",
        "@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+A\n",
        "*** Begin Patch\n*** Update File: file\n@@ target\n-target\n+changed\n@@ a\n-a\n+A\n*** End Patch\n",
    ] {
        let result = invoke(&tools, "apply_patch", arguments("file", patch))
            .await
            .unwrap();
        assert_eq!(result["status"], "failed", "{result}");
        assert_eq!(result["write_outcome"], "not_applied");
        assert_eq!(result["error"]["code"], "conflict");
        assert_eq!(
            fs::read(root.path().join("file")).unwrap(),
            source.as_bytes()
        );
    }
    tools.finish().await;
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 6);
    assert!(
        edits
            .iter()
            .all(|(_, edit)| edit.outcome == EditOutcome::NotApplied && edit.before == edit.after)
    );
}

#[tokio::test]
async fn patch_source_and_result_have_no_reference_byte_quota() {
    let (root, tools, _) = fixture().await;
    let source = "x".repeat(600_000) + "\n";
    let replacement = "y".repeat(700_000) + "\n";
    fs::write(root.path().join("file"), &source).unwrap();
    let patch = format!("@@ -1 +1 @@\n-{source}+{replacement}");
    let result = invoke(&tools, "apply_patch", arguments("file", &patch))
        .await
        .unwrap();
    assert_eq!(result["status"], "applied");
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        replacement
    );
    tools.finish().await;
}

#[tokio::test]
async fn many_codex_anchors_have_no_reference_matching_work_quota() {
    let (root, tools, _) = fixture().await;
    let mut source = String::new();
    let mut patch = String::from("*** Begin Patch\n*** Update File: file\n");
    // The old reference stopped after 512 KiB * 8 matching visits. These
    // ordered header-only anchors perform more work, without any ambiguity.
    for index in 0..3100 {
        source.push_str(&format!("anchor{index:04}\n"));
        patch.push_str(&format!("@@ anchor{index:04}\n"));
    }
    source.push_str("old\n");
    patch.push_str("-old\n+new\n*** End Patch\n");
    fs::write(root.path().join("file"), &source).unwrap();
    let result = invoke(&tools, "apply_patch", arguments("file", &patch))
        .await
        .unwrap();
    assert_eq!(result["status"], "applied");
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        source.replace("old\n", "new\n")
    );
    tools.finish().await;
}

#[tokio::test]
async fn zero_start_insertions_and_multiple_hunks_keep_declared_order() {
    let (root, tools, _) = fixture().await;
    fs::write(root.path().join("file"), "tail\n").unwrap();
    let result = invoke(
        &tools,
        "apply_patch",
        arguments("file", "@@ -0,0 +1 @@\n+head\n"),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "applied");
    assert_eq!(fs::read(root.path().join("file")).unwrap(), b"head\ntail\n");
    let mut source = String::new();
    let mut patch = String::new();
    let mut expected = String::new();
    for index in 0..3000 {
        source.push_str(&format!("v{index:04}\n"));
        expected.push_str(&format!("V{index:04}\n"));
        let line = index + 1;
        patch.push_str(&format!(
            "@@ -{line} +{line} @@\n-v{index:04}\n+V{index:04}\n"
        ));
    }
    fs::write(root.path().join("file"), source).unwrap();
    let result = invoke(&tools, "apply_patch", arguments("file", &patch))
        .await
        .unwrap();
    assert_eq!(result["status"], "applied");
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        expected
    );
    tools.finish().await;
}

#[tokio::test]
async fn patch_requires_an_existing_text_file_and_rejects_unsafe_targets() {
    let (root, tools, _) = fixture().await;
    fs::create_dir(root.path().join("directory")).unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::write(root.path().join(".git/config"), "old\n").unwrap();
    fs::write(root.path().join("binary"), [0xff, 0xfe]).unwrap();
    fs::write(root.path().join("embedded-nul.bin"), b"old\0\n").unwrap();
    let patch = "@@ -1 +1 @@\n-old\n+new\n";
    for path in [
        "missing",
        "directory",
        "binary",
        "embedded-nul.bin",
        "../escape",
        "/absolute",
        ".git/config",
        "",
    ] {
        let result = invoke(&tools, "apply_patch", arguments(path, patch))
            .await
            .unwrap();
        assert_eq!(result["status"], "failed", "{path}: {result}");
    }
    assert!(!root.path().join("missing").exists());
    assert_eq!(fs::read(root.path().join("binary")).unwrap(), [0xff, 0xfe]);
    assert_eq!(
        fs::read(root.path().join("embedded-nul.bin")).unwrap(),
        b"old\0\n"
    );
    assert_eq!(fs::read(root.path().join(".git/config")).unwrap(), b"old\n");
    tools.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn quoted_unicode_paths_decode_exactly_and_preserve_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let (root, tools, _) = fixture().await;
    for (path, old_header, new_header) in [
        ("tab\tname", r#""a/tab\tname""#, r#""b/tab\tname""#),
        ("quote\"name", r#""a/quote\"name""#, r#""b/quote\"name""#),
        ("slash\\name", r#""a/slash\\name""#, r#""b/slash\\name""#),
        ("é.txt", r#""a/\303\251.txt""#, r#""b/\303\251.txt""#),
    ] {
        let target = root.path().join(path);
        fs::write(&target, "old\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o751)).unwrap();
        let patch =
            format!("--- {old_header}\tbefore\n+++ {new_header}\tafter\n@@ -1 +1 @@\n-old\n+new\n");
        let result = invoke(&tools, "apply_patch", arguments(path, &patch))
            .await
            .unwrap();
        assert_eq!(result["status"], "applied");
        assert_eq!(fs::read(&target).unwrap(), b"new\n");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }
    tools.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn patch_cannot_follow_final_or_parent_symlinks() {
    use std::os::unix::fs::symlink;
    let (root, tools, records) = fixture().await;
    let outside = tempfile::TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), "old\n").unwrap();
    fs::write(root.path().join("real"), "old\n").unwrap();
    symlink("real", root.path().join("link")).unwrap();
    symlink(outside.path(), root.path().join("outside")).unwrap();
    for path in ["link", "outside/secret"] {
        let result = invoke(
            &tools,
            "apply_patch",
            arguments(path, "@@ -1 +1 @@\n-old\n+new\n"),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "failed");
    }
    assert_eq!(fs::read(root.path().join("real")).unwrap(), b"old\n");
    assert_eq!(fs::read(outside.path().join("secret")).unwrap(), b"old\n");
    assert!(records.edits.lock().unwrap().is_empty());
    tools.finish().await;
}

#[tokio::test]
async fn cancellation_and_deadline_while_waiting_for_the_real_mutation_guard_do_not_write() {
    for timed_out in [false, true] {
        let root = tempfile::TempDir::new().unwrap();
        fs::write(root.path().join("file"), "old\n").unwrap();
        let workspace = Workspace::open(root.path()).await.unwrap();
        let records = Arc::new(Recorder::default());
        let tools = LocalTools::new(
            workspace.clone(),
            records.clone(),
            Arc::new(InheritCommandEnvironment),
        );
        let gate = workspace.begin_mutation().await;
        let tool = tools.tool("apply_patch").unwrap();
        let mut call_context = context();
        if timed_out {
            call_context.deadline = Instant::now() + Duration::from_millis(25);
        }
        let cancellation = call_context.cancellation.clone();
        let mut call = Box::pin(tool.execute(
            invocation("guarded", "@@ -1 +1 @@\n-old\n+new\n"),
            call_context,
        ));
        assert!(futures_util::poll!(call.as_mut()).is_pending());
        if !timed_out {
            cancellation.cancel();
        }
        assert_eq!(
            call.await.unwrap_err(),
            if timed_out {
                ToolError::TimedOut
            } else {
                ToolError::Cancelled
            }
        );
        let completed = tools.finish().await;
        assert_eq!(completed.len(), 1);
        assert!(completed[0].cleanup_confirmed);
        drop(gate);
        assert_eq!(fs::read(root.path().join("file")).unwrap(), b"old\n");
        assert!(records.edits.lock().unwrap().is_empty());
        assert_eq!(
            *records.completions.lock().unwrap(),
            vec![("guarded".into(), true)]
        );
    }
}

#[tokio::test]
async fn dropping_the_caller_wait_does_not_detach_accepted_patch_work() {
    let root = tempfile::TempDir::new().unwrap();
    fs::write(root.path().join("file"), "old\n").unwrap();
    let workspace = Workspace::open(root.path()).await.unwrap();
    let records = Arc::new(Recorder::default());
    let tools = LocalTools::new(
        workspace.clone(),
        records.clone(),
        Arc::new(InheritCommandEnvironment),
    );
    let gate = workspace.begin_mutation().await;
    let tool = tools.tool("apply_patch").unwrap();
    let mut call = Box::pin(tool.execute(
        invocation("retained", "@@ -1 +1 @@\n-old\n+new\n"),
        context(),
    ));
    assert!(futures_util::poll!(call.as_mut()).is_pending());
    drop(call);
    let mut finished = Box::pin(tools.finish());
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut finished)
            .await
            .is_err()
    );
    assert!(records.edits.lock().unwrap().is_empty());
    drop(gate);
    let completions = finished.await;
    assert_eq!(completions.len(), 1);
    assert!(completions[0].cleanup_confirmed);
    assert_eq!(completions[0].error, None);
    assert_eq!(fs::read(root.path().join("file")).unwrap(), b"new\n");
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].1.before, FileSnapshot::Content(b"old\n".to_vec()));
    assert_eq!(edits[0].1.after, FileSnapshot::Content(b"new\n".to_vec()));
}

#[tokio::test]
async fn patch_and_edit_in_separate_turn_bindings_share_one_mutation_boundary() {
    let root = tempfile::TempDir::new().unwrap();
    fs::write(root.path().join("file"), "left_old\nright_old\n").unwrap();
    let workspace = Workspace::open(root.path()).await.unwrap();
    let patch_records = Arc::new(Recorder::default());
    let edit_records = Arc::new(Recorder::default());
    let patch_tools = LocalTools::new(
        workspace.clone(),
        patch_records.clone(),
        Arc::new(InheritCommandEnvironment),
    );
    let edit_tools = LocalTools::new(
        workspace.clone(),
        edit_records.clone(),
        Arc::new(InheritCommandEnvironment),
    );
    let gate = workspace.begin_mutation().await;
    let mut patch = Box::pin(invoke(
        &patch_tools,
        "apply_patch",
        arguments("file", "@@ -1 +1 @@\n-left_old\n+left_new\n"),
    ));
    let mut edit = Box::pin(invoke(
        &edit_tools,
        "edit",
        json!({"path":"file","old_text":"right_old","new_text":"right_new"}),
    ));
    assert!(futures_util::poll!(patch.as_mut()).is_pending());
    assert!(futures_util::poll!(edit.as_mut()).is_pending());
    assert!(patch_records.edits.lock().unwrap().is_empty());
    assert!(edit_records.edits.lock().unwrap().is_empty());
    drop(gate);
    let (patch, edit) = tokio::join!(patch, edit);
    assert_eq!(patch.unwrap()["status"], "applied");
    assert_eq!(edit.unwrap()["status"], "applied");
    tokio::join!(patch_tools.finish(), edit_tools.finish());
    let patch_records = patch_records.edits.lock().unwrap();
    let edit_records = edit_records.edits.lock().unwrap();
    let patch = &patch_records[0].1;
    let edit = &edit_records[0].1;
    let initial = FileSnapshot::Content(b"left_old\nright_old\n".to_vec());
    let (first, second) = if patch.before == initial {
        (patch, edit)
    } else {
        (edit, patch)
    };
    assert_eq!(first.before, initial);
    assert_eq!(first.after, second.before);
    assert_eq!(
        second.after,
        FileSnapshot::Content(b"left_new\nright_new\n".to_vec())
    );
    assert_eq!(
        fs::read(root.path().join("file")).unwrap(),
        b"left_new\nright_new\n"
    );
}

#[derive(Default)]
struct RejectPatchRecord(AtomicUsize);
impl LocalToolSink for RejectPatchRecord {
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
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(RecordError)
    }
    fn command(&self, _: &ToolCallId, _: &CommandResult) -> Result<(), RecordError> {
        Ok(())
    }
}

#[tokio::test]
async fn owner_record_failure_is_internal_after_one_real_patch_without_rollback() {
    let root = tempfile::TempDir::new().unwrap();
    fs::write(root.path().join("file"), "old\n").unwrap();
    let records = Arc::new(RejectPatchRecord::default());
    let tools = LocalTools::new(
        Workspace::open(root.path()).await.unwrap(),
        records.clone(),
        Arc::new(InheritCommandEnvironment),
    );
    let result = invoke(
        &tools,
        "apply_patch",
        arguments("file", "@@ -1 +1 @@\n-old\n+new\n"),
    )
    .await;
    assert_eq!(result, Err(ToolError::Internal));
    assert_eq!(fs::read(root.path().join("file")).unwrap(), b"new\n");
    assert_eq!(records.0.load(Ordering::SeqCst), 1);
    let completions = tools.finish().await;
    assert_eq!(completions.len(), 1);
    assert!(completions[0].cleanup_confirmed);
    assert_eq!(completions[0].error, Some(ToolError::Internal));
}
