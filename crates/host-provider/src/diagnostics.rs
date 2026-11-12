//! Transport facts only. The provider has no Host registry or invented local
//! chain. The owning Session separately reports authoritative Runtime outcomes.
use futures_util::Stream;
use minicore_host_diagnostics::{Context, EventData, EventSpec, Scope};
use minicore_host_runtime::model::{
    DeliveryState, ModelCallContext, ModelError, ModelErrorKind, ModelEvent, ModelStream,
    RetryHint, Usage,
};
use std::{pin::Pin, task::Poll, time::Instant};
use tracing::Level;

const STARTED: EventSpec = EventSpec {
    level: Level::DEBUG,
    scope: Scope::Runtime,
    component: "provider.http",
    event: "model.transport_started",
    message: "Model transport attempt started",
    attributes: &["loop_id", "request_index"],
};
const FINISHED: EventSpec = EventSpec {
    level: Level::DEBUG,
    scope: Scope::Runtime,
    component: "provider.http",
    event: "model.transport_finished",
    message: "Model transport attempt ended; Runtime owns its business outcome",
    attributes: &[
        "loop_id",
        "request_index",
        "duration_ms",
        "outcome",
        "upstream_status",
        "error_kind",
        "delivery",
        "retry_after_ms",
        "input_tokens",
        "output_tokens",
        "reasoning_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "provider_total_tokens",
    ],
};

pub(super) struct Request {
    loop_id: minicore_host_runtime::LoopId,
    request_index: u32,
    started: Instant,
    status: Option<u16>,
    usage: Option<Usage>,
    finished: bool,
    span: tracing::Span,
}
impl Request {
    pub(super) fn start(context: &ModelCallContext) -> Self {
        let request = Self {
            loop_id: context.loop_id,
            request_index: context.request_index,
            started: Instant::now(),
            status: None,
            usage: None,
            finished: false,
            span: Context::default().span(),
        };
        STARTED.emit(|| request.data());
        request
    }
    pub(super) fn span(&self) -> tracing::Span {
        self.span.clone()
    }
    fn data(&self) -> EventData {
        // A provider invocation may run standalone or in a spawned Runtime
        // task. Do not accidentally inherit an unrelated caller's Host span.
        EventData::default()
            .context(Context::default())
            .attribute("loop_id", self.loop_id.to_string())
            .attribute("request_index", u64::from(self.request_index))
    }
    pub(super) fn response(&mut self, status: u16) {
        self.status = Some(status);
    }
    fn finish(&mut self, outcome: &'static str, error: Option<&ModelError>) {
        if self.finished {
            return;
        }
        self.finished = true;
        FINISHED.emit(|| {
            let mut data = self.data().attribute("outcome", outcome);
            if let Ok(duration) = u64::try_from(self.started.elapsed().as_millis()) {
                data = data.attribute("duration_ms", duration);
            }
            if let Some(status) = self.status {
                data = data.attribute("upstream_status", u64::from(status));
            }
            if let Some(error) = error {
                data = data.attribute("error_kind", kind(error.kind())).attribute(
                    "delivery",
                    match error.delivery() {
                        DeliveryState::NotStarted => "not_started",
                        DeliveryState::Started => "started",
                        DeliveryState::Unknown => "unknown",
                    },
                );
                if let RetryHint::Retryable {
                    retry_after: Some(delay),
                } = error.retry_hint()
                    && let Ok(delay) = u64::try_from(delay.as_millis())
                {
                    data = data.attribute("retry_after_ms", delay);
                }
            }
            if let Some(usage) = self.usage {
                for (name, value) in [
                    ("input_tokens", usage.input_tokens()),
                    ("output_tokens", usage.output_tokens()),
                    ("reasoning_tokens", usage.reasoning_tokens()),
                    ("cache_read_tokens", usage.cache_read_tokens()),
                    ("cache_write_tokens", usage.cache_write_tokens()),
                    ("provider_total_tokens", usage.provider_total_tokens()),
                ] {
                    if let Some(value) = value {
                        data = data.attribute(name, value);
                    }
                }
            }
            data
        });
    }
    pub(super) fn failed(&mut self, error: &ModelError) {
        self.finish("error", Some(error));
    }
    pub(super) fn observe(self, stream: ModelStream) -> ModelStream {
        Box::pin(ObservedStream {
            stream,
            diagnostic: self,
        })
    }
}
impl Drop for Request {
    fn drop(&mut self) {
        // Dropped is not a fabricated completion, cancellation or upstream error.
        self.finish("dropped", None);
    }
}
struct ObservedStream {
    stream: ModelStream,
    diagnostic: Request,
}
impl Stream for ObservedStream {
    type Item = Result<ModelEvent, ModelError>;
    fn poll_next(
        self: Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        // This guard lasts for one synchronous poll, never across an await.
        let span = this.diagnostic.span.clone();
        let _span = span.enter();
        let result = this.stream.as_mut().poll_next(context);
        match &result {
            Poll::Ready(Some(Ok(ModelEvent::Usage { usage }))) => {
                this.diagnostic.usage = Some(*usage)
            }
            Poll::Ready(Some(Ok(ModelEvent::Finish { .. }))) => {
                this.diagnostic.finish("stream_finished", None)
            }
            Poll::Ready(Some(Err(error))) => this.diagnostic.failed(error),
            Poll::Ready(None) => this.diagnostic.finish("stream_ended", None),
            _ => {}
        }
        result
    }
}
fn kind(kind: ModelErrorKind) -> &'static str {
    match kind {
        ModelErrorKind::InvalidRequest => "invalid_request",
        ModelErrorKind::Unavailable => "unavailable",
        ModelErrorKind::ProviderUnavailable => "provider_unavailable",
        ModelErrorKind::AuthMissing => "auth_missing",
        ModelErrorKind::AuthRejected => "auth_rejected",
        ModelErrorKind::RateLimited => "rate_limited",
        ModelErrorKind::QuotaExceeded => "quota_exceeded",
        ModelErrorKind::ContextOverflow => "context_overflow",
        ModelErrorKind::Timeout => "timeout",
        ModelErrorKind::TransportUnavailable => "transport_unavailable",
        ModelErrorKind::Cancelled => "cancelled",
        ModelErrorKind::InvalidProviderResponse => "invalid_provider_response",
        ModelErrorKind::IncompleteResponse => "incomplete_response",
        ModelErrorKind::StreamInterrupted => "stream_interrupted",
        ModelErrorKind::RequestOutcomeUnknown => "request_outcome_unknown",
        ModelErrorKind::UnexpectedToolCall => "unexpected_tool_call",
        ModelErrorKind::Panicked => "panicked",
        ModelErrorKind::Internal => "internal",
    }
}
