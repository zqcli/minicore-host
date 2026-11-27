use super::*;
use futures_util::SinkExt;
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{Request, Response},
    },
};

#[tokio::test]
async fn responses_websocket_uses_the_same_runtime_event_and_replay_path() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = provider_with_search(
        &format!("ws://{}/v1/responses", listener.local_addr().unwrap()),
        "fixture-responses-model",
        Some(true),
    );
    let server = tokio::spawn(async move {
        for second in [false, true] {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = accept_hdr_async(tcp, |req: &Request, response: Response| {
                assert_eq!(req.uri().path(), "/v1/responses");
                assert_eq!(
                    req.headers()["authorization"],
                    "Bearer fixture-not-a-real-key"
                );
                Ok(response)
            })
            .await
            .unwrap();
            let frame = ws.next().await.unwrap().unwrap();
            let input: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(input["type"], "response.create");
            assert_eq!(input["model"], "fixture-responses-model");
            assert_eq!(input["store"], false);
            assert_eq!(
                input["tools"],
                json!([{"type":"web_search","external_web_access":true}])
            );
            assert_eq!(input["include"][1], "web_search_call.action.sources");
            assert!(input.get("stream").is_none());
            assert!(input.get("previous_response_id").is_none());
            assert_eq!(input["input"][0]["role"], "system");
            assert_eq!(input["input"][1]["role"], "user");
            if second {
                assert_eq!(input["input"][2]["id"], "msg_ws");
                assert_eq!(input["input"][2]["content"][0]["text"], "from WS");
                assert_eq!(input["input"][3]["role"], "user");
            }
            ws.send(Message::Ping(vec![1].into())).await.unwrap();
            assert!(matches!(ws.next().await, Some(Ok(Message::Pong(_)))));
            let mut script = Script::new();
            script.message("msg_ws", Some("final_answer"), &["from ", "WS"]);
            script.finish(None);
            for (sequence, mut event) in script.events.into_iter().enumerate() {
                event["sequence_number"] = json!(sequence);
                ws.send(Message::Text(event.to_string().into()))
                    .await
                    .unwrap();
            }
        }
    });
    let first = call(model.as_ref(), request(false)).await;
    assert!(
        matches!(answer(&first).parts(), [AssistantPart::Text(text)] if text.as_ref() == "from WS")
    );
    let mut messages = request(false).messages().to_vec();
    messages.push(replay_message(answer(&first)));
    messages.push(ModelMessage::user("continue").unwrap());
    let second = call(
        model.as_ref(),
        ModelRequest::new(
            messages,
            vec![],
            ModelLimits::default(),
            ReasoningPreference::Auto,
        )
        .unwrap(),
    )
    .await;
    assert!(
        matches!(answer(&second).parts(), [AssistantPart::Text(text)] if text.as_ref() == "from WS")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn responses_websocket_close_without_completion_keeps_partial_text() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = provider(&format!(
        "ws://{}/v1/responses",
        listener.local_addr().unwrap()
    ));
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        assert!(matches!(ws.next().await, Some(Ok(Message::Text(_)))));
        let mut script = Script::new();
        script.message("msg_ws", None, &["partial"]);
        for (sequence, mut event) in script.events.into_iter().enumerate() {
            event["sequence_number"] = json!(sequence);
            ws.send(Message::Text(event.to_string().into()))
                .await
                .unwrap();
        }
        ws.close(None).await.unwrap();
    });
    let report = call(model.as_ref(), request(false)).await;
    assert_eq!(report.attempts, 1);
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::IncompleteResponse);
    assert_eq!(failure.error().delivery(), DeliveryState::Started);
    assert!(
        matches!(&failure.partial.unwrap().content[0], AssistantPart::Text(text) if text.as_ref() == "partial")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn rejected_upgrade_uses_one_sse_post_with_the_same_search_policy() {
    use minicore_host_runtime::model::WebSearchMode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = provider(&format!(
        "ws://{}/v1/responses",
        listener.local_addr().unwrap()
    ));
    let server = tokio::spawn(async move {
        let (mut handshake, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(handshake.read_u8().await.unwrap());
        }
        assert!(head.starts_with(b"GET /v1/responses HTTP/1.1\r\n"));
        handshake
            .write_all(
                b"HTTP/1.1 426 Upgrade Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        handshake.shutdown().await.unwrap();
        let (mut http, _) = listener.accept().await.unwrap();
        let post = support::read_request(&mut http).await;
        assert!(post.head.starts_with("POST /v1/responses HTTP/1.1"));
        assert_eq!(post.body["stream"], true);
        assert!(post.body.get("type").is_none());
        assert_eq!(
            post.body["tools"],
            json!([{"type":"web_search","external_web_access":false}])
        );
        support::write_sse(&mut http, &final_text("from SSE")).await;
    });
    let report = call(
        model.as_ref(),
        request(false).with_web_search(Some(WebSearchMode::Cached)),
    )
    .await;
    assert_eq!(report.attempts, 1);
    assert!(
        matches!(answer(&report).parts(), [AssistantPart::Text(text)] if text.as_ref() == "from SSE")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn explicit_ws_route_rejection_falls_back_but_authentication_does_not() {
    for code in ["model_not_found", "invalid_api_key"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let model = provider(&format!(
            "ws://{}/v1/responses",
            listener.local_addr().unwrap()
        ));
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            assert!(matches!(ws.next().await, Some(Ok(Message::Text(_)))));
            ws.send(Message::Text(
                json!({"type":"error","code":code,"message":"fixture"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
            if code == "model_not_found" {
                let (mut http, _) = listener.accept().await.unwrap();
                assert_eq!(support::read_request(&mut http).await.body["stream"], true);
                support::write_sse(&mut http, &final_text("fallback")).await;
            }
        });
        let report = call(model.as_ref(), request(false)).await;
        assert_eq!(report.attempts, 1);
        if code == "model_not_found" {
            assert!(
                matches!(answer(&report).parts(), [AssistantPart::Text(text)] if text.as_ref() == "fallback")
            );
        } else {
            assert_eq!(
                report.outcome.unwrap_err().error().kind(),
                ModelErrorKind::AuthRejected
            );
        }
        server.await.unwrap();
    }
}
