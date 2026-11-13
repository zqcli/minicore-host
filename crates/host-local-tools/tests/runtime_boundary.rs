mod support;
use minicore_host_runtime::{
    ExecutionOptions,
    tools::{
        ToolDecision, ToolPolicy, ToolPolicyFuture, ToolPolicyRequest, ToolResultOutcome,
        run_tool_batch,
    },
};
use serde_json::json;
use std::{fs, sync::Arc};
use support::*;

#[tokio::test]
async fn selected_local_tools_share_the_registered_static_declarations() {
    let (_root, tools, _records) = fixture().await;
    for declared in minicore_host_local_tools::local_tool_specs() {
        let first = tools.tool(declared.name().as_str()).unwrap();
        let second = tools.tool(declared.name().as_str()).unwrap();
        assert_eq!(first.spec(), &declared);
        assert!(std::ptr::eq(
            first.spec().input_schema(),
            declared.input_schema()
        ));
        assert!(std::ptr::eq(
            first.spec().input_schema(),
            second.spec().input_schema()
        ));
    }
    assert!(tools.tool("not-registered").is_none());
    assert!(tools.finish().await.is_empty());
}

#[tokio::test]
async fn binding_survives_successive_tool_batches_until_public_turn_finish() {
    let (root, tools, records) = fixture().await;
    let first = vec![(
        "first-batch",
        "write",
        json!({"path":"file","content":"first"}),
    )];
    let report = run_calls(&tools, first, ExecutionOptions::default()).await;
    assert_eq!(report.stop, None);
    // Public Turn is not finalizing: do not call LocalTools::finish here.
    let second = vec![(
        "second-batch",
        "edit",
        json!({"path":"file","old_text":"first","new_text":"continued"}),
    )];
    let report = run_calls(&tools, second, ExecutionOptions::default()).await;
    assert_eq!(report.stop, None);
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        "continued"
    );
    assert_eq!(records.edits.lock().unwrap().len(), 2);
    assert_eq!(tools.finish().await.len(), 2);
}

struct Deny;
impl ToolPolicy for Deny {
    fn decide<'a>(&'a self, _: ToolPolicyRequest<'a>) -> ToolPolicyFuture<'a> {
        Box::pin(async { ToolDecision::deny("explicit Runtime policy fixture") })
    }
}
#[tokio::test]
async fn real_runtime_policy_denial_prevents_all_file_side_effects() {
    let (root, tools, records) = fixture().await;
    let call = tool_call(
        "denied",
        "write",
        json!({"path":"not-written","content":"denied"}),
    );
    let config = tool_config(&tools, &["write"], Some(Arc::new(Deny)));
    let report = run_tool_batch(
        &config,
        &[&call],
        batch_context(&ExecutionOptions::default()),
        Arc::new(NoInteractions),
    )
    .await;
    assert_eq!(report.stop, None);
    assert_eq!(report.results[0].outcome, ToolResultOutcome::Denied);
    assert!(!root.path().join("not-written").exists());
    assert!(records.edits.lock().unwrap().is_empty());
    assert!(tools.finish().await.is_empty());
}

#[tokio::test]
async fn concurrent_same_binding_edits_capture_each_actual_preceding_write() {
    let (root, tools, records) = fixture().await;
    fs::write(root.path().join("file"), "root").unwrap();
    let mut calls = Vec::new();
    for index in 0..12 {
        let tool = tools.tool("edit").unwrap();
        calls.push(tokio::spawn(async move {
            tool.execute(
                minicore_host_runtime::tools::ToolInvocation::new(
                    format!("parallel-{index}").parse().unwrap(),
                    "edit".parse().unwrap(),
                    json!({"path":"file","old_text":"root","new_text":"rootX"}),
                )
                .unwrap(),
                context(),
            )
            .await
            .unwrap()
        }));
    }
    for call in calls {
        call.await.unwrap();
    }
    tools.finish().await;
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        "root".to_owned() + &"X".repeat(12)
    );
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 12);
    for pair in edits.windows(2) {
        assert_eq!(pair[0].1.after, pair[1].1.before);
    }
}
