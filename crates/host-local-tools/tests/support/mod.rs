#![allow(dead_code)]
//! Explicit scripted model and Turn-bound recording test fixtures. Runtime,
//! filesystem operations, subprocesses, and HTTP transport remain real.
use futures_util::stream;
use minicore_host_local_tools::*;
use minicore_host_workspace::Workspace;
use minicore_runtime::{
    AgentLoop, ExecutionConfig, HistoryItem, LoopInputs, LoopOptions, LoopReport, LoopRequest,
    ToolCallId, UserInput,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::{ToolContext, ToolError, ToolExecutionOutcome, ToolInvocation, ToolSet},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tempfile::TempDir;

pub type RecordedChunk = (String, OutputChannel, u64, Vec<u8>);

#[derive(Default)]
pub struct Recorder {
    pub edits: Mutex<Vec<(String, CapturedEdit)>>,
    pub commands: Mutex<Vec<(String, CommandResult)>>,
    pub chunks: Mutex<Vec<RecordedChunk>>,
}
impl LocalToolSink for Recorder {
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
    let tools = LocalTools::new(Workspace::open(root.path()).await.unwrap(), records.clone());
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

pub struct ScriptedModel {
    descriptor: ModelDescriptor,
    rounds: Mutex<VecDeque<Vec<ModelEvent>>>,
    pub requests: Mutex<Vec<ModelRequest>>,
}
impl ScriptedModel {
    pub fn calls(calls: Vec<(&str, &str, Value)>) -> Arc<Self> {
        let mut rounds: VecDeque<_> = calls
            .into_iter()
            .map(|(id, name, args)| {
                vec![
                    ModelEvent::ToolCallStart {
                        tool_call_id: id.parse().unwrap(),
                        tool_name: name.parse().unwrap(),
                    },
                    ModelEvent::tool_call_arguments_delta(id.parse().unwrap(), args.to_string()),
                    ModelEvent::ToolCallEnd {
                        tool_call_id: id.parse().unwrap(),
                    },
                    ModelEvent::Finish {
                        reason: ModelFinishReason::ToolCalls,
                    },
                ]
            })
            .collect();
        rounds.push_back(vec![
            ModelEvent::text_delta("Scripted model complete"),
            ModelEvent::Finish {
                reason: ModelFinishReason::Stop,
            },
        ]);
        Arc::new(Self {
            descriptor: descriptor(),
            rounds: Mutex::new(rounds),
            requests: Mutex::new(Vec::new()),
        })
    }
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
impl Model for ScriptedModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, request: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        self.requests.lock().unwrap().push(request);
        let events = self
            .rounds
            .lock()
            .unwrap()
            .pop_front()
            .expect("explicit scripted model round");
        Box::pin(
            async move { Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as ModelStream) },
        )
    }
}
struct NoInputs;
impl LoopInputs for NoInputs {
    fn take(&self) -> Vec<UserInput> {
        Vec::new()
    }
}
pub fn start(
    model: Arc<dyn Model>,
    tools: &LocalTools,
    selected: &[&str],
    options: LoopOptions,
) -> AgentLoop {
    let mut builder = ToolSet::builder();
    for name in selected {
        builder.register_arc(tools.tool(name).unwrap());
    }
    let config = ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        builder.build().unwrap(),
        None,
        Arc::new(DefaultPromptProvider::new(None)),
    )
    .unwrap();
    AgentLoop::start(
        LoopRequest::new(
            Arc::from([]),
            UserInput::text("Run the explicit local-tool fixture").unwrap(),
            config,
        ),
        options,
        Arc::new(NoInputs),
    )
    .unwrap()
}
pub async fn finish(runtime: AgentLoop, tools: &LocalTools) -> Arc<LoopReport> {
    let report = tokio::time::timeout(Duration::from_secs(15), runtime.join())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), tools.finish())
        .await
        .unwrap();
    report
}
pub fn outputs(report: &LoopReport) -> Vec<Value> {
    report
        .appended
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(result) => Some(
                serde_json::from_str(result.output.content().as_str())
                    .unwrap_or(json!({"runtime":result.output.content().as_str()})),
            ),
            _ => None,
        })
        .collect()
}
