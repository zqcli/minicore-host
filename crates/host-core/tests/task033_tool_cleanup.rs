//! Real Host/Runtime and child-process ownership with explicitly gated fixtures.
//! No public event subscription, remote provider, or TUI is required by these tests.
use futures_util::{StreamExt, stream};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::{
        Tool, ToolContext, ToolError, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolOutput,
        ToolSet, ToolSpec,
    },
};
use std::{
    collections::{BTreeSet, VecDeque},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};

#[test]
fn owned_process_fixture_child() {
    let Some(path) = std::env::var_os("MINICORE_TASK033_PROCESS_PATH") else {
        return;
    };
    let mut token = String::new();
    std::io::stdin().read_line(&mut token).unwrap();
    let token = token
        .strip_suffix('\n')
        .expect("actual worker release token");
    std::fs::write(path, token).unwrap();
}

struct Round {
    tool: bool,
    call_id: &'static str,
    interrupt: bool,
    gate: Option<Arc<Semaphore>>,
}
impl Round {
    fn tool() -> Self {
        Self {
            tool: true,
            call_id: "shared-call",
            interrupt: false,
            gate: None,
        }
    }
    fn stop() -> Self {
        Self {
            tool: false,
            call_id: "shared-call",
            interrupt: false,
            gate: None,
        }
    }
}
struct ScriptedModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<Round>>,
    requests: AtomicUsize,
}
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let round = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("declared fixture round");
        Box::pin(async move {
            let events = stream::once(async move {
                if let Some(gate) = round.gate {
                    gate.acquire().await.unwrap().forget();
                }
                let events = if round.tool {
                    vec![
                        ModelEvent::ToolCallStart {
                            tool_call_id: round.call_id.parse().unwrap(),
                            tool_name: "process_fixture".parse().unwrap(),
                        },
                        ModelEvent::tool_call_arguments_delta(round.call_id.parse().unwrap(), "{}"),
                        ModelEvent::ToolCallEnd {
                            tool_call_id: round.call_id.parse().unwrap(),
                        },
                        ModelEvent::Finish {
                            reason: ModelFinishReason::ToolCalls,
                        },
                    ]
                } else if round.interrupt {
                    vec![ModelEvent::text_delta(
                        "actual partial before stream interruption",
                    )]
                } else {
                    vec![
                        ModelEvent::text_delta("done"),
                        ModelEvent::Finish {
                            reason: ModelFinishReason::Stop,
                        },
                    ]
                };
                stream::iter(events.into_iter().map(Ok))
            })
            .flatten();
            Ok(Box::pin(events) as ModelStream)
        })
    }
}
#[derive(Clone, Copy, Default)]
enum CleanupResult {
    #[default]
    Clean,
    Issue,
    Unconfirmed,
    Panic,
}
struct ProcessBundle {
    spec: ToolSpec,
    capability: TurnCapability,
    workers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    invoked: Semaphore,
    release: Arc<Semaphore>,
    finish_entered: Semaphore,
    finish_calls: AtomicUsize,
    future_dropped: Arc<AtomicUsize>,
    waited: Arc<AtomicUsize>,
    paths: Mutex<Vec<PathBuf>>,
    root: PathBuf,
    cleanup_result: CleanupResult,
    ordinary_output: Mutex<Vec<ToolOutputSink>>,
    owned_output: Mutex<Vec<ToolOutputLease>>,
}
impl ProcessBundle {
    fn new(binding: &PreparedTurnBinding, root: PathBuf, cleanup_result: CleanupResult) -> Self {
        Self {
            spec: ToolSpec::new(
                "process_fixture".parse().unwrap(),
                "gated actual child",
                serde_json::json!({"type":"object"}),
            )
            .unwrap(),
            capability: binding.capability(),
            workers: Mutex::new(Vec::new()),
            invoked: Semaphore::new(0),
            release: Arc::new(Semaphore::new(0)),
            finish_entered: Semaphore::new(0),
            finish_calls: AtomicUsize::new(0),
            future_dropped: Arc::new(AtomicUsize::new(0)),
            waited: Arc::new(AtomicUsize::new(0)),
            paths: Mutex::new(Vec::new()),
            root,
            cleanup_result,
            ordinary_output: Mutex::new(Vec::new()),
            owned_output: Mutex::new(Vec::new()),
        }
    }
}
struct FutureDropped(Arc<AtomicUsize>);
impl Drop for FutureDropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Tool for ProcessBundle {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }
    fn execute<'a>(&'a self, _: ToolInvocation, _: ToolContext) -> ToolFuture<'a> {
        Box::pin(async move {
            self.capability
                .check_active()
                .map_err(|_| ToolError::Internal)?;
            let _dropped = FutureDropped(self.future_dropped.clone());
            if let Some(output) = ToolOutputSink::current() {
                self.owned_output
                    .lock()
                    .unwrap()
                    .push(output.retain_owned().unwrap());
                self.ordinary_output.lock().unwrap().push(output);
            }
            let path = {
                let mut paths = self.paths.lock().unwrap();
                let path = self.root.join(format!(
                    "{}-{}",
                    self.capability.source().turn.turn_id,
                    paths.len()
                ));
                paths.push(path.clone());
                path
            };
            // The tiny process cannot finish until its real worker writes stdin.
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "owned_process_fixture_child", "--nocapture"])
                .env("MINICORE_TASK033_PROCESS_PATH", &path)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let gate = self.release.clone();
            let waited = self.waited.clone();
            let (sender, receiver) = oneshot::channel();
            let worker = tokio::spawn(async move {
                gate.acquire().await.unwrap().forget();
                tokio::task::spawn_blocking(move || {
                    child
                        .stdin
                        .take()
                        .unwrap()
                        .write_all(b"actual child exit\n")
                        .unwrap();
                    assert!(child.wait().unwrap().success());
                })
                .await
                .unwrap();
                waited.fetch_add(1, Ordering::SeqCst);
                let _ = sender.send(());
            });
            self.workers.lock().unwrap().push(worker);
            self.invoked.add_permits(1);
            receiver.await.map_err(|_| ToolError::Internal)?;
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("child joined").unwrap(),
            ))
        })
    }
}
impl TurnToolCleanup for ProcessBundle {
    fn finish(&self) -> OwnerFuture<'_, Vec<Fault>> {
        Box::pin(async move {
            self.finish_calls.fetch_add(1, Ordering::SeqCst);
            self.finish_entered.add_permits(1);
            let workers = std::mem::take(&mut *self.workers.lock().unwrap());
            for worker in workers {
                worker.await.unwrap();
            }
            let fault = || {
                Fault::new(
                    FaultKind::TurnInternalUnexpected,
                    "explicit cleanup fixture diagnostic",
                )
            };
            match self.cleanup_result {
                CleanupResult::Clean => Ok(Vec::new()),
                CleanupResult::Issue => Ok(vec![fault()]),
                CleanupResult::Unconfirmed => Err(fault()),
                CleanupResult::Panic => panic!("explicit cleanup fixture panic"),
            }
        })
    }
}
struct Resources {
    model: Arc<ScriptedModel>,
    root: PathBuf,
    bundles: Mutex<Vec<Arc<ProcessBundle>>>,
    bindings: Mutex<Vec<PreparedTurnBinding>>,
    reject: AtomicBool,
    cleanup_result: CleanupResult,
}
impl ResourceResolver for Resources {
    fn validate(&self, _: &SessionConfig) -> Result<(), Fault> {
        Ok(())
    }
    fn resolve<'a>(&'a self, _: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async {
            Err(Fault::new(
                FaultKind::SessionConfigInvalid,
                "fixture requires real binding",
            ))
        })
    }
    fn resolve_for_turn<'a>(
        &'a self,
        _: &'a SessionConfig,
        binding: PreparedTurnBinding,
    ) -> ResourceFuture<'a> {
        Box::pin(async move {
            assert!(binding.capability().check_active().is_err());
            let bundle = Arc::new(ProcessBundle::new(
                &binding,
                self.root.clone(),
                self.cleanup_result,
            ));
            binding.register_tool_cleanup(bundle.clone())?;
            self.bundles.lock().unwrap().push(bundle.clone());
            self.bindings.lock().unwrap().push(binding);
            if self.reject.load(Ordering::SeqCst) {
                return Err(Fault::new(
                    FaultKind::SessionResourceMissing,
                    "explicit rejected fixture candidate",
                ));
            }
            let mut tools = ToolSet::builder();
            tools.register_arc(bundle);
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    tools.build().unwrap(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
    fn bind_tool_output(
        &self,
        execution: &ExecutionConfig,
        output: ToolOutputBinding,
    ) -> ExecutionConfig {
        let mut tools = ToolSet::builder();
        if let Some(tool) = execution.tools().get(&"process_fixture".parse().unwrap()) {
            tools.register_arc(output.wrap(tool));
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
fn resources(root: PathBuf, rounds: Vec<Round>, cleanup_result: CleanupResult) -> Arc<Resources> {
    Arc::new(Resources {
        model: Arc::new(ScriptedModel {
            descriptor: ModelDescriptor::new(
                "fixture/model".parse().unwrap(),
                128_000,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
            rounds: Mutex::new(rounds.into()),
            requests: AtomicUsize::new(0),
        }),
        root,
        bundles: Mutex::new(Vec::new()),
        bindings: Mutex::new(Vec::new()),
        reject: AtomicBool::new(false),
        cleanup_result,
    })
}
fn request(mode: StorageMode) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "cleanup fixture".into(),
        mode,
        config,
        default_session: None,
        initial_message: None,
    }
}
async fn create(host: &Arc<Host>, caller: &CallerContext, mode: StorageMode) -> SessionRef {
    let receipt = host
        .create_agent_async(caller, request(mode))
        .await
        .unwrap();
    SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    }
}
fn send(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    mode: MessageMode,
) -> MessageAccepted {
    host.send(
        caller,
        SendMessage::new(
            MessageInput {
                message_id: MessageId::new().unwrap(),
                text: "execute fixture".into(),
                mode,
                in_reply_to: None,
            },
            target.clone(),
        ),
    )
    .unwrap()
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
    .expect("declared fixture condition")
}
async fn bundle(resources: &Resources, index: usize) -> Arc<ProcessBundle> {
    until(|| resources.bundles.lock().unwrap().get(index).cloned()).await
}
async fn invoked(bundle: &ProcessBundle) {
    tokio::time::timeout(Duration::from_secs(5), bundle.invoked.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}
async fn terminal(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    turn: &TurnId,
) -> TurnResult {
    tokio::time::timeout(Duration::from_secs(5), host.wait_turn(caller, target, turn))
        .await
        .unwrap()
        .unwrap()
}
fn cancel(host: &Host, caller: &CallerContext, target: &SessionRef, turn: &TurnId) {
    assert!(
        host.turn_cancel(
            caller,
            target,
            TurnCancelRequest {
                operation_id: OperationId::new().unwrap(),
                turn_id: turn.clone(),
                keep_inbox: false
            }
        )
        .unwrap()
        .result
        .cancel_claimed
    );
}

#[tokio::test]
async fn cancelled_runtime_future_is_not_actual_tool_completion() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(dir.path().into(), vec![Round::tool()], CleanupResult::Issue);
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    let turn = bundle.capability.source().turn.turn_id.clone();
    cancel(&host, &caller, &target, &turn);
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    assert_eq!(bundle.future_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 0);
    let state = host.turn_info(&caller, &target, &turn).unwrap();
    assert_eq!(
        (state.state, state.outcome),
        (TurnState::Finalizing, Some(TurnOutcome::Cancelled))
    );
    assert!(host.turn_result(&caller, &target, &turn).is_err());
    assert!(!bundle.paths.lock().unwrap()[0].exists());
    bundle.release.add_permits(1);
    let result = terminal(&host, &caller, &target, &turn).await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert!(result.info.fault.is_none());
    assert_eq!(result.info.cleanup_faults.len(), 1);
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read(&bundle.paths.lock().unwrap()[0]).unwrap(),
        b"actual child exit"
    );
    assert!(bundle.capability.check_active().is_err());
    assert!(
        resources.bindings.lock().unwrap()[0]
            .register_tool_cleanup(bundle.clone())
            .is_err()
    );
}

#[tokio::test]
async fn force_and_dropped_waiter_keep_directory_guard_until_actual_child_wait() {
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let resources = resources(
        files.path().into(),
        vec![Round::tool()],
        CleanupResult::Clean,
    );
    let host = Host::open_persistent(
        data.path(),
        HostSettings {
            name: "cleanup fixture".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    let turn = bundle.capability.source().turn.turn_id.clone();
    let waiting = {
        let host = host.clone();
        let caller = caller.clone();
        let target = target.clone();
        let turn = turn.clone();
        tokio::spawn(async move { host.wait_turn(&caller, &target, &turn).await })
    };
    waiting.abort();
    let _ = waiting.await;
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    assert_ne!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Stopped
    );
    assert!(
        Host::open_persistent(
            data.path(),
            HostSettings {
                name: "cleanup fixture".into()
            },
            resources.clone()
        )
        .await
        .is_err()
    );
    drop(connection);
    bundle.release.add_permits(1);
    // No connection or observer drives completion; a new connection only reads.
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    until(|| (host.info(&caller).unwrap().lifecycle == HostLifecycle::Stopped).then_some(())).await;
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 1);
    let reopened = Host::open_persistent(
        data.path(),
        HostSettings {
            name: "cleanup fixture".into(),
        },
        resources,
    )
    .await
    .unwrap();
    drop(reopened);
}

#[tokio::test]
async fn failed_preparation_keeps_queued_input_and_never_runs_cleanup_or_tools() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(dir.path().into(), vec![], CleanupResult::Clean);
    resources.reject.store(true, Ordering::SeqCst);
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    let receipt = send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    until(|| {
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .block_reason
    })
    .await;
    assert_eq!(bundle.finish_calls.load(Ordering::SeqCst), 0);
    assert_eq!(bundle.invoked.available_permits(), 0);
    assert!(bundle.capability.check_active().is_err());
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    assert!(matches!(
        host.message(&caller, &target, &receipt.message_id)
            .unwrap()
            .binding,
        MessageBinding::Queued
    ));
}

#[tokio::test]
async fn same_turn_loops_share_cleanup_and_cross_turn_reused_call_id_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let final_gate = Arc::new(Semaphore::new(0));
    let resources = resources(
        dir.path().into(),
        vec![
            Round::tool(),
            Round {
                tool: false,
                call_id: "shared-call",
                interrupt: false,
                gate: Some(final_gate.clone()),
            },
            Round {
                tool: true,
                call_id: "internal-call",
                interrupt: false,
                gate: None,
            },
            Round::stop(),
            Round::tool(),
            Round::stop(),
        ],
        CleanupResult::Clean,
    );
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let first = bundle(&resources, 0).await;
    invoked(&first).await;
    first.release.add_permits(1);
    until(|| (resources.model.requests.load(Ordering::SeqCst) == 2).then_some(())).await;
    send(&host, &caller, &target, MessageMode::Steer);
    final_gate.add_permits(1);
    invoked(&first).await;
    assert_eq!(first.finish_calls.load(Ordering::SeqCst), 0);
    assert_eq!(resources.bundles.lock().unwrap().len(), 1);
    first.release.add_permits(1);
    let result = terminal(
        &host,
        &caller,
        &target,
        &first.capability.source().turn.turn_id,
    )
    .await;
    assert_eq!(result.info.requests, 4);
    assert_eq!(first.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.waited.load(Ordering::SeqCst), 2);
    send(&host, &caller, &target, MessageMode::FollowUp);
    let second = bundle(&resources, 1).await;
    invoked(&second).await;
    second.release.add_permits(1);
    terminal(
        &host,
        &caller,
        &target,
        &second.capability.source().turn.turn_id,
    )
    .await;
    assert_ne!(
        first.capability.source().turn,
        second.capability.source().turn
    );
    assert_eq!(second.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.waited.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn unconfirmed_cleanup_and_panics_remain_visible_without_false_finished_or_retry() {
    for cleanup_result in [CleanupResult::Unconfirmed, CleanupResult::Panic] {
        let dir = tempfile::tempdir().unwrap();
        let resources = resources(dir.path().into(), vec![Round::tool()], cleanup_result);
        let host = Host::memory(resources.clone()).unwrap();
        let connection = host.connect().unwrap();
        let caller = connection.caller();
        let target = create(&host, &caller, StorageMode::Memory).await;
        send(&host, &caller, &target, MessageMode::FollowUp);
        let bundle = bundle(&resources, 0).await;
        invoked(&bundle).await;
        let turn = bundle.capability.source().turn.turn_id.clone();
        cancel(&host, &caller, &target, &turn);
        bundle.release.add_permits(1);
        until(|| {
            (!host
                .turn_info(&caller, &target, &turn)
                .unwrap()
                .cleanup_faults
                .is_empty())
            .then_some(())
        })
        .await;
        let info = host.turn_info(&caller, &target, &turn).unwrap();
        assert_eq!(
            (info.state, info.outcome),
            (TurnState::Finalizing, Some(TurnOutcome::Cancelled))
        );
        assert!(info.fault.is_none());
        let owned = bundle.owned_output.lock().unwrap()[0].clone();
        let observed = host
            .toolcall_get(
                &caller,
                &target,
                &ToolCallRef {
                    turn_id: turn.clone(),
                    tool_call_id: owned.source().tool_call_id.clone(),
                },
            )
            .unwrap();
        assert!(
            observed.data.result.is_some(),
            "the native result is handed off despite unconfirmed cleanup"
        );
        assert_ne!(
            observed.data.state.state,
            ToolCallState::Finished,
            "unconfirmed cleanup must not publish a finished ToolCall"
        );
        host.shutdown(
            &caller,
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Force,
            },
        )
        .unwrap();
        until(|| host.info(&caller).unwrap().shutdown_fault).await;
        assert_ne!(
            host.info(&caller).unwrap().lifecycle,
            HostLifecycle::Stopped
        );
        assert_eq!(bundle.finish_calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn failed_history_does_not_skip_child_cleanup_or_repeat_it_on_original_save_resume() {
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let resources = resources(
        files.path().into(),
        vec![Round::tool()],
        CleanupResult::Clean,
    );
    let host = Host::open_persistent(
        data.path(),
        HostSettings {
            name: "cleanup fixture".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    let turn = bundle.capability.source().turn.turn_id.clone();
    let path = data
        .path()
        .join("agents")
        .join(target.agent_id.to_string())
        .join("sessions")
        .join(target.session_id.to_string())
        .join("history.jsonl");
    let original = path.with_extension("original");
    std::fs::rename(&path, &original).unwrap();
    std::fs::write(&path, b"").unwrap();
    cancel(&host, &caller, &target, &turn);
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    let info = host.turn_info(&caller, &target, &turn).unwrap();
    assert_eq!(info.state, TurnState::Finalizing);
    assert_eq!(info.persistence, PersistenceState::Failed);
    assert_eq!(info.outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 0);
    bundle.release.add_permits(1);
    until(|| (bundle.waited.load(Ordering::SeqCst) == 1).then_some(())).await;
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&original, &path).unwrap();
    host.session_resume(
        &caller,
        &target,
        SessionControlRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    let result = terminal(&host, &caller, &target, &turn).await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Cancelled));
    assert_eq!(result.info.persistence, PersistenceState::Persisted);
    assert_eq!(bundle.finish_calls.load(Ordering::SeqCst), 1);
    assert_eq!(bundle.paths.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn primary_runtime_failure_and_confirmed_cleanup_diagnostic_remain_separate() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(
        dir.path().into(),
        vec![
            Round::tool(),
            Round {
                tool: false,
                call_id: "unused",
                interrupt: true,
                gate: None,
            },
        ],
        CleanupResult::Issue,
    );
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    bundle.release.add_permits(1);
    let result = terminal(
        &host,
        &caller,
        &target,
        &bundle.capability.source().turn.turn_id,
    )
    .await;
    assert_eq!(result.info.outcome, Some(TurnOutcome::Failed));
    assert_eq!(
        result.info.fault.unwrap().kind(),
        Some(FaultKind::TurnModelStreamInterrupted)
    );
    assert_eq!(result.info.cleanup_faults.len(), 1);
    assert_eq!(
        result.info.cleanup_faults[0].kind(),
        Some(FaultKind::TurnInternalUnexpected)
    );
    assert!(result.info.persistence_fault.is_none());
}

#[tokio::test]
async fn memory_close_cannot_reclaim_while_actual_tool_worker_is_still_running() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(dir.path().into(), vec![Round::tool()], CleanupResult::Clean);
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    host.session_close(
        &caller,
        &target,
        SessionCloseRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    let state = host.session_state(&caller, &target).unwrap();
    assert_eq!(state.data.lifecycle, SessionLifecycle::Closing);
    assert!(host.history(&caller, &target).is_ok());
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 0);
    bundle.release.add_permits(1);
    until(|| host.session_info(&caller, &target).err()).await;
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn owned_output_survives_cancelled_future_only_until_actual_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let resources = resources(dir.path().into(), vec![Round::tool()], CleanupResult::Clean);
    let host = Host::memory(resources.clone()).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Memory).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    let turn = bundle.capability.source().turn.turn_id.clone();
    let regular = bundle.ordinary_output.lock().unwrap()[0].clone();
    let owned = bundle.owned_output.lock().unwrap()[0].clone();
    assert_eq!(owned.source().turn.turn_id, turn);
    assert_eq!(owned.source().turn.session.session_id, target.session_id);
    assert!(owned.source().turn.load_id.is_some());
    assert!(regular.emit("stdout", "before "));
    cancel(&host, &caller, &target, &turn);
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    assert!(!regular.emit("stdout", "expired"));
    assert!(regular.retain_owned().is_err());
    assert!(owned.emit("stdout", "真实 child cleanup"));
    let call = ToolCallRef {
        turn_id: turn.clone(),
        tool_call_id: owned.source().tool_call_id.clone(),
    };
    let observed = host.toolcall_get(&caller, &target, &call).unwrap();
    assert!(
        observed.data.result.is_some(),
        "native result handoff is not delayed by actual cleanup"
    );
    assert_ne!(observed.data.state.state, ToolCallState::Finished);
    assert!(host.history(&caller, &target).unwrap().iter().any(|item| {
        matches!(item, minicore_host_runtime::HistoryItem::ToolResult(result) if result.call_id.as_str() == call.tool_call_id.0)
    }));
    let text = "before 真实 child cleanup";
    let range = TextReadRequest {
        channel: "stdout".into(),
        start: 0,
        end: text.len() as u64,
        max_bytes: 1000,
    };
    assert_eq!(
        host.toolcall_output_read(&caller, &target, &call, &range)
            .unwrap()
            .data
            .text,
        text
    );
    bundle.release.add_permits(1);
    terminal(&host, &caller, &target, &turn).await;
    assert!(!owned.emit("stdout", "after confirmed cleanup"));
    assert_eq!(
        host.toolcall_get(&caller, &target, &call)
            .unwrap()
            .data
            .state
            .state,
        ToolCallState::Finished
    );
    assert_eq!(
        host.toolcall_output_read(&caller, &target, &call, &range)
            .unwrap()
            .data
            .text,
        text
    );
}

// Task 051 reuses this real process fixture rather than substituting a cleanup
// acknowledgement for an actual child wait.
#[tokio::test]
async fn task051_delete_acceptance_and_disconnect_wait_for_actual_tool_cleanup() {
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let resources = resources(
        files.path().into(),
        vec![Round::tool()],
        CleanupResult::Clean,
    );
    let host = Host::open_persistent(
        data.path(),
        HostSettings {
            name: "deletion cleanup fixture".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    let receipt = host
        .session_delete(
            &caller,
            &target,
            SessionDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                close: true,
            },
        )
        .unwrap();
    assert_eq!(receipt.result.lifecycle, SessionCloseLifecycle::Closing);
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    let dir = data
        .path()
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str());
    assert!(dir.join("history.jsonl").exists());
    assert_eq!(bundle.waited.load(Ordering::SeqCst), 0);
    assert_eq!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .deletion
            .unwrap()
            .data,
        DataDeletion::Pending {}
    );
    assert!(
        host.send(
            &caller,
            SendMessage::new(
                MessageInput {
                    message_id: MessageId::new().unwrap(),
                    mode: MessageMode::FollowUp,
                    text: "late".into(),
                    in_reply_to: None
                },
                target.clone()
            )
        )
        .is_err()
    );
    assert!(
        host.session_delete(
            &caller,
            &target,
            SessionDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                close: true
            }
        )
        .is_err()
    );
    connection.close();
    let reader = host.connect().unwrap();
    let caller = reader.caller();
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionDelete(receipt.clone())
    );
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Force,
        },
    )
    .unwrap();
    assert_ne!(
        host.info(&caller).unwrap().lifecycle,
        HostLifecycle::Stopped
    );
    bundle.release.add_permits(1);
    until(|| (bundle.waited.load(Ordering::SeqCst) == 1 && !dir.exists()).then_some(())).await;
    until(|| (host.info(&caller).unwrap().lifecycle == HostLifecycle::Stopped).then_some(())).await;
    assert_eq!(
        std::fs::read(&bundle.paths.lock().unwrap()[0]).unwrap(),
        b"actual child exit"
    );
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionDelete(receipt)
    );
}

#[tokio::test]
async fn task051_failed_required_save_retains_source_and_does_not_skip_actual_worker_cleanup() {
    let data = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let resources = resources(
        files.path().into(),
        vec![Round::tool()],
        CleanupResult::Clean,
    );
    let host = Host::open_persistent(
        data.path(),
        HostSettings {
            name: "deletion save fixture".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, StorageMode::Persistent).await;
    send(&host, &caller, &target, MessageMode::FollowUp);
    let bundle = bundle(&resources, 0).await;
    invoked(&bundle).await;
    let dir = data
        .path()
        .join("agents")
        .join(target.agent_id.as_str())
        .join("sessions")
        .join(target.session_id.as_str());
    let path = dir.join("history.jsonl");
    let original = dir.join("history.original");
    std::fs::rename(&path, &original).unwrap();
    std::fs::write(&path, b"").unwrap();
    let receipt = host
        .session_delete(
            &caller,
            &target,
            SessionDeleteRequest {
                operation_id: OperationId::new().unwrap(),
                close: true,
            },
        )
        .unwrap();
    until(|| {
        host.session_state(&caller, &target)
            .ok()
            .and_then(|s| s.data.deletion)
            .filter(|d| matches!(d.data, DataDeletion::Failed { .. }))
    })
    .await;
    assert!(dir.join("session.json").exists());
    assert!(original.exists());
    assert_eq!(
        host.receipt_get(&caller, &receipt.operation_id).unwrap(),
        Receipt::SessionDelete(receipt.clone())
    );
    until(|| (bundle.finish_calls.load(Ordering::SeqCst) == 1).then_some(())).await;
    bundle.release.add_permits(1);
    until(|| (bundle.waited.load(Ordering::SeqCst) == 1).then_some(())).await;
    assert!(dir.exists());
    // Restore only this test's renamed file, then explicitly reconcile the
    // original save. Deletion itself did not retry, recreate or remove it.
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&original, &path).unwrap();
    host.session_resume(
        &caller,
        &target,
        SessionControlRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    until(|| {
        host.session_state(&caller, &target)
            .ok()
            .filter(|s| !s.data.saving && s.data.active_turn_id.is_none())
    })
    .await;
    assert_eq!(bundle.finish_calls.load(Ordering::SeqCst), 1);
    assert!(dir.join("history.jsonl").exists());
}
