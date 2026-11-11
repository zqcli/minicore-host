//! Real owner/registry/Runtime/local tools with a named in-memory record fixture.
//! This is not a production persistence sink or a live provider test.
mod support;
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_local_tools::{
    CapturedEdit, CommandResult, CommandStatus, LocalToolSink, OutputChannel, RecordError,
    local_tool_specs,
};
use minicore_host_protocol::*;
use minicore_host_tools::*;
use minicore_host_workspace::{FileSnapshot, Workspace};
use minicore_runtime::{
    ExecutionConfig, LoopOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::{ToolContext, ToolError, ToolInvocation, ToolSet},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct TemporaryDirectory(PathBuf);
impl TemporaryDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("minicore-registry-{}", SessionId::new().unwrap()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }
    #[cfg(windows)]
    {
        let output = std::process::Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .unwrap();
        assert!(output.status.success(), "native process query must succeed");
        String::from_utf8_lossy(&output.stdout).contains(&format!(",\"{pid}\","))
    }
}

#[derive(Default)]
struct FixtureRecords {
    outputs: Mutex<BTreeMap<String, ToolOutputLease>>,
    bytes: Mutex<BTreeMap<(String, String), Vec<u8>>>,
    edits: Mutex<Vec<(String, CapturedEdit)>>,
    commands: Mutex<Vec<(String, CommandResult)>>,
    fail_command_record: AtomicBool,
}
impl LocalToolInvocationSink for FixtureRecords {
    fn begin(
        &self,
        invocation: &ToolInvocation,
        output: ToolOutputLease,
    ) -> Result<(), RecordError> {
        assert_eq!(
            output.source().tool_call_id.0,
            invocation.tool_call_id().as_str()
        );
        let old = self
            .outputs
            .lock()
            .unwrap()
            .insert(invocation.tool_call_id().as_str().into(), output);
        assert!(
            old.is_none(),
            "fixture calls are unique within the public Turn"
        );
        Ok(())
    }
}
impl LocalToolSink for FixtureRecords {
    fn output(
        &self,
        call: &minicore_runtime::ToolCallId,
        channel: OutputChannel,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), RecordError> {
        let channel = match channel {
            OutputChannel::Stdout => "stdout",
            OutputChannel::Stderr => "stderr",
        };
        let mut all = self.bytes.lock().unwrap();
        let body = all
            .entry((call.as_str().into(), channel.into()))
            .or_default();
        assert_eq!(body.len() as u64, offset);
        body.extend_from_slice(bytes);
        // These fixture commands intentionally output only complete ASCII chunks.
        let text = std::str::from_utf8(bytes).map_err(|_| RecordError)?;
        self.outputs
            .lock()
            .unwrap()
            .get(call.as_str())
            .ok_or(RecordError)?
            .emit(channel, text)
            .then_some(())
            .ok_or(RecordError)
    }
    fn edit(
        &self,
        call: &minicore_runtime::ToolCallId,
        edit: CapturedEdit,
    ) -> Result<(), RecordError> {
        assert!(self.outputs.lock().unwrap().contains_key(call.as_str()));
        self.edits
            .lock()
            .unwrap()
            .push((call.as_str().into(), edit));
        Ok(())
    }
    fn command(
        &self,
        call: &minicore_runtime::ToolCallId,
        result: &CommandResult,
    ) -> Result<(), RecordError> {
        assert!(self.outputs.lock().unwrap().contains_key(call.as_str()));
        self.commands
            .lock()
            .unwrap()
            .push((call.as_str().into(), result.clone()));
        if self.fail_command_record.load(Ordering::SeqCst) {
            Err(RecordError)
        } else {
            Ok(())
        }
    }
}
struct Call {
    id: &'static str,
    name: &'static str,
    arguments: Value,
}
struct ScriptedModel {
    descriptor: ModelDescriptor,
    calls: Mutex<VecDeque<Option<Call>>>,
    declarations: Mutex<Vec<Vec<String>>>,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.declarations.lock().unwrap().push(
            request
                .tools()
                .iter()
                .map(|tool| tool.name().as_str().to_owned())
                .collect(),
        );
        let call = self
            .calls
            .lock()
            .unwrap()
            .pop_front()
            .expect("declared model fixture call");
        let events = match call {
            Some(call) => vec![
                ModelEvent::ToolCallStart {
                    tool_call_id: call.id.parse().unwrap(),
                    tool_name: call.name.parse().unwrap(),
                },
                ModelEvent::tool_call_arguments_delta(
                    call.id.parse().unwrap(),
                    call.arguments.to_string(),
                ),
                ModelEvent::ToolCallEnd {
                    tool_call_id: call.id.parse().unwrap(),
                },
                ModelEvent::Finish {
                    reason: ModelFinishReason::ToolCalls,
                },
            ],
            None => vec![
                ModelEvent::text_delta("done"),
                ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                },
            ],
        };
        Box::pin(async { Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as ModelStream) })
    }
}
struct Resources {
    registry: ToolRegistry,
    model: Arc<ScriptedModel>,
    records: Mutex<Vec<Arc<FixtureRecords>>>,
    reject: bool,
    fail_command_record: bool,
}
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionConfigInvalid,
                "fixture requires actual owner binding",
            ))
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        config: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            let records = Arc::new(FixtureRecords::default());
            records
                .fail_command_record
                .store(self.fail_command_record, Ordering::SeqCst);
            self.records.lock().unwrap().push(records.clone());
            let workspace = Workspace::bind(config.workspace.as_ref()).await.unwrap();
            let local = BoundLocalTools::prepare(binding.clone(), workspace, records)?;
            let tools = self.registry.bind_with_local(
                &config.tools,
                CoreTurnPort::prepared(binding),
                &local,
            )?;
            if self.reject {
                // A prepared implementation must not execute before activation.
                let error = tools
                    .get(&"write".parse().unwrap())
                    .unwrap()
                    .execute(
                        ToolInvocation::new(
                            "premature".parse().unwrap(),
                            "write".parse().unwrap(),
                            json!({"path":"forbidden","content":"never"}),
                        )
                        .unwrap(),
                        ToolContext {
                            cancellation: Default::default(),
                            deadline: Instant::now() + Duration::from_secs(2),
                            progress: Default::default(),
                        },
                    )
                    .await
                    .unwrap_err();
                assert_eq!(error, ToolError::Internal);
                return Err(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "explicit rejected registry fixture",
                ));
            }
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    tools,
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: LoopOptions::default_checked().unwrap(),
            })
        })
    }
    fn bind_tool_output(
        &self,
        execution: &ExecutionConfig,
        output: ToolOutputBinding,
    ) -> ExecutionConfig {
        let mut tools = ToolSet::builder();
        for spec in self.registry.specs() {
            if let Some(tool) = execution.tools().get(spec.name()) {
                tools.register_arc(output.wrap(tool));
            }
        }
        ExecutionConfig::new(
            execution.model().clone(),
            execution.reasoning(),
            tools.build().unwrap(),
            execution.policy().cloned(),
            execution.prompt().clone(),
        )
        .unwrap()
    }
}
fn resources(calls: Vec<Option<Call>>, reject: bool, fail_command_record: bool) -> Arc<Resources> {
    let mut registry = ToolRegistry::new();
    registry.register_local_tools().unwrap();
    Arc::new(Resources {
        registry,
        model: Arc::new(ScriptedModel {
            descriptor: ModelDescriptor::new(
                "fixture/model".parse().unwrap(),
                128_000,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
            calls: Mutex::new(calls.into()),
            declarations: Mutex::new(Vec::new()),
        }),
        records: Mutex::new(Vec::new()),
        reject,
        fail_command_record,
    })
}
async fn setup(
    resources: Arc<Resources>,
    root: &TemporaryDirectory,
    selected: &[&str],
) -> (Arc<Host>, ConnectionHandle, SessionRef) {
    let host = Host::memory(resources).unwrap();
    let connection = host.connect().unwrap();
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    config.session_defaults.workspace = Some(WorkspaceRef {
        path: root.0.to_str().unwrap().into(),
    });
    config.session_defaults.tools = selected
        .iter()
        .map(|name| ToolBinding {
            name: (*name).into(),
        })
        .collect();
    let created = host
        .create_agent_async(
            &connection.caller(),
            AgentCreate {
                operation_id: OperationId::new().unwrap(),
                name: "local registry fixture".into(),
                mode: StorageMode::Memory,
                config,
                default_session: None,
                initial_message: None,
            },
        )
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    (host, connection, target)
}
fn send(host: &Host, caller: &CallerContext, target: &SessionRef) {
    host.send(
        caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                mode: MessageMode::FollowUp,
                text: "run local fixture".into(),
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap();
}
async fn until<T>(mut condition: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(value) = condition() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("fixture state")
}
async fn finish(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    count: usize,
) -> TurnResult {
    let info = until(|| {
        let turns = host.turns(caller, target).unwrap();
        (turns.len() == count)
            .then(|| turns.last().cloned())
            .flatten()
    })
    .await;
    tokio::time::timeout(
        Duration::from_secs(5),
        host.wait_turn(caller, target, &info.id),
    )
    .await
    .unwrap()
    .unwrap()
}

#[test]
fn static_declarations_require_a_real_workspace_record_binding_and_reject_duplicates() {
    let mut registry = ToolRegistry::new();
    registry.register_local_tools().unwrap();
    let mut expected = local_tool_specs();
    expected.sort_by(|a, b| a.name().cmp(b.name()));
    assert_eq!(registry.specs(), expected);
    assert!(registry.register_local_tools().is_err());
    let port = support::FixtureTurn::new();
    let result = registry.bind(
        &[ToolBinding {
            name: "write".into(),
        }],
        port.port(),
    );
    assert_eq!(
        result.err().unwrap().kind(),
        Some(FaultKind::SessionResourceMissing)
    );
}

#[tokio::test]
async fn real_local_tools_are_selected_recorded_and_joined_under_actual_turn() {
    let root = TemporaryDirectory::new();
    let resources = resources(
        vec![
            Some(Call {
                id: "write-1",
                name: "write",
                arguments: json!({"path":"file","content":"old"}),
            }),
            Some(Call {
                id: "edit-1",
                name: "edit",
                arguments: json!({"path":"file","old_text":"old","new_text":"actual"}),
            }),
            Some(Call {
                id: "read-1",
                name: "read",
                arguments: json!({"path":"file","limit":10}),
            }),
            Some(Call {
                id: "bash-1",
                name: "bash",
                arguments: json!({"command":"cat file"}),
            }),
            None,
        ],
        false,
        false,
    );
    let (host, connection, target) =
        setup(resources.clone(), &root, &["read", "write", "edit", "bash"]).await;
    let caller = connection.caller();
    send(&host, &caller, &target);
    let result = finish(&host, &caller, &target, 1).await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Completed));
    assert!(result.info.cleanup_faults.is_empty());
    assert_eq!(std::fs::read(root.0.join("file")).unwrap(), b"actual");
    let records = resources.records.lock().unwrap()[0].clone();
    let edits = records.edits.lock().unwrap();
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0].1.before, FileSnapshot::Missing);
    assert_eq!(edits[0].1.after, edits[1].1.before);
    assert_eq!(edits[1].1.after, FileSnapshot::Content(b"actual".to_vec()));
    drop(edits);
    let command = records.commands.lock().unwrap()[0].1.clone();
    assert_eq!(command.status, CommandStatus::Exited);
    assert!(command.process_waited && command.cleanup_confirmed);
    let output = records.outputs.lock().unwrap()["bash-1"].clone();
    assert_eq!(output.source().turn.turn_id, result.info.id);
    assert_eq!(
        output.source().turn.load_id.as_ref(),
        Some(&result.info.load_id)
    );
    let data = host
        .toolcall_output_read(
            &caller,
            &target,
            &ToolCallRef {
                turn_id: result.info.id,
                tool_call_id: ToolCallId("bash-1".into()),
            },
            &TextReadRequest {
                channel: "stdout".into(),
                start: 0,
                end: 6,
                max_bytes: 100,
            },
        )
        .unwrap();
    assert_eq!(data.data.text, "actual");
    assert!(!output.emit("stdout", "late"));
}

#[tokio::test]
async fn rejected_candidate_cannot_execute_prepared_local_tool() {
    let root = TemporaryDirectory::new();
    let resources = resources(vec![], true, false);
    let (host, connection, target) = setup(resources.clone(), &root, &["write"]).await;
    let caller = connection.caller();
    send(&host, &caller, &target);
    until(|| {
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .block_reason
    })
    .await;
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    assert!(!root.0.join("forbidden").exists());
    assert!(
        resources.records.lock().unwrap()[0]
            .outputs
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn registry_selection_restricts_both_model_declarations_and_actual_execution() {
    let root = TemporaryDirectory::new();
    let resources = resources(
        vec![
            Some(Call {
                id: "unselected",
                name: "write",
                arguments: json!({"path":"forbidden","content":"never"}),
            }),
            None,
        ],
        false,
        false,
    );
    let (host, connection, target) = setup(resources.clone(), &root, &["read"]).await;
    let caller = connection.caller();
    send(&host, &caller, &target);
    let result = finish(&host, &caller, &target, 1).await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
    assert!(!root.0.join("forbidden").exists());
    assert!(
        resources
            .model
            .declarations
            .lock()
            .unwrap()
            .iter()
            .all(|names| names == &["read"])
    );
    assert!(
        resources.records.lock().unwrap()[0]
            .outputs
            .lock()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn cancelled_real_bash_is_waited_before_finished_and_same_call_id_is_turn_scoped() {
    let root = TemporaryDirectory::new();
    let resources = resources(
        vec![
            Some(Call {
                id: "same-call",
                name: "bash",
                arguments: json!({"command":"printf ready; sleep 30"}),
            }),
            Some(Call {
                id: "same-call",
                name: "bash",
                arguments: json!({"command":"printf next"}),
            }),
            None,
        ],
        false,
        false,
    );
    let (host, connection, target) = setup(resources.clone(), &root, &["bash"]).await;
    let caller = connection.caller();
    send(&host, &caller, &target);
    let records = until(|| resources.records.lock().unwrap().first().cloned()).await;
    until(|| {
        records
            .bytes
            .lock()
            .unwrap()
            .get(&("same-call".into(), "stdout".into()))
            .filter(|bytes| bytes.as_slice() == b"ready")
            .cloned()
    })
    .await;
    let output = records.outputs.lock().unwrap()["same-call"].clone();
    let turn = output.source().turn.turn_id.clone();
    host.turn_cancel(
        &caller,
        &target,
        TurnCancelRequest {
            operation_id: OperationId::new().unwrap(),
            turn_id: turn.clone(),
            keep_inbox: false,
        },
    )
    .unwrap();
    assert_eq!(host.turns(&caller, &target).unwrap().len(), 1);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        host.wait_turn(&caller, &target, &turn),
    )
    .await
    .unwrap_or_else(|error| {
        let commands = records.commands.lock().unwrap().clone();
        let turns = host.turns(&caller, &target).unwrap();
        panic!("cancelled Turn did not finish: {error}; commands: {commands:?}; turns: {turns:?}")
    })
    .unwrap();
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert!(result.info.cleanup_faults.is_empty());
    let command = records.commands.lock().unwrap()[0].1.clone();
    assert_eq!(command.status, CommandStatus::Cancelled);
    assert!(command.process_waited && command.cleanup_confirmed);
    assert!(!process_exists(command.pid));
    assert!(!output.emit("stdout", "late"));
    send(&host, &caller, &target);
    let second = finish(&host, &caller, &target, 2).await;
    let second_output =
        resources.records.lock().unwrap()[1].outputs.lock().unwrap()["same-call"].clone();
    assert_ne!(second.info.id, turn);
    assert_eq!(
        second_output.source().tool_call_id,
        output.source().tool_call_id
    );
    assert_eq!(second_output.source().turn.turn_id, second.info.id);
}

#[tokio::test]
async fn joined_record_failure_is_diagnostic_not_unconfirmed_cleanup() {
    let root = TemporaryDirectory::new();
    let resources = resources(
        vec![Some(Call {
            id: "record-error",
            name: "bash",
            arguments: json!({"command":"printf actual"}),
        })],
        false,
        true,
    );
    let (host, connection, target) = setup(resources.clone(), &root, &["bash"]).await;
    let caller = connection.caller();
    send(&host, &caller, &target);
    let result = finish(&host, &caller, &target, 1).await;
    assert_eq!(result.info.state, TurnState::Finished);
    assert_eq!(result.info.cleanup_faults.len(), 1);
    assert!(
        resources.records.lock().unwrap()[0]
            .commands
            .lock()
            .unwrap()[0]
            .1
            .process_waited
    );
}
