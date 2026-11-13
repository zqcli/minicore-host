mod common;
mod observation_common;
mod os_pipe;
mod wss_common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::model::{ModelEvent, ModelFinishReason};
use observation_common::*;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct GatedFileTool {
    spec: minicore_host_runtime::tools::ToolSpec,
    entered: mpsc::UnboundedSender<()>,
    gate: Arc<tokio::sync::Semaphore>,
    retained_sink: Arc<Mutex<Option<ToolOutputSink>>>,
}
impl minicore_host_runtime::tools::Tool for GatedFileTool {
    fn spec(&self) -> &minicore_host_runtime::tools::ToolSpec {
        &self.spec
    }
    fn execute<'a>(
        &'a self,
        invocation: minicore_host_runtime::tools::ToolInvocation,
        context: minicore_host_runtime::tools::ToolContext,
    ) -> minicore_host_runtime::tools::ToolFuture<'a> {
        Box::pin(async move {
            let content = tokio::fs::read_to_string(invocation.arguments["path"].as_str().unwrap())
                .await
                .unwrap();
            let sink = ToolOutputSink::current().expect("trusted invocation binding");
            assert!(sink.emit("stdout", &content));
            *self.retained_sink.lock().unwrap() = Some(sink);
            context.progress.emit(
                minicore_host_runtime::tools::ToolProgress::new(
                    Some(minicore_host_runtime::Text::new("file read")),
                    Some(1),
                    Some(1),
                )
                .unwrap(),
            );
            self.entered.send(()).unwrap();
            self.gate.acquire().await.unwrap().forget();
            Ok(
                minicore_host_runtime::tools::ToolExecutionOutcome::Completed(
                    minicore_host_runtime::tools::ToolOutput::new(format!("result: {content}"))
                        .unwrap(),
                ),
            )
        })
    }
}
fn call_file(events: &Events, path: &std::path::Path) {
    let id: minicore_host_runtime::ToolCallId = "repeated-call".parse().unwrap();
    events
        .send(ModelEvent::ToolCallStart {
            tool_call_id: id.clone(),
            tool_name: "read_fixture".parse().unwrap(),
        })
        .unwrap();
    events
        .send(ModelEvent::tool_call_arguments_delta(
            id.clone(),
            serde_json::json!({"path":path}).to_string(),
        ))
        .unwrap();
    events
        .send(ModelEvent::ToolCallEnd { tool_call_id: id })
        .unwrap();
    events
        .send(ModelEvent::Finish {
            reason: ModelFinishReason::ToolCalls,
        })
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_file_tool_arguments_progress_output_and_result_cross_os_pipes_and_tls() {
    for wss in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("actual.txt");
        std::fs::write(&path, "读🙂").unwrap();
        let (entered, mut arrivals) = mpsc::unbounded_channel();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let retained_sink = Arc::new(Mutex::new(None));
        let tool = Arc::new(GatedFileTool {
            spec: minicore_host_runtime::tools::ToolSpec::new(
                "read_fixture".parse().unwrap(),
                "read a real fixture file",
                serde_json::json!({"type":"object"}),
            )
            .unwrap(),
            entered,
            gate: gate.clone(),
            retained_sink: retained_sink.clone(),
        });
        let (host, events, mut started) = setup(2, Some(tool));
        let wss_server = if wss {
            Some(wss_common::Server::start(host.clone()).await)
        } else {
            None
        };
        let (client, stdio_server) = if let Some(server) = &wss_server {
            (server.client(wss_common::FIRST).await.unwrap(), None)
        } else {
            let (client, task) = stdio_pipes(host).await;
            (client, Some(task))
        };
        let session = create(&client).await;
        let mut sub = client
            .subscribe(
                request(
                    scope(session.target()),
                    &[
                        EventCategory::State,
                        EventCategory::Output,
                        EventCategory::Completion,
                    ],
                ),
                options(2000),
            )
            .await
            .unwrap();
        session
            .send(input("read file"), options(2000))
            .await
            .unwrap();
        started.recv().await.unwrap();
        call_file(&events[0], &path);
        arrivals.recv().await.unwrap();
        let (source, delta) = loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) =
                next(&mut sub).await
            {
                break (source, data);
            }
        };
        assert_eq!(delta.text, "读🙂");
        let target = ToolCallRef {
            turn_id: source.turn.turn_id.clone(),
            tool_call_id: source.tool_call_id.clone(),
        };
        let snapshot = session
            .toolcall(target.clone(), options(2000))
            .await
            .unwrap();
        assert_eq!(snapshot.source, source);
        assert_eq!(snapshot.data.arguments, serde_json::json!({"path":path}));
        assert!(snapshot.data.result.is_none());
        let range = session
            .tool_output(
                target.clone(),
                TextReadRequest {
                    channel: "stdout".into(),
                    start: 0,
                    end: 7,
                    max_bytes: 1,
                },
                options(2000),
            )
            .await
            .unwrap();
        assert_eq!(range.data.text, "读");
        assert!(range.data.has_more);
        gate.add_permits(1);
        let done = loop {
            if let ObservationEvent::Activity(ActivityEvent::ToolFinished {
                source: actual,
                data,
            }) = next(&mut sub).await
            {
                assert_eq!(actual, source);
                break data;
            }
        };
        assert_eq!(done.result.output.content().as_str(), "result: 读🙂");
        assert_eq!(
            done.output_positions,
            vec![ChannelPosition {
                channel: "stdout".into(),
                end: 7
            }]
        );
        assert!(
            !retained_sink
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .emit("stdout", "late")
        );
        let snapshot = session.toolcall(target, options(2000)).await.unwrap();
        assert_eq!(
            snapshot.data.result.unwrap().output.content().as_str(),
            "result: 读🙂"
        );
        started.recv().await.unwrap();
        events[1].send(ModelEvent::text_delta("done")).unwrap();
        finish(&events[1]);
        turn_finished(&mut sub).await;
        client.close();
        if let Some(task) = stdio_server {
            task.await.unwrap().unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn observed_and_unobserved_runs_preserve_native_content_and_cold_rpc_record_identity() {
    let mut saved = Vec::new();
    let message = input("durable input");
    for observe in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (resources, events, mut started) = resources(1, None);
        let host = Host::open_persistent(
            directory.path(),
            HostSettings {
                name: "persistent observation".into(),
            },
            resources.clone(),
        )
        .await
        .unwrap();
        let (client, server) = common::pipes::stdio(host.clone(), options(2000)).await;
        let mut create = create_request();
        create.mode = StorageMode::Persistent;
        let created = client.create_agent(create, options(2000)).await.unwrap();
        let target = SessionRef {
            agent_id: created.agent_id.unwrap(),
            session_id: created.session_id.unwrap(),
        };
        let session = client.session(target.clone());
        let mut observation = if observe {
            Some(
                client
                    .subscribe(
                        request(
                            scope(&target),
                            &[
                                EventCategory::State,
                                EventCategory::Output,
                                EventCategory::Completion,
                            ],
                        ),
                        options(2000),
                    )
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        session.send(message.clone(), options(2000)).await.unwrap();
        started.recv().await.unwrap();
        let turn = running(&session).await;
        events[0]
            .send(ModelEvent::text_delta("保存🙂完整正文"))
            .unwrap();
        finish(&events[0]);
        let retained = session
            .wait_turn(turn.clone(), options(2000))
            .await
            .unwrap();
        let page = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor: None,
                    limit: 100,
                },
                options(2000),
            )
            .await
            .unwrap();
        assert_eq!(page.data.upper_bound, 3);
        assert!(page.data.next_cursor.is_none());
        assert_eq!(page.data.items.len(), 3);
        for (position, row) in page.data.items.iter().enumerate() {
            assert_eq!(row.position, position as u64);
            assert_eq!(row.turn_id.as_ref(), Some(&turn));
        }
        assert!(matches!(
            &page.data.items[0].item,
            HistoryItemData::User {
                received_at: Some(_),
                ..
            }
        ));
        assert_eq!(page.data.items[1].response_index, Some(0));
        for item in &retained.data.history {
            let row = &page.data.items[item.position as usize];
            assert_eq!(item.recorded_at, row.recorded_at);
            assert_eq!(item.response_index, row.response_index);
            assert_eq!(
                serde_json::to_value(&item.item).unwrap(),
                serde_json::to_value(&row.item).unwrap()
            );
        }
        let HistoryItemData::TurnFinished { terminal } = &page.data.items[2].item else {
            panic!("actual terminal record")
        };
        assert_eq!(terminal.requests, 1);
        assert_eq!(terminal.tool_rounds, 0);
        assert_eq!(terminal.model_attempts, [1]);
        assert_eq!(terminal.outcome, TurnOutcome::Completed);
        let terminal_state = retained.data.info.as_ref().unwrap();
        assert_eq!(terminal_state.history_position, Some(2));
        assert_eq!(
            terminal_state.recorded_at,
            Some(page.data.items[2].recorded_at)
        );
        assert_eq!(terminal_state.finished_at, Some(terminal.finished_at));
        if let Some(observation) = observation.as_mut() {
            let mut model_finished = false;
            loop {
                match next(observation).await {
                    ObservationEvent::Activity(ActivityEvent::ModelFinished { source, data }) => {
                        assert_eq!(source.turn.turn_id, turn);
                        assert_eq!(source.response_index, 0);
                        assert_eq!(data.history_position, 1);
                        assert_eq!(data.recorded_at, page.data.items[1].recorded_at);
                        model_finished = true;
                    }
                    ObservationEvent::Activity(ActivityEvent::TurnFinished { source, data }) => {
                        assert!(model_finished);
                        assert_eq!(source, retained.source);
                        assert_eq!(data.history_position, terminal_state.history_position);
                        assert_eq!(data.recorded_at, terminal_state.recorded_at);
                        assert_eq!(data.finished_at, terminal_state.finished_at);
                        break;
                    }
                    _ => {}
                }
            }
        }
        saved.push(serde_json::to_value(&page.data.items).unwrap());
        client
            .shutdown(
                HostShutdownRequest {
                    operation_id: OperationId::new().unwrap(),
                    strategy: HostShutdownStrategy::Graceful,
                },
                options(2000),
            )
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if client.info(options(2000)).await.unwrap().lifecycle == HostLifecycle::Stopped {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.close(&client, None).await;
        drop(observation);
        drop(session);
        drop(client);
        drop(host);
        let reopened = Host::open_persistent(
            directory.path(),
            HostSettings {
                name: "ignored".into(),
            },
            resources,
        )
        .await
        .unwrap();
        let (client, server) = common::pipes::stdio(reopened.clone(), options(2000)).await;
        let session = client.session(target);
        let before = session.state(options(2000)).await.unwrap();
        assert_eq!(before.data.residency, Residency::Unloaded);
        let page = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor: None,
                    limit: 100,
                },
                options(2000),
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(page.data.items).unwrap(),
            saved.last().unwrap().clone()
        );
        assert_eq!(
            session.state(options(2000)).await.unwrap().data.residency,
            Residency::Unloaded
        );
        let cold_result = session
            .turn_result(turn.clone(), options(2000))
            .await
            .unwrap();
        assert!(cold_result.source.load_id.is_none());
        assert!(cold_result.data.info.is_none());
        assert_eq!(
            serde_json::to_value(&cold_result.data.history).unwrap(),
            serde_json::to_value(&retained.data.history).unwrap()
        );
        let cold_terminal = cold_result.data.saved_terminal.unwrap();
        assert_eq!(
            cold_terminal.position,
            terminal_state.history_position.unwrap()
        );
        assert_eq!(
            cold_terminal.recorded_at,
            terminal_state.recorded_at.unwrap()
        );
        assert_eq!(
            cold_terminal.finished_at,
            terminal_state.finished_at.unwrap()
        );
        assert!(session.turn(turn.clone(), options(2000)).await.is_err());
        assert!(
            session
                .turn_output(
                    turn,
                    0,
                    TextReadRequest {
                        channel: "text".into(),
                        start: 0,
                        end: 1,
                        max_bytes: 1
                    },
                    options(2000)
                )
                .await
                .is_err()
        );
        assert!(matches!(
            started.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        server.close(&client, None).await;
        drop(session);
        drop(client);
        drop(reopened);
    }
    // These are independent executions. Preserve exact comparisons above for
    // each execution's retained/events/cold views; only its original TurnId and
    // owner-captured times differ in the comparison of observed/unobserved work.
    let normalized = saved
        .into_iter()
        .map(|mut rows| {
            for row in rows.as_array_mut().unwrap() {
                let object = row.as_object_mut().unwrap();
                serde_json::from_value::<TurnId>(
                    object.remove("turn_id").expect("original Turn identity"),
                )
                .unwrap();
                serde_json::from_value::<Timestamp>(
                    object.remove("recorded_at").expect("original record time"),
                )
                .unwrap();
                match object["kind"].as_str().unwrap() {
                    "user" => {
                        serde_json::from_value::<Timestamp>(
                            object.remove("received_at").expect("original receipt time"),
                        )
                        .unwrap();
                    }
                    "turn_finished" => {
                        serde_json::from_value::<Timestamp>(
                            object
                                .remove("finished_at")
                                .expect("original terminal time"),
                        )
                        .unwrap();
                    }
                    _ => {}
                }
            }
            rows
        })
        .collect::<Vec<_>>();
    assert_eq!(normalized[0], normalized[1]);
}
