//! Real production tools and child wait; the HTTP server supplies only model replies.
#![cfg(unix)]
mod diagnostic_support;
use diagnostic_support::*;
use minicore_host_core::Host;
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_transport::HostClient;
use serde_json::{Value, json};
use std::time::Duration;

const SCRIPT: &str = "printf 'PRIVATE_TOOL_STDOUT'; printf 'PRIVATE_TOOL_STDERR' >&2; exit 7";
const MISSING: &str = "PRIVATE_MISSING_PATH";
fn tool(id: &str, name: &str, arguments: Value) -> Reply {
    Reply {
        status: 200,
        gate: None,
        body: format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
        ),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_tool_completion_and_process_exit_keep_original_ids_and_exclude_private_content() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-tools");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let guard = minicore_host_diagnostics::install(logging).unwrap();
    let http = Http::start(vec![
        tool("original-shell", "bash", json!({"command":SCRIPT})),
        tool("original-read", "read", json!({"path":MISSING})),
        Reply::success(),
    ])
    .await;
    let tools: Vec<_> = ["bash", "read"]
        .into_iter()
        .map(|name| ToolBinding { name: name.into() })
        .collect();
    let mut config = configuration(&http.endpoint);
    config.tools = tools.clone();
    let host = Host::memory_configured(prepare(config).await).unwrap();
    let client = HostClient::in_process(host.clone()).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let message = input();
    let mut creation = create(Some(message.clone()));
    creation.config.session_defaults.tools = tools;
    creation.config.session_defaults.workspace = Some(WorkspaceRef {
        path: workspace.path().to_str().unwrap().into(),
    });
    // This checks tool execution and logging, not interactive approval.
    creation.config.session_defaults.execution.approval = ApprovalMode::Auto;
    let created = client.create_agent(creation, options()).await.unwrap();
    let target = target(&created);
    let session = client.session(target.clone());
    let done = result(&session, &message.message_id).await;
    assert_eq!(done.data.info.outcome, Some(TurnOutcome::Completed));
    assert!(done.data.info.fault.is_none());
    let turn = done.source.turn_id;
    stop(&host).await;
    let requests = http.finish().await;
    assert_eq!(requests.len(), 3);
    // This is the actual production CommandResult sent back through Runtime,
    // not a scripted tool result or a process inferred from a returned future.
    let command = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == "original-shell")
        .unwrap();
    let command: Value = serde_json::from_str(command["content"].as_str().unwrap()).unwrap();
    assert_eq!(command["exit_code"], 7);
    assert_eq!(command["process_waited"], true);
    assert_eq!(command["cleanup_confirmed"], true);
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let process: Vec<_> = rows
        .iter()
        .filter(|row| row["event"] == "tool.process_observed")
        .collect();
    assert_eq!(process.len(), 1);
    assert_eq!(process[0]["tool_call_id"], "original-shell");
    assert_eq!(process[0]["turn_id"], turn.as_str());
    assert_eq!(process[0]["session_id"], target.session_id.as_str());
    assert_eq!(process[0]["attributes"]["tool_name"], "bash");
    for attribute in ["pid", "exit_code", "process_waited", "cleanup_confirmed"] {
        assert_eq!(process[0]["attributes"][attribute], command[attribute]);
    }
    assert!(process[0]["attributes"].get("signal").is_none());
    for (call, outcome) in [("original-shell", "success"), ("original-read", "failed")] {
        let finished: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == "tool.finished" && row["tool_call_id"] == call)
            .collect();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0]["turn_id"], turn.as_str());
        assert_eq!(finished[0]["agent_id"], target.agent_id.as_str());
        assert_eq!(finished[0]["attributes"]["outcome"], outcome);
        assert_eq!(finished[0]["level"], "DEBUG");
    }
    assert!(rows.iter().all(|row| row["level"] != "ERROR"));
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        SCRIPT,
        MISSING,
        "PRIVATE_TOOL_STDOUT",
        "PRIVATE_TOOL_STDERR",
        workspace.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
}
