//! Synthetic metadata events through the production adapter and Runtime.

use super::*;

fn metadata() -> Value {
    json!({"type":"response.metadata","response_id":"resp_fixture","metadata":{"openai_chatgpt_moderation_metadata":{"prompt":{},"generation":{},"tool_call":{},"tool_response":{}}}})
}

fn script() -> Script {
    let mut script = Script::new();
    script.reasoning(true);
    let index = script.add_call("fc_safe", "call_safe");
    script.arguments(index, "{\"value\":\"fixture\"}");
    script.end_call(index);
    script.finish(None);
    script
}

async fn successful(script: Script) {
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(true)).await;
    let response = report.outcome.unwrap();
    assert_eq!(
        response
            .parts()
            .iter()
            .filter(|part| part.as_tool_call().is_some())
            .count(),
        1
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

async fn rejected(raw: String, expected: ModelErrorKind) {
    let fixture = Fixture::start(vec![Reply::sse(raw)]).await;
    let report = call(provider(&fixture.endpoint).as_ref(), request(true)).await;
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), expected);
    if let Some(partial) = failure.partial {
        assert!(
            partial
                .content
                .iter()
                .all(|part| part.as_tool_call().is_none())
        );
    }
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn metadata_at_multiple_positions_does_not_finish_or_duplicate_calls() {
    for insertion in [1, 8, 12] {
        let mut script = script();
        script.events.insert(insertion, metadata());
        let before_terminal = script.events.len() - 1;
        script.events.insert(before_terminal, metadata());
        successful(script).await;
    }
}

#[tokio::test]
async fn metadata_requires_matching_started_response() {
    for case in ["missing_id", "wrong_id", "before_created", "bad_id_type"] {
        let mut script = script();
        let mut event = metadata();
        match case {
            "missing_id" => {
                event.as_object_mut().unwrap().remove("response_id");
            }
            "wrong_id" => event["response_id"] = json!("other"),
            "bad_id_type" => event["response_id"] = json!(12),
            _ => (),
        }
        script.events.insert(
            if case == "before_created" {
                0
            } else {
                script.events.len() - 1
            },
            event,
        );
        rejected(script.body(), ModelErrorKind::InvalidProviderResponse).await;
    }
}

#[tokio::test]
async fn metadata_requires_increasing_sequence() {
    for problem in ["missing", "duplicate", "regressed"] {
        let mut script = script();
        script.events.insert(script.events.len() - 1, metadata());
        let frames: Vec<_> = script
            .events
            .into_iter()
            .enumerate()
            .map(|(index, mut event)| {
                event["sequence_number"] = json!(index);
                if event["type"] == "response.metadata" {
                    match problem {
                        "missing" => {
                            event.as_object_mut().unwrap().remove("sequence_number");
                        }
                        "duplicate" => event["sequence_number"] = json!(index - 1),
                        _ => event["sequence_number"] = json!(0),
                    }
                }
                format!("data: {event}\n\n")
            })
            .collect();
        rejected(frames.join(""), ModelErrorKind::InvalidProviderResponse).await;
    }
}

#[tokio::test]
async fn metadata_never_completes_eof_or_done_sentinel() {
    for suffix in ["", "data: [DONE]\n\n"] {
        let mut script = script();
        script.events.pop();
        script.events.push(metadata());
        rejected(script.body() + suffix, ModelErrorKind::IncompleteResponse).await;
    }
}

#[tokio::test]
async fn metadata_preserves_failed_and_incomplete_terminals() {
    for failed in [true, false] {
        let mut script = script();
        script.events.pop();
        script.events.push(metadata());
        if failed {
            script.events.push(json!({"type":"response.failed","response":{"id":"resp_fixture","status":"failed","output":script.output,"error":{"code":"invalid_request_error","message":"fixture failure"}}}));
        } else {
            script.events.push(json!({"type":"response.incomplete","response":{"id":"resp_fixture","status":"incomplete","output":script.output,"incomplete_details":{"reason":"max_output_tokens"}}}));
        }
        rejected(
            script.body(),
            if failed {
                ModelErrorKind::InvalidRequest
            } else {
                ModelErrorKind::IncompleteResponse
            },
        )
        .await;
    }
}

#[tokio::test]
async fn metadata_preserves_unknown_events_and_terminal_conflict_rejections() {
    for case in ["unknown", "changed_args", "empty_output"] {
        let mut script = script();
        let mut event = metadata();
        if case == "unknown" {
            event["type"] = json!("response.unknown_fixture");
        }
        script.events.insert(script.events.len() - 1, event);
        if case == "changed_args" {
            script.events.last_mut().unwrap()["response"]["output"][1]["arguments"] = json!("{}");
        }
        if case == "empty_output" {
            script.events.last_mut().unwrap()["response"]["output"] = json!([]);
        }
        rejected(script.body(), ModelErrorKind::InvalidProviderResponse).await;
    }
}

#[tokio::test]
async fn metadata_does_not_mask_top_level_error_or_duplicate_identity() {
    let mut with_error = script();
    let mut event = metadata();
    event["error"] = json!({"code":"invalid_request_error","message":"fixture"});
    with_error.events.insert(with_error.events.len() - 1, event);
    rejected(with_error.body(), ModelErrorKind::InvalidRequest).await;

    let mut with_duplicate = script();
    with_duplicate
        .events
        .insert(with_duplicate.events.len() - 1, metadata());
    let raw = with_duplicate.body().replace(
        "\"response_id\":\"resp_fixture\"",
        "\"response_id\":\"resp_fixture\",\"response_id\":\"resp_fixture\"",
    );
    rejected(raw, ModelErrorKind::InvalidProviderResponse).await;
}
