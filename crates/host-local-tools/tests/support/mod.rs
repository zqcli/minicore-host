#![allow(dead_code)]
//! Explicit scripted model and Turn-bound recording test fixtures. Runtime,
//! filesystem operations, subprocesses, and HTTP transport remain real.
use minicore_host_local_tools::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions, InteractionId, LoopId, ToolCallId,
    interaction::{InteractionAnswer, PendingInteraction},
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelRequest, ModelStartFuture,
        ReasoningPreference, ToolCall,
    },
    prompt::DefaultPromptProvider,
    tools::{
        ToolBatchContext, ToolBatchEvent, ToolBatchHost, ToolBatchReport, ToolBatchStop,
        ToolContext, ToolError, ToolExecutionOutcome, ToolInvocation, ToolPolicy, ToolSet,
        run_tool_batch,
    },
};
use minicore_host_workspace::Workspace;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::sync::oneshot;

pub type RecordedChunk = (String, OutputChannel, u64, Vec<u8>);

#[derive(Default)]
pub struct Recorder {
    pub edits: Mutex<Vec<(String, CapturedEdit)>>,
    pub completions: Mutex<Vec<(String, bool)>>,
    pub commands: Mutex<Vec<(String, CommandResult)>>,
    pub chunks: Mutex<Vec<RecordedChunk>>,
}
impl LocalToolSink for Recorder {
    fn complete(&self, call: &ToolCallId, cleanup_confirmed: bool) -> Result<(), RecordError> {
        self.completions
            .lock()
            .unwrap()
            .push((call.as_str().to_owned(), cleanup_confirmed));
        Ok(())
    }
    fn output(
        &self,
        call: &ToolCallId,
        channel: OutputChannel,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), RecordError> {
        self.chunks.lock().unwrap().push((
            call.as_str().to_owned(),
            channel,
            offset,
            bytes.to_vec(),
        ));
        Ok(())
    }
    fn edit(&self, call: &ToolCallId, edit: CapturedEdit) -> Result<(), RecordError> {
        self.edits
            .lock()
            .unwrap()
            .push((call.as_str().to_owned(), edit));
        Ok(())
    }
    fn command(&self, call: &ToolCallId, result: &CommandResult) -> Result<(), RecordError> {
        self.commands
            .lock()
            .unwrap()
            .push((call.as_str().to_owned(), result.clone()));
        Ok(())
    }
}
pub async fn fixture() -> (TempDir, LocalTools, Arc<Recorder>) {
    let root = TempDir::new().unwrap();
    let records = Arc::new(Recorder::default());
    let tools = LocalTools::new(
        Workspace::open(root.path()).await.unwrap(),
        records.clone(),
        Arc::new(InheritCommandEnvironment),
    );
    (root, tools, records)
}
pub fn context() -> ToolContext {
    ToolContext {
        cancellation: Default::default(),
        deadline: Instant::now() + Duration::from_secs(10),
        progress: Default::default(),
    }
}
pub async fn invoke(tools: &LocalTools, name: &str, arguments: Value) -> Result<Value, ToolError> {
    let output = tools
        .tool(name)
        .unwrap()
        .execute(
            ToolInvocation::new(
                "fixture-call".parse().unwrap(),
                name.parse().unwrap(),
                arguments,
            )
            .unwrap(),
            context(),
        )
        .await?;
    let ToolExecutionOutcome::Completed(output) = output else {
        panic!("unexpected interaction")
    };
    Ok(serde_json::from_str(output.content().as_str()).unwrap())
}

/// Only tool batches use this descriptor; no model call is part of these tests.
struct UnusedModel {
    descriptor: ModelDescriptor,
}
pub fn descriptor() -> ModelDescriptor {
    ModelDescriptor::new(
        "fixture/local-tools".parse().unwrap(),
        128_000,
        BTreeSet::from([ReasoningPreference::Auto]),
        true,
    )
    .unwrap()
}
impl Model for UnusedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        panic!("a tool batch must not call the model")
    }
}
pub struct NoInteractions;
impl ToolBatchHost for NoInteractions {
    fn observe(&self, _: ToolBatchEvent) {}
    fn request_interaction(
        &self,
        _: PendingInteraction,
        _: oneshot::Sender<InteractionAnswer>,
    ) -> Result<(), ToolBatchStop> {
        panic!("these selected tools do not request interaction")
    }
    fn finish_interaction(&self, _: InteractionId) {
        panic!("no interaction was registered")
    }
}
pub fn execution(
    model: Arc<dyn Model>,
    tools: &LocalTools,
    selected: &[&str],
    policy: Option<Arc<dyn ToolPolicy>>,
) -> ExecutionConfig {
    let mut builder = ToolSet::builder();
    for name in selected {
        builder.register_arc(tools.tool(name).unwrap());
    }
    ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        builder.build().unwrap(),
        policy,
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap()
}
pub fn batch_context(options: &ExecutionOptions) -> ToolBatchContext {
    ToolBatchContext {
        loop_id: LoopId::new().unwrap(),
        request_index: 0,
        cancellation: Default::default(),
        deadline: options.deadline.map(Into::into),
        policy_timeout: options.policy_timeout,
        tool_timeout: options.tool_timeout,
    }
}
pub fn tool_config(
    tools: &LocalTools,
    selected: &[&str],
    policy: Option<Arc<dyn ToolPolicy>>,
) -> ExecutionConfig {
    execution(
        Arc::new(UnusedModel {
            descriptor: descriptor(),
        }),
        tools,
        selected,
        policy,
    )
}
pub fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall::new(id.parse().unwrap(), name.parse().unwrap(), arguments, 0).unwrap()
}
pub async fn run_calls(
    tools: &LocalTools,
    calls: Vec<(&str, &str, Value)>,
    options: ExecutionOptions,
) -> ToolBatchReport {
    let selected: BTreeSet<_> = calls.iter().map(|(_, name, _)| *name).collect();
    let config = tool_config(tools, &selected.into_iter().collect::<Vec<_>>(), None);
    let calls: Vec<_> = calls
        .into_iter()
        .enumerate()
        .map(|(index, (id, name, args))| {
            ToolCall::new(
                id.parse().unwrap(),
                name.parse().unwrap(),
                args,
                index.try_into().unwrap(),
            )
            .unwrap()
        })
        .collect();
    tokio::time::timeout(
        Duration::from_secs(15),
        run_tool_batch(
            &config,
            &calls.iter().collect::<Vec<_>>(),
            batch_context(&options),
            Arc::new(NoInteractions),
        ),
    )
    .await
    .unwrap()
}
pub fn outputs(report: &ToolBatchReport) -> Vec<Value> {
    report
        .results
        .iter()
        .map(|result| {
            serde_json::from_str(result.output.content().as_str())
                .unwrap_or(json!({"runtime":result.output.content().as_str()}))
        })
        .collect()
}
