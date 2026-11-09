use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use minicore_runtime::{
    ToolCallId,
    tools::{ToolError, ToolExecutionOutcome},
};
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
};

type ExecutionResult = Result<ToolExecutionOutcome, ToolError>;
type Worker = (ToolCallId, JoinHandle<Result<(), ToolError>>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalToolCompletion {
    pub tool_call_id: ToolCallId,
    pub error: Option<ToolError>,
}

#[derive(Default)]
struct State {
    workers: Vec<Worker>,
    completion: Option<watch::Receiver<Option<Arc<[LocalToolCompletion]>>>>,
    collector: Option<JoinHandle<()>>,
}

#[derive(Default)]
pub(crate) struct Scope(Mutex<State>);
impl Scope {
    pub fn start(
        &self,
        call: ToolCallId,
        work: impl Future<Output = ExecutionResult> + Send + 'static,
    ) -> Result<oneshot::Receiver<ExecutionResult>, ToolError> {
        let mut state = self.0.lock().map_err(|_| ToolError::Internal)?;
        if state.completion.is_some() {
            return Err(ToolError::Internal);
        }
        let (sender, receiver) = oneshot::channel();
        let worker = tokio::spawn(async move {
            let result = work.await;
            let status = result.as_ref().map(|_| ()).map_err(|error| *error);
            let _ = sender.send(result);
            status
        });
        state.workers.push((call, worker));
        Ok(receiver)
    }

    pub async fn finish(&self) -> Arc<[LocalToolCompletion]> {
        let mut receiver = {
            let mut state = self
                .0
                .lock()
                .expect("local tool scope lock is not poisoned");
            if let Some(receiver) = &state.completion {
                receiver.clone()
            } else {
                let workers = std::mem::take(&mut state.workers);
                let (sender, receiver) = watch::channel(None);
                state.completion = Some(receiver.clone());
                state.collector = Some(tokio::spawn(async move {
                    let mut results = Vec::with_capacity(workers.len());
                    for (tool_call_id, worker) in workers {
                        let error = match worker.await {
                            Ok(result) => result.err(),
                            Err(_) => Some(ToolError::Internal),
                        };
                        results.push(LocalToolCompletion {
                            tool_call_id,
                            error,
                        });
                    }
                    sender.send_replace(Some(results.into()));
                }));
                receiver
            }
        };
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .expect("the collector retains its result");
        }
    }
}
