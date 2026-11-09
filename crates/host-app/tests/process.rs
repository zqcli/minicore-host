//! Real executables and local HTTP/TLS fixtures; no live provider or TUI claims.
#![cfg(unix)]
mod support;
use minicore_host_core::LogOutput;
use minicore_host_protocol::*;
use minicore_host_transport::{HostClient, tls};
use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use support::*;

#[tokio::test]
async fn help_invalid_arguments_and_invalid_configuration_never_write_stdout_or_raw_values() {
    for args in [
        vec!["--help"],
        vec!["--private-invalid-option"],
        vec!["--data-root", "--stdio"],
        vec![],
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minicore-host"));
        command.args(&args);
        let output = Process::command(command).finish().await;
        assert!(output.stdout.is_empty());
        assert_eq!(output.status.success(), args == ["--help"]);
        assert!(
            !output
                .stderr
                .join("\n")
                .contains("--private-invalid-option")
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.json");
    std::fs::write(&path, format!(r#"{{"private":"{MODEL_KEY}""#)).unwrap();
    let output = Process::start(&dir.path().join("root"), Some(&path), true, None)
        .finish()
        .await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    output.safe_diagnostics();
    assert!(!output.stderr.join("\n").contains(path.to_str().unwrap()));
    assert!(!dir.path().join("root/host.json").exists());
}

#[tokio::test]
async fn actual_identity_is_saved_new_incarnation_is_generated_and_root_is_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    let original = config(None, false);
    write_config(&path, &original);
    let mut first = Process::start(&root, Some(&path), true, None);
    let initial = first.rpc("initialize", json!({})).await;
    assert_eq!(initial["can_operate"], true);
    assert_eq!(initial["control"]["revision"], 1);
    let conflict = Process::start(&root, None, true, None).finish().await;
    assert!(!conflict.status.success());
    assert!(conflict.stdout.is_empty());
    let before = std::fs::read(root.join("host.json")).unwrap();
    first.send("host.shutdown", shutdown("graceful"));
    let output = first.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
    assert!(output.events().iter().any(|event| event["event"] == "host.stopped" && event["host_id"] == initial["host_id"]));
    let mut changed = original.clone();
    changed.name = "Initial defaults do not overwrite saved config".into();
    write_config(&path, &changed);
    let mut second = Process::start(&root, Some(&path), true, None);
    let reopened = second.rpc("initialize", json!({})).await;
    assert_eq!(reopened["host_id"], initial["host_id"]);
    assert_ne!(reopened["host_incarnation"], initial["host_incarnation"]);
    let saved = second.rpc("host.config.get", json!({})).await;
    assert_eq!(saved["config"]["name"], original.name);
    assert_eq!(std::fs::read(root.join("host.json")).unwrap(), before);
    second.send("host.shutdown", shutdown("graceful"));
    assert!(second.finish().await.status.success());
}

#[tokio::test]
async fn stdio_message_executes_production_http_adapter_and_persists_real_output() {
    let mut http = HttpFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    write_config(&path, &config(Some(http.endpoint.clone()), false));
    let mut child = Process::start(&root, Some(&path), true, None);
    child.rpc("initialize", json!({})).await;
    let created = child.rpc("agent.create", agent_request()).await;
    let target = target(&created);
    let accepted = child
        .rpc("message.send", message(&target, "local fixture input"))
        .await;
    assert_eq!(accepted["result"]["delivery"], "accepted");
    let request = http.request().await;
    assert_eq!(request["model"], "local-http-fixture");
    assert!(request.to_string().contains("local fixture input"));
    let turns = child.rpc("turn.list", json!({"target":target})).await;
    assert_eq!(turns.as_array().unwrap().len(), 1);
    http.finish_response();
    let result = child
        .rpc(
            "turn.wait",
            json!({"target":target,"turn_id":turns[0]["id"]}),
        )
        .await;
    assert_eq!(result["info"]["outcome"], "completed");
    assert!(result.to_string().contains("actual local HTTP response"));
    child.send("host.shutdown", shutdown("graceful"));
    let output = child.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
    let history = root
        .join("agents")
        .join(target["agent_id"].as_str().unwrap())
        .join("sessions")
        .join(target["session_id"].as_str().unwrap())
        .join("history.jsonl");
    assert!(
        std::fs::read_to_string(history)
            .unwrap()
            .contains("actual local HTTP response")
    );
}

#[tokio::test]
async fn graceful_process_shutdown_keeps_root_until_accepted_http_work_really_finishes() {
    let mut http = HttpFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    write_config(&path, &config(Some(http.endpoint.clone()), false));
    let mut child = Process::start(&root, Some(&path), true, None);
    child.rpc("initialize", json!({})).await;
    let created = child.rpc("agent.create", agent_request()).await;
    let target = target(&created);
    child
        .rpc("message.send", message(&target, "finish owned work"))
        .await;
    http.request().await;
    let receipt = child.rpc("host.shutdown", shutdown("graceful")).await;
    assert_eq!(receipt["result"]["lifecycle"], "draining");
    child.signal("-TERM");
    child.event("host.shutdown_requested").await;
    let facts = child.rpc("host.info", json!({})).await;
    assert_eq!(facts["lifecycle"], "draining");
    assert_eq!(facts["shutdown_strategy"], "graceful");
    let conflict = Process::start(&root, None, true, None).finish().await;
    assert!(!conflict.status.success());
    http.finish_response();
    let output = child.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
    let mut reopened = Process::start(&root, None, true, None);
    reopened.rpc("initialize", json!({})).await;
    reopened.send("host.shutdown", shutdown("graceful"));
    assert!(reopened.finish().await.status.success());
}

#[tokio::test]
async fn configured_wss_requires_valid_tls_and_key_and_first_real_client_gets_rights() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    let (cert, key, tls) = certificates(dir.path());
    write_config(&path, &config(None, true));
    let missing_tls = Process::start(&root, Some(&path), false, None)
        .finish()
        .await;
    assert!(!missing_tls.status.success());
    assert!(missing_tls.stdout.is_empty());
    assert!(!root.join("host.json").exists());
    let mut child = Process::start(&root, Some(&path), false, Some((&cert, &key)));
    let started = child.event("host.started").await;
    let port = started["attributes"]["listen_port"].as_u64().unwrap();
    let url = format!("wss://localhost:{port}/");
    let untrusted = Arc::new(
        tls::ClientConfig::builder()
            .with_root_certificates(tls::RootCertStore::empty())
            .with_no_client_auth(),
    );
    assert!(
        HostClient::wss(&url, untrusted, HOST_KEY, options())
            .await
            .is_err()
    );
    assert!(
        HostClient::wss(
            &format!("wss://127.0.0.1:{port}/"),
            tls.clone(),
            HOST_KEY,
            options()
        )
        .await
        .is_err()
    );
    assert!(
        HostClient::wss(&url, tls.clone(), "fixture-invalid-key", options())
            .await
            .is_err()
    );
    let first = HostClient::wss(&url, tls.clone(), HOST_KEY, options())
        .await
        .unwrap();
    assert!(first.initial_info().can_operate);
    assert_eq!(first.initial_info().control.revision, 1);
    let observer = HostClient::wss(&url, tls, HOST_KEY, options())
        .await
        .unwrap();
    assert!(!observer.initial_info().can_operate);
    assert!(
        matches!(observer.shutdown(HostShutdownRequest { operation_id: OperationId::new().unwrap(), strategy: HostShutdownStrategy::Force },options()).await,Err(CallError::Business(fault)) if fault.kind() == Some(FaultKind::HostAccessReadOnly))
    );
    let _receipt = first
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(),
        )
        .await;
    let output = child.finish().await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    output.safe_diagnostics();
}

#[tokio::test]
async fn stdio_eof_releases_only_connection_and_wss_observes_accepted_work_then_new_operator() {
    let mut http = HttpFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    let (cert, key, tls) = certificates(dir.path());
    write_config(&path, &config(Some(http.endpoint.clone()), true));
    let mut child = Process::start(&root, Some(&path), true, Some((&cert, &key)));
    let initial = child.rpc("initialize", json!({})).await;
    assert_eq!(initial["can_operate"], true);
    let started = child.event("host.started").await;
    let url = format!("wss://localhost:{}/", started["attributes"]["listen_port"]);
    let observer = HostClient::wss(&url, tls.clone(), HOST_KEY, options())
        .await
        .unwrap();
    assert!(!observer.initial_info().can_operate);
    let created = child.rpc("agent.create", agent_request()).await;
    let address = target(&created);
    child
        .rpc(
            "message.send",
            message(&address, "survive client disconnect"),
        )
        .await;
    http.request().await;
    child.close_input();
    child.event("transport.stdio_disconnected").await;
    assert!(child.alive());
    assert_eq!(
        observer.info(options()).await.unwrap().lifecycle,
        HostLifecycle::Running
    );
    let replacement = HostClient::wss(&url, tls, HOST_KEY, options())
        .await
        .unwrap();
    assert!(replacement.initial_info().can_operate);
    assert_eq!(replacement.initial_info().control.revision, 3);
    let session = replacement.session(serde_json::from_value(address).unwrap());
    let turns = session.turns(options()).await.unwrap().data;
    assert_eq!(turns.len(), 1);
    http.finish_response();
    assert_eq!(
        session
            .wait_turn(turns[0].turn_id.clone(), options())
            .await
            .unwrap()
            .data
            .info
            .outcome,
        Some(TurnOutcome::Completed)
    );
    let _ = replacement
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(),
        )
        .await;
    let output = child.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
}

#[tokio::test]
async fn repeated_signals_do_not_force_paused_backlog_but_explicit_rpc_force_finishes() {
    let http = HttpFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    write_config(&path, &config(Some(http.endpoint.clone()), false));
    let mut child = Process::start(&root, Some(&path), true, None);
    child.rpc("initialize", json!({})).await;
    let created = child.rpc("agent.create", agent_request()).await;
    let address = target(&created);
    child
        .rpc(
            "session.pause",
            json!({"target":address,"operation_id":OperationId::new().unwrap()}),
        )
        .await;
    child
        .rpc("message.send", message(&address, "paused retained input"))
        .await;
    for signal in ["-TERM", "-INT"] {
        child.signal(signal);
        child.event("host.shutdown_requested").await;
        let info = child.rpc("host.info", json!({})).await;
        assert_eq!(info["lifecycle"], "draining");
        assert_eq!(info["shutdown_strategy"], "graceful");
    }
    assert!(child.alive());
    child.send("host.shutdown", shutdown("force"));
    let output = child.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
}

#[tokio::test]
async fn failed_original_save_stays_stopping_and_external_kill_is_not_logged_as_stopped() {
    let http = HttpFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    write_config(&path, &config(Some(http.endpoint.clone()), false));
    let mut child = Process::start(&root, Some(&path), true, None);
    child.rpc("initialize", json!({})).await;
    std::fs::write(
        root.join("agents"),
        b"controlled fixture storage obstruction",
    )
    .unwrap();
    let receipt = child.rpc("agent.create", agent_request()).await;
    assert!(receipt["agent_id"].is_null());
    child.rpc("host.shutdown", shutdown("force")).await;
    child.event("host.shutdown_blocked").await;
    std::fs::remove_file(root.join("agents")).unwrap();
    let repeated = child.rpc("host.shutdown", shutdown("graceful")).await;
    assert_eq!(repeated["result"]["strategy"], "force");
    assert_eq!(repeated["result"]["lifecycle"], "stopping");
    let conflict = Process::start(&root, None, true, None).finish().await;
    assert!(!conflict.status.success());
    assert!(child.alive());
    child.signal("-KILL");
    let output = child.finish().await;
    assert!(!output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
    assert!(
        !output
            .events()
            .iter()
            .any(|record| record["event"] == "host.stopped")
    );
}

#[tokio::test]
async fn configured_file_diagnostics_are_exclusive_across_data_roots_and_released_after_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let log = dir.path().join("output.jsonl");
    let mut settings = config(None, false);
    settings.logging.output = LogOutput::File {
        path: log.to_str().unwrap().into(),
    };
    write_config(&path, &settings);
    let mut first = Process::start(&dir.path().join("one"), Some(&path), true, None);
    first.rpc("initialize", json!({})).await;
    let second = Process::start(&dir.path().join("two"), Some(&path), true, None)
        .finish()
        .await;
    assert!(!second.status.success());
    assert!(second.stdout.is_empty());
    first.send("host.shutdown", shutdown("graceful"));
    let output = first.finish().await;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    output.protocol_only();
    let rows: Vec<Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(rows.iter().any(|record| record["event"] == "host.started"));
    assert!(rows.iter().any(|record| record["event"] == "host.stopped"));
    let mut second = Process::start(&dir.path().join("two"), Some(&path), true, None);
    second.rpc("initialize", json!({})).await;
    second.send("host.shutdown", shutdown("graceful"));
    assert!(second.finish().await.status.success());
}

#[tokio::test]
async fn missing_credentials_bad_tls_and_unavailable_log_output_fail_without_saved_identity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let root = dir.path().join("root");
    let mut settings = config(Some("http://127.0.0.1:1/v1/chat/completions".into()), false);
    settings.models[0].api_key_env = "MINICORE_APP_ABSENT_FIXTURE_KEY".into();
    write_config(&path, &settings);
    let mut command = Command::new(env!("CARGO_BIN_EXE_minicore-host"));
    command
        .arg("--data-root")
        .arg(&root)
        .arg("--config")
        .arg(&path)
        .arg("--stdio")
        .env_remove("MINICORE_APP_ABSENT_FIXTURE_KEY");
    let output = Process::command(command).finish().await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    output.safe_diagnostics();
    assert!(!root.join("host.json").exists());

    let mut settings = config(None, false);
    settings.logging.output = LogOutput::File {
        path: dir
            .path()
            .join(MODEL_KEY)
            .join("missing/output.jsonl")
            .to_str()
            .unwrap()
            .into(),
    };
    write_config(&path, &settings);
    let output = Process::start(&root, Some(&path), true, None)
        .finish()
        .await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    output.safe_diagnostics();
    assert!(!root.join("host.json").exists());

    let cert = dir.path().join("invalid-cert.pem");
    let key = dir.path().join("invalid-key.pem");
    std::fs::write(&cert, HOST_KEY).unwrap();
    std::fs::write(&key, MODEL_KEY).unwrap();
    write_config(&path, &config(None, true));
    let output = Process::start(&root, Some(&path), true, Some((&cert, &key)))
        .finish()
        .await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    output.safe_diagnostics();
    assert!(!root.join("host.json").exists());
    // Failed startup released the real root, so a corrected configuration starts normally.
    write_config(&path, &config(None, false));
    let mut child = Process::start(&root, Some(&path), true, None);
    assert_eq!(
        child.rpc("initialize", json!({})).await["can_operate"],
        true
    );
    child.send("host.shutdown", shutdown("graceful"));
    assert!(child.finish().await.status.success());
}

#[tokio::test]
async fn broken_stdio_frame_does_not_shutdown_the_host_or_disable_the_wss_listener() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let (cert, key, tls) = certificates(dir.path());
    write_config(&path, &config(None, true));
    let mut child = Process::start(
        &dir.path().join("root"),
        Some(&path),
        true,
        Some((&cert, &key)),
    );
    child.rpc("initialize", json!({})).await;
    let started = child.event("host.started").await;
    child.raw_input(b"{\"jsonrpc\":");
    child.close_input();
    child.event("transport.stdio_disconnected").await;
    let url = format!("wss://localhost:{}/", started["attributes"]["listen_port"]);
    let client = HostClient::wss(&url, tls, HOST_KEY, options())
        .await
        .unwrap();
    assert!(client.initial_info().can_operate);
    assert_eq!(
        client.info(options()).await.unwrap().lifecycle,
        HostLifecycle::Running
    );
    let _ = client
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(),
        )
        .await;
    let output = child.finish().await;
    assert!(!output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
    assert!(
        output
            .events()
            .iter()
            .any(|event| event["event"] == "host.stopped")
    );
}

fn saved_log_output(root: &std::path::Path, output: &std::path::Path) -> Vec<u8> {
    let path = root.join("host.json");
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["data"]["config"]["logging"]["output"] = json!({"kind":"file","path":output});
    let bytes = serde_json::to_vec(&record).unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

#[tokio::test]
async fn log_targets_cannot_append_to_resolved_host_metadata_or_owned_session_history() {
    let mut http = HttpFixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    write_config(&path, &config(Some(http.endpoint.clone()), false));
    let mut child = Process::start(&root, Some(&path), true, None);
    child.rpc("initialize", json!({})).await;
    let created = child.rpc("agent.create", agent_request()).await;
    let address = target(&created);
    child
        .rpc(
            "message.send",
            message(&address, "preserve this original history"),
        )
        .await;
    http.request().await;
    let turns = child.rpc("turn.list", json!({"target":address})).await;
    http.finish_response();
    child
        .rpc(
            "turn.wait",
            json!({"target":address,"turn_id":turns[0]["id"]}),
        )
        .await;
    child.send("host.shutdown", shutdown("graceful"));
    assert!(child.finish().await.status.success());
    let history = root
        .join("agents")
        .join(address["agent_id"].as_str().unwrap())
        .join("sessions")
        .join(address["session_id"].as_str().unwrap())
        .join("history.jsonl");
    let history_before = std::fs::read(&history).unwrap();
    assert!(!history_before.is_empty());
    let alias = dir.path().join("metadata-log-alias");
    std::os::unix::fs::symlink(root.join("host.json"), &alias).unwrap();
    for destination in [root.join("host.json"), history.clone(), alias] {
        let metadata_before = saved_log_output(&root, &destination);
        let output = Process::start(&root, None, true, None).finish().await;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        output.safe_diagnostics();
        assert_eq!(
            std::fs::read(root.join("host.json")).unwrap(),
            metadata_before
        );
        assert_eq!(std::fs::read(&history).unwrap(), history_before);
        assert!(
            output
                .stderr
                .join("\n")
                .contains("conflicts with Host business storage")
        );
        assert!(
            !output
                .stderr
                .join("\n")
                .contains(destination.to_str().unwrap())
        );
    }
}

#[tokio::test]
async fn dedicated_log_directory_inside_data_root_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let path = dir.path().join("config.json");
    std::fs::create_dir_all(root.join("logs")).unwrap();
    let log = root.join("logs/host.jsonl");
    let mut settings = config(None, false);
    settings.logging.output = LogOutput::File {
        path: log.to_str().unwrap().into(),
    };
    write_config(&path, &settings);
    let mut child = Process::start(&root, Some(&path), true, None);
    child.rpc("initialize", json!({})).await;
    child.send("host.shutdown", shutdown("graceful"));
    let output = child.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    assert!(output.stderr.is_empty());
    let records: Vec<Value> = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        records
            .iter()
            .any(|record| record["event"] == "host.stopped")
    );
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(root.join("host.json")).unwrap()).unwrap();
    assert!(metadata["id"].is_string());
}
