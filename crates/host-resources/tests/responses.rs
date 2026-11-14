//! Real Host/Runtime/provider composition with explicitly scripted loopback SSE.
//! These tests do not contact a live provider or execute a TUI.
#[path = "support/production.rs"]
#[allow(dead_code)]
mod support;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use support::*;
use tokio::sync::Semaphore;

fn response(id: &str, text: &str) -> String {
    let item_id = format!("msg_{id}");
    let response_id = format!("resp_{id}");
    let part = json!({"type":"output_text", "text":text});
    let item = json!({
        "type":"message", "id":item_id, "role":"assistant",
        "status":"completed", "phase":"final_answer", "content":[part]
    });
    let frames = [
        json!({"type":"response.created", "response":{"id":response_id,"status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added", "output_index":0, "item":{"type":"message","id":item_id,"role":"assistant","status":"in_progress","phase":"final_answer","content":[]}}),
        json!({"type":"response.content_part.added", "output_index":0,"item_id":item_id,"content_index":0,"part":{"type":"output_text","text":""}}),
        json!({"type":"response.output_text.delta", "output_index":0,"item_id":item_id,"content_index":0,"delta":text}),
        json!({"type":"response.output_text.done", "output_index":0,"item_id":item_id,"content_index":0,"text":text}),
        json!({"type":"response.content_part.done", "output_index":0,"item_id":item_id,"content_index":0,"part":part}),
        json!({"type":"response.output_item.done", "output_index":0,"item":item}),
        json!({"type":"response.completed", "response":{"id":response_id,"status":"completed","output":[item],"usage":{"input_tokens":41,"output_tokens":18,"total_tokens":59}}}),
    ];
    frames
        .into_iter()
        .enumerate()
        .map(|(sequence, mut frame)| {
            frame["sequence_number"] = json!(sequence);
            format!("data: {frame}\n\n")
        })
        .collect()
}

fn responses_config(endpoint: &str) -> HostConfig {
    let mut config = config(endpoint);
    config.models[0].api = OpenAiModelApi::Responses;
    config.tools.clear();
    config
}

fn assert_responses_request(request: &Value) {
    assert_eq!(request["stream"], true);
    assert_eq!(request["store"], false);
    assert!(request["input"].is_array());
    assert!(request.get("messages").is_none());
    assert!(request.get("previous_response_id").is_none());
    assert!(
        request["include"]
            .as_array()
            .unwrap()
            .contains(&json!("reasoning.encrypted_content"))
    );
}

#[tokio::test]
async fn responses_selection_reaches_ordinary_and_manual_compaction_requests() {
    let mut server = Server::start(vec![
        (
            response("first", &"first historical answer ".repeat(150)),
            None,
        ),
        (
            response("second", &"recent historical answer ".repeat(170)),
            None,
        ),
        (response("summary", "A concise fixture summary."), None),
        (response("third", "Answer after compaction"), None),
    ])
    .await;
    // The fixture deliberately uses a /chat/completions path: only `api`
    // selects the protocol, not a URL suffix.
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut config = responses_config(&server.endpoint);
    config.models[0].context_window = 4096;
    let host = Host::memory_configured(prepare(&builder, config).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut creation = request(vec![], None);
    creation.config.session_defaults.execution.compaction = CompactionSettings {
        enabled: false,
        trigger_percent: 50,
        target_percent: 20,
    };
    let receipt = host.create_agent_async(&caller, creation).await.unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    for index in 0..2 {
        send(
            &host,
            &caller,
            &target,
            &format!("task {index} {}", "recorded task ".repeat(100)),
        );
        assert_responses_request(&server.request().await);
        let turns = settled(&host, &caller, &target, index + 1).await;
        assert!(
            turns
                .iter()
                .all(|turn| turn.outcome == Some(TurnOutcome::Completed)),
            "{turns:?}"
        );
    }
    let original = host.history(&caller, &target).unwrap();
    host.session_compact(
        &caller,
        &target,
        CompactRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    let utility = server.request().await;
    assert_responses_request(&utility);
    assert!(utility.get("tools").is_none());
    assert!(
        utility
            .to_string()
            .contains("Summarize historical conversation data")
    );
    let mut states = host.session_states(&caller, &target).unwrap();
    let activity = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(activity) = states.snapshot().unwrap().data.compaction
                && activity.state.finished()
            {
                break activity;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(
            activity.state,
            CompactionState::Finished {
                result: CompactionOutcome::Compacted { .. }
            }
        ),
        "{activity:?}"
    );
    assert_eq!(activity.utility_usage.len(), 1);
    assert!(activity.utility_usage[0].completed);
    assert_eq!(host.history(&caller, &target).unwrap(), original);
    send(&host, &caller, &target, "continue after summary");
    let next = server.request().await;
    assert_responses_request(&next);
    assert!(next.to_string().contains("A concise fixture summary."));
    let turns = settled(&host, &caller, &target, 3).await;
    assert!(
        turns
            .iter()
            .all(|turn| turn.outcome == Some(TurnOutcome::Completed))
    );
}

#[tokio::test]
async fn reload_selects_responses_for_new_bindings_and_keeps_an_accepted_chat_turn() {
    let gate = Arc::new(Semaphore::new(0));
    let mut chat = Server::start(vec![(answer("original Chat binding"), Some(gate.clone()))]).await;
    let mut responses =
        Server::start(vec![(response("reload", "new Responses binding"), None)]).await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut original = config(&chat.endpoint);
    original.tools.clear();
    let host = Host::memory_configured(prepare(&builder, original).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let existing = create(&host, &caller, vec![], None).await;
    send(&host, &caller, &existing, "keep original binding");
    let original_request = chat.request().await;
    assert!(original_request["messages"].is_array());
    assert!(original_request.get("input").is_none());
    let candidate = prepare(&builder, responses_config(&responses.endpoint)).await;
    assert_eq!(
        host.reload_configuration(&caller, 0, candidate)
            .await
            .unwrap(),
        HostReloadResult::Applied {
            config_revision: 1,
            agents: Vec::new()
        }
    );
    let next = create(&host, &caller, vec![], None).await;
    send(&host, &caller, &next, "use configured Responses protocol");
    assert_responses_request(&responses.request().await);
    assert_eq!(
        settled(&host, &caller, &next, 1).await[0].outcome,
        Some(TurnOutcome::Completed)
    );
    gate.add_permits(1);
    assert_eq!(
        settled(&host, &caller, &existing, 1).await[0].outcome,
        Some(TurnOutcome::Completed)
    );
}
