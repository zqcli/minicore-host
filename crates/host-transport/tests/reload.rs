//! Canonical reload uses the same real owner from typed, duplex stdio and TLS
//! WebSocket callers. Credential and scheduling fixtures are named explicitly.
mod common;
mod os_pipe;
mod reload_common;
mod wss_common;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use reload_common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn production_default_preparer_is_available_to_typed_and_stdio_clients() {
    for transport in 0..2 {
        let builder = HostResourceBuilder::builtins().unwrap();
        let original = HostConfig::default();
        let host =
            Host::memory_configured(builder.prepare(original.clone()).await.unwrap()).unwrap();
        let peer = Connected::new(host, transport).await;
        let mut candidate = original;
        candidate.name = "Production assembler reload".into();
        let returned = peer
            .client
            .reload(request(candidate.clone(), 0), options())
            .await
            .unwrap();
        assert_eq!(
            returned.result,
            HostReloadResult::Preparing { config_revision: 0 }
        );
        assert_eq!(
            terminal(&peer.client, returned.operation_id).await.result,
            HostReloadResult::Applied { config_revision: 1 }
        );
        assert_eq!(
            peer.client.config(options()).await.unwrap().config,
            candidate
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn all_transports_keep_immutable_receipts_and_distinct_duplicate_invocations() {
    for transport in 0..3 {
        let (host, preparer) = CredentialFixture::host(config(), true).await;
        let peer = Connected::new(host, transport).await;
        let mut candidate = config();
        candidate.name = "First accepted candidate".into();
        let first = peer
            .client
            .reload(request(candidate.clone(), 0), options())
            .await
            .unwrap();
        preparer.started().await;
        assert_eq!(
            first.result,
            HostReloadResult::Preparing { config_revision: 0 }
        );
        assert_eq!(
            peer.client
                .receipt(first.operation_id.clone(), options())
                .await
                .unwrap(),
            Receipt::HostReload(first.clone())
        );
        let rejected = request(candidate.clone(), 0);
        let rejected_id = rejected.operation_id.clone();
        assert_eq!(
            fault(peer.client.reload(rejected, options()).await.unwrap_err()).kind(),
            Some(FaultKind::HostConfigConflict)
        );
        assert_eq!(
            fault(
                peer.client
                    .receipt(rejected_id, options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostReceiptNotFound)
        );
        assert_eq!(preparer.count(), 1);
        preparer.release.add_permits(1);
        let applied = terminal(&peer.client, first.operation_id.clone()).await;
        assert_eq!(
            applied.result,
            HostReloadResult::Applied { config_revision: 1 }
        );
        assert_eq!(applied.host_id, peer.client.initial_info().host_id);
        assert_eq!(
            applied.host_incarnation,
            peer.client.initial_info().host_incarnation
        );
        assert_eq!(
            first.result,
            HostReloadResult::Preparing { config_revision: 0 }
        );
        candidate.name = "Independent duplicate ID".into();
        let duplicate = peer
            .client
            .reload(
                HostReloadRequest {
                    operation_id: first.operation_id.clone(),
                    expected_revision: 1,
                    candidate: candidate.clone(),
                },
                options(),
            )
            .await
            .unwrap();
        assert_eq!(
            duplicate.result,
            HostReloadResult::Preparing { config_revision: 1 }
        );
        preparer.started().await;
        preparer.release.add_permits(1);
        revision(&peer.client, 2).await;
        assert_eq!(preparer.count(), 2);
        assert_eq!(
            peer.client.config(options()).await.unwrap().config,
            candidate
        );
        assert_eq!(
            fault(
                peer.client
                    .receipt(first.operation_id, options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostReceiptAmbiguous)
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn all_transports_check_readonly_and_stale_revision_before_preparation() {
    for transport in 0..3 {
        let (host, preparer) = CredentialFixture::host(config(), false).await;
        let operator = host.connect().unwrap();
        let reader = Connected::new(host.clone(), transport).await;
        assert!(!reader.client.initial_info().can_operate);
        for close in [false, true] {
            if close {
                operator.close();
            }
            assert_eq!(
                fault(
                    reader
                        .client
                        .reload(request(config(), 0), options())
                        .await
                        .unwrap_err()
                )
                .kind(),
                Some(FaultKind::HostAccessReadOnly)
            );
        }
        assert_eq!(preparer.count(), 0);
        let writer = Connected::new(host, transport).await;
        assert!(writer.client.initial_info().can_operate);
        let returned = writer
            .client
            .reload(request(config(), 0), options())
            .await
            .unwrap();
        assert_eq!(
            terminal(&reader.client, returned.operation_id).await.result,
            HostReloadResult::Applied { config_revision: 1 }
        );
        assert_eq!(
            fault(
                writer
                    .client
                    .reload(request(config(), 0), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostConfigConflict)
        );
        assert_eq!(preparer.count(), 1);
        reader.finish().await;
        writer.finish().await;
    }
}

#[tokio::test]
async fn all_transports_retain_preparation_faults_and_whole_candidate_restart_reasons() {
    for transport in 0..3 {
        let (host, preparer) = CredentialFixture::host(config(), false).await;
        let peer = Connected::new(host, transport).await;
        let before = peer.client.config(options()).await.unwrap();
        let mut invalid = config();
        invalid.network.access_key_env = Some(SOURCE_B.into());
        invalid.tools = vec![ToolBinding {
            name: "missing_static_tool".into(),
        }];
        let returned = peer
            .client
            .reload(request(invalid, 0), options())
            .await
            .unwrap();
        let failed = terminal(&peer.client, returned.operation_id).await;
        let HostReloadResult::Failed {
            config_revision,
            error,
        } = failed.result
        else {
            panic!("preparation should fail");
        };
        assert_eq!(config_revision, 0);
        assert_eq!(error.kind(), Some(FaultKind::HostConfigInvalid));
        assert_eq!(peer.client.config(options()).await.unwrap(), before);
        assert_eq!(preparer.count(), 1);
        let mut restart = config();
        restart.name = "Must not publish this name".into();
        restart.logging.level = LogLevel::Debug;
        restart.network.listen = Some("127.0.0.1:9999".parse().unwrap());
        restart.network.access_key_env = Some(SOURCE_B.into());
        let receipt = peer
            .client
            .reload(request(restart, 0), options())
            .await
            .unwrap();
        assert_eq!(
            receipt.result,
            HostReloadResult::RestartRequired {
                config_revision: 0,
                fields: vec![RestartField::NetworkListen, RestartField::Logging]
            }
        );
        assert_eq!(
            peer.client
                .receipt(receipt.operation_id.clone(), options())
                .await
                .unwrap(),
            Receipt::HostReload(receipt)
        );
        assert_eq!(preparer.count(), 1);
        assert_eq!(peer.client.config(options()).await.unwrap(), before);
        peer.finish().await;
    }
}

#[tokio::test]
async fn hosts_without_an_initial_preparer_remain_unavailable_on_every_transport() {
    for transport in 0..3 {
        let (host, _) = common::fixture();
        host.configure_network_access(Some(KEY_A.into())).unwrap();
        let peer = Connected::new(host, transport).await;
        assert_eq!(
            fault(
                peer.client
                    .reload(request(HostConfig::default(), 0), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostMethodUnavailable)
        );
        peer.finish().await;
    }
}

#[tokio::test]
async fn reload_rpc_strict_shape_and_business_errors_match_on_duplex_and_tls() {
    let (host, preparer) = CredentialFixture::host(config(), false).await;
    let _operator = host.connect().unwrap();
    let mut pipe = RawStdio::new(host.clone());
    let server = tls_server(host).await;
    let mut wss = server.raw().await;
    assert_eq!(
        pipe.request(0, "initialize", json!({})).await["result"]["can_operate"],
        false
    );
    assert_eq!(wss.initialize(KEY_A).await["result"]["can_operate"], false);
    let valid = serde_json::to_value(request(config(), 0)).unwrap();
    let mut bad = vec![json!({}), json!({"request": valid.clone()})];
    for field in ["operation_id", "expected_revision", "candidate"] {
        let mut value = valid.clone();
        value.as_object_mut().unwrap().remove(field);
        bad.push(value);
    }
    for (field, value) in [
        ("wait_timeout_ms", json!(1)),
        ("caller", json!({"can_operate":true})),
        ("access_key", json!(KEY_A)),
        ("host_id", json!(HostId::new().unwrap())),
        ("method", json!("host.reload")),
    ] {
        let mut params = valid.clone();
        params[field] = value;
        bad.push(params);
    }
    for (field, value) in [
        ("operation_id", json!("not-an-operation-id")),
        ("expected_revision", json!(-1)),
        ("expected_revision", json!(1.5)),
        ("candidate", Value::Null),
    ] {
        let mut params = valid.clone();
        params[field] = value;
        bad.push(params);
    }
    for (path, value) in [
        ("access_key", json!(KEY_A)),
        ("preparer", json!({})),
        ("wait_timeout_ms", json!(1)),
    ] {
        let mut params = valid.clone();
        params["candidate"][path] = value;
        bad.push(params);
    }
    let mut nested = valid.clone();
    nested["candidate"]["network"]["access_key"] = json!(KEY_A);
    bad.push(nested);
    for (i, params) in bad.into_iter().enumerate() {
        let local = pipe
            .request(i as i64 + 1, "host.reload", params.clone())
            .await;
        let network = wss.request(i as i64 + 1, "host.reload", params).await;
        assert_eq!(local, network);
        assert_eq!(local["error"]["code"], -32602, "{local}");
        assert!(!serde_json::to_string(&local).unwrap().contains(KEY_A));
    }
    let local = pipe.request(50, "host.reload", valid.clone()).await;
    assert_eq!(local, wss.request(50, "host.reload", valid).await);
    assert_eq!(local["error"]["code"], -32000);
    assert_eq!(
        local["error"]["data"]["code"],
        FaultKind::HostAccessReadOnly.code()
    );
    assert_eq!(
        pipe.request(51, "host.reload_config", json!({})).await["error"]["code"],
        -32601
    );
    assert_eq!(
        wss.request(51, "host.reload_config", json!({})).await["error"]["code"],
        -32601
    );
    assert_eq!(preparer.count(), 0);
    pipe.finish().await;
}
