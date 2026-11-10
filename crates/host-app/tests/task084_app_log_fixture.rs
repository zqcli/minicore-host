//! Real process diagnostics; no provider or TUI acceptance claim.
#![cfg(unix)]
mod support;

use minicore_host_core::LogLevel;
use serde_json::{Value, json};
use std::process::Command;
use support::*;

#[tokio::test]
async fn process_readiness_selects_its_component_after_owner_started() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let (certificate, key, _) = certificates(directory.path());
    let mut settings = config(None, true);
    settings.logging.level = LogLevel::Trace;
    write_config(&path, &settings);
    let mut command = Command::new(env!("CARGO_BIN_EXE_minicore-host"));
    command
        .env_clear()
        .env("MINICORE_APP_HOST_KEY", HOST_KEY)
        .arg("--data-root")
        .arg(directory.path().join("data"))
        .arg("--config")
        .arg(&path)
        .arg("--stdio")
        .arg("--tls-cert")
        .arg(certificate)
        .arg("--tls-key")
        .arg(key);
    let mut process = Process::command(command);
    let selected = process.event("host.process", "host.started").await;
    let initial = process.rpc("initialize", json!({})).await;
    process.send("host.shutdown", shutdown("graceful"));
    let output = process.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
    let records = output.events();
    let started: Vec<&Value> = records
        .iter()
        .filter(|record| record["event"] == "host.started")
        .collect();
    assert_eq!(started.len(), 2);
    for record in &started {
        assert_eq!(record["scope"], "HOST");
        assert_eq!(record["host_id"], initial["host_id"]);
        assert_eq!(record["host_incarnation"], initial["host_incarnation"]);
    }
    assert_eq!(started[0]["component"], "host.owner");
    assert_eq!(
        started[0]["attributes"],
        json!({"mode":"persistent","phase":"running"})
    );
    assert_eq!(started[1]["component"], "host.process");
    assert_eq!(started[1]["attributes"]["stdio"], true);
    assert_eq!(started[1]["attributes"]["wss"], true);
    assert!(started[1]["attributes"]["listen_port"].as_u64().unwrap() > 0);
    assert_eq!(selected, *started[1]);
}
