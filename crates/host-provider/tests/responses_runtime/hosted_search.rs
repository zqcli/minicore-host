use super::*;

#[tokio::test]
async fn hosted_search_keeps_native_sources_without_dispatching_a_local_tool() {
    let mut script = Script::new();
    let search = json!({
        "type":"web_search_call","id":"ws_fixture","status":"completed",
        "action":{"type":"search","query":"fixture agenda",
                  "sources":[{"type":"url","url":"https://example.com/agenda"}]}
    });
    let citation = json!({
        "type":"url_citation","url":"https://example.com/agenda","title":"Agenda",
        "start_index":0,"end_index":6
    });
    script.events.extend([
        json!({"type":"response.output_item.added","output_index":0,
               "item":{"type":"web_search_call","id":"ws_fixture","status":"in_progress"}}),
        json!({"type":"response.web_search_call.in_progress","output_index":0,"item_id":"ws_fixture"}),
        json!({"type":"response.web_search_call.searching","output_index":0,"item_id":"ws_fixture"}),
        json!({"type":"response.web_search_call.completed","output_index":0,"item_id":"ws_fixture"}),
        json!({"type":"response.output_item.done","output_index":0,"item":search}),
    ]);
    script.output.push(search.clone());
    script.message("msg_search", Some("final_answer"), &["Agenda"]);
    let text_done = script
        .events
        .iter()
        .position(|event| event["type"] == "response.output_text.done")
        .unwrap();
    script.events.insert(
        text_done,
        json!({"type":"response.output_text.annotation.added","output_index":1,"item_id":"msg_search",
               "content_index":0,"annotation_index":0,"annotation":citation}),
    );
    for event in &mut script.events {
        match event["type"].as_str() {
            Some("response.content_part.done") => {
                event["part"]["annotations"] = json!([citation]);
            }
            Some("response.output_item.done") if event["output_index"] == 1 => {
                event["item"]["content"][0]["annotations"] = json!([citation]);
            }
            _ => {}
        }
    }
    script.output[1]["content"][0]["annotations"] = json!([citation]);
    script.finish(None);
    for live in [true, false] {
        let fixture = Fixture::start(vec![
            Reply::sse(script.body()),
            Reply::sse(final_text("next answer")),
        ])
        .await;
        let model = provider_with_search(&fixture.endpoint, "fixture-responses-model", Some(live));
        let report = call(model.as_ref(), request(true)).await;
        let response = answer(&report);
        assert_eq!(response.finish_reason(), ModelFinishReason::Stop);
        assert!(
            matches!(response.parts(), [AssistantPart::Text(text)] if text.as_ref() == "Agenda")
        );
        let replay = response.provider_replay().unwrap().payload();
        assert_eq!(replay["items"][0]["item"], search);
        assert_eq!(
            replay["items"][1]["content"][0]["annotations"],
            json!([citation])
        );

        let mut messages = request(false).messages().to_vec();
        messages.push(replay_message(response));
        messages.push(ModelMessage::user("continue").unwrap());
        let next = ModelRequest::new(
            messages,
            vec![],
            ModelLimits::default(),
            ReasoningPreference::Auto,
        )
        .unwrap();
        assert!(call(model.as_ref(), next).await.outcome.is_ok());

        let requests = fixture.finish().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[0].head.starts_with("POST /v1/responses HTTP/1.1"));
        assert_eq!(requests[0].body["tools"][0]["type"], "function");
        assert_eq!(
            requests[0].body["tools"][1],
            json!({"type":"web_search","external_web_access":live})
        );
        assert_eq!(
            requests[0].body["include"],
            json!([
                "reasoning.encrypted_content",
                "web_search_call.action.sources"
            ])
        );
        assert_eq!(requests[1].body["input"][2], search);
        assert_eq!(requests[1].body["input"][3]["id"], "msg_search");
    }
}
