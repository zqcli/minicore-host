//! Raw stdio/TLS parameters retain canonical identities and strict boundaries.
mod common;
mod compaction_common;
mod os_pipe;
mod wss_common;

use compaction_common::*;
use minicore_host_protocol::*;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Semaphore;

enum Raw {
    Stdio(common::RawPeer),
    Wss(Box<wss_common::RawPeer>),
}
impl Raw {
    async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        let result = match self {
            Self::Stdio(peer) => peer.request(json!(id), method, params).await,
            Self::Wss(peer) => peer.request(id, method, params).await,
        };
        assert_eq!(result["id"], id);
        result
    }
}

#[tokio::test]
async fn stdio_and_tls_reject_forged_or_incomplete_params_and_keep_rpc_ids_separate() {
    for transport in 1..3 {
        let gate = Arc::new(Semaphore::new(0));
        let mut replies = seed_replies();
        replies.push(gated(SUMMARY, &gate));
        let mut http = HttpFixture::start(replies).await;
        let host = host(&http).await;
        let operator = Connected::new(host.clone(), 0).await;
        let session = seed(&operator.client, &mut http).await;
        let target = session.target().clone();
        session.pause(control(), options()).await.unwrap();
        operator.client.close();
        let mut pipe = None;
        let mut tls = None;
        let mut raw = if transport == 1 {
            let (mut peer, task) = common::raw(host);
            assert_eq!(peer.initialize().await["result"]["can_operate"], true);
            pipe = Some(task);
            Raw::Stdio(peer)
        } else {
            let server = tls_server(host).await;
            let mut peer = server.raw().await;
            assert_eq!(peer.initialize(KEY).await["result"]["can_operate"], true);
            tls = Some(server);
            Raw::Wss(Box::new(peer))
        };
        let original = compact();
        let cancel = cancel(original.operation_id.clone());
        let start = json!({"target":target,"operation_id":original.operation_id});
        let stop = json!({"target":target,"operation_id":cancel.operation_id,"target_operation_id":cancel.target_operation_id});
        let mut id = 10;
        for (method, valid, required) in [
            ("session.compact", &start, vec!["target", "operation_id"]),
            (
                "session.compact.cancel",
                &stop,
                vec!["target", "operation_id", "target_operation_id"],
            ),
        ] {
            let mut invalid = vec![Value::Null, json!({"request":valid}), json!([])];
            for field in required {
                let mut params = valid.clone();
                params.as_object_mut().unwrap().remove(field);
                invalid.push(params);
                let mut params = valid.clone();
                params[field] = Value::Null;
                invalid.push(params);
            }
            for (field, value) in [
                ("wait_timeout_ms", json!(1)),
                ("caller", json!({"can_operate":true})),
                ("access_key", json!(KEY)),
                ("keep_inbox", json!(false)),
                ("operation_id", json!("invalid-operation-id")),
                ("target", json!({"agent_id":target.agent_id})),
                (
                    "target",
                    json!({"agent_id":target.agent_id,"session_id":target.session_id,"host_id":HostId::new().unwrap()}),
                ),
            ] {
                let mut params = valid.clone();
                params[field] = value;
                invalid.push(params);
            }
            if method == "session.compact.cancel" {
                let mut params = valid.clone();
                params["target_operation_id"] = json!("invalid-operation-id");
                invalid.push(params);
            } else {
                let mut params = valid.clone();
                params["target_operation_id"] = json!(original.operation_id);
                invalid.push(params);
            }
            for params in invalid {
                let result = raw.request(id, method, params).await;
                assert_eq!(result["error"]["code"], -32602, "{result}");
                id += 1;
            }
        }
        let missing = raw
            .request(
                id,
                "host.receipt.get",
                json!({"operation_id":original.operation_id}),
            )
            .await;
        assert_eq!(
            serde_json::from_value::<Fault>(missing["error"]["data"].clone())
                .unwrap()
                .kind(),
            Some(FaultKind::HostReceiptNotFound)
        );
        http.no_request();
        let accepted = raw.request(700, "session.compact", start).await;
        assert_eq!(accepted["result"]["kind"], "session.compact");
        assert_eq!(
            accepted["result"]["operation_id"],
            json!(original.operation_id)
        );
        assert_ne!(accepted["id"], accepted["result"]["operation_id"]);
        let receipt: CompactReceipt = serde_json::from_value(accepted["result"].clone()).unwrap();
        assert_eq!(receipt.result.state, CompactionState::Preparing {});
        assert!(is_utility(&http.request().await));
        let cancelled = raw.request(701, "session.compact.cancel", stop).await;
        let receipt: CompactCancelReceipt =
            serde_json::from_value(cancelled["result"].clone()).unwrap();
        assert_eq!(receipt.operation_id, cancel.operation_id);
        assert_eq!(receipt.target_operation_id, original.operation_id);
        assert!(receipt.result.cancel_claimed);
        let queried = raw
            .request(
                702,
                "host.receipt.get",
                json!({"operation_id":cancel.operation_id}),
            )
            .await;
        assert_eq!(queried["result"], cancelled["result"]);
        gate.add_permits(1);
        drop(raw);
        if let Some(pipe) = pipe {
            pipe.await.unwrap().unwrap();
        }
        drop(tls);
    }
}
