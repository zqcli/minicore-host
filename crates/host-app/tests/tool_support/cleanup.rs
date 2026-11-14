//! Linux evidence for actual leader/descendant cleanup. SIGSTOP is a fixture
//! gate for the Host only, not cancellation, ptrace, or a replacement Runtime.
use super::*;
use minicore_host_core::{StateEvent, ToolCallState};
use std::{
    cell::Cell,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tokio::{io::AsyncReadExt, net::TcpListener};

fn fifo(path: &Path) {
    assert!(Command::new("mkfifo").arg(path).status().unwrap().success());
}
async fn witness(listener: &TcpListener, expected: &str) {
    tokio::time::timeout(support::WAIT, async {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, expected.as_bytes());
    })
    .await
    .unwrap();
}
struct Children {
    root: PathBuf,
    cleaned: Cell<bool>,
}
impl Children {
    fn watch(root: &Path) -> Self {
        Self {
            root: root.into(),
            cleaned: Cell::new(false),
        }
    }
    fn pid(&self, name: &str) -> u32 {
        fs::read_to_string(self.root.join(name))
            .unwrap()
            .parse()
            .unwrap()
    }
    fn live(&self) {
        for name in ["leader.pid", "descendant.pid", "host.pid"] {
            let pid = self.pid(name);
            assert!(
                Path::new(&format!("/proc/{pid}")).exists(),
                "expected real live PID {pid}"
            );
        }
    }
    fn gone(&self) {
        for name in ["leader.pid", "descendant.pid"] {
            let pid = self.pid(name);
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "owned PID {pid} still exists"
            );
        }
        assert!(
            !Command::new("kill")
                .args(["-0", "--", &format!("-{}", self.pid("leader.pid"))])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success(),
            "owned process group remains"
        );
        self.cleaned.set(true);
    }
    async fn stopped_host(&self) {
        tokio::time::timeout(support::WAIT, async {
            loop {
                let state =
                    fs::read_to_string(format!("/proc/{}/status", self.pid("host.pid"))).unwrap();
                if state.lines().any(|line| line.starts_with("State:\tT")) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Children {
    fn drop(&mut self) {
        // Failure cleanup only. Normal assertions above require actual Host
        // cleanup before this guard can run. Avoid a stopped Host/orphaned tool
        // if a regression panics while the fixture gate is held.
        if self.cleaned.get() {
            return;
        }
        for (file, signal, group) in [("host.pid", "-CONT", false), ("leader.pid", "-KILL", true)] {
            if let Some(pid) = fs::read_to_string(self.root.join(file))
                .ok()
                .and_then(|text| text.parse::<u32>().ok())
            {
                let target = if group {
                    format!("-{pid}")
                } else {
                    pid.to_string()
                };
                let _ = Command::new("kill")
                    .args([signal, "--", &target])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}
fn child_command(port: u16, output: &str) -> String {
    format!(
        "printf '%s' \"$BASHPID\" > leader.pid; printf '%s' \"$PPID\" > host.pid; (printf '%s' \"$BASHPID\" > descendant.pid; printf child-ready > /dev/tcp/127.0.0.1/{port}; IFS= read -r _ < never) & {output}; IFS= read -r _ < gate; printf '尾部🙂'; printf '末尾' >&2; printf tail-ready > /dev/tcp/127.0.0.1/{port}; IFS= read -r _ < never"
    )
}
fn assert_saved_cancel(root: &Path, target: &Value, turn: &Value, call: &str) -> Value {
    let records: Vec<Value> = fs::read_to_string(history_path(root, target))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let outcomes: Vec<_> = records
        .iter()
        .filter(|record| record["entry"]["kind"] == "turn_finished")
        .collect();
    assert_eq!(outcomes.len(), 1, "one actual durable Turn outcome");
    assert_eq!(outcomes[0]["entry"]["data"]["turn_id"], *turn);
    assert_eq!(outcomes[0]["entry"]["data"]["outcome"], "cancelled");
    let results: Vec<_> = records
        .iter()
        .filter_map(|record| {
            let data = &record["entry"]["data"];
            (record["entry"]["kind"] == "runtime"
                && data["item"]["type"] == "tool_result"
                && data["item"]["data"]["call_id"] == call)
                .then_some(data)
        })
        .collect();
    assert_eq!(
        results.len(),
        1,
        "cancellation cannot discard or duplicate native ToolResult"
    );
    assert_eq!(results[0]["turn_id"], *turn);
    assert_eq!(results[0]["item"]["data"]["outcome"], "cancelled");
    results[0]["item"]["data"].clone()
}

#[tokio::test]
async fn binary_cancel_drains_real_buffered_tails_before_tool_and_turn_completion() {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["bash"]).await;
    let workspace = tempfile::tempdir().unwrap();
    for name in ["gate", "never"] {
        fifo(&workspace.path().join(name));
    }
    let mut gate = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(workspace.path().join("gate"))
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = app.create(workspace.path(), &["bash"]).await;
    let children = Children::watch(workspace.path());
    let mut events = subscribe(&app.observer, &target).await;
    app.process
        .rpc(
            "message.send",
            message(
                &target,
                "cancel actual running command with buffered output",
            ),
        )
        .await;
    provider.request().await.answer(tool(
        "cancel-call",
        "bash",
        json!({"command":child_command(listener.local_addr().unwrap().port(), "printf ready")}),
    ));
    let source = loop {
        if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
            event(&mut events).await
        {
            assert_eq!(data.text, "ready");
            assert_eq!((data.start, data.end), (0, 5));
            break source;
        }
    };
    witness(&listener, "child-ready").await;
    children.live();
    let turn = json!(source.turn.turn_id);
    app.process.signal("-STOP");
    children.stopped_host().await;
    gate.write_all(b"release\n").unwrap();
    witness(&listener, "tail-ready").await;
    children.live();
    // All tail bytes are in real OS pipes while Host is confirmed stopped.
    // Queue the actual RPC, then resume the Host. Elapsed time proves nothing.
    let cancel_id = app.process.send("turn.cancel", json!({"target":target,"operation_id":OperationId::new().unwrap(),"turn_id":turn,"keep_inbox":false}));
    app.process.signal("-CONT");
    let mut stdout = String::from("ready");
    let mut stderr = String::new();
    let mut completions = Vec::new();
    let mut finished_states = 0;
    loop {
        match event(&mut events).await {
            ObservationEvent::Activity(ActivityEvent::ToolOutput {
                source: actual,
                data,
            }) => {
                assert_eq!(actual, source);
                assert!(
                    completions.is_empty(),
                    "real tail must not follow the tool completion boundary"
                );
                let text = match data.channel.as_str() {
                    "stdout" => &mut stdout,
                    "stderr" => &mut stderr,
                    other => panic!("unexpected channel {other}"),
                };
                assert_eq!(data.start, text.len() as u64);
                text.push_str(&data.text);
                assert_eq!(data.end, text.len() as u64);
            }
            ObservationEvent::State(StateEvent::Toolcall {
                source: actual,
                data,
            }) if data.state == ToolCallState::Finished => {
                assert_eq!(actual, source);
                children.gone();
                finished_states += 1;
            }
            ObservationEvent::Activity(ActivityEvent::ToolFinished {
                source: actual,
                data,
            }) => {
                assert_eq!(actual, source);
                children.gone();
                completions.push(data);
            }
            ObservationEvent::Activity(ActivityEvent::TurnFinished { .. }) => break,
            _ => {}
        }
    }
    assert_eq!(stdout, "ready尾部🙂");
    assert_eq!(stderr, "末尾");
    assert_eq!(finished_states, 1);
    assert_eq!(completions.len(), 1);
    assert_eq!(json!(completions[0].result.outcome), "cancelled");
    for (channel, text) in [("stdout", &stdout), ("stderr", &stderr)] {
        assert_eq!(
            completions[0]
                .output_positions
                .iter()
                .find(|p| p.channel == channel)
                .unwrap()
                .end,
            text.len() as u64
        );
        let read = app.process.rpc("toolcall.output.read", json!({"target":target,"tool":{"turn_id":turn,"tool_call_id":"cancel-call"},"request":{"channel":channel,"start":0,"end":text.len(),"max_bytes":4096}})).await;
        assert_eq!(read["data"]["text"], *text);
        assert_eq!(read["load_id"], json!(source.turn.load_id));
    }
    let result = app
        .process
        .rpc("turn.wait", json!({"target":target,"turn_id":turn}))
        .await;
    assert_eq!(result["data"]["info"]["outcome"], "cancelled");
    assert_eq!(result["data"]["info"]["persistence"], "persisted");
    assert_eq!(result["data"]["info"]["cleanup_faults"], json!([]));
    let history = history(&mut app.process, &target).await;
    assert_eq!(
        history_result(&history, "cancel-call"),
        &json!(completions[0].result)
    );
    assert_saved_cancel(&app.root, &target, &turn, "cancel-call");
    app.process.send("host.shutdown", shutdown("graceful"));
    let stopped = app.process.finish().await;
    assert!(stopped.status.success());
    let cancel: Vec<_> = stopped
        .stdout
        .iter()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|reply| reply["id"] == cancel_id)
        .collect();
    assert_eq!(cancel.len(), 1);
    assert!(cancel[0].get("error").is_none());
    assert_eq!(cancel[0]["result"]["result"]["cancel_claimed"], true);
    stopped.protocol_only();
    children.gone();
}

#[tokio::test]
async fn binary_force_waits_for_actual_child_cleanup_and_preserves_native_cancel_history() {
    let mut provider = Provider::start().await;
    let mut app = App::start(&provider, &["write", "bash"]).await;
    let workspace = tempfile::tempdir().unwrap();
    for name in ["gate", "never"] {
        fifo(&workspace.path().join(name));
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = app.create(workspace.path(), &["write", "bash"]).await;
    let children = Children::watch(workspace.path());
    let mut events = subscribe(&app.observer, &target).await;
    app.process
        .rpc(
            "message.send",
            message(&target, "force a real blocked tool"),
        )
        .await;
    provider.request().await.answer(tool(
        "completed-write",
        "write",
        json!({"path":"committed-file","content":"confirmed before force\n"}),
    ));
    let next = provider.request().await;
    let completed_write = next.tool_result("completed-write");
    assert_eq!(completed_write["status"], "applied");
    assert_eq!(
        fs::read(workspace.path().join("committed-file")).unwrap(),
        b"confirmed before force\n"
    );
    next.answer(tool("force-call", "bash", json!({"command":child_command(listener.local_addr().unwrap().port(), "printf ready-out; printf ready-err >&2")})));
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut turn = Value::Null;
    while stdout != "ready-out" || stderr != "ready-err" {
        if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
            event(&mut events).await
        {
            turn = json!(source.turn.turn_id);
            let text = match data.channel.as_str() {
                "stdout" => &mut stdout,
                "stderr" => &mut stderr,
                other => panic!("unexpected channel {other}"),
            };
            assert_eq!(data.start, text.len() as u64);
            text.push_str(&data.text);
        }
    }
    witness(&listener, "child-ready").await;
    children.live();
    let force = app.process.send("host.shutdown", shutdown("force"));
    let stopped = app.process.finish().await;
    assert!(stopped.status.success());
    children.gone();
    stopped.protocol_only();
    stopped.safe_diagnostics();
    let replies: Vec<Value> = stopped
        .stdout
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    if let Some(reply) = replies.iter().find(|reply| reply["id"] == force) {
        assert!(reply.get("error").is_none());
        assert_eq!(reply["result"]["result"]["strategy"], "force");
    }
    assert!(
        stopped
            .events()
            .iter()
            .any(|event| event["event"] == "host.stopped")
    );
    let native = assert_saved_cancel(&app.root, &target, &turn, "force-call");
    let saved = fs::read(history_path(&app.root, &target)).unwrap();
    // Reopen through the same actual binary, not through a storage test double.
    let mut reopened = Process::start(
        &app.root,
        &app.config_dir,
        true,
        Some((&app.certificate, &app.key)),
    );
    let initial = reopened.rpc("initialize", json!({})).await;
    assert_eq!(initial["host_id"], app.initial["host_id"]);
    assert_ne!(initial["host_incarnation"], app.initial["host_incarnation"]);
    let history = history(&mut reopened, &target).await;
    let earlier = history_result(&history, "completed-write");
    assert_eq!(
        earlier["outcome"], "success",
        "force cannot replace an already handed-over native result"
    );
    assert_eq!(
        serde_json::from_str::<Value>(earlier["output"]["content"].as_str().unwrap()).unwrap(),
        completed_write
    );
    assert_eq!(
        fs::read(workspace.path().join("committed-file")).unwrap(),
        b"confirmed before force\n"
    );
    let result = history_result(&history, "force-call");
    assert_eq!(result["outcome"], native["outcome"]);
    assert_eq!(result["output"], native["output"]);
    assert_eq!(fs::read(history_path(&app.root, &target)).unwrap(), saved);
    assert_eq!(
        reopened.rpc("turn.list", json!({"target":target})).await["data"],
        json!([])
    );
    reopened.send("host.shutdown", shutdown("graceful"));
    assert!(reopened.finish().await.status.success());
}
