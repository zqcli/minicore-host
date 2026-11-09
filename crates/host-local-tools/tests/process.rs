#![cfg(unix)]
mod support;
use minicore_host_local_tools::*;
use minicore_host_workspace::Workspace;
use minicore_runtime::{
    LoopOptions, LoopOutcome, ToolCallId,
    tools::{ToolError, ToolInvocation},
};
use nix::{errno::Errno, sys::signal::kill, unistd::Pid};
use serde_json::json;
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use support::*;

async fn wait_file(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn gone(pid: u32) {
    assert_eq!(
        kill(Pid::from_raw(pid as i32), None),
        Err(Errno::ESRCH),
        "direct child must actually be gone"
    );
}

#[tokio::test]
async fn runtime_nonzero_exit_preserves_complete_stdout_stderr_and_side_effects_once() {
    let (root, tools, records) = fixture().await;
    let model = ScriptedModel::calls(vec![(
        "shell-42",
        "bash",
        json!({"command":"printf once >> effects; printf '真实 stdout'; printf 'real stderr' >&2; exit 7"}),
    )]);
    let report = finish(
        start(
            model,
            &tools,
            &["bash"],
            LoopOptions::default_checked().unwrap(),
        ),
        &tools,
    )
    .await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    let output = outputs(&report);
    assert_eq!(output[0]["status"], "exited");
    assert_eq!(output[0]["exit_code"], 7);
    assert_eq!(output[0]["stdout"]["output"]["data"], "真实 stdout");
    assert_eq!(output[0]["stderr"]["output"]["data"], "real stderr");
    assert_eq!(output[0]["stdout"]["complete"], true);
    assert_eq!(output[0]["file_changes"], "unknown");
    assert_eq!(fs::read(root.path().join("effects")).unwrap(), b"once");
    let commands = records.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, "shell-42");
    assert!(commands[0].1.process_waited);
    gone(commands[0].1.pid);
    let chunks = records.chunks.lock().unwrap();
    for channel in [OutputChannel::Stdout, OutputChannel::Stderr] {
        let mut actual: Vec<u8> = Vec::new();
        for (_, _, offset, bytes) in chunks.iter().filter(|(_, c, _, _)| *c == channel) {
            assert_eq!(*offset, actual.len() as u64);
            actual.extend(bytes);
        }
        assert_eq!(
            actual,
            match channel {
                OutputChannel::Stdout => commands[0].1.stdout.output.as_bytes(),
                OutputChannel::Stderr => commands[0].1.stderr.output.as_bytes(),
            }
        );
    }
}

#[tokio::test]
async fn full_large_and_binary_output_never_uses_hidden_tail_truncation() {
    let (_root, tools, _) = fixture().await;
    let output = invoke(
        &tools,
        "bash",
        json!({"command":"head -c 100000 /dev/zero | tr '\\0' x; printf '\\377\\000' >&2"}),
    )
    .await
    .unwrap();
    assert_eq!(
        output["stdout"]["output"]["data"].as_str().unwrap().len(),
        100000
    );
    assert_eq!(
        output["stderr"]["output"],
        json!({"encoding":"bytes","data":[255,0]})
    );
    assert_eq!(output["stderr"]["complete"], true);
    tools.finish().await;
}

#[tokio::test]
async fn bound_workspace_and_relative_cwd_never_change_process_cwd() {
    let (root, tools, _) = fixture().await;
    let process_cwd = std::env::current_dir().unwrap();
    fs::create_dir(root.path().join("sub")).unwrap();
    let output = invoke(&tools, "bash", json!({"command":"pwd","cwd":"sub"}))
        .await
        .unwrap();
    assert_eq!(
        output["stdout"]["output"]["data"],
        format!("{}\n", root.path().join("sub").display())
    );
    assert_eq!(std::env::current_dir().unwrap(), process_cwd);
    assert_eq!(
        invoke(&tools, "bash", json!({"command":"true","cwd":"../"}))
            .await
            .unwrap()["status"],
        "failed"
    );
    tools.finish().await;
}

#[tokio::test]
async fn runtime_cancel_drops_port_but_finish_waits_for_real_pid_and_cleanup() {
    let (root, tools, records) = fixture().await;
    let model = ScriptedModel::calls(vec![(
        "cancel-shell",
        "bash",
        json!({"command":"echo $$ > leader.pid; printf once >> effects; printf started; exec sleep 30"}),
    )]);
    let runtime = start(
        model,
        &tools,
        &["bash"],
        LoopOptions::default_checked().unwrap(),
    );
    wait_file(&root.path().join("effects")).await;
    let started = Instant::now();
    assert!(runtime.handle().cancel());
    let _report = runtime.join().await.unwrap();
    let done = tokio::time::timeout(Duration::from_secs(3), tools.finish())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].error, Some(ToolError::Cancelled));
    let commands = records.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].1.status, CommandStatus::Cancelled);
    assert!(commands[0].1.process_waited);
    gone(commands[0].1.pid);
    assert_eq!(fs::read(root.path().join("effects")).unwrap(), b"once");
}

#[tokio::test]
async fn runtime_deadline_joins_subprocess_and_never_repeats_side_effects() {
    let (root, tools, records) = fixture().await;
    let model = ScriptedModel::calls(vec![(
        "timeout-shell",
        "bash",
        json!({"command":"printf once >> effects; exec sleep 30"}),
    )]);
    let mut options = LoopOptions::default_checked().unwrap();
    options.tool_timeout = Duration::from_millis(100);
    let started = Instant::now();
    let report = finish(start(model, &tools, &["bash"], options), &tools).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert!(started.elapsed() < Duration::from_secs(3));
    let commands = records.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].1.status, CommandStatus::TimedOut);
    assert!(commands[0].1.process_waited);
    gone(commands[0].1.pid);
    assert_eq!(fs::read(root.path().join("effects")).unwrap(), b"once");
}

#[tokio::test]
async fn abandoned_execute_and_finish_waiters_do_not_abandon_owned_work() {
    let (root, tools, records) = fixture().await;
    let tools = Arc::new(tools);
    let tool = tools.tool("bash").unwrap();
    let context = context();
    let cancellation = context.cancellation.clone();
    let caller = tokio::spawn(async move {
        tool.execute(
            ToolInvocation::new(
                "dropped".parse().unwrap(),
                "bash".parse().unwrap(),
                json!({"command":"echo $$ > leader.pid; printf once >> effects; exec sleep 30"}),
            )
            .unwrap(),
            context,
        )
        .await
    });
    wait_file(&root.path().join("effects")).await;
    caller.abort();
    let _ = caller.await;
    let waiting = Arc::clone(&tools);
    let finish_waiter = tokio::spawn(async move { waiting.finish().await });
    tokio::task::yield_now().await;
    finish_waiter.abort();
    let _ = finish_waiter.await;
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(3), tools.finish())
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].error, Some(ToolError::Cancelled));
    let commands = records.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    gone(commands[0].1.pid);
    assert_eq!(fs::read(root.path().join("effects")).unwrap(), b"once");
}

#[tokio::test]
async fn normal_shell_exit_stops_background_group_and_waits_for_pipe_eof() {
    let (_root, tools, records) = fixture().await;
    let started = Instant::now();
    let output = invoke(&tools, "bash", json!({"command":"sleep 30 & printf done"}))
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(output["stdout"]["output"]["data"], "done");
    assert_eq!(output["stdout"]["complete"], true);
    tools.finish().await;
    gone(records.commands.lock().unwrap()[0].1.pid);
}

struct BrokenOutput;
impl LocalToolSink for BrokenOutput {
    fn output(
        &self,
        _: &ToolCallId,
        _: OutputChannel,
        _: u64,
        _: &[u8],
    ) -> Result<(), RecordError> {
        Err(RecordError)
    }
    fn edit(&self, _: &ToolCallId, _: CapturedEdit) -> Result<(), RecordError> {
        Ok(())
    }
    fn command(&self, _: &ToolCallId, _: &CommandResult) -> Result<(), RecordError> {
        Ok(())
    }
}
#[tokio::test]
async fn owner_output_fault_is_internal_and_stops_and_joins_process() {
    let root = tempfile::TempDir::new().unwrap();
    let tools = LocalTools::new(
        Workspace::open(root.path()).await.unwrap(),
        Arc::new(BrokenOutput),
    );
    assert_eq!(invoke(&tools,"bash",json!({"command":"echo $$ > leader.pid; printf once >> effects; printf trigger; exec sleep 30"})).await,Err(ToolError::Internal));
    let done = tools.finish().await;
    assert_eq!(done[0].error, Some(ToolError::Internal));
    let pid = fs::read_to_string(root.path().join("leader.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    gone(pid);
    assert_eq!(fs::read(root.path().join("effects")).unwrap(), b"once");
}

#[tokio::test]
async fn cancelled_before_start_has_no_process_or_file_effect() {
    let (root, tools, records) = fixture().await;
    let context = context();
    context.cancellation.cancel();
    let result = tools
        .tool("bash")
        .unwrap()
        .execute(
            ToolInvocation::new(
                "pre-cancel".parse().unwrap(),
                "bash".parse().unwrap(),
                json!({"command":"touch should-not-exist"}),
            )
            .unwrap(),
            context,
        )
        .await;
    assert_eq!(result.unwrap_err(), ToolError::Cancelled);
    assert!(!root.path().join("should-not-exist").exists());
    assert!(records.commands.lock().unwrap().is_empty());
    assert!(tools.finish().await.is_empty());
}
