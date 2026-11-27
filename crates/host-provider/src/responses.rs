//! Stateless Responses adapter with shared event semantics for HTTP/SSE and WebSocket.

use std::{fmt, time::Instant};

use futures_util::{StreamExt, stream as futures_stream};
use minicore_host_runtime::model::{
    Model, ModelCallContext, ModelDescriptor, ModelError, ModelErrorKind, ModelRequest,
    ModelStartFuture, ModelStream,
};
use tokio::time::Instant as TokioInstant;
use tracing::Instrument;

use crate::{
    OpenAiChatModel, OpenAiResponsesConfig, ProviderConfigError, SecretApiKey, diagnostics, error,
};

mod replay;
mod request;
mod stream;
mod websocket;
mod wire;

/// Immutable, shareable OpenAI Responses client using complete stateless input.
pub struct OpenAiResponsesModel {
    connection: OpenAiChatModel,
    request_timeout: Option<std::time::Duration>,
    web_search: Option<bool>,
}

impl fmt::Debug for OpenAiResponsesModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiResponsesModel")
            .field("descriptor", &self.connection.descriptor)
            .finish_non_exhaustive()
    }
}

impl OpenAiResponsesModel {
    pub fn new(
        config: OpenAiResponsesConfig,
        key: SecretApiKey,
    ) -> Result<Self, ProviderConfigError> {
        Ok(Self {
            request_timeout: config.request_timeout,
            web_search: config.web_search,
            connection: OpenAiChatModel::build(config.into_connection(), key, true)?,
        })
    }

    pub fn from_env(
        config: OpenAiResponsesConfig,
        name: &str,
    ) -> Result<Self, ProviderConfigError> {
        Self::new(config, SecretApiKey::from_env(name)?)
    }

    async fn start_request(
        &self,
        request: ModelRequest,
        context: ModelCallContext,
        diagnostic: &mut diagnostics::Request,
    ) -> Result<ModelStream, ModelError> {
        if context.cancellation.is_cancelled() {
            return Err(error::local(ModelErrorKind::Cancelled));
        }
        if Instant::now() >= context.deadline {
            return Err(error::local(ModelErrorKind::Timeout));
        }
        let websocket = matches!(self.connection.endpoint.scheme(), "ws" | "wss");
        let body = request::encode(
            &self.connection.provider_model,
            &self.connection.descriptor,
            &request,
            websocket,
            self.web_search,
        )?;
        let deadline = TokioInstant::from_std(context.deadline);
        let (source, error_context, deadline) = if websocket {
            let deadline = self
                .request_timeout
                .and_then(|timeout| TokioInstant::now().checked_add(timeout))
                .map_or(deadline, |transport_deadline| {
                    deadline.min(transport_deadline)
                });
            let (socket, error_context) =
                websocket::start(&self.connection, body, &context, deadline, diagnostic).await?;
            (stream::Source::websocket(socket), error_context, deadline)
        } else {
            let response = self.connection.send(body, &context, diagnostic).await?;
            let error_context =
                error::ResponseContext::new(&response, &self.connection.authorization);
            (stream::Source::sse(response), error_context, deadline)
        };
        let state = stream::State::new(
            source,
            error_context,
            context.cancellation,
            deadline,
            self.connection.provider_model.clone(),
        );
        let (first, state) = stream::next(state)
            .await
            .ok_or_else(|| error::unknown(ModelErrorKind::IncompleteResponse))?;
        let first = first?;
        Ok(Box::pin(
            futures_stream::once(async move { Ok(first) })
                .chain(futures_stream::unfold(state, stream::next)),
        ))
    }
}

impl Model for OpenAiResponsesModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.connection.descriptor
    }

    fn start<'a>(
        &'a self,
        request: ModelRequest,
        context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        Box::pin(async move {
            let mut diagnostic = diagnostics::Request::start(&context);
            let span = diagnostic.span();
            let result = self
                .start_request(request, context, &mut diagnostic)
                .instrument(span)
                .await;
            match result {
                Ok(stream) => Ok(diagnostic.observe(stream)),
                Err(error) => {
                    diagnostic.failed(&error);
                    Err(error)
                }
            }
        })
    }
}
