//! Real binary, provider authentication, WSS and Runtime bash execution. Only
//! loopback model replies are scripted; no ambient credentials enter the child.
mod support;
#[allow(dead_code)]
mod tool_support;

use minicore_host_core::{ActivityEvent, LogLevel, ObservationEvent};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
use support::{HOST_KEY, MODEL_KEY, Process, message, shutdown};
use tool_support::*;

const ORDINARY: &str = "fixture-only-ordinary-environment";
const CALL: &str = "credential-environment";

fn isolated_process(
    root: &Path,
    home: &Path,
    config_dir: &Path,
    certificate: &Path,
    key: &Path,
) -> Process {
    let mut command = Command::new(env!("CARGO_BIN_EXE_minicore-host"));
    command
        .env_clear()
        .env("PATH", "/bin:/usr/bin")
        .env("HOME", home)
        .env("TMPDIR", home)
        .env("LANG", "C.UTF-8")
        .env("MINICORE_APP_MODEL_KEY", MODEL_KEY)
        .env("MINICORE_APP_HOST_KEY", HOST_KEY)
        .env("CREDENTIAL_TEST_ORDINARY", ORDINARY)
        .arg("--data-root")
        .arg(root)
        .arg("--stdio")
        .arg("--tls-cert")
        .arg(certificate)
        .arg("--tls-key")
        .arg(key);
    support::isolated_platform_environment(&mut command);
    #[cfg(windows)]
    command
        .env("TEMP", home)
        .env("TMP", home)
        .env("USERPROFILE", home);
    command.arg("--config-dir").arg(config_dir);
    Process::command(command)
}

#[tokio::test]
async fn binary_keeps_configured_credentials_out_of_tool_output_and_persistent_history() {
    let mut provider = Provider::start().await;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("data");
    let home = directory.path().join("empty-home");
    let path = directory.path().join("config");
    fs::create_dir(&home).unwrap();
    let (certificate, key, tls) = support::certificates(directory.path());
    let mut config = support::config(Some(provider.endpoint.clone()), true);
    config.tools = bindings(&["bash"]);
    config.logging.level = LogLevel::Trace;
    support::write_config(&path, &config);
    let mut process = isolated_process(&root, &home, &path, &certificate, &key);
    let initial = process.rpc("initialize", json!({})).await;
    assert_eq!(initial["can_operate"], true);
    // A verified-TLS WSS connection must authenticate with the configured key.
    let observer = connect(&mut process, tls.clone()).await;
    assert!(!observer.initial_info().can_operate);
    let public_config = process.rpc("host.config.get", json!({})).await;
    let created = process
        .rpc("agent.create", agent(directory.path(), &["bash"]))
        .await;
    let target = support::target(&created);
    let mut events = subscribe(&observer, &target).await;
    process
        .rpc(
            "message.send",
            message(&target, "check the explicitly named fixture environment"),
        )
        .await;

    // Provider::request verifies the actual Authorization header before making
    // each request available. The fixture sends names, never credential values.
    let first = provider.request().await;
    for value in [MODEL_KEY, HOST_KEY, ORDINARY] {
        assert!(!first.body.to_string().contains(value));
    }
    let turns = process.rpc("turn.list", json!({"target":target})).await;
    assert_eq!(turns["data"].as_array().unwrap().len(), 1);
    let turn = turns["data"][0]["turn_id"].clone();
    first.answer(tool(
        CALL,
        "bash",
        json!({"command":"printf 'model=%s\\nhost=%s\\nordinary=%s\\n' \"${MINICORE_APP_MODEL_KEY-unset}\" \"${MINICORE_APP_HOST_KEY-unset}\" \"${CREDENTIAL_TEST_ORDINARY-unset}\""}),
    ));
    let next = provider.request().await;
    let native = next.tool_result(CALL);
    assert_eq!(native["status"], "exited");
    assert_eq!(native["exit_code"], 0);
    assert_eq!(native["process_waited"], true);
    assert_eq!(native["cleanup_confirmed"], true);
    assert_eq!(native["cleanup_errors"], json!([]));
    assert_eq!(native["stdout"]["output"]["encoding"], "utf8");
    assert_eq!(
        native["stderr"]["output"],
        json!({"encoding":"utf8","data":""})
    );
    let stdout = native["stdout"]["output"]["data"].as_str().unwrap();

    let mut streamed = String::new();
    let mut stderr = String::new();
    let finished = loop {
        match event(&mut events).await {
            ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) => {
                assert_eq!(source.tool_call_id.0, CALL);
                let stream = match data.channel.as_str() {
                    "stdout" => &mut streamed,
                    "stderr" => &mut stderr,
                    other => panic!("unexpected output channel {other}"),
                };
                assert_eq!(data.start, stream.len() as u64);
                stream.push_str(&data.text);
                assert_eq!(data.end, stream.len() as u64);
            }
            ObservationEvent::Activity(ActivityEvent::ToolFinished { source, data }) => {
                assert_eq!(source.tool_call_id.0, CALL);
                assert_eq!(json!(source.turn.turn_id), turn);
                break json!(data.result);
            }
            _ => {}
        }
    };
    assert_eq!(streamed, stdout);
    assert!(stderr.is_empty());
    let tool_ref = json!({"turn_id":turn,"tool_call_id":CALL});
    let call = process
        .rpc("toolcall.get", json!({"target":target,"tool":tool_ref}))
        .await;
    assert_eq!(call["data"]["state"], "finished");
    assert_eq!(call["data"]["result"], finished);
    assert_eq!(
        serde_json::from_str::<Value>(finished["output"]["content"].as_str().unwrap()).unwrap(),
        native
    );
    let retained = process.rpc("toolcall.output.read", json!({"target":target,"tool":tool_ref,"request":{"channel":"stdout","start":0,"end":stdout.len(),"max_bytes":4096}})).await;
    assert_eq!(retained["data"]["text"], stdout);
    let next_body = next.body.clone();
    next.answer(answer("environment check complete"));
    let completed = process
        .rpc("turn.wait", json!({"target":target,"turn_id":turn}))
        .await;
    assert_eq!(completed["data"]["info"]["outcome"], "completed");
    assert_eq!(completed["data"]["info"]["persistence"], "persisted");
    assert_eq!(completed["data"]["info"]["cleanup_faults"], json!([]));
    let before = history(&mut process, &target).await;
    assert_eq!(history_result(&before, CALL), &finished);
    process.send("host.shutdown", shutdown("graceful"));
    let stopped = process.finish().await;
    assert!(stopped.status.success());
    stopped.protocol_only();
    stopped.safe_diagnostics();
    observer.close();

    let saved = fs::read_to_string(history_path(&root, &target)).unwrap();
    let persisted: Vec<Value> = saved
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|entry| {
            entry["entry"]["kind"] == "runtime"
                && entry["entry"]["data"]["item"]["type"] == "tool_result"
        })
        .collect();
    assert_eq!(persisted.len(), 1);
    let result = &persisted[0]["entry"]["data"]["item"]["data"];
    assert_eq!(result["call_id"], CALL);
    assert_eq!(result["outcome"], finished["outcome"]);
    assert_eq!(result["output"], finished["output"]);
    // Reopen the same actual binary and saved root, not a storage test double.
    let mut reopened = isolated_process(&root, &home, &path, &certificate, &key);
    let current = reopened.rpc("initialize", json!({})).await;
    assert_eq!(current["host_id"], initial["host_id"]);
    assert_ne!(current["host_incarnation"], initial["host_incarnation"]);
    assert_eq!(history(&mut reopened, &target).await, before);
    assert_eq!(
        fs::read_to_string(history_path(&root, &target)).unwrap(),
        saved
    );
    let observer = connect(&mut reopened, tls).await;
    observer.close();
    reopened.send("host.shutdown", shutdown("graceful"));
    let stopped = reopened.finish().await;
    assert!(stopped.status.success());
    stopped.protocol_only();
    stopped.safe_diagnostics();

    // Keep this after persistence/reopen: a red run proves the same actual
    // inherited output travelled through every surface above before failing.
    assert_eq!(
        stdout,
        format!("model=unset\nhost=unset\nordinary={ORDINARY}\n")
    );
    for surface in [
        public_config,
        native,
        next_body,
        call,
        retained,
        before,
        json!(saved),
    ] {
        for secret in [MODEL_KEY, HOST_KEY] {
            assert!(
                !surface.to_string().contains(secret),
                "configured credential escaped"
            );
        }
    }
}
