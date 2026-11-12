mod support;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_runtime::{
    ExecutionOptions,
    model::{
        AssistantPart, ModelCallContext, ModelDriver, ModelDriverConfig, ModelFinishReason,
        ModelMessage,
    },
    tools::run_tool_batch,
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use support::*;
use tokio::net::TcpListener;

fn frame(delta: Value, finish: Option<&str>) -> String {
    format!(
        "data: {}\n\n",
        json!({"id":"explicit-http-fixture","choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
    )
}
fn tool(id: &str, name: &str, args: Value) -> String {
    frame(
        json!({"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]}),
        None,
    ) + &frame(json!({}), Some("tool_calls"))
        + "data: [DONE]\n\n"
}
#[path = "../../../tests/support/http.rs"]
mod http;

/// Scripted loopback HTTP/SSE replies exercise the production provider and
/// actual model/tool primitives. Full Turn ownership is covered by host-tools'
/// local_binding tests. This is not a live model-provider claim.
#[tokio::test]
async fn production_http_provider_round_trip_executes_real_files_and_shell() {
    let (root, tools, records) = fixture().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        let replies = [
            tool(
                "http-write",
                "write",
                json!({"path":"from-http.txt","content":"actual filesystem text"}),
            ),
            tool(
                "http-bash",
                "bash",
                json!({"command":"cat from-http.txt; printf observed-stderr >&2; exit 7"}),
            ),
            frame(json!({"content":"Verified the actual tool results"}), None)
                + &frame(json!({}), Some("stop"))
                + "data: [DONE]\n\n",
        ];
        let mut requests = Vec::new();
        for body in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(http::read_request(&mut socket).await.body);
            http::write_sse(&mut socket, &body).await;
        }
        requests
    });
    let model = Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(endpoint, "scripted-http-model", descriptor()),
            SecretApiKey::new("fixture-only-key").unwrap(),
        )
        .unwrap(),
    );
    let config = execution(model, &tools, &["write", "bash"], None);
    let options = ExecutionOptions::default();
    let context = batch_context(&options);
    let driver = ModelDriver::new(
        config.model().as_ref(),
        ModelDriverConfig::new(options.model_timeout, 1, options.model_retry_base_delay),
    )
    .unwrap();
    let mut messages = vec![ModelMessage::user("Run the explicit local-tool fixture").unwrap()];
    // Three declared exchanges exercise protocol encoding and result replay.
    for (index, expected) in [
        ModelFinishReason::ToolCalls,
        ModelFinishReason::ToolCalls,
        ModelFinishReason::Stop,
    ]
    .into_iter()
    .enumerate()
    {
        let response = driver
            .call(
                config.request(messages.clone()).unwrap(),
                ModelCallContext::new(
                    context.loop_id,
                    index as u32,
                    context.cancellation.clone(),
                    Instant::now() + Duration::from_secs(10),
                ),
                &mut |_| {},
            )
            .await
            .outcome
            .unwrap();
        assert_eq!(response.finish_reason(), expected);
        messages.push(
            ModelMessage::assistant_with_provider_replay(
                response.parts().to_vec(),
                response.provider_replay().cloned(),
            )
            .unwrap(),
        );
        let calls: Vec<_> = response
            .parts()
            .iter()
            .filter_map(|part| match part {
                AssistantPart::ToolCall(call) => Some(call),
                _ => None,
            })
            .collect();
        if !calls.is_empty() {
            let mut batch = context.clone();
            batch.request_index = index as u32;
            let report = run_tool_batch(&config, &calls, batch, Arc::new(NoInteractions)).await;
            assert_eq!(report.stop, None);
            messages.extend(report.results.into_iter().map(|result| {
                ModelMessage::tool_with_outcome(result.call_id, result.output, result.outcome)
                    .unwrap()
            }));
        }
    }
    tools.finish().await;
    assert_eq!(
        fs::read_to_string(root.path().join("from-http.txt")).unwrap(),
        "actual filesystem text"
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 2);
    let messages = requests[2]["messages"].as_array().unwrap();
    let bash = messages
        .iter()
        .find(|message| message["tool_call_id"] == "http-bash")
        .unwrap();
    let result: Value = serde_json::from_str(bash["content"].as_str().unwrap()).unwrap();
    assert_eq!(result["exit_code"], 7);
    assert_eq!(result["stdout"]["output"]["data"], "actual filesystem text");
    assert_eq!(result["stderr"]["output"]["data"], "observed-stderr");
    assert_eq!(records.edits.lock().unwrap().len(), 1);
    assert_eq!(records.commands.lock().unwrap().len(), 1);
}
