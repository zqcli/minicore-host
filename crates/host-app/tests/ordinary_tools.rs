//! Actual minicore-host binary -> stdio/WSS -> owners -> production HTTP adapter
//! -> pinned Runtime -> ordinary tools. Only loopback model replies are scripted.
#![cfg(unix)]
mod support;
mod tool_support;

use minicore_host_core::{ActivityEvent, ObservationEvent};
use minicore_host_protocol::*;
use serde_json::{Value, json};
use std::fs;
use support::{Process, message, options, shutdown};
use tool_support::*;

#[tokio::test]
async fn binary_tools_produce_exact_files_native_results_and_canonical_sources() {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["read", "write", "edit", "bash"]).await;
    let workspace = tempfile::tempdir().unwrap();
    repository(workspace.path());
    let target = app
        .create(workspace.path(), &["read", "write", "edit", "bash"])
        .await;
    let mut events = subscribe(&app.observer, &target).await;
    app.process
        .rpc(
            "message.send",
            message(&target, "exercise actual ordinary tools"),
        )
        .await;
    let first = provider.request().await;
    first.declarations(&["read", "write", "edit", "bash"]);
    let turn = only_turn(&mut app.process, &target).await;
    first.answer(tool(
        "write",
        "write",
        json!({"path":"nested/file","content":"old\r\nsecond🙂\n"}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("write")["status"], "applied");
    assert_eq!(
        fs::read(workspace.path().join("nested/file")).unwrap(),
        b"old\r\nsecond\xf0\x9f\x99\x82\n"
    );
    next.answer(tool(
        "read",
        "read",
        json!({"path":"nested/file","offset":2,"limit":1}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("read")["text"], "second🙂\n");
    next.answer(tool(
        "edit",
        "edit",
        json!({"path":"nested/file","old_text":"old","new_text":"实际"}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("edit")["replacements"], 1);
    let expected = "实际\r\nsecond🙂\n";
    assert_eq!(
        fs::read(workspace.path().join("nested/file")).unwrap(),
        expected.as_bytes()
    );
    next.answer(tool(
        "rejected-edit",
        "edit",
        json!({"path":"nested/file","old_text":"absent","new_text":"not applied"}),
    ));
    let next = provider.request().await;
    assert_eq!(next.tool_result("rejected-edit")["status"], "failed");
    assert_eq!(
        next.tool_result("rejected-edit")["write_outcome"],
        "not_applied"
    );
    assert_eq!(
        fs::read(workspace.path().join("nested/file")).unwrap(),
        expected.as_bytes()
    );
    next.answer(tool(
        "bash",
        "bash",
        json!({"command":"cat nested/file; printf '诊断' >&2; printf 'shell bytes' > shell-file"}),
    ));
    let last = provider.request().await;
    last.declarations(&["read", "write", "edit", "bash"]);
    let native = last.tool_result("bash");
    assert_eq!(native["status"], "exited");
    assert_eq!(native["exit_code"], 0);
    assert_eq!(native["process_waited"], true);
    assert_eq!(native["cleanup_confirmed"], true);
    assert_eq!(native["cleanup_errors"], json!([]));
    assert_eq!(native["file_changes"], "unknown");
    assert_eq!(
        native["stdout"]["output"],
        json!({"encoding":"utf8","data":expected})
    );
    assert_eq!(
        native["stderr"]["output"],
        json!({"encoding":"utf8","data":"诊断"})
    );
    assert_eq!(
        fs::read(workspace.path().join("shell-file")).unwrap(),
        b"shell bytes"
    );

    let mut stdout = String::new();
    let mut stderr = String::new();
    let (source, finished) = loop {
        match event(&mut events).await {
            ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) => {
                assert_eq!(source.tool_call_id.0, "bash");
                let output = match data.channel.as_str() {
                    "stdout" => &mut stdout,
                    "stderr" => &mut stderr,
                    other => panic!("unexpected channel {other}"),
                };
                assert_eq!(data.start, output.len() as u64);
                output.push_str(&data.text);
                assert_eq!(data.end, output.len() as u64);
            }
            ObservationEvent::Activity(ActivityEvent::ToolFinished { source, data })
                if source.tool_call_id.0 == "bash" =>
            {
                break (source, data);
            }
            _ => {}
        }
    };
    assert_eq!(stdout, expected);
    assert_eq!(stderr, "诊断");
    assert_eq!(json!(source.turn.turn_id), turn);
    assert_eq!(json!(source.turn.session.host_id), app.initial["host_id"]);
    assert_eq!(
        json!(source.turn.session.host_incarnation),
        app.initial["host_incarnation"]
    );
    assert_eq!(json!(source.turn.session.agent_id), target["agent_id"]);
    assert_eq!(json!(source.turn.session.session_id), target["session_id"]);
    let tool_ref = json!({"turn_id":turn,"tool_call_id":"bash"});
    let call = app
        .process
        .rpc("toolcall.get", json!({"target":target,"tool":tool_ref}))
        .await;
    assert_eq!(
        call["data"]["state"], "finished",
        "tool finishes while next model request is gated"
    );
    assert_eq!(call["data"]["result"], json!(finished.result));
    assert_eq!(call["load_id"], json!(source.turn.load_id));
    for (channel, text) in [("stdout", expected), ("stderr", "诊断")] {
        assert_eq!(
            finished
                .output_positions
                .iter()
                .find(|p| p.channel == channel)
                .unwrap()
                .end,
            text.len() as u64
        );
        let read = app.process.rpc("toolcall.output.read", json!({"target":target,"tool":tool_ref,"request":{"channel":channel,"start":0,"end":text.len(),"max_bytes":4096}})).await;
        assert_eq!(read["data"]["text"], text);
        assert_eq!(read["turn_id"], turn);
        assert_eq!(read["tool_call_id"], "bash");
        assert_eq!(read["host_incarnation"], app.initial["host_incarnation"]);
    }

    let edits = app
        .process
        .rpc(
            "changes.list",
            json!({"target":target,"request":list("session")}),
        )
        .await;
    assert!(edits["config_revision"].is_null());
    let records = edits["data"]["records"].as_array().unwrap();
    assert_eq!(
        records.len(),
        3,
        "read and shell mutations must not be attributed as write/edit"
    );
    for (record, id) in records.iter().zip(["write", "edit", "rejected-edit"]) {
        assert_eq!(record["origin"], "tool");
        assert_eq!(record["path"], "nested/file");
        assert_eq!(record["change_ref"]["session_id"], target["session_id"]);
        assert_eq!(
            record["change_ref"]["tool_call"],
            json!({"turn_id":turn,"tool_call_id":id})
        );
    }
    assert_eq!(records[2]["edit_outcome"], "not_applied");
    assert_eq!(records[2]["kind"], "unchanged");
    let edit = app
        .process
        .rpc(
            "changes.diff",
            json!({"target":target,"request":diff(&records[1]["change_ref"], "tool_before_after")}),
        )
        .await;
    assert_eq!(lines(&edit, "removed"), "old\r\n");
    assert_eq!(lines(&edit, "added"), "实际\r\n");
    assert_eq!(edit["data"]["complete"], true);
    assert_eq!(edit["data"]["stale"], false);
    let files = app.process.rpc("workspace.files", json!({"target":target,"request":{"directory":"","recursive":true,"query":null,"cursor":null,"limit":100}})).await;
    assert!(
        files["data"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "nested/file")
    );
    let read_back = app
        .process
        .rpc(
            "workspace.read",
            json!({"target":target,"request":read("nested/file")}),
        )
        .await;
    assert_eq!(read_back["data"]["text"], expected);
    let search = app.process.rpc("workspace.search", json!({"target":target,"request":{"path":"nested","query":"实际","case_sensitive":true,"max_matches":10,"cursor":null}})).await;
    assert_eq!(search["data"]["matches"][0]["path"], "nested/file");
    assert_eq!(search["data"]["matches"][0]["line_number"], 1);
    let status = app
        .process
        .rpc("workspace.status", json!({"target":target}))
        .await;
    assert_eq!(status["data"]["repo_available"], true);
    let git = app
        .process
        .rpc(
            "changes.list",
            json!({"target":target,"request":list("workspace")}),
        )
        .await;
    assert_eq!(git["config_revision"], 0);
    assert!(
        git["data"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|record| record["origin"] == "workspace_unknown")
    );
    let shell = git["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["path"] == "shell-file")
        .unwrap();
    let shell_diff = app
        .process
        .rpc(
            "changes.diff",
            json!({"target":target,"request":diff(&shell["change_ref"], "head_to_worktree")}),
        )
        .await;
    assert_eq!(lines(&shell_diff, "added"), "shell bytes");

    last.answer(answer("real tool sequence complete"));
    let result = app
        .process
        .rpc("turn.wait", json!({"target":target,"turn_id":turn}))
        .await;
    assert_eq!(result["data"]["info"]["outcome"], "completed");
    assert_eq!(result["data"]["info"]["persistence"], "persisted");
    assert_eq!(result["data"]["info"]["cleanup_faults"], json!([]));
    let before = history(&mut app.process, &target).await;
    for id in ["write", "read", "edit", "rejected-edit", "bash"] {
        assert_eq!(history_result(&before, id)["outcome"], "success");
    }
    assert_eq!(
        serde_json::from_str::<Value>(
            history_result(&before, "bash")["output"]["content"]
                .as_str()
                .unwrap()
        )
        .unwrap(),
        native
    );
    app.process.send("host.shutdown", shutdown("graceful"));
    let stopped = app.process.finish().await;
    assert!(stopped.status.success());
    stopped.protocol_only();
    stopped.safe_diagnostics();
    app.observer.close();
    let saved = fs::read(history_path(&app.root, &target)).unwrap();
    let mut reopened = Process::start(&app.root, None, true, Some((&app.certificate, &app.key)));
    let initial = reopened.rpc("initialize", json!({})).await;
    assert_eq!(initial["host_id"], app.initial["host_id"]);
    assert_ne!(initial["host_incarnation"], app.initial["host_incarnation"]);
    assert_eq!(history(&mut reopened, &target).await, before);
    assert_eq!(fs::read(history_path(&app.root, &target)).unwrap(), saved);
    let observer = connect(&mut reopened, app.tls).await;
    let session = observer.session(serde_json::from_value(target.clone()).unwrap());
    assert_eq!(
        fault(
            session
                .changes_list(serde_json::from_value(list("session")).unwrap(), options())
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::SessionChangeSourceUnavailable)
    );
    assert_eq!(
        fault(
            session
                .changes_diff(
                    serde_json::from_value(diff(&records[1]["change_ref"], "tool_before_after"))
                        .unwrap(),
                    options()
                )
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::SessionChangeNotFound)
    );
    reopened.send("host.shutdown", shutdown("graceful"));
    assert!(reopened.finish().await.status.success());
    observer.close();
}

#[tokio::test]
async fn binary_turns_keep_frozen_shared_roots_and_changes_reject_wrong_sources() {
    let mut old_provider = Provider::start().await;
    let mut new_provider = Provider::start().await;
    let mut app = App::start(&old_provider, &["edit", "read"]).await;
    let original = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    repository(original.path());
    repository(other.path());
    let initial_bytes = "left=old right=old\n";
    fs::write(original.path().join("file"), initial_bytes).unwrap();
    fs::write(other.path().join("file"), "different root\n").unwrap();
    let alias = app.directory.path().join("workspace-alias");
    std::os::unix::fs::symlink(original.path(), &alias).unwrap();
    let target = app.create(original.path(), &["edit", "read"]).await;
    let wrong = app.create(other.path(), &["read"]).await;
    app.process
        .rpc("message.send", message(&target, "old binding"))
        .await;
    let old_request = old_provider.request().await;
    old_request.declarations(&["edit", "read"]);
    let old_turn = only_turn(&mut app.process, &target).await;
    let listed = app
        .process
        .rpc(
            "changes.list",
            json!({"target":target,"request":list("workspace")}),
        )
        .await;
    let reference = listed["data"]["records"][0]["change_ref"].clone();
    let wrong_session = app.observer.session(serde_json::from_value(wrong).unwrap());
    assert_eq!(
        fault(
            wrong_session
                .changes_diff(
                    serde_json::from_value(diff(&reference, "head_to_worktree")).unwrap(),
                    options()
                )
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::SessionChangeNotFound)
    );

    let update = app.process.rpc("session.update", json!({"target":target,"operation_id":OperationId::new().unwrap(),"expected_config_revision":0,"changes":{"config":{"workspace":{"path":other.path()},"tools":bindings(&["read"])}}})).await;
    assert!(matches!(
        update["result"]["status"].as_str(),
        Some("saving" | "committed")
    ));
    tokio::time::timeout(support::WAIT, async {
        loop {
            let current = app
                .process
                .rpc("session.get", json!({"target":target}))
                .await;
            if current["config_revision"] == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let current = app
        .process
        .rpc(
            "workspace.read",
            json!({"target":target,"request":read("file")}),
        )
        .await;
    assert_eq!(current["config_revision"], 1);
    assert_eq!(current["data"]["text"], "different root\n");
    let session = app
        .observer
        .session(serde_json::from_value(target.clone()).unwrap());
    assert_eq!(
        fault(
            session
                .changes_diff(
                    serde_json::from_value(diff(&reference, "head_to_worktree")).unwrap(),
                    options()
                )
                .await
                .unwrap_err()
        )
        .kind(),
        Some(FaultKind::SessionChangeNotFound)
    );

    app.config.models[0].endpoint = new_provider.endpoint.clone();
    let operation = OperationId::new().unwrap();
    app.process
        .rpc(
            "host.reload",
            json!({"operation_id":operation,"expected_revision":0,"candidate":app.config}),
        )
        .await;
    tokio::time::timeout(support::WAIT, async {
        loop {
            let progress = app
                .process
                .rpc("host.receipt.get", json!({"operation_id":operation}))
                .await;
            match progress["result"]["status"].as_str() {
                Some("applied") => break,
                Some("preparing" | "applying") => tokio::task::yield_now().await,
                _ => panic!("reload failed: {progress}"),
            }
        }
    })
    .await
    .unwrap();
    let sibling = app.create(&alias, &["edit", "read"]).await;
    let same_root = app
        .observer
        .session(serde_json::from_value(sibling.clone()).unwrap());
    let positive = same_root
        .changes_diff(
            serde_json::from_value(diff(&reference, "head_to_worktree")).unwrap(),
            options(),
        )
        .await
        .unwrap();
    assert_eq!(lines(&json!(positive), "added"), initial_bytes);
    app.process
        .rpc("message.send", message(&sibling, "new binding"))
        .await;
    let new_request = new_provider.request().await;
    new_request.declarations(&["edit", "read"]);
    let new_turn = only_turn(&mut app.process, &sibling).await;
    // Both actual Turns are admitted before the model gates release. The old
    // binding must not follow the changed Session root or the replaced provider.
    old_request.answer(tool(
        "shared-edit",
        "edit",
        json!({"path":"file","old_text":"left=old","new_text":"left=new"}),
    ));
    new_request.answer(tool(
        "shared-edit",
        "edit",
        json!({"path":"file","old_text":"right=old","new_text":"right=new"}),
    ));
    let old_final = old_provider.request().await;
    let new_final = new_provider.request().await;
    for request in [&old_final, &new_final] {
        request.declarations(&["edit", "read"]);
        assert_eq!(request.tool_result("shared-edit")["status"], "applied");
    }
    let final_bytes = "left=new right=new\n";
    assert_eq!(
        fs::read(original.path().join("file")).unwrap(),
        final_bytes.as_bytes()
    );
    assert_eq!(
        fs::read(other.path().join("file")).unwrap(),
        b"different root\n"
    );
    let mut snapshots = Vec::new();
    for (target, turn) in [(&target, &old_turn), (&sibling, &new_turn)] {
        let changes = app.process.rpc("changes.list", json!({"target":target,"request":{"scope":{"kind":"turn","turn_id":turn},"cursor":null,"limit":100}})).await;
        assert!(changes["config_revision"].is_null());
        let records = changes["data"]["records"].as_array().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["change_ref"]["tool_call"]["turn_id"], *turn);
        assert_eq!(records[0]["change_ref"]["session_id"], target["session_id"]);
        let difference = app.process.rpc("changes.diff", json!({"target":target,"request":diff(&records[0]["change_ref"], "tool_before_after")})).await;
        snapshots.push((lines(&difference, "removed"), lines(&difference, "added")));
        if *turn == old_turn {
            assert_eq!(
                fault(
                    same_root
                        .changes_diff(
                            serde_json::from_value(diff(
                                &records[0]["change_ref"],
                                "tool_before_after"
                            ))
                            .unwrap(),
                            options()
                        )
                        .await
                        .unwrap_err()
                )
                .kind(),
                Some(FaultKind::SessionChangeNotFound)
            );
        }
    }
    if snapshots[1].0 == initial_bytes {
        snapshots.swap(0, 1);
    }
    assert_eq!(snapshots[0].0, initial_bytes);
    assert_eq!(
        snapshots[0].1, snapshots[1].0,
        "actual edits form one serialized before/after chain"
    );
    assert_eq!(snapshots[1].1, final_bytes);
    old_final.answer(answer("old provider completed"));
    new_final.answer(answer("new provider completed"));
    for (target, turn) in [(&target, &old_turn), (&sibling, &new_turn)] {
        let result = app
            .process
            .rpc("turn.wait", json!({"target":target,"turn_id":turn}))
            .await;
        assert_eq!(result["data"]["info"]["outcome"], "completed");
        assert_eq!(result["data"]["info"]["config_revision"], 0);
    }
    app.process
        .rpc(
            "message.send",
            message(&target, "next Turn uses current root"),
        )
        .await;
    let next = new_provider.request().await;
    next.declarations(&["read"]);
    next.answer(tool(
        "current-read",
        "read",
        json!({"path":"file","offset":1,"limit":1}),
    ));
    let next = new_provider.request().await;
    assert_eq!(next.tool_result("current-read")["text"], "different root\n");
    let turns = app.process.rpc("turn.list", json!({"target":target})).await;
    let later = turns["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|turn| turn["turn_id"] != old_turn)
        .unwrap();
    assert_eq!(later["config_revision"], 1);
    next.answer(answer("new binding complete"));
    assert_eq!(
        app.process
            .rpc(
                "turn.wait",
                json!({"target":target,"turn_id":later["turn_id"]})
            )
            .await["data"]["info"]["outcome"],
        "completed"
    );
    app.process.send("host.shutdown", shutdown("graceful"));
    let stopped = app.process.finish().await;
    assert!(stopped.status.success());
    stopped.protocol_only();
    stopped.safe_diagnostics();
}

#[cfg(target_os = "linux")]
#[path = "tool_support/cleanup.rs"]
mod cleanup;
