#![cfg(any(unix, windows))]
mod support;
use minicore_host_local_tools::*;
use minicore_host_workspace::Workspace;
use minicore_runtime::{
    LoopOptions, LoopOutcome, ToolCallId,
    tools::{ToolError, ToolInvocation},
};
#[cfg(unix)]
use nix::{
    errno::Errno,
    sys::signal::{kill, killpg},
    unistd::Pid,
};
use serde_json::json;
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use support::*;

async fn wait_file(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fs::read(path).is_ok_and(|bytes| !bytes.is_empty()) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
#[cfg(unix)]
fn gone(pid: u32) {
    assert_eq!(
        kill(Pid::from_raw(pid as i32), None),
        Err(Errno::ESRCH),
        "direct child must actually be gone"
    );
}

#[cfg(windows)]
fn gone(pid: u32) {
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 1 }}"
        ))
        .status()
        .unwrap();
    assert!(
        status.success(),
        "owned native process {pid} must actually be gone"
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
    assert!(commands[0].1.cleanup_confirmed);
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
    let command = if cfg!(windows) {
        "cygpath -w \"$PWD\""
    } else {
        "pwd"
    };
    let output = invoke(&tools, "bash", json!({"command":command,"cwd":"sub"}))
        .await
        .unwrap();
    assert_eq!(
        fs::canonicalize(output["stdout"]["output"]["data"].as_str().unwrap().trim()).unwrap(),
        fs::canonicalize(root.path().join("sub")).unwrap()
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
    let done = tokio::time::timeout(Duration::from_secs(10), tools.finish())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].error, Some(ToolError::Cancelled));
    assert!(done[0].cleanup_confirmed);
    let commands = records.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].1.status, CommandStatus::Cancelled);
    assert!(commands[0].1.process_waited);
    assert!(commands[0].1.cleanup_confirmed);
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
    options.tool_timeout = Duration::from_millis(if cfg!(windows) { 1_000 } else { 100 });
    let started = Instant::now();
    let report = finish(start(model, &tools, &["bash"], options), &tools).await;
    assert_eq!(report.outcome, LoopOutcome::Completed);
    assert!(started.elapsed() < Duration::from_secs(10));
    let commands = records.commands.lock().unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].1.status, CommandStatus::TimedOut);
    assert!(commands[0].1.process_waited);
    assert!(commands[0].1.cleanup_confirmed);
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
    let result = tokio::time::timeout(Duration::from_secs(10), tools.finish())
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
    assert!(started.elapsed() < Duration::from_secs(10));
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
        Arc::new(InheritCommandEnvironment),
    );
    assert_eq!(invoke(&tools,"bash",json!({"command":"echo $$ > leader.pid; printf once >> effects; printf trigger; exec sleep 30"})).await,Err(ToolError::Internal));
    let done = tools.finish().await;
    assert_eq!(done[0].error, Some(ToolError::Internal));
    assert!(done[0].cleanup_confirmed);
    #[cfg(unix)]
    gone(
        fs::read_to_string(root.path().join("leader.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    );
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

/// Adapted from minicore-agent command tests: pipe EOF alone says nothing about
/// a background process that redirected both of its output descriptors.
#[cfg(unix)]
#[tokio::test]
async fn redirected_background_members_end_before_turn_finish() {
    let (root, tools, records) = fixture().await;
    let output = invoke(&tools, "bash", json!({"command":
        "for i in {1..12}; do sleep 30 >/dev/null 2>&1 & printf '%s\\n' $! >> children; done; printf done"
    })).await.unwrap();
    let done = tools.finish().await;
    assert_eq!(output["status"], "exited");
    assert!(done[0].cleanup_confirmed);
    let commands = records.commands.lock().unwrap();
    let command = &commands[0].1;
    assert!(command.process_waited && command.cleanup_confirmed);
    assert!(command.stdout.complete && command.stderr.complete);
    assert_eq!(command.stdout.output.as_bytes(), b"done");
    assert_eq!(
        killpg(Pid::from_raw(command.pid as i32), None),
        Err(Errno::ESRCH),
        "confirmed cleanup must observe the original group gone, not merely signal it"
    );
    for pid in fs::read_to_string(root.path().join("children"))
        .unwrap()
        .lines()
    {
        gone(pid.parse().unwrap());
    }
}

/// Adapted from minicore-agent's leader-and-group-member reap regression, with
/// the Host's public Turn finish barrier rather than the old Loop owner join.
#[cfg(unix)]
#[tokio::test]
async fn cancellation_joins_leader_and_background_member_before_turn_finish() {
    let (root, tools, records) = fixture().await;
    let tool = tools.tool("bash").unwrap();
    let context = context();
    let cancellation = context.cancellation.clone();
    let caller = tokio::spawn(async move {
        tool.execute(ToolInvocation::new(
            "group-cancel".parse().unwrap(), "bash".parse().unwrap(),
            json!({"command":"sleep 30 >/dev/null 2>&1 & echo $! > child.pid; echo $$ > leader.pid; printf once > ready; wait"})
        ).unwrap(), context).await
    });
    wait_file(&root.path().join("ready")).await;
    cancellation.cancel();
    assert_eq!(caller.await.unwrap().unwrap_err(), ToolError::Cancelled);
    let done = tools.finish().await;
    assert!(done[0].cleanup_confirmed);
    let commands = records.commands.lock().unwrap();
    let command = &commands[0].1;
    assert_eq!(command.status, CommandStatus::Cancelled);
    assert!(command.process_waited && command.cleanup_confirmed);
    assert!(command.stdout.complete && command.stderr.complete);
    assert_eq!(
        killpg(Pid::from_raw(command.pid as i32), None),
        Err(Errno::ESRCH)
    );
    for name in ["leader.pid", "child.pid"] {
        gone(
            fs::read_to_string(root.path().join(name))
                .unwrap()
                .trim()
                .parse()
                .unwrap(),
        );
    }
    assert_eq!(fs::read(root.path().join("ready")).unwrap(), b"once");
}

#[cfg(windows)]
#[test]
#[allow(
    clippy::zombie_processes,
    reason = "the probe deliberately leaves a live descendant for the actual job owner to settle"
)]
fn windows_scope_probe() {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    let Ok(role) = std::env::var("MINICORE_WINDOWS_SCOPE_ROLE") else {
        return;
    };
    let port: u16 = std::env::var("MINICORE_WINDOWS_SCOPE_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let _descendant = (role == "root").then(|| {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "windows_scope_probe", "--nocapture"])
            .env("MINICORE_WINDOWS_SCOPE_ROLE", "leaf")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    });
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    writeln!(socket, "{role} {}", std::process::id()).unwrap();
    let mut release = [0];
    socket.read_exact(&mut release).unwrap();
    // Returning the root leaves the native descendant alive with closed stdio.
    // The command job, rather than a test-side kill, must settle that descendant.
}

#[cfg(windows)]
async fn windows_scope_peers(
    listener: &tokio::net::TcpListener,
) -> Vec<(String, tokio::io::BufReader<tokio::net::TcpStream>)> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut peers = Vec::new();
    for _ in 0..2 {
        let (socket, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut socket = BufReader::new(socket);
        let mut identity = String::new();
        tokio::time::timeout(Duration::from_secs(10), socket.read_line(&mut identity))
            .await
            .unwrap()
            .unwrap();
        peers.push((identity, socket));
    }
    assert!(peers.iter().any(|(id, _)| id.starts_with("root ")));
    assert!(peers.iter().any(|(id, _)| id.starts_with("leaf ")));
    peers
}

#[cfg(windows)]
#[tokio::test]
async fn windows_job_wait_does_not_confuse_a_leader_exit_with_an_empty_job() {
    use process_wrap::tokio::{CommandWrap, JobObject, KillOnDrop};
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "windows_scope_probe", "--nocapture"])
        .env("MINICORE_WINDOWS_SCOPE_ROLE", "root")
        .env(
            "MINICORE_WINDOWS_SCOPE_PORT",
            listener.local_addr().unwrap().port().to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut command = CommandWrap::from(command);
    command.wrap(JobObject).wrap(KillOnDrop);
    let mut child = command.spawn().unwrap();
    let mut peers = windows_scope_peers(&listener).await;
    peers
        .iter_mut()
        .find(|(id, _)| id.starts_with("root "))
        .unwrap()
        .1
        .get_mut()
        .write_all(b"x")
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), child.inner_mut().wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );

    // The job has received NEW_PROCESS and EXIT_PROCESS notifications, while
    // the native descendant still blocks on its socket with both pipes closed.
    // Keep the same wait future alive across the observation and release.
    let mut waiting = child.wait();
    let early = tokio::time::timeout(Duration::from_millis(200), &mut waiting).await;
    let returned_early = early.is_ok();
    peers
        .iter_mut()
        .find(|(id, _)| id.starts_with("leaf "))
        .unwrap()
        .1
        .get_mut()
        .write_all(b"x")
        .await
        .unwrap();
    let exit = match early {
        Ok(exit) => exit.unwrap(),
        Err(_) => tokio::time::timeout(Duration::from_secs(10), waiting)
            .await
            .unwrap()
            .unwrap(),
    };
    assert!(exit.success());
    for (_, mut socket) in peers {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
    assert!(
        !returned_early,
        "job wait returned while a native descendant was alive"
    );
}

#[cfg(windows)]
async fn windows_job_ends_native_descendants(cancel: bool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (_root, tools, records) = fixture().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let executable = std::env::current_exe().unwrap();
    let executable = executable.to_str().unwrap().trim_start_matches(r"\\?\");
    let executable = executable.replace('\\', "/").replace('\'', "'\\''");
    let script = format!(
        "MINICORE_WINDOWS_SCOPE_PORT={} MINICORE_WINDOWS_SCOPE_ROLE=root '{}' --exact windows_scope_probe --nocapture >/dev/null 2>&1; printf done",
        listener.local_addr().unwrap().port(),
        executable,
    );
    let tool = tools.tool("bash").unwrap();
    let context = context();
    let cancellation = context.cancellation.clone();
    let caller = tokio::spawn(async move {
        tool.execute(
            ToolInvocation::new(
                "native-job".parse().unwrap(),
                "bash".parse().unwrap(),
                json!({"command":script}),
            )
            .unwrap(),
            context,
        )
        .await
    });
    let mut peers = windows_scope_peers(&listener).await;
    if cancel {
        cancellation.cancel();
    } else {
        let (_, socket) = peers
            .iter_mut()
            .find(|(id, _)| id.starts_with("root "))
            .unwrap();
        socket.get_mut().write_all(b"x").await.unwrap();
    }
    let outcome = tokio::time::timeout(Duration::from_secs(10), caller)
        .await
        .unwrap()
        .unwrap();
    if cancel {
        assert_eq!(outcome.unwrap_err(), ToolError::Cancelled);
    } else {
        assert!(outcome.is_ok());
    }
    let done = tools.finish().await;
    assert!(done[0].cleanup_confirmed);
    let command = records.commands.lock().unwrap()[0].1.clone();
    assert!(command.process_waited && command.cleanup_confirmed);
    assert!(command.stdout.complete && command.stderr.complete);
    assert_eq!(command.signal, None);
    for (identity, mut socket) in peers {
        let mut byte = [0];
        let ended = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ended, 0, "confirmed cleanup left {identity} alive");
    }
}

#[cfg(windows)]
#[tokio::test]
async fn windows_job_waits_for_redirected_native_descendants_after_shell_exit() {
    windows_job_ends_native_descendants(false).await;
}

#[cfg(windows)]
#[tokio::test]
async fn windows_job_cancellation_ends_native_descendants_before_finish() {
    windows_job_ends_native_descendants(true).await;
}
