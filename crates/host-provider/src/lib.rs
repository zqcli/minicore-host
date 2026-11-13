#![doc = include_str!("../README.md")]

//! Production HTTP/SSE transport for the Runtime `Model` capability.
//!
//! This crate owns neither sessions nor retries. One `start` sends one HTTP
//! request. The Runtime ModelDriver alone decides whether another attempt is
//! safe. Fixtures in this crate exercise real transport, not a live provider.

mod config;
mod diagnostics;
mod error;
mod request;
mod responses;
mod sse;
mod stream;

pub use config::{OpenAiChatConfig, OpenAiResponsesConfig, ProviderConfigError, SecretApiKey};
pub use responses::OpenAiResponsesModel;

use std::{fmt, time::Instant};

use futures_util::{StreamExt, stream as futures_stream};
use minicore_host_runtime::model::{
    Model, ModelCallContext, ModelDescriptor, ModelError, ModelErrorKind, ModelRequest,
    ModelStartFuture, ModelStream,
};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use tokio::time::Instant as TokioInstant;
use tracing::Instrument;

/// Immutable, shareable OpenAI-compatible Chat Completions client.
///
/// Construct a new instance for a changed configuration. Existing Turn bindings
/// keep their original instance and credentials until their owners release it.
pub struct OpenAiChatModel {
    descriptor: ModelDescriptor,
    client: reqwest::Client,
    endpoint: reqwest::Url,
    provider_model: String,
    authorization: HeaderValue,
}

impl fmt::Debug for OpenAiChatModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenAiChatModel")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl OpenAiChatModel {
    /// Validate private Host connection settings without contacting a provider.
    pub fn new(config: OpenAiChatConfig, key: SecretApiKey) -> Result<Self, ProviderConfigError> {
        let endpoint = config.validate()?;
        let authorization = key.into_authorization()?;
        let mut client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            // reqwest's default policy can retry protocol NACKs. Disable it:
            // only Runtime has the evidence and budgets to authorize retries.
            .retry(reqwest::retry::never())
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!(
                "minicore-host-provider/",
                env!("CARGO_PKG_VERSION")
            ));
        if let Some(timeout) = config.request_timeout {
            client = client.timeout(timeout);
        }
        let client = client
            .build()
            .map_err(|_| ProviderConfigError::ClientBuild)?;
        Ok(Self {
            descriptor: config.descriptor,
            provider_model: config.provider_model,
            endpoint,
            authorization,
            client,
        })
    }

    /// Read a key from the named environment variable, never from Session config.
    pub fn from_env(config: OpenAiChatConfig, name: &str) -> Result<Self, ProviderConfigError> {
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
        let body = request::encode(&self.provider_model, &self.descriptor, &request)?;
        let response = self.send(body, &context, diagnostic).await?;
        let deadline = TokioInstant::from_std(context.deadline);
        let state = stream::State::new(response, context.cancellation, deadline);
        // Initial explicit SSE rejections must reach start(), before Runtime
        // consumes a model event. Never retry a partially observed generation.
        let (first, state) = stream::next(state)
            .await
            .ok_or_else(|| error::unknown(ModelErrorKind::IncompleteResponse))?;
        let first = first?;
        Ok(Box::pin(
            futures_stream::once(async move { Ok(first) })
                .chain(futures_stream::unfold(state, stream::next)),
        ))
    }

    // Both wire protocols share exactly the same HTTP policy, credentials,
    // classification and cancellation/deadline boundary. Encoding and SSE
    // event interpretation remain protocol-specific.
    async fn send(
        &self,
        body: Vec<u8>,
        context: &ModelCallContext,
        diagnostic: &mut diagnostics::Request,
    ) -> Result<reqwest::Response, ModelError> {
        let http_request = self
            .client
            .post(self.endpoint.clone())
            .header(AUTHORIZATION, self.authorization.clone())
            .header(ACCEPT, "text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .build()
            .map_err(|_| error::local(ModelErrorKind::InvalidRequest))?;
        let deadline = TokioInstant::from_std(context.deadline);
        let response = tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => return Err(error::unknown(ModelErrorKind::Cancelled)),
            _ = tokio::time::sleep_until(deadline) => return Err(error::unknown(ModelErrorKind::Timeout)),
            response = self.client.execute(http_request) => response.map_err(error::send)?,
        };
        diagnostic.response(response.status().as_u16());
        if !response.status().is_success() {
            return Err(error::http(response, context, deadline).await);
        }
        let is_sse = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"));
        if !is_sse {
            return Err(error::unknown(ModelErrorKind::InvalidProviderResponse));
        }
        Ok(response)
    }
}

impl Model for OpenAiChatModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
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
