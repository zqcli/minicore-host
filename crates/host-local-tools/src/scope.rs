use std::{
    future::Future,
    sync::{Arc, Mutex},
};

use minicore_host_runtime::{
    ToolCallId,
    tools::{ToolError, ToolExecutionOutcome},
};
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
};

type ExecutionResult = Result<ToolExecutionOutcome, ToolError>;
type Worker = (ToolCallId, JoinHandle<(Option<ToolError>, bool)>);

pub(crate) struct OwnedOutcome {
    pub result: ExecutionResult,
    pub cleanup_confirmed: bool,
}
impl OwnedOutcome {
    pub fn settled(result: ExecutionResult) -> Self {
        Self {
            result,
            cleanup_confirmed: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalToolCompletion {
    pub tool_call_id: ToolCallId,
    pub error: Option<ToolError>,
    /// Actual owned work was joined. False on a worker panic or an unconfirmed
    /// child wait, process-group end, or pipe cleanup; callers must not publish
    /// finished in that case.
    pub cleanup_confirmed: bool,
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
        work: impl Future<Output = OwnedOutcome> + Send + 'static,
    ) -> Result<oneshot::Receiver<ExecutionResult>, ToolError> {
        let mut state = self.0.lock().map_err(|_| ToolError::Internal)?;
        if state.completion.is_some() {
            return Err(ToolError::Internal);
        }
        let (sender, receiver) = oneshot::channel();
        let worker = tokio::spawn(async move {
            let outcome = work.await;
            let status = (
                outcome.result.as_ref().err().copied(),
                outcome.cleanup_confirmed,
            );
            let _ = sender.send(outcome.result);
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
                        let (error, cleanup_confirmed) = match worker.await {
                            Ok(result) => result,
                            Err(_) => (Some(ToolError::Internal), false),
                        };
                        results.push(LocalToolCompletion {
                            tool_call_id,
                            error,
                            cleanup_confirmed,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn explicit_worker_panic_is_not_confirmed_cleanup() {
        let scope = Scope::default();
        let result = scope
            .start("panic-fixture".parse().unwrap(), async {
                panic!("explicit worker panic fixture");
            })
            .unwrap();
        assert!(result.await.is_err());
        let completions = scope.finish().await;
        assert_eq!(completions[0].error, Some(ToolError::Internal));
        assert!(!completions[0].cleanup_confirmed);
    }

    #[tokio::test]
    async fn joined_internal_record_error_is_distinct_from_unconfirmed_cleanup() {
        let scope = Scope::default();
        let result = scope
            .start("record-fixture".parse().unwrap(), async {
                OwnedOutcome::settled(Err(ToolError::Internal))
            })
            .unwrap();
        assert_eq!(result.await.unwrap().unwrap_err(), ToolError::Internal);
        let completions = scope.finish().await;
        assert_eq!(completions[0].error, Some(ToolError::Internal));
        assert!(completions[0].cleanup_confirmed);
    }

    #[tokio::test]
    async fn dropped_waiter_retains_real_file_worker_and_exactly_one_result() {
        let scope = Arc::new(Scope::default());
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("written-once");
        let output = path.clone();
        let (written_tx, written_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let result = scope
            .start("file-fixture".parse().unwrap(), async move {
                tokio::task::spawn_blocking(move || std::fs::write(output, b"once"))
                    .await
                    .unwrap()
                    .unwrap();
                written_tx.send(()).unwrap();
                release_rx.await.unwrap();
                OwnedOutcome::settled(crate::completed(serde_json::json!({"status":"applied"})))
            })
            .unwrap();
        written_rx.await.unwrap();
        drop(result);
        let closing = Arc::clone(&scope);
        let waiter = tokio::spawn(async move { closing.finish().await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        waiter.abort();
        let _ = waiter.await;
        release_tx.send(()).unwrap();
        let completions = scope.finish().await;
        assert_eq!(completions.len(), 1);
        assert!(completions[0].cleanup_confirmed);
        assert_eq!(std::fs::read(path).unwrap(), b"once");
        assert!(Arc::ptr_eq(&completions, &scope.finish().await));
    }
}
