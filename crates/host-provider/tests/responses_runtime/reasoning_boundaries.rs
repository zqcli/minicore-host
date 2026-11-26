use super::*;

fn reasoning_parts(
    script: &mut Script,
    id: &str,
    summary: bool,
    parts: &[&[&str]],
    streamed: bool,
) {
    let index = script.output.len();
    let kind = if summary {
        "summary_text"
    } else {
        "reasoning_text"
    };
    let part_event = if summary {
        "response.reasoning_summary_part"
    } else {
        "response.content_part"
    };
    let text_event = if summary {
        "response.reasoning_summary_text"
    } else {
        "response.reasoning_text"
    };
    let index_field = if summary {
        "summary_index"
    } else {
        "content_index"
    };
    script.events.push(json!({"type":"response.output_item.added","output_index":index,"item":{"type":"reasoning","id":id,"summary":[]}}));
    let mut content = Vec::new();
    for (part_index, deltas) in parts.iter().enumerate() {
        let text = deltas.concat();
        let part = json!({"type":kind,"text":text});
        if streamed {
            let mut events =
                vec![json!({"type":format!("{part_event}.added"),"part":{"type":kind,"text":""}})];
            events.extend(
                deltas
                    .iter()
                    .map(|delta| json!({"type":format!("{text_event}.delta"),"delta":delta})),
            );
            events.push(json!({"type":format!("{text_event}.done"),"text":text}));
            events.push(json!({"type":format!("{part_event}.done"),"part":part}));
            for mut event in events {
                event["output_index"] = json!(index);
                event["item_id"] = json!(id);
                event[index_field] = json!(part_index);
                script.events.push(event);
            }
        }
        content.push(part);
    }
    let mut item = json!({"type":"reasoning","id":id,"summary":[],"encrypted_content":format!("{id}-cipher"),"status":"completed"});
    item[if summary { "summary" } else { "content" }] = json!(content);
    script
        .events
        .push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    script.output.push(item);
}

async fn observed_call(model: &dyn Model) -> (ModelCallReport, String) {
    let mut reasoning = String::new();
    let report = ModelDriver::new(
        model,
        ModelDriverConfig::new(Duration::from_secs(5), 1, Duration::from_millis(1)),
    )
    .unwrap()
    .call(
        request(false),
        ModelCallParams {
            loop_id: LoopId::new().unwrap(),
            request_index: 0,
            cancellation: CancellationToken::new(),
            deadline: None,
        },
        &mut |event| {
            if let ModelCallEvent::ReasoningDelta(delta) = event {
                reasoning.push_str(delta.as_str());
            }
        },
    )
    .await;
    (report, reasoning)
}

fn reasoning_text(parts: &[AssistantPart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            AssistantPart::Reasoning(value) => value.text(),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn responses_reasoning_boundaries_match_stream_history_and_native_replay() {
    let mut script = Script::new();
    reasoning_parts(
        &mut script,
        "rs_first",
        true,
        &[
            &["", "**Preparing ", "project inspection**"],
            &[],
            &["**Preparing project inspection**"],
        ],
        true,
    );
    reasoning_parts(&mut script, "rs_second", true, &[&["检查 ", "🐈"]], true);
    reasoning_parts(
        &mut script,
        "rs_done",
        true,
        &[
            &["**Configuring", " project inspection**"],
            &[""],
            &["After"],
        ],
        false,
    );
    reasoning_parts(
        &mut script,
        "rs_content",
        false,
        &[&["Thinking"], &["", " again"]],
        true,
    );
    script.message("msg_one", Some("commentary"), &["frag", "ment"]);
    script.message("msg_two", Some("final_answer"), &["tail"]);
    script.finish(None);
    let fixture = Fixture::start(vec![
        Reply::sse(script.body()),
        Reply::sse(final_text("done")),
    ])
    .await;
    let model = provider(&fixture.endpoint);
    let (report, observed) = observed_call(model.as_ref()).await;
    let response = answer(&report);
    let expected = "**Preparing project inspection**\n**Preparing project inspection**\n检查 🐈\n**Configuring project inspection**\nAfter\nThinking\n again";
    assert_eq!(observed, expected);
    assert_eq!(reasoning_text(response.parts()), expected);
    assert!(
        matches!(&response.parts()[1], AssistantPart::Text(text) if text.as_ref() == "fragmenttail")
    );
    let restored: ModelResponse =
        serde_json::from_slice(&serde_json::to_vec(response).unwrap()).unwrap();
    assert_eq!(reasoning_text(restored.parts()), expected);
    let next = ModelRequest::new(
        vec![
            replay_message(&restored),
            ModelMessage::user("next").unwrap(),
        ],
        vec![],
        ModelLimits::default(),
        ReasoningPreference::Auto,
    )
    .unwrap();
    assert!(call(model.as_ref(), next).await.outcome.is_ok());
    let requests = fixture.finish().await;
    let input = &requests[1].body["input"];
    for index in 0..4 {
        assert_eq!(
            input[index], script.output[index],
            "native reasoning item {index}"
        );
    }
    assert_eq!(input[4]["content"][0]["text"], "fragment");
    assert_eq!(input[5]["content"][0]["text"], "tail");
}

#[tokio::test]
async fn responses_reasoning_boundaries_preserve_newlines_and_skip_empty_parts() {
    let cases: &[(&[&[&str]], &str)] = &[
        (&[&["Alpha\n"], &["Beta"]], "Alpha\nBeta"),
        (&[&["Alpha"], &["\nBeta"]], "Alpha\nBeta"),
        (&[&["Alpha\r"], &["\nBeta"]], "Alpha\nBeta"),
        (&[&["Alpha\r", "\n"], &["Beta"]], "Alpha\nBeta"),
        (&[&["Alpha"], &["\r", "\nBeta"]], "Alpha\nBeta"),
        (
            &[&[""], &[], &["Alpha"], &[], &["", "Beta"], &[]],
            "Alpha\nBeta",
        ),
        (&[&["Alpha\r"], &[""], &["\n"], &["Beta"]], "Alpha\nBeta"),
        (&[&["Alpha\n"], &["\nBeta"]], "Alpha\n\nBeta"),
        (&[&["Plan", "", "ning ", "🐈"]], "Planning 🐈"),
    ];
    let replies = cases
        .iter()
        .map(|(parts, _)| {
            let mut script = Script::new();
            reasoning_parts(&mut script, "rs_newlines", true, parts, true);
            script.finish(None);
            Reply::sse(script.body())
        })
        .collect();
    let fixture = Fixture::start(replies).await;
    let model = provider(&fixture.endpoint);
    for (parts, expected) in cases {
        let (report, observed) = observed_call(model.as_ref()).await;
        assert_eq!(observed, *expected, "{parts:?}");
        assert_eq!(
            reasoning_text(answer(&report).parts()),
            *expected,
            "{parts:?}"
        );
    }
    assert_eq!(fixture.finish().await.len(), cases.len());
}

#[tokio::test]
async fn responses_reasoning_boundaries_remain_in_unfinished_output() {
    let mut script = Script::new();
    reasoning_parts(
        &mut script,
        "rs_partial",
        true,
        &[&["Plan", "ning"], &["检查 🐈"]],
        true,
    );
    let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
    let (report, observed) = observed_call(provider(&fixture.endpoint).as_ref()).await;
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::IncompleteResponse);
    let partial = failure.partial.unwrap();
    assert_eq!(observed, "Planning\n检查 🐈");
    assert_eq!(reasoning_text(&partial.content), observed);
    let restored: minicore_host_runtime::model::ModelPartial =
        serde_json::from_slice(&serde_json::to_vec(&partial).unwrap()).unwrap();
    assert_eq!(restored, *partial);
    assert_eq!(
        partial.provider_replay.unwrap().payload()["items"][0]["item"],
        script.output[0]
    );
    assert_eq!(fixture.finish().await.len(), 1);
}

#[tokio::test]
async fn responses_reasoning_boundaries_keep_raw_snapshot_validation() {
    for target in ["text", "part", "item", "terminal", "duplicate"] {
        let mut script = Script::new();
        reasoning_parts(&mut script, "rs_checked", true, &[&["One"], &["Two"]], true);
        script.finish(None);
        match target {
            "text" => {
                let event = script
                    .events
                    .iter_mut()
                    .rfind(|event| event["type"] == "response.reasoning_summary_text.done")
                    .unwrap();
                event["text"] = json!("Two fabricated");
            }
            "part" => {
                let event = script
                    .events
                    .iter_mut()
                    .rfind(|event| event["type"] == "response.reasoning_summary_part.done")
                    .unwrap();
                event["part"]["text"] = json!("Two fabricated");
            }
            "item" => {
                let event = script
                    .events
                    .iter_mut()
                    .find(|event| event["type"] == "response.output_item.done")
                    .unwrap();
                event["item"]["summary"][1]["text"] = json!("Two fabricated");
            }
            "terminal" => {
                script.events.last_mut().unwrap()["response"]["output"][0]["summary"][1]["text"] =
                    json!("Two fabricated")
            }
            _ => {
                let at = script
                    .events
                    .iter()
                    .position(|event| event["type"] == "response.output_item.done")
                    .unwrap();
                script.events.insert(at + 1, script.events[at].clone());
            }
        }
        let fixture = Fixture::start(vec![Reply::sse(script.body())]).await;
        let (report, observed) = observed_call(provider(&fixture.endpoint).as_ref()).await;
        let failure = report.outcome.unwrap_err();
        assert_eq!(
            failure.error().kind(),
            ModelErrorKind::InvalidProviderResponse,
            "{target}"
        );
        assert_eq!(observed, "One\nTwo", "{target}");
        assert_eq!(
            reasoning_text(&failure.partial.unwrap().content),
            observed,
            "{target}"
        );
        assert_eq!(fixture.finish().await.len(), 1);
    }
}
