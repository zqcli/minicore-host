//! Real owners, storage and Runtime; only model responses and validation timing are fixtures.
use futures_util::stream;
use minicore_host_core::*;
use minicore_host_diagnostics::{Guard, Options, Output, ShutdownOutcome};
use minicore_host_protocol::*;
use minicore_host_runtime::{
    ExecutionConfig, ExecutionOptions,
    model::{
        Model, ModelCallContext, ModelDescriptor, ModelEvent, ModelFinishReason, ModelRequest,
        ModelStartFuture, ModelStream, ReasoningPreference,
    },
    prompt::DefaultPromptProvider,
    tools::ToolSet,
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub struct ModelFixture {
    descriptor: ModelDescriptor,
}
impl Model for ModelFixture {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
    fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
        Box::pin(async {
            Ok(Box::pin(stream::iter([
                Ok(ModelEvent::text_delta("private fixture response")),
                Ok(ModelEvent::Finish {
                    reason: ModelFinishReason::Stop,
                }),
            ])) as ModelStream)
        })
    }
}
pub struct ResourcesFixture {
    model: Arc<ModelFixture>,
}
impl ResourceResolver for ResourcesFixture {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        config
            .validate()
            .map_err(|error| Fault::new(FaultKind::SessionConfigInvalid, error.to_string()))?;
        if config.model.as_str() == "fixture/model" {
            Ok(())
        } else {
            Err(Fault::new(
                FaultKind::SessionResourceMissing,
                "private missing resource name",
            )
            .with_diagnostic_id("original-resource-diagnostic"))
        }
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        Box::pin(async move {
            self.validate(config)?;
            Ok(BoundResources {
                execution: ExecutionConfig::new(
                    self.model.clone(),
                    ReasoningPreference::Auto,
                    ToolSet::default(),
                    None,
                    Arc::new(DefaultPromptProvider::new(None)),
                )
                .unwrap(),
                options: ExecutionOptions::default(),
            })
        })
    }
}
pub fn resources() -> Arc<dyn ResourceResolver> {
    Arc::new(ResourcesFixture {
        model: Arc::new(ModelFixture {
            descriptor: ModelDescriptor::new(
                "fixture/model".parse().unwrap(),
                128_000,
                BTreeSet::from([ReasoningPreference::Auto]),
                true,
            )
            .unwrap(),
        }),
    })
}
pub fn request(mode: StorageMode) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "fixture/model".parse().unwrap();
    config.role.instructions = "private role instructions".into();
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "private Agent name".into(),
        mode,
        config,
        default_session: None,
        initial_message: None,
    }
}
pub fn input(reply: bool) -> MessageInput {
    MessageInput {
        message_id: MessageId::new().unwrap(),
        mode: MessageMode::FollowUp,
        text: "private message body".into(),
        in_reply_to: reply.then(|| MessageId::new().unwrap()),
    }
}
pub fn settings() -> HostSettings {
    HostSettings {
        name: "private Host name".into(),
    }
}
pub fn logging() -> (tempfile::NamedTempFile, tracing::Dispatch, Guard) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut options = Options::new("host-agent-owner-test");
    options.level = tracing::Level::TRACE;
    options.output = Output::File(file.path().into());
    let (dispatch, guard) = minicore_host_diagnostics::prepare(options).unwrap();
    (file, dispatch, guard)
}
pub fn records(file: &tempfile::NamedTempFile, guard: Guard) -> Vec<Value> {
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let text = std::fs::read_to_string(file.path()).unwrap();
    for private in [
        "private role instructions",
        "private Agent name",
        "private Host name",
        "private message body",
        "private fixture response",
        "private missing resource name",
    ] {
        assert!(!text.contains(private), "diagnostics leaked {private}");
    }
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub async fn until<T>(mut f: impl FnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(value) = f() {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner did not settle")
}
pub async fn shutdown(host: &Arc<Host>, caller: &CallerContext) -> OperationId {
    let operation_id = OperationId::new().unwrap();
    host.shutdown(
        caller,
        HostShutdownRequest {
            operation_id: operation_id.clone(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    until(|| (host.info(caller).unwrap().lifecycle == HostLifecycle::Stopped).then_some(())).await;
    operation_id
}
pub struct ValidationGateFixture {
    pub entered: tokio::sync::Notify,
    count: AtomicUsize,
    released: (Mutex<bool>, Condvar),
    inner: Arc<dyn ResourceResolver>,
}
impl ValidationGateFixture {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: tokio::sync::Notify::new(),
            count: AtomicUsize::new(0),
            released: (Mutex::new(false), Condvar::new()),
            inner: resources(),
        })
    }
    pub fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}
impl ResourceResolver for ValidationGateFixture {
    fn validate(&self, config: &SessionConfig) -> Result<(), Fault> {
        if self.count.fetch_add(1, Ordering::SeqCst) == 1 {
            self.entered.notify_one();
            let mut released = self.released.0.lock().unwrap();
            while !*released {
                released = self.released.1.wait(released).unwrap();
            }
        }
        self.inner.validate(config)
    }
    fn resolve<'a>(&'a self, config: &'a SessionConfig) -> ResourceFuture<'a> {
        self.inner.resolve(config)
    }
}
