use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant, SystemTime};

use futures_util::{FutureExt, StreamExt};
#[cfg(test)]
use tokio::sync::mpsc;
use tokio::time::Instant as TokioInstant;
use tokio_util::sync::CancellationToken;

use crate::ids::LoopId;
use crate::time::{DeadlineSource, effective_deadline};
use crate::value::Text;

use super::port::{Model, ModelCallContext, ModelDescriptor};
#[cfg(test)]
use super::response::ModelEvent;
use super::{DeliveryState, ModelError, ModelErrorKind};
use super::{ModelRequest, ModelResponse, ModelRetryProgress};

mod assembler;
mod failure;

use assembler::Assembler;
pub use failure::ModelDriverFailure;
use failure::{
    AttemptFailure, RetryPolicySnapshot, generated_error, normalize_after_event, observed_delivery,
    wait_for_retry,
};

pub(crate) const MAX_MODEL_CALL_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
pub(crate) const MAX_MODEL_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct ModelDriverConfig {
    model_call_timeout: Duration,
    retry_policy: RetryPolicySnapshot,
}

impl ModelDriverConfig {
    pub fn new(model_call_timeout: Duration, max_attempts: u8, base_delay: Duration) -> Self {
        Self {
            model_call_timeout,
            retry_policy: RetryPolicySnapshot::new(max_attempts, base_delay),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ModelDriverProgress {
    TextDelta(Text),
    ReasoningDelta(Text),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelCallEvent {
    TextDelta(Text),
    ReasoningDelta(Text),
    AttemptStarted(u8),
    RetryWaiting(ModelRetryProgress),
}

#[derive(Debug)]
pub struct ModelCallReport {
    pub outcome: Result<ModelResponse, ModelDriverFailure>,
    pub attempts: u8,
    pub retry_wait_elapsed: Duration,
}

/// Owner-supplied identity and cancellation for one logical model request.
#[derive(Clone, Debug)]
pub struct ModelCallParams {
    pub loop_id: LoopId,
    pub request_index: u32,
    pub cancellation: CancellationToken,
    /// The actual enclosing execution deadline, when one was configured.
    pub deadline: Option<Instant>,
}

type Observer<'a> = &'a mut (dyn FnMut(ModelCallEvent) + Send);

pub struct ModelDriver<'a> {
    model: &'a dyn Model,
    descriptor: ModelDescriptor,
    model_call_timeout: Duration,
    retry_policy: RetryPolicySnapshot,
}

impl<'a> ModelDriver<'a> {
    pub fn new(model: &'a dyn Model, config: ModelDriverConfig) -> Result<Self, ModelError> {
        let invalid = || generated_error(ModelErrorKind::InvalidRequest, DeliveryState::NotStarted);
        let descriptor = catch_unwind(AssertUnwindSafe(|| model.descriptor().clone()))
            .map_err(|_| generated_error(ModelErrorKind::Panicked, DeliveryState::NotStarted))?;
        descriptor.validate().map_err(|_| invalid())?;
        if config.model_call_timeout.is_zero()
            || config.model_call_timeout > MAX_MODEL_CALL_TIMEOUT
            || config.retry_policy.max_attempts == 0
            || config.retry_policy.max_attempts > 4
            || config.retry_policy.base_delay.is_zero()
            || config.retry_policy.base_delay > MAX_MODEL_RETRY_DELAY
        {
            return Err(invalid());
        }
        Ok(Self::from_validated(model, descriptor, config))
    }

    fn from_validated(
        model: &'a dyn Model,
        descriptor: ModelDescriptor,
        config: ModelDriverConfig,
    ) -> Self {
        Self {
            model,
            descriptor,
            model_call_timeout: config.model_call_timeout,
            retry_policy: config.retry_policy,
        }
    }

    pub fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    pub async fn call(
        &self,
        request: ModelRequest,
        params: ModelCallParams,
        observer: Observer<'_>,
    ) -> ModelCallReport {
        let mut attempts = 0;
        let mut retry_wait_elapsed = Duration::ZERO;
        let outcome = self
            .run_requests(
                request,
                params,
                &mut |event| {
                    if let ModelCallEvent::AttemptStarted(attempt) = &event {
                        attempts = *attempt;
                    }
                    // Observation cannot change a model request's execution result.
                    let _ = catch_unwind(AssertUnwindSafe(|| observer(event)));
                },
                &mut retry_wait_elapsed,
            )
            .await;
        ModelCallReport {
            outcome,
            attempts,
            retry_wait_elapsed,
        }
    }

    #[cfg(test)]
    async fn run_detailed(
        &self,
        request: ModelRequest,
        params: ModelCallParams,
        progress: &mpsc::UnboundedSender<ModelDriverProgress>,
    ) -> Result<ModelResponse, ModelDriverFailure> {
        self.call(request, params, &mut |event| {
            let event = match event {
                ModelCallEvent::TextDelta(text) => ModelDriverProgress::TextDelta(text),
                ModelCallEvent::ReasoningDelta(text) => ModelDriverProgress::ReasoningDelta(text),
                _ => return,
            };
            let _ = progress.send(event);
        })
        .await
        .outcome
    }

    async fn run_requests(
        &self,
        request: ModelRequest,
        params: ModelCallParams,
        observer: Observer<'_>,
        retry_wait_elapsed: &mut Duration,
    ) -> Result<ModelResponse, ModelDriverFailure> {
        let deadline =
            effective_deadline(params.deadline, self.model_call_timeout).map_err(|_| {
                ModelDriverFailure::ordinary(generated_error(
                    ModelErrorKind::InvalidRequest,
                    DeliveryState::NotStarted,
                ))
            })?;
        precheck(&params.cancellation, deadline.tokio(), deadline.source())?;
        self.validate_request(&request)
            .map_err(ModelDriverFailure::ordinary)?;

        let mut attempt = 0;
        loop {
            precheck(&params.cancellation, deadline.tokio(), deadline.source())?;
            let attempt_context = ModelCallContext::new(
                params.loop_id,
                params.request_index,
                params.cancellation.child_token(),
                deadline.standard(),
            );
            let _attempt_cancel = attempt_context.cancellation.clone().drop_guard();
            let start = catch_unwind(AssertUnwindSafe(|| {
                self.model.start(request.clone(), attempt_context.clone())
            }));
            observer(ModelCallEvent::AttemptStarted(attempt + 1));
            let start = start.map_err(|_| {
                ModelDriverFailure::ordinary(generated_error(
                    ModelErrorKind::Panicked,
                    DeliveryState::Unknown,
                ))
            })?;
            match self
                .run_attempt(
                    &request,
                    attempt_context,
                    deadline.source(),
                    start,
                    observer,
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(failure) => {
                    match self
                        .retry_policy
                        .evaluate_retry(attempt, &failure, deadline.tokio())
                    {
                        Some(delay) => {
                            let retry_deadline = TokioInstant::now() + delay;
                            observer(ModelCallEvent::RetryWaiting(ModelRetryProgress {
                                attempt: attempt + 2,
                                max_attempts: self.retry_policy.max_attempts,
                                error: failure.error.clone(),
                                retry_at: SystemTime::now() + delay,
                                retry_deadline,
                            }));
                            let wait_started = TokioInstant::now();
                            let waited = wait_for_retry(
                                &params.cancellation,
                                deadline.tokio(),
                                deadline.source(),
                                retry_deadline,
                                failure.error,
                            )
                            .await;
                            *retry_wait_elapsed += wait_started.elapsed();
                            waited?;
                            attempt += 1;
                        }
                        None => return Err(failure.into_driver_failure()),
                    }
                }
            }
        }
    }

    fn validate_request(&self, request: &ModelRequest) -> Result<(), ModelError> {
        let invalid = || generated_error(ModelErrorKind::InvalidRequest, DeliveryState::NotStarted);
        if !self.descriptor.supports_reasoning(request.reasoning()) {
            return Err(invalid());
        }
        if !request.tools().is_empty() && !self.descriptor.supports_tools {
            return Err(invalid());
        }
        if request
            .limits()
            .context_window_tokens()
            .is_some_and(|limit| u64::from(limit) > self.descriptor.context_window)
        {
            return Err(invalid());
        }
        Ok(())
    }

    async fn run_attempt(
        &self,
        request: &ModelRequest,
        context: ModelCallContext,
        deadline_source: DeadlineSource,
        start: super::ModelStartFuture<'_>,
        observer: Observer<'_>,
    ) -> Result<ModelResponse, AttemptFailure> {
        let cancellation = context.cancellation.clone();
        let request_index = context.request_index;
        let deadline = TokioInstant::from_std(context.deadline);
        let start = AssertUnwindSafe(start).catch_unwind();
        tokio::pin!(start);
        let _start_cancel = cancellation.clone().drop_guard();
        let timeout = tokio::time::sleep_until(deadline);
        tokio::pin!(timeout);
        let stream = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(AttemptFailure::new(
                    generated_error(ModelErrorKind::Cancelled, DeliveryState::Unknown),
                    false,
                ));
            }
            _ = &mut timeout => {
                return Err(AttemptFailure::deadline(
                    generated_error(ModelErrorKind::Timeout, DeliveryState::Unknown),
                    false,
                    deadline_source,
                ));
            }
            result = &mut start => match result {
                Ok(Ok(stream)) => stream,
                Ok(Err(error)) => return Err(AttemptFailure::new(error, false)),
                Err(_) => return Err(AttemptFailure::new(
                    generated_error(ModelErrorKind::Panicked, DeliveryState::Unknown),
                    false,
                )),
            }
        };

        let mut stream = stream;
        let _stream_cancel = cancellation.clone().drop_guard();
        let mut assembler = Assembler::new(request.tools());
        let result = async {
            loop {
                let next = AssertUnwindSafe(stream.next()).catch_unwind();
                tokio::pin!(next);
                let item = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => {
                        return Err(AttemptFailure::new(
                            generated_error(
                                ModelErrorKind::Cancelled,
                                observed_delivery(assembler.observed_event),
                            ),
                            assembler.observed_event,
                        ));
                    }
                    _ = &mut timeout => {
                        return Err(AttemptFailure::deadline(
                            generated_error(
                                ModelErrorKind::Timeout,
                                observed_delivery(assembler.observed_event),
                            ),
                            assembler.observed_event,
                            deadline_source,
                        ));
                    }
                    result = &mut next => result,
                };
                match item {
                    Ok(Some(Ok(event))) => {
                        assembler.observed_event = true;
                        let progress_event = assembler.push(event).map_err(|error| {
                            AttemptFailure::new(error, assembler.observed_event)
                        })?;
                        if let Some(progress_event) = progress_event {
                            observer(match progress_event {
                                ModelDriverProgress::TextDelta(text) => {
                                    ModelCallEvent::TextDelta(text)
                                }
                                ModelDriverProgress::ReasoningDelta(text) => {
                                    ModelCallEvent::ReasoningDelta(text)
                                }
                            });
                            // Let cancellation and controls run between immediately ready deltas.
                            tokio::task::yield_now().await;
                        }
                    }
                    Ok(Some(Err(error))) => {
                        let error = normalize_after_event(error, assembler.observed_event);
                        return Err(AttemptFailure::new(error, assembler.observed_event));
                    }
                    Ok(None) => {
                        let observed_event = assembler.observed_event;
                        return assembler
                            .finish()
                            .map_err(|error| AttemptFailure::new(error, observed_event));
                    }
                    Err(_) => {
                        return Err(AttemptFailure::new(
                            generated_error(
                                ModelErrorKind::Panicked,
                                observed_delivery(assembler.observed_event),
                            ),
                            assembler.observed_event,
                        ));
                    }
                }
            }
        }
        .await;
        result.map_err(|failure: AttemptFailure| {
            let usage = assembler.usage();
            failure.with_observation(usage, assembler.into_partial(request_index))
        })
    }
}

#[cfg(test)]
mod tests;

fn precheck(
    cancellation: &CancellationToken,
    deadline: TokioInstant,
    source: DeadlineSource,
) -> Result<(), ModelDriverFailure> {
    if cancellation.is_cancelled() {
        return Err(ModelDriverFailure::ordinary(generated_error(
            ModelErrorKind::Cancelled,
            DeliveryState::NotStarted,
        )));
    }
    if TokioInstant::now() >= deadline {
        return Err(ModelDriverFailure::deadline(
            generated_error(ModelErrorKind::Timeout, DeliveryState::NotStarted),
            source,
        ));
    }
    Ok(())
}
