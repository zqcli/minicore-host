mod support;
use minicore_runtime::{
    AgentLoop, ExecutionConfig, LoopInputs, LoopOptions, LoopOutcome, LoopRequest, UserInput,
    model::ReasoningPreference,
    prompt::DefaultPromptProvider,
    tools::{ToolDecision, ToolPolicy, ToolPolicyFuture, ToolPolicyRequest, ToolSet},
};
use serde_json::json;
use std::{fs, sync::Arc};
use support::*;

#[tokio::test]
async fn binding_survives_internal_runtime_loop_continuation_until_public_turn_finish() {
    let (root, tools, records) = fixture().await;
    let first = ScriptedModel::calls(vec![(
        "first-loop",
        "write",
        json!({"path":"file","content":"first"}),
    )]);
    let report = start(
        first,
        &tools,
        &["write"],
        LoopOptions::default_checked().unwrap(),
    )
    .join()
    .await
    .unwrap();
    assert_eq!(report.outcome, LoopOutcome::Completed);
    // Public Turn is not finalizing: do not call LocalTools::finish here.
    let second = ScriptedModel::calls(vec![(
        "second-loop",
        "edit",
        json!({"path":"file","old_text":"first","new_text":"continued"}),
    )]);
    let report = finish(
        start(
            second,
            &tools,
            &["edit"],
            LoopOptions::default_checked().unwrap(),
        ),
        &tools,
    )
    .await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        "continued"
    );
    assert_eq!(records.edits.lock().unwrap().len(), 2);
    assert_eq!(tools.finish().await.len(), 2);
}

struct Deny;
impl ToolPolicy for Deny {
    fn decide<'a>(&'a self, _: ToolPolicyRequest) -> ToolPolicyFuture<'a> {
        Box::pin(async { ToolDecision::deny("explicit Runtime policy fixture") })
    }
}
struct NoInputs;
impl LoopInputs for NoInputs {
    fn take(&self) -> Vec<UserInput> {
        Vec::new()
    }
}
#[tokio::test]
async fn real_runtime_policy_denial_prevents_all_file_side_effects() {
    let (root, tools, records) = fixture().await;
    let model = ScriptedModel::calls(vec![(
        "denied",
        "write",
        json!({"path":"not-written","content":"denied"}),
    )]);
    let mut builder = ToolSet::builder();
    builder.register_arc(tools.tool("write").unwrap());
    let config = ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        builder.build().unwrap(),
        Some(Arc::new(Deny)),
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap();
    let runtime = AgentLoop::start(
        LoopRequest::new(
            Arc::from([]),
            UserInput::text("explicit denied fixture").unwrap(),
            config,
        ),
        LoopOptions::default_checked().unwrap(),
        Arc::new(NoInputs),
    )
    .unwrap();
    let report = finish(runtime, &tools).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
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
                minicore_runtime::tools::ToolInvocation::new(
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
