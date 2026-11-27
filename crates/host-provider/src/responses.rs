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
    websocket_endpoint: Option<reqwest::Url>,
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
        let request_timeout = config.request_timeout;
        let web_search = config.web_search;
        let prefer_websocket = config.prefer_websocket;
        let mut connection = OpenAiChatModel::build(config.into_connection(), key, true)?;
        let try_websocket = prefer_websocket.unwrap_or(connection.endpoint.scheme() != "http");
        let websocket_endpoint = match connection.endpoint.scheme() {
            "https" | "http" => try_websocket.then(|| {
                let scheme = if connection.endpoint.scheme() == "https" {
                    "wss"
                } else {
                    "ws"
                };
                with_scheme(&connection.endpoint, scheme)
            }),
            "ws" | "wss" => {
                let endpoint = connection.endpoint.clone();
                let sse_scheme = if endpoint.scheme() == "wss" {
                    "https"
                } else {
                    "http"
                };
                connection.endpoint = with_scheme(&endpoint, sse_scheme);
                try_websocket.then_some(endpoint)
            }
            _ => unreachable!("validated Responses endpoint"),
        };
        Ok(Self {
            connection,
            websocket_endpoint,
            request_timeout,
            web_search,
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
        let deadline = TokioInstant::from_std(context.deadline);
        let deadline = self
            .request_timeout
            .and_then(|timeout| TokioInstant::now().checked_add(timeout))
            .map_or(deadline, |transport_deadline| {
                deadline.min(transport_deadline)
            });
        use minicore_host_runtime::model::WebSearchMode;
        let search = match request.web_search() {
            None => self.web_search,
            Some(WebSearchMode::Off) => None,
            Some(WebSearchMode::Live) => Some(true),
            Some(WebSearchMode::Cached) => Some(false),
        };
        let encode = |websocket| {
            request::encode(
                &self.connection.provider_model,
                &self.connection.descriptor,
                &request,
                websocket,
                search,
            )
        };
        if let Some(endpoint) = &self.websocket_endpoint
            && let Some((socket, error_context)) = websocket::start(
                &self.connection,
                endpoint,
                encode(true)?,
                &context,
                deadline,
                diagnostic,
            )
            .await?
        {
            let state = stream::State::new(
                stream::Source::websocket(socket),
                error_context,
                context.cancellation.clone(),
                deadline,
                self.connection.provider_model.clone(),
            );
            match first_stream(state).await {
                Err(error) if ws_route_rejected(&error) => {}
                result => return result,
            }
        }
        let context = ModelCallContext {
            deadline: deadline.into_std(),
            ..context
        };
        let response = self
            .connection
            .send(encode(false)?, &context, diagnostic)
            .await?;
        let error_context = error::ResponseContext::new(&response, &self.connection.authorization);
        let source = stream::Source::sse(response);
        let state = stream::State::new(
            source,
            error_context,
            context.cancellation,
            deadline,
            self.connection.provider_model.clone(),
        );
        first_stream(state).await
    }
}

fn with_scheme(endpoint: &reqwest::Url, scheme: &str) -> reqwest::Url {
    let mut endpoint = endpoint.clone();
    endpoint
        .set_scheme(scheme)
        .expect("supported transport schemes");
    endpoint
}

fn ws_route_rejected(error: &ModelError) -> bool {
    use minicore_host_runtime::model::DeliveryState;
    error.delivery() == DeliveryState::NotStarted
        && error.upstream().is_some_and(|upstream| {
            matches!(
                upstream.provider_code.as_deref(),
                Some("model_not_found" | "unsupported_transport")
            )
        })
}

async fn first_stream(state: stream::State) -> Result<ModelStream, ModelError> {
    let (first, state) = stream::next(state)
        .await
        .ok_or_else(|| error::unknown(ModelErrorKind::IncompleteResponse))?;
    let first = first?;
    Ok(Box::pin(
        futures_stream::once(async move { Ok(first) })
            .chain(futures_stream::unfold(state, stream::next)),
    ))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use minicore_host_runtime::model::ReasoningPreference;

    #[test]
    fn https_responses_tries_wss_on_the_same_origin_and_path() {
        let descriptor = ModelDescriptor::new(
            "fixture/responses".parse().unwrap(),
            128_000,
            BTreeSet::from([ReasoningPreference::Auto]),
            true,
        )
        .unwrap();
        for scheme in ["https", "wss"] {
            let model = OpenAiResponsesModel::new(
                OpenAiResponsesConfig::new(
                    format!("{scheme}://example.com:8443/custom/v1/responses"),
                    "fixture",
                    descriptor.clone(),
                ),
                SecretApiKey::new("fixture-key").unwrap(),
            )
            .unwrap();
            assert_eq!(
                model.websocket_endpoint.unwrap().as_str(),
                "wss://example.com:8443/custom/v1/responses"
            );
            assert_eq!(
                model.connection.endpoint.as_str(),
                "https://example.com:8443/custom/v1/responses"
            );
        }
    }
}
