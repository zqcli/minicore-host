//! Real Host/Runtime and OS pipes. Only the model event source is scripted.
mod common;
mod observation_common;
mod os_pipe;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::model::{
    Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelRequest, ModelStartFuture,
};
use minicore_host_runtime::tools::{Tool, ToolContext, ToolFuture, ToolInvocation, ToolSpec};
use minicore_host_transport::*;
use observation_common::*;
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const PARTIAL: &str = "真实未完成🙂answer";

struct UnfinishedTool(ToolSpec);
impl Tool for UnfinishedTool {
    fn spec(&self) -> &ToolSpec {
        &self.0
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        panic!("unfinished arguments must never execute")
    }
}

struct CaptureModel {
    inner: Arc<dyn Model>,
    requests: Mutex<Vec<ModelRequest>>,
}
impl Model for CaptureModel {
    fn descriptor(&self) -> &ModelDescriptor {
        self.inner.descriptor()
    }
    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        self.requests.lock().unwrap().push(request.clone());
        self.inner.start(request, context)
    }
}

async fn pages(session: &SessionClient, limit: u64) -> Value {
    let mut items = Vec::new();
    let mut cursor = None;
    let mut upper_bound = None;
    loop {
        let page = session
            .history(
                HistoryReadRequest {
                    direction: minicore_host_protocol::HistoryDirection::Forward,
                    cursor,
                    limit,
                },
                options(3000),
            )
            .await
            .unwrap();
        assert!(page.data.items.len() <= limit as usize);
        if let Some(upper) = upper_bound {
            assert_eq!(page.data.upper_bound, upper);
        } else {
            upper_bound = Some(page.data.upper_bound);
        }
        items.extend(page.data.items);
        cursor = page.data.next_cursor;
        if cursor.is_none() {
            assert_eq!(items.len() as u64, upper_bound.unwrap());
            for (position, row) in items.iter().enumerate() {
                assert_eq!(row.position, position as u64);
            }
            return value(items);
        }
    }
}
async fn loaded(session: &SessionClient) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while session.state(options(3000)).await.unwrap().data.residency != Residency::Loaded {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn stop(client: &HostClient) {
    client
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(3000),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while client.info(options(3000)).await.unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn assert_context(model: &CaptureModel, index: usize) {
    let requests = model.requests.lock().unwrap();
    let context = serde_json::to_string(requests[index].messages()).unwrap();
    assert!(context.contains("first input"));
    assert!(!context.contains(PARTIAL));
    assert!(!context.contains("fragmented"));
    if index == 2 {
        assert!(context.contains("complete answer"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_partial_keeps_order_through_paging_unload_reload_and_restart() {
    for mode in [StorageMode::Memory, StorageMode::Persistent] {
        let directory = tempfile::tempdir().unwrap();
        let (source, events, mut starts) = resources(3, None);
        let execution = source
            .resolve(&SessionConfig::default())
            .await
            .unwrap()
            .execution;
        let model = Arc::new(CaptureModel {
            inner: Arc::clone(execution.model()),
            requests: Mutex::new(Vec::new()),
        });
        let resources: Arc<dyn ResourceResolver> = Arc::new(Resources {
            model: model.clone(),
            tool: Some(Arc::new(UnfinishedTool(
                ToolSpec::new(
                    "fragmented".parse().unwrap(),
                    "Never execute unfinished arguments",
                    serde_json::json!({"type": "object"}),
                )
                .unwrap(),
            ))),
        });
        let host = if mode == StorageMode::Memory {
            Host::memory(resources.clone()).unwrap()
        } else {
            Host::open_persistent(
                directory.path(),
                HostSettings {
                    name: "partial history".into(),
                },
                resources.clone(),
            )
            .await
            .unwrap()
        };
        let (client, server) = stdio_pipes(host.clone()).await;
        let mut create = create_request();
        create.mode = mode;
        let made = client.create_agent(create, options(3000)).await.unwrap();
        let target = SessionRef {
            agent_id: made.agent_id.unwrap(),
            session_id: made.session_id.unwrap(),
        };
        let session = client.session(target.clone());
        let observer = client
            .subscribe(
                request(scope(&target), &[EventCategory::Output]),
                options(3000),
            )
            .await
            .unwrap();
        client
            .unsubscribe(
                UnsubscribeRequest {
                    subscription_id: observer.result().subscription_id.clone(),
                },
                options(3000),
            )
            .await
            .unwrap();
        drop(observer);
        session
            .send(input("first input"), options(3000))
            .await
            .unwrap();
        starts.recv().await.unwrap();
        let original = running(&session).await;
        events[0]
            .send(ModelEvent::ToolCallStart {
                tool_call_id: "unfinished".parse().unwrap(),
                tool_name: "fragmented".parse().unwrap(),
            })
            .unwrap();
        events[0]
            .send(ModelEvent::tool_call_arguments_delta(
                "unfinished".parse().unwrap(),
                "{\"unfinished\":",
            ))
            .unwrap();
        let usage = minicore_host_runtime::model::Usage::new(5, 7, 2);
        events[0].send(ModelEvent::Usage { usage }).unwrap();
        events[0].send(ModelEvent::text_delta(PARTIAL)).unwrap();
        // A direct range read confirms assembly; no external observer is alive.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(read) = session
                    .turn_output(
                        original.clone(),
                        0,
                        TextReadRequest {
                            channel: "text".into(),
                            start: 0,
                            end: PARTIAL.len() as u64,
                            max_bytes: 100,
                        },
                        options(3000),
                    )
                    .await
                    && read.data.text == PARTIAL
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        session
            .cancel_turn(
                TurnCancelRequest {
                    operation_id: OperationId::new().unwrap(),
                    turn_id: original.clone(),
                    keep_inbox: false,
                },
                options(3000),
            )
            .await
            .unwrap();
        let result = session
            .wait_turn(original.clone(), options(3000))
            .await
            .unwrap();
        assert_eq!(
            result.data.info.as_ref().unwrap().outcome,
            Some(TurnOutcome::Cancelled)
        );
        assert_eq!(
            result.data.info.as_ref().unwrap().persistence,
            if mode == StorageMode::Persistent {
                PersistenceState::Persisted
            } else {
                PersistenceState::NotApplicable
            }
        );
        assert_eq!(result.data.partials.len(), 1);
        let accepted = pages(&session, 1).await;
        assert_eq!(accepted.as_array().unwrap().len(), 3);
        for row in accepted.as_array().unwrap() {
            assert_eq!(row["turn_id"], value(&original));
            serde_json::from_value::<Timestamp>(row["recorded_at"].clone()).unwrap();
        }
        assert_eq!(accepted[1]["kind"], "partial");
        assert_eq!(accepted[1]["response_index"], 0);
        assert_eq!(accepted[1]["position"], result.data.partials[0].position);
        assert_eq!(
            accepted[1]["recorded_at"],
            value(result.data.partials[0].recorded_at)
        );
        assert_eq!(
            accepted[1]["partial"],
            value(&result.data.partials[0].partial)
        );
        assert_eq!(accepted[2]["kind"], "turn_finished");
        assert_eq!(accepted[2]["outcome"], "cancelled");
        assert_eq!(accepted[2]["requests"], 1);
        assert_eq!(accepted[2]["tool_rounds"], 0);
        assert_eq!(accepted[2]["model_attempts"], value(vec![1_u8]));
        let terminal_state = result.data.info.as_ref().unwrap();
        assert_eq!(
            accepted[2]["position"],
            value(terminal_state.history_position.unwrap())
        );
        assert_eq!(
            accepted[2]["recorded_at"],
            value(terminal_state.recorded_at.unwrap())
        );
        assert_eq!(
            accepted[2]["finished_at"],
            value(terminal_state.finished_at.unwrap())
        );
        let partial = &accepted[1]["partial"];
        assert!(partial.to_string().contains(PARTIAL));
        assert_eq!(
            partial["incomplete_tool_calls"][0]["tool_name"],
            "fragmented"
        );
        assert_eq!(
            partial["incomplete_tool_calls"][0]["arguments_text"],
            "{\"unfinished\":"
        );
        assert!(partial.get("finish_reason").is_none());
        assert!(partial.get("request_index").is_none());
        assert_eq!(partial["usage"], value(usage));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let state = session.state(options(3000)).await.unwrap().data;
                if !state.metadata_saving {
                    assert!(state.metadata_fault.is_none());
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        session.unload(control(), options(3000)).await.unwrap();
        assert_eq!(pages(&session, 2).await, accepted);
        assert_eq!(
            session.state(options(3000)).await.unwrap().data.residency,
            Residency::Unloaded
        );
        session.load(control(), options(3000)).await.unwrap();
        loaded(&session).await;
        assert_eq!(pages(&session, 100).await, accepted);
        session
            .send(input("second input"), options(3000))
            .await
            .unwrap();
        starts.recv().await.unwrap();
        let second = running(&session).await;
        assert_context(&model, 1);
        events[1]
            .send(ModelEvent::text_delta("complete answer"))
            .unwrap();
        finish(&events[1]);
        session.wait_turn(second, options(3000)).await.unwrap();
        let expected = pages(&session, 100).await;
        let kinds: Vec<_> = expected
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            [
                "user",
                "partial",
                "turn_finished",
                "user",
                "assistant",
                "turn_finished"
            ]
        );
        for limit in [1, 2, 3] {
            assert_eq!(pages(&session, limit).await, expected);
        }
        stop(&client).await;
        client.close();
        server.await.unwrap().unwrap();
        drop(session);
        drop(client);
        drop(host);
        if mode == StorageMode::Memory {
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
            continue;
        }
        let path = directory
            .path()
            .join("agents")
            .join(target.agent_id.as_str())
            .join("sessions")
            .join(target.session_id.as_str())
            .join("history.jsonl");
        let bytes = std::fs::read(&path).unwrap();
        let records: Vec<Value> = std::str::from_utf8(&bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            records
                .iter()
                .filter(|r| r["entry"]["kind"] == "partial")
                .count(),
            1
        );
        let reopened = Host::open_persistent(
            directory.path(),
            HostSettings {
                name: "ignored".into(),
            },
            resources,
        )
        .await
        .unwrap();
        let (client, server) = stdio_pipes(reopened.clone()).await;
        let session = client.session(target);
        let before = session.state(options(3000)).await.unwrap();
        assert_eq!(before.data.residency, Residency::Unloaded);
        for limit in [1, 2, 3, 100] {
            assert_eq!(pages(&session, limit).await, expected);
        }
        assert_eq!(
            value(session.state(options(3000)).await.unwrap().data),
            value(before.data)
        );
        let saved_result = session
            .turn_result(original.clone(), options(3000))
            .await
            .unwrap();
        assert!(saved_result.source.load_id.is_none());
        assert!(saved_result.data.info.is_none());
        assert_eq!(
            value(&saved_result.data.history),
            value(&result.data.history)
        );
        assert_eq!(
            value(&saved_result.data.partials),
            value(&result.data.partials)
        );
        let saved_terminal = saved_result.data.saved_terminal.unwrap();
        assert_eq!(
            saved_terminal.position,
            terminal_state.history_position.unwrap()
        );
        assert_eq!(
            saved_terminal.recorded_at,
            terminal_state.recorded_at.unwrap()
        );
        assert_eq!(
            saved_terminal.finished_at,
            terminal_state.finished_at.unwrap()
        );
        assert!(session.turn(original, options(3000)).await.is_err());
        assert!(session.turns(options(3000)).await.unwrap().data.is_empty());
        assert!(starts.try_recv().is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "reads never rewrite JSONL"
        );
        session
            .send(input("after restart"), options(3000))
            .await
            .unwrap();
        starts.recv().await.unwrap();
        let third = running(&session).await;
        assert_context(&model, 2);
        events[2]
            .send(ModelEvent::text_delta("new answer"))
            .unwrap();
        finish(&events[2]);
        session.wait_turn(third, options(3000)).await.unwrap();
        let after = pages(&session, 1).await;
        assert_eq!(
            &after.as_array().unwrap()[..expected.as_array().unwrap().len()],
            expected.as_array().unwrap()
        );
        stop(&client).await;
        client.close();
        server.await.unwrap().unwrap();
    }
}
