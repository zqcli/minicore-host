//! Actual application startup binds read-only health; stdout remains RPC-only.
#[path = "support/mod.rs"]
mod support;
use serde_json::json;
use support::*;

#[tokio::test]
async fn application_host_info_exposes_the_installed_process_pipeline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config");
    write_config(&path, &config(None, false));
    let mut process = Process::start(&directory.path().join("root"), &path, true, None);
    let initial = process.rpc("initialize", json!({})).await;
    let info = process.rpc("host.info", json!({})).await;
    assert_eq!(info["host_id"], initial["host_id"]);
    assert_eq!(info["host_incarnation"], initial["host_incarnation"]);
    assert_eq!(
        info["logging"],
        json!({"status":"ok", "dropped_records":0, "write_failures":0})
    );
    assert!(info.get("capabilities").is_some());
    assert!(info.get("resources").is_some());
    process.send("host.shutdown", shutdown("graceful"));
    let output = process.finish().await;
    assert!(output.status.success());
    output.protocol_only();
    output.safe_diagnostics();
}
