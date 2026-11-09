use super::*;

async fn finished_compaction(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
) -> CompactionActivity {
    let mut states = host.session_states(caller, target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(activity) = states.snapshot().unwrap().data.compaction
                && activity.state.finished()
            {
                return *activity;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap()
}
struct BindProbe {
    inner: Probe,
    binds: Arc<AtomicUsize>,
}
impl ToolFactory for BindProbe {
    fn spec(&self) -> &ToolSpec {
        &self.inner.spec
    }
    fn bind(&self, port: Arc<dyn TurnPort>) -> Result<Arc<dyn Tool>, Fault> {
        self.binds.fetch_add(1, Ordering::SeqCst);
        self.inner.bind(port)
    }
}
fn compaction_builder() -> (HostResourceBuilder, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let binds = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut tools = ToolRegistry::new();
    tools
        .register(Arc::new(BindProbe {
            inner: Probe {
                spec: ToolSpec::new(
                    "probe".parse().unwrap(),
                    "a real selected spec",
                    json!({"type":"object"}),
                )
                .unwrap(),
                calls: calls.clone(),
            },
            binds: binds.clone(),
        }))
        .unwrap();
    (HostResourceBuilder::new(Arc::new(tools)), binds, calls)
}
fn small_config(endpoint: &str) -> HostConfig {
    let mut config = config(endpoint);
    config.models[0].context_window = 4096;
    config.tools = selection();
    config
}
fn small_request(automatic: bool) -> AgentCreate {
    let mut request = request(selection(), None);
    request.config.session_defaults.execution.compaction = CompactionSettings {
        enabled: automatic,
        trigger_percent: 50,
        target_percent: 20,
    };
    request
}
#[tokio::test]
async fn http_manual_compaction_uses_real_model_prompt_specs_without_a_turn_or_tool_binding() {
    let gate = Arc::new(Semaphore::new(0));
    let mut server = Server::start(vec![
        (answer(&"recorded answer ".repeat(150)), None),
        (answer(&"second answer ".repeat(170)), None),
        (answer("Concise actual HTTP summary."), Some(gate.clone())),
        (answer("after summary"), None),
    ])
    .await;
    let (builder, binds, calls) = compaction_builder();
    let host =
        Host::memory_configured(prepare(&builder, small_config(&server.endpoint), "key").await)
            .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let receipt = host.create_agent(&caller, small_request(false)).unwrap();
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
        server.request().await;
        settled(&host, &caller, &target, index + 1).await;
    }
    let raw = host.history(&caller, &target).unwrap();
    host.session_pause(
        &caller,
        &target,
        SessionControlRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    let operation = OperationId::new().unwrap();
    let formed = host
        .session_compact(
            &caller,
            &target,
            CompactRequest {
                operation_id: operation.clone(),
            },
        )
        .unwrap();
    let utility = server.request().await;
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::Compact(formed.clone())
    );
    assert!(
        utility["messages"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("Summarize historical conversation data")
    );
    assert!(utility.get("tools").is_none());
    assert_eq!(
        binds.load(Ordering::SeqCst),
        2,
        "manual utility must not fabricate a Turn to prepare selected tools"
    );
    assert_eq!(host.turns(&caller, &target).unwrap().len(), 2);
    send(
        &host,
        &caller,
        &target,
        "queued while HTTP utility is pending",
    );
    gate.add_permits(1);
    let activity = finished_compaction(&host, &caller, &target).await;
    assert!(matches!(
        activity.state,
        CompactionState::Finished {
            result: CompactionOutcome::Compacted {
                persistence: PersistenceState::NotApplicable,
                ..
            }
        }
    ));
    assert_eq!(activity.utility_usage.len(), 1);
    assert!(activity.utility_usage[0].completed);
    assert_eq!(activity.utility_usage[0].usage, None);
    assert_eq!(host.history(&caller, &target).unwrap(), raw);
    assert_eq!(binds.load(Ordering::SeqCst), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::Compact(formed.clone())
    );
    let cancel = host
        .session_compact_cancel(
            &caller,
            &target,
            CompactCancelRequest {
                operation_id: OperationId::new().unwrap(),
                target_operation_id: operation.clone(),
            },
        )
        .unwrap();
    assert!(!cancel.result.cancel_claimed);
    assert_eq!(cancel.result.target.state, activity.state);
    assert!(!cancel.result.target.cancellation_requested);
    assert!(cancel.result.target.controller_revision > formed.result.controller_revision);
    assert_eq!(
        host.receipt_get(&caller, &operation).unwrap(),
        Receipt::Compact(formed)
    );
    host.session_resume(
        &caller,
        &target,
        SessionControlRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    let next = server.request().await;
    settled(&host, &caller, &target, 3).await;
    assert_eq!(
        host.receipt_get(&caller, &cancel.operation_id).unwrap(),
        Receipt::CompactCancel(cancel)
    );
    let messages = next["messages"].as_array().unwrap();
    assert_eq!(messages[0]["content"], "FRAMEWORK A");
    assert_eq!(messages[1]["content"], "ROLE A");
    assert!(messages.iter().any(|message| {
        message["content"]
            .as_str()
            .is_some_and(|text| text.starts_with("Conversation summary:"))
    }));
    assert_eq!(next["tools"][0]["function"]["name"], "probe");
    assert_eq!(binds.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn http_automatic_compaction_runs_at_real_prompt_boundary_and_usage_stays_separate() {
    let mut server = Server::start(vec![
        (answer(&"recorded answer ".repeat(150)), None),
        (answer(&"second answer ".repeat(170)), None),
        (answer("Automatic HTTP summary."), None),
        (answer("third normal response"), None),
    ])
    .await;
    let (builder, _, calls) = compaction_builder();
    let host =
        Host::memory_configured(prepare(&builder, small_config(&server.endpoint), "key").await)
            .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let receipt = host.create_agent(&caller, small_request(true)).unwrap();
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
        let request = server.request().await;
        assert!(
            !request["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("Summarize")
        );
        settled(&host, &caller, &target, index + 1).await;
    }
    send(
        &host,
        &caller,
        &target,
        "new protected input ".repeat(20).as_str(),
    );
    let utility = server.request().await;
    assert!(
        utility["messages"][0]["content"]
            .as_str()
            .unwrap()
            .starts_with("Summarize")
    );
    assert!(utility.get("tools").is_none());
    let request = server.request().await;
    settled(&host, &caller, &target, 3).await;
    assert!(
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["content"]
                .as_str()
                .is_some_and(|text| text.starts_with("Conversation summary:")))
    );
    let turns = host.turns(&caller, &target).unwrap();
    assert_eq!(turns.len(), 3);
    assert!(turns.iter().all(|turn| turn.requests == 1));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let activity = finished_compaction(&host, &caller, &target).await;
    assert!(matches!(
        activity.source,
        CompactionSource::Automatic {
            response_index: 0,
            ..
        }
    ));
    assert_eq!(activity.utility_usage.len(), 1);
    assert_eq!(host.history(&caller, &target).unwrap().len(), 6);
}
