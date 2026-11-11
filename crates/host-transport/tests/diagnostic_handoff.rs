//! Real Host/Runtime/HTTP; the bound delegation tool and gated result sink are
//! explicit test fixtures. No public observer or synthesized diagnostic is used.
mod diagnostic_support;
mod os_pipe;
use diagnostic_support::*;
use minicore_host_core::{
    BoundResources, DelegationCompletion, DelegationDisposition, DelegationPermit,
    DelegationRegistration, DelegationSink, Host, OwnerFuture, PreparedTurnBinding, ResourceFuture,
    ResourceResolver, ScopeHooks, TurnCapability,
};
use minicore_host_diagnostics::{Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_provider::{OpenAiChatConfig, OpenAiChatModel, SecretApiKey};
use minicore_host_transport::HostClient;
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{Model, ModelDescriptor, ReasoningPreference as RuntimeReasoning},
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput, ToolSet,
        ToolSpec,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

struct Hooks;
impl ScopeHooks for Hooks {
    fn seal(&self) {}
    fn close_members(&self) -> OwnerFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn root_closed(&self) {}
}
struct Sink {
    entered: Semaphore,
    release: Mutex<bool>,
    changed: Condvar,
    result: Mutex<Option<DelegationCompletion>>,
    fail: bool,
}
impl DelegationSink for Sink {
    fn handoff(&self, completion: DelegationCompletion) -> Result<(), Fault> {
        *self.result.lock().unwrap() = Some(completion);
        self.entered.add_permits(1);
        let (_released, timeout) = self
            .changed
            .wait_timeout_while(self.release.lock().unwrap(), WAIT, |released| !*released)
            .unwrap();
        assert!(!timeout.timed_out(), "test must release its result sink");
        if self.fail {
            Err(Fault::new(
                FaultKind::SessionInternalUnexpected,
                "PRIVATE_HANDOFF_SINK_FAULT",
            ))
        } else {
            Ok(())
        }
    }
}
struct Plan {
    child: Mutex<Option<SessionRef>>,
    sink: Arc<Sink>,
    accepted: mpsc::UnboundedSender<(DelegationPermit, DelegationRegistration, MessageInput)>,
}
struct Delegate {
    spec: ToolSpec,
    cap: TurnCapability,
    plan: Arc<Plan>,
}
impl Tool for Delegate {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, call: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            self.cap.prepare_scope(Arc::new(Hooks)).unwrap();
            let source = self.cap.source();
            let task = input();
            let registration = DelegationRegistration {
                host_id: source.host_id.clone(),
                host_incarnation: source.host_incarnation.clone(),
                root: source.session.clone(),
                parent: source.session.clone(),
                child: self.plan.child.lock().unwrap().clone().unwrap(),
                task_message_id: task.message_id.clone(),
                origin: ToolCallRef {
                    turn_id: source.turn.turn_id.clone(),
                    tool_call_id: ToolCallId(call.tool_call_id.as_str().into()),
                },
                temporary: false,
            };
            let permit = self
                .cap
                .prepare_delegation(registration.clone(), self.plan.sink.clone())
                .unwrap();
            let rejected = input();
            let fault = self
                .cap
                .send_delegated(&registration, rejected.clone())
                .unwrap_err();
            assert_eq!(fault.kind(), Some(FaultKind::SessionInputInvalid));
            self.cap.send_delegated(&registration, task).unwrap();
            self.cap
                .record_delegation(&registration.origin, &registration);
            self.plan
                .accepted
                .send((permit, registration, rejected))
                .unwrap();
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("PRIVATE_DELEGATION_TOOL_RESULT").unwrap(),
            ))
        })
    }
}
struct Resources {
    parent: Arc<dyn Model>,
    child: Arc<dyn Model>,
    plan: Arc<Plan>,
}
impl ResourceResolver for Resources {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config.validate().map_err(|_| {
            Fault::new(
                FaultKind::SessionConfigInvalid,
                "invalid test configuration",
            )
        })
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        panic!("test uses actual Turn binding")
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            let parent = config.model.as_str() == "diagnostic/parent";
            let mut tools = ToolSet::builder();
            if parent {
                tools.register_arc(Arc::new(Delegate {
                    spec: ToolSpec::new(
                        "delegate_fixture".parse().unwrap(),
                        "Fixture delegation of real child execution",
                        json!({"type":"object"}),
                    )
                    .unwrap(),
                    cap: binding.capability(),
                    plan: self.plan.clone(),
                }));
            }
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    if parent {
                        self.parent.clone()
                    } else {
                        self.child.clone()
                    },
                    RuntimeReasoning::Auto,
                    tools.build().unwrap(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
}
fn model(id: &str, endpoint: &str) -> Arc<dyn Model> {
    Arc::new(
        OpenAiChatModel::new(
            OpenAiChatConfig::new(
                endpoint,
                PROVIDER_MODEL,
                ModelDescriptor::new(
                    id.parse().unwrap(),
                    128_000,
                    BTreeSet::from([RuntimeReasoning::Auto]),
                    true,
                )
                .unwrap(),
            ),
            SecretApiKey::new(KEY).unwrap(),
        )
        .unwrap(),
    )
}
fn delegate_reply() -> Reply {
    Reply {
        status: 200,
        gate: None,
        body: format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"original-delegation-call","type":"function","function":{"name":"delegate_fixture","arguments":"{}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
        ),
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn handoff_records_actual_child_completion_and_error_after_ended_parent_without_body() {
    let directory = tempfile::tempdir().unwrap();
    let log = directory.path().join("diagnostic.jsonl");
    let mut logging = Options::new("diagnostic-handoff");
    logging.level = tracing::Level::TRACE;
    logging.output = Output::File(log.clone());
    let mut logging = Some(logging);
    let mut guard = None;
    let mut expected = Vec::new();
    // The same real business path first runs without any installed subscriber.
    for (fail, enabled) in [(false, false), (false, true), (true, true)] {
        if enabled && guard.is_none() {
            guard = Some(minicore_host_diagnostics::install(logging.take().unwrap()).unwrap());
        }
        let parent_http = Http::start(vec![
            delegate_reply(),
            Reply::gated(Arc::new(Semaphore::new(0))),
        ])
        .await;
        let child_http = Http::start(vec![Reply::success()]).await;
        let sink = Arc::new(Sink {
            entered: Semaphore::new(0),
            release: Mutex::new(false),
            changed: Condvar::new(),
            result: Mutex::new(None),
            fail,
        });
        let (accepted, mut registrations) = mpsc::unbounded_channel();
        let plan = Arc::new(Plan {
            child: Mutex::new(None),
            sink: sink.clone(),
            accepted,
        });
        let host = Host::memory(Arc::new(Resources {
            parent: model("diagnostic/parent", &parent_http.endpoint),
            child: model("diagnostic/child", &child_http.endpoint),
            plan: plan.clone(),
        }))
        .unwrap();
        let client = HostClient::in_process(host.clone()).unwrap();
        let mut creation = create(None);
        creation.config.session_defaults.model = "diagnostic/child".parse().unwrap();
        let child = target(&client.create_agent(creation, options()).await.unwrap());
        *plan.child.lock().unwrap() = Some(child.clone());
        let mut creation = create(None);
        creation.config.session_defaults.model = "diagnostic/parent".parse().unwrap();
        creation.config.session_defaults.execution.approval = ApprovalMode::Auto;
        creation.config.session_defaults.tools = vec![ToolBinding {
            name: "delegate_fixture".into(),
        }];
        let parent = target(&client.create_agent(creation, options()).await.unwrap());
        let parent_client = client.session(parent.clone());
        parent_client.send(input(), options()).await.unwrap();
        let (permit, registration, rejected_send) =
            tokio::time::timeout(WAIT, registrations.recv())
                .await
                .unwrap()
                .unwrap();
        tokio::time::timeout(WAIT, parent_http.started.acquire_many(2))
            .await
            .unwrap()
            .unwrap()
            .forget();
        tokio::time::timeout(WAIT, sink.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        let child_client = client.session(child.clone());
        assert!(
            child_client
                .state(options())
                .await
                .unwrap()
                .data
                .owned_work
                .result_handoff
        );
        parent_client
            .cancel(
                SessionCancelRequest {
                    operation_id: OperationId::new().unwrap(),
                    keep_inbox: false,
                },
                options(),
            )
            .await
            .unwrap();
        let parent_done = parent_client
            .wait_turn(registration.origin.turn_id.clone(), options())
            .await
            .unwrap();
        assert_eq!(
            parent_done.data.info.unwrap().outcome,
            Some(TurnOutcome::Cancelled)
        );
        parent_client
            .pause(
                SessionControlRequest {
                    operation_id: OperationId::new().unwrap(),
                },
                options(),
            )
            .await
            .unwrap();
        *sink.release.lock().unwrap() = true;
        sink.changed.notify_all();
        tokio::time::timeout(WAIT, async {
            while child_client
                .state(options())
                .await
                .unwrap()
                .data
                .owned_work
                .result_handoff
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let completion = sink.result.lock().unwrap().take().unwrap();
        assert_eq!(completion.results.len(), 1);
        assert_eq!(
            completion.results[0].info.outcome,
            Some(TurnOutcome::Completed)
        );
        let notice_input = input();
        let notice = permit.notify(notice_input.clone()).await;
        let rejected_notice;
        if fail {
            rejected_notice = notice_input;
            let fault = notice.unwrap_err();
            assert_eq!(fault.kind(), Some(FaultKind::SessionBusy));
            assert_eq!(
                child_client
                    .state(options())
                    .await
                    .unwrap()
                    .data
                    .metadata_fault
                    .unwrap()
                    .kind(),
                Some(FaultKind::SessionInternalUnexpected)
            );
            permit
                .release(DelegationDisposition::NotificationFailed(fault))
                .await
                .unwrap();
            // The original close fault stays owned. No clean Host shutdown or
            // successful handoff is claimed for this deliberately rejected sink.
        } else {
            rejected_notice = input();
            let duplicate = permit.notify(rejected_notice.clone()).await.unwrap_err();
            assert_eq!(duplicate.kind(), Some(FaultKind::SessionInputInvalid));
            permit
                .release(DelegationDisposition::Notified(notice.unwrap()))
                .await
                .unwrap();
            parent_client
                .close(
                    SessionCloseRequest {
                        operation_id: OperationId::new().unwrap(),
                    },
                    options(),
                )
                .await
                .unwrap();
            stop(&host).await;
        }
        assert_eq!(child_http.finish().await.len(), 1);
        assert_eq!(parent_http.count(), 2);
        parent_http.abort().await;
        if enabled {
            expected.push((
                registration,
                fail,
                completion.results[0].info.id.clone(),
                rejected_send,
                rejected_notice,
            ));
        }
    }
    assert_eq!(
        guard.unwrap().shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(&log).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    for (source, fail, turn, rejected_send, rejected_notice) in expected {
        for (message, phase, target) in [
            (&rejected_send, "delegated_send", &source.child),
            (&rejected_notice, "delegation_notify", &source.parent),
        ] {
            let errors: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row["event"] == "message.send_error"
                        && row["message_id"] == message.message_id.as_str()
                })
                .collect();
            assert_eq!(errors.len(), 1);
            assert_eq!(
                errors[0]["in_reply_to"],
                message.in_reply_to.as_ref().unwrap().as_str()
            );
            assert_eq!(errors[0]["attributes"]["phase"], phase);
            assert_eq!(
                errors[0]["attributes"]["task_message_id"],
                source.task_message_id.as_str()
            );
            assert_eq!(
                errors[0]["attributes"]["target_session_id"],
                target.session_id.as_str()
            );
            assert_eq!(
                errors[0]["attributes"]["target_agent_id"],
                target.agent_id.as_str()
            );
            assert_ne!(errors[0]["level"], "ERROR");
            if phase == "delegation_notify" {
                assert_eq!(errors[0]["session_id"], source.child.session_id.as_str());
                assert!(errors[0].get("turn_id").is_none());
                assert!(errors[0].get("tool_call_id").is_none());
                assert_eq!(
                    errors[0]["attributes"]["origin_turn_id"],
                    source.origin.turn_id.as_str()
                );
            } else {
                assert_eq!(errors[0]["session_id"], source.parent.session_id.as_str());
                assert_eq!(errors[0]["turn_id"], source.origin.turn_id.as_str());
            }
        }
        let events: Vec<_> = rows
            .iter()
            .filter(|row| {
                row["event"]
                    == if fail {
                        "message.result_handoff_failed"
                    } else {
                        "message.result_handed_off"
                    }
                    && row["session_id"] == source.child.session_id.as_str()
            })
            .collect();
        assert_eq!(events.len(), 1);
        let event = events[0];
        assert_eq!(event["agent_id"], source.child.agent_id.as_str());
        assert_eq!(event["message_id"], source.task_message_id.as_str());
        assert!(event.get("turn_id").is_none());
        assert!(event.get("operation_id").is_none());
        assert!(event.get("tool_call_id").is_none());
        assert_eq!(
            event["attributes"]["origin_session_id"],
            source.parent.session_id.as_str()
        );
        assert_eq!(
            event["attributes"]["origin_turn_id"],
            source.origin.turn_id.as_str()
        );
        assert_eq!(
            event["attributes"]["origin_tool_call_id"],
            source.origin.tool_call_id.as_str()
        );
        assert_eq!(event["attributes"]["turn_results"], 1);
        assert_eq!(event["attributes"]["result_turn_ids"], json!([turn]));
        assert_eq!(event["attributes"]["input_ranges"], 1);
        assert_eq!(event["attributes"]["discarded_inputs"], 0);
        assert_eq!(event["level"], if fail { "ERROR" } else { "INFO" });
        let errors = rows
            .iter()
            .filter(|row| {
                row["level"] == "ERROR" && row["session_id"] == source.child.session_id.as_str()
            })
            .count();
        assert_eq!(errors, usize::from(fail));
    }
    for private in [
        KEY,
        ROLE,
        BODY,
        ANSWER,
        FRAMEWORK,
        PROVIDER_MODEL,
        "PRIVATE_HANDOFF_SINK_FAULT",
        "PRIVATE_DELEGATION_TOOL_RESULT",
        directory.path().to_str().unwrap(),
    ] {
        assert!(!text.contains(private));
    }
}
