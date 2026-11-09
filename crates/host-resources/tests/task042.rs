//! Real local-tool integration through HostResourceBuilder + HTTP/SSE provider + Runtime.
//! Only the explicit loopback model replies are fixtures; filesystem, processes,
//! owner records, tool declarations and result handoffs are production paths.
#[cfg(target_os = "linux")]
#[path = "task042/completion.rs"]
mod completion;
#[cfg(unix)]
#[path = "task042/process.rs"]
mod process;
#[path = "task042/support.rs"]
mod support;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use minicore_host_workspace::{
    ChangeKind, ChangeRecord, ChangeScope, ChangesListRequest, EditOutcome, FileSnapshot,
};
use minicore_runtime::{HistoryItem, ToolResultHistory, tools::ToolResultOutcome};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use support::*;
use tokio::sync::Semaphore;

fn changes(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    scope: ChangeScope,
) -> Vec<ChangeRecord> {
    let result = host
        .session_changes(
            caller,
            target,
            ChangesListRequest {
                scope,
                cursor: None,
                limit: 100,
            },
        )
        .unwrap();
    assert!(result.next_cursor.is_none());
    result.records
}
fn native_result<'a>(history: &'a [HistoryItem], id: &str) -> &'a ToolResultHistory {
    let results: Vec<_> = history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(result) if result.call_id.as_str() == id => Some(result),
            _ => None,
        })
        .collect();
    assert_eq!(
        results.len(),
        1,
        "exactly one authoritative native ToolResult per invocation"
    );
    results[0]
}
fn result_json(history: &[HistoryItem], id: &str) -> Value {
    let result = native_result(history, id);
    assert_eq!(result.outcome, ToolResultOutcome::Success);
    serde_json::from_str(result.output.content().as_str()).unwrap()
}
fn call(turn: &TurnId, id: &str) -> ToolCallRef {
    ToolCallRef {
        turn_id: turn.clone(),
        tool_call_id: ToolCallId(id.into()),
    }
}
fn read_output(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    call: &ToolCallRef,
    channel: &str,
    start: u64,
    end: u64,
) -> Result<ToolTextSnapshot, Fault> {
    host.toolcall_output_read(
        caller,
        target,
        call,
        &TextReadRequest {
            channel: channel.into(),
            start,
            end,
            max_bytes: end.max(1),
        },
    )
}
fn subscribe(host: &Host, caller: &CallerContext, target: &SessionRef) -> EventSubscription {
    host.events_subscribe(
        caller,
        SubscribeRequest {
            scope: SubscriptionScope::Session {
                agent_id: target.agent_id.clone(),
                session_id: target.session_id.clone(),
            },
            categories: [EventCategory::Output, EventCategory::Completion]
                .into_iter()
                .collect(),
        },
    )
    .unwrap()
}
async fn next(subscription: &mut EventSubscription) -> ObservationEvent {
    match tokio::time::timeout(Duration::from_secs(10), subscription.recv())
        .await
        .expect("actual event")
        .unwrap()
        .expect("live subscription")
    {
        ObservationNotification::Event(event) => event.event,
        other => panic!("unexpected notification: {other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ordinary_tools_execute_and_retain_actual_results_edits_and_output_without_observers() {
    let mut server = Server::start(vec![
        (tool_answer("write-original", "write", json!({"path":"nested/file", "content":"old\nsecond\n"})), None),
        (tool_answer("read-original", "read", json!({"path":"nested/file", "offset":2, "limit":1})), None),
        (tool_answer("edit-original", "edit", json!({"path":"nested/file", "old_text":"old", "new_text":"实际"})), None),
        (tool_answer("edit-failed", "edit", json!({"path":"nested/file", "old_text":"absent", "new_text":"not written"})), None),
        (tool_answer("bash-original", "bash", json!({"command":"cat nested/file; printf diagnostic >&2; printf shell-owned > shell-file"})), None),
        (answer("Finished actual tools"), None),
    ]).await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let selected = selection(&["read", "write", "edit", "bash"]);
    let target = create(&host, &caller, selected.clone(), workspace(root.path()));
    send(&host, &caller, &target, "exercise actual files and process");
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    assert!(turn.cleanup_faults.is_empty());
    assert_eq!(
        std::fs::read(root.path().join("nested/file")).unwrap(),
        "实际\nsecond\n".as_bytes()
    );
    assert_eq!(
        std::fs::read(root.path().join("shell-file")).unwrap(),
        b"shell-owned"
    );
    let history = host.history(&caller, &target).unwrap();
    assert_eq!(result_json(&history, "write-original")["status"], "applied");
    assert_eq!(result_json(&history, "read-original")["text"], "second\n");
    assert_eq!(result_json(&history, "edit-original")["replacements"], 1);
    let failed = result_json(&history, "edit-failed");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["write_outcome"], "not_applied");
    let command = result_json(&history, "bash-original");
    assert_eq!(command["status"], "exited");
    assert_eq!(command["exit_code"], 0);
    assert_eq!(command["process_waited"], true);
    assert_eq!(command["cleanup_confirmed"], true);
    assert_eq!(command["cleanup_errors"], json!([]));
    assert_eq!(command["file_changes"], "unknown");
    assert_eq!(
        command["stdout"]["output"],
        json!({"encoding":"utf8","data":"实际\nsecond\n"})
    );
    assert_eq!(
        command["stderr"]["output"],
        json!({"encoding":"utf8","data":"diagnostic"})
    );
    for (channel, text) in [("stdout", "实际\nsecond\n"), ("stderr", "diagnostic")] {
        let output = read_output(
            &host,
            &caller,
            &target,
            &call(&turn.id, "bash-original"),
            channel,
            0,
            text.len() as u64,
        )
        .unwrap();
        assert_eq!(output.data.text, text);
        assert_eq!(output.source.turn.load_id, Some(turn.load_id.clone()));
        assert_eq!(output.source.turn.turn_id, turn.id);
        assert_eq!(output.source.turn.session.session_id, target.session_id);
    }
    let records = changes(&host, &caller, &target, ChangeScope::Session);
    assert_eq!(
        records.len(),
        3,
        "read and arbitrary bash writes are not fabricated edits"
    );
    let snapshots: Vec<_> = records
        .iter()
        .map(|record| {
            host.session_change_record(&caller, &target, &record.change_ref)
                .unwrap()
        })
        .collect();
    assert_eq!(snapshots[0].before, FileSnapshot::Missing);
    assert_eq!(
        snapshots[0].after,
        FileSnapshot::Content(b"old\nsecond\n".to_vec())
    );
    assert_eq!(snapshots[1].before, snapshots[0].after);
    assert_eq!(
        snapshots[1].after,
        FileSnapshot::Content("实际\nsecond\n".as_bytes().to_vec())
    );
    assert_eq!(snapshots[2].before, snapshots[1].after);
    assert_eq!(snapshots[2].after, snapshots[2].before);
    assert_eq!(snapshots[2].outcome, EditOutcome::NotApplied);
    assert_eq!(records[2].kind, ChangeKind::Unchanged);
    for (record, id) in snapshots
        .iter()
        .zip(["write-original", "edit-original", "edit-failed"])
    {
        assert_eq!(record.tool_call, call(&turn.id, id));
        assert_eq!(record.path, "nested/file");
    }
    let declarations = host.tool_list_for_session(&caller, &target).unwrap();
    let mut declared: Vec<_> = declarations
        .iter()
        .map(|spec| spec.name().as_str())
        .collect();
    declared.sort();
    assert_eq!(declared, ["bash", "edit", "read", "write"]);
    for completed_tools in 0..6 {
        let request = server.request().await;
        let mut names: Vec<_> = request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, declared);
        for spec in &declarations {
            let declared = request["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["function"]["name"] == spec.name().as_str())
                .unwrap();
            assert_eq!(
                declared["function"]["description"],
                spec.description().as_str()
            );
            assert_eq!(&declared["function"]["parameters"], spec.input_schema());
        }
        let returned: Vec<_> = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert_eq!(returned.len(), completed_tools);
        for message in returned {
            let result = native_result(&history, message["tool_call_id"].as_str().unwrap());
            assert_eq!(message["content"], result.output.content().as_str());
        }
    }
    let native = native_result(&history, "bash-original");
    let snapshot = host
        .toolcall_get(&caller, &target, &call(&turn.id, "bash-original"))
        .unwrap();
    assert_eq!(snapshot.data.result.unwrap().output, native.output);
}

#[tokio::test]
async fn unselected_registered_write_is_rejected_before_any_filesystem_side_effect() {
    let mut server = Server::start(vec![(
        tool_answer(
            "not-selected",
            "write",
            json!({"path":"forbidden", "content":"never"}),
        ),
        None,
    )])
    .await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, selection(&["read"]), workspace(root.path()));
    send(&host, &caller, &target, "selected read only");
    let request = server.request().await;
    assert_eq!(request["tools"].as_array().unwrap().len(), 1);
    assert_eq!(request["tools"][0]["function"]["name"], "read");
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(turn.outcome, Some(TurnOutcome::Failed));
    assert!(!root.path().join("forbidden").exists());
    assert!(changes(&host, &caller, &target, ChangeScope::Session).is_empty());
    assert!(
        !host
            .history(&caller, &target)
            .unwrap()
            .iter()
            .any(|item| matches!(item, HistoryItem::ToolResult(_)))
    );
}

#[tokio::test]
async fn repeated_provider_call_ids_keep_distinct_turn_results_and_edit_records() {
    let mut server = Server::start(vec![
        (
            tool_answer(
                "same-id",
                "write",
                json!({"path":"file", "content":"first"}),
            ),
            None,
        ),
        (answer("first finished"), None),
        (
            tool_answer(
                "same-id",
                "write",
                json!({"path":"file", "content":"second"}),
            ),
            None,
        ),
        (answer("second finished"), None),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let host = Host::memory_configured(prepare(&builder, config(&server.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(
        &host,
        &caller,
        selection(&["write"]),
        workspace(root.path()),
    );
    send(&host, &caller, &target, "first task");
    let first = settled(&host, &caller, &target, 1).await.remove(0);
    for _ in 0..2 {
        server.request().await;
    }
    send(&host, &caller, &target, "second task");
    let second = settled(&host, &caller, &target, 2)
        .await
        .into_iter()
        .find(|turn| turn.id != first.id)
        .unwrap();
    assert_eq!(first.outcome, Some(TurnOutcome::Completed));
    assert_eq!(second.outcome, Some(TurnOutcome::Completed));
    let records = changes(&host, &caller, &target, ChangeScope::Session);
    assert_eq!(records.len(), 2);
    let a = host
        .session_change_record(&caller, &target, &records[0].change_ref)
        .unwrap();
    let b = host
        .session_change_record(&caller, &target, &records[1].change_ref)
        .unwrap();
    assert_eq!(a.tool_call, call(&first.id, "same-id"));
    assert_eq!(b.tool_call, call(&second.id, "same-id"));
    assert_eq!(a.before, FileSnapshot::Missing);
    assert_eq!(a.after, FileSnapshot::Content(b"first".to_vec()));
    assert_eq!(b.before, a.after);
    assert_eq!(b.after, FileSnapshot::Content(b"second".to_vec()));
    for turn in [first, second] {
        let result = host.turn_result(&caller, &target, &turn.id).unwrap();
        let native = native_result(&result.history, "same-id");
        let record = host
            .toolcall_get(&caller, &target, &call(&turn.id, "same-id"))
            .unwrap();
        assert_eq!(record.data.result.unwrap().output, native.output);
        let scoped = changes(
            &host,
            &caller,
            &target,
            ChangeScope::Turn { turn_id: turn.id },
        );
        assert_eq!(scoped.len(), 1);
    }
}

#[tokio::test]
async fn current_local_tool_workspace_and_selection_stay_frozen_across_reload_and_update() {
    let gate = Arc::new(Semaphore::new(0));
    let mut old = Server::start(vec![
        (
            tool_answer(
                "old-write",
                "write",
                json!({"path":"file", "content":"old bound workspace"}),
            ),
            Some(gate.clone()),
        ),
        (
            tool_answer("old-read", "read", json!({"path":"file", "limit":1})),
            None,
        ),
        (answer("old finished"), None),
    ])
    .await;
    let mut new = Server::start(vec![
        (
            tool_answer("new-read", "read", json!({"path":"file", "limit":1})),
            None,
        ),
        (answer("new finished"), None),
    ])
    .await;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    std::fs::write(b.path().join("file"), "new bound workspace").unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let original = config(&old.endpoint);
    let host = Host::memory_configured(prepare(&builder, original.clone()).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(
        &host,
        &caller,
        selection(&["read", "write"]),
        workspace(a.path()),
    );
    send(&host, &caller, &target, "old task");
    let first_request = old.request().await;
    let mut candidate = original;
    candidate.models[0].endpoint = new.endpoint.clone();
    candidate.models[0].provider_model = "provider-B".into();
    candidate.framework_instructions = "FRAMEWORK B".into();
    candidate.tools = selection(&["read"]);
    assert_eq!(
        host.reload_configuration(&caller, 0, prepare(&builder, candidate).await)
            .await
            .unwrap(),
        HostReloadResult::Applied { config_revision: 1 }
    );
    host.update_session_async(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            tools: Some(selection(&["read"])),
            workspace: NullablePatch::Set(workspace(b.path()).unwrap()),
            role: Some(Role {
                name: "changed".into(),
                description: String::new(),
                instructions: "ROLE B".into(),
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    gate.add_permits(1);
    let second_request = old.request().await;
    let final_request = old.request().await;
    let first = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(first.outcome, Some(TurnOutcome::Completed));
    assert_eq!(first.config_revision, 0);
    assert_eq!(
        std::fs::read_to_string(a.path().join("file")).unwrap(),
        "old bound workspace"
    );
    assert_eq!(
        std::fs::read_to_string(b.path().join("file")).unwrap(),
        "new bound workspace"
    );
    for request in [&first_request, &second_request, &final_request] {
        assert_eq!(request["model"], "provider-A");
        assert_eq!(request["tools"].as_array().unwrap().len(), 2);
        let messages = request["messages"].as_array().unwrap();
        assert!(
            messages
                .iter()
                .any(|message| message["content"] == "FRAMEWORK A")
        );
        assert!(
            messages
                .iter()
                .any(|message| message["content"] == "ROLE A")
        );
        assert!(messages.iter().any(|message| {
            message["content"]
                .as_str()
                .is_some_and(|text| text.contains(a.path().to_str().unwrap()))
        }));
    }
    let history = host.history(&caller, &target).unwrap();
    assert_eq!(
        result_json(&history, "old-read")["text"],
        "old bound workspace"
    );
    send(&host, &caller, &target, "new task");
    let new_request = new.request().await;
    let second = settled(&host, &caller, &target, 2)
        .await
        .into_iter()
        .find(|turn| turn.id != first.id)
        .unwrap();
    assert_eq!(second.config_revision, 1);
    assert_eq!(second.outcome, Some(TurnOutcome::Completed));
    assert_eq!(new_request["model"], "provider-B");
    assert_eq!(new_request["tools"].as_array().unwrap().len(), 1);
    assert_eq!(new_request["tools"][0]["function"]["name"], "read");
    assert!(
        new_request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["content"] == "ROLE B")
    );
    assert_eq!(
        result_json(&host.history(&caller, &target).unwrap(), "new-read")["text"],
        "new bound workspace"
    );
}
