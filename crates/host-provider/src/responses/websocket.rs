//! One Responses request per socket. Host history remains the source of truth.

use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use minicore_host_runtime::model::{ModelCallContext, ModelError, ModelErrorKind};
use reqwest::header::AUTHORIZATION;
use tokio::{net::TcpStream, time::Instant};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error, Message, client::IntoClientRequest},
};

use crate::{OpenAiChatModel, diagnostics, error};

pub(super) type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) async fn start(
    connection: &OpenAiChatModel,
    body: Vec<u8>,
    context: &ModelCallContext,
    deadline: Instant,
    diagnostic: &mut diagnostics::Request,
) -> Result<(Socket, error::ResponseContext), ModelError> {
    let mut request = connection
        .endpoint
        .as_str()
        .into_client_request()
        .map_err(|_| error::local(ModelErrorKind::InvalidRequest))?;
    request
        .headers_mut()
        .insert(AUTHORIZATION, connection.authorization.clone());
    let body = String::from_utf8(body).map_err(|_| error::local(ModelErrorKind::InvalidRequest))?;
    let connect_deadline = deadline.min(Instant::now() + Duration::from_secs(10));
    let opened = tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => return Err(error::local(ModelErrorKind::Cancelled)),
        _ = tokio::time::sleep_until(connect_deadline) => {
            return Err(error::local(ModelErrorKind::Timeout));
        }
        opened = connect_async(request) => opened,
    };
    let response_context = |status, headers| {
        error::ResponseContext::from_headers(status, headers, &connection.authorization)
    };
    let (mut socket, response) = match opened {
        Ok(opened) => opened,
        Err(Error::Http(response)) => {
            let status = response.status().as_u16();
            diagnostic.response(status);
            return Err(error::http_rejection(
                response_context(status, response.headers()),
                response.body().as_deref().unwrap_or_default(),
            ));
        }
        // No response.create was sent yet. Runtime owns any new attempt.
        Err(_) => return Err(error::connect_unavailable()),
    };
    let status = response.status().as_u16();
    diagnostic.response(status);
    let error_context = response_context(status, response.headers());
    tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => return Err(error::unknown(ModelErrorKind::Cancelled)),
        _ = tokio::time::sleep_until(deadline) => return Err(error::unknown(ModelErrorKind::Timeout)),
        sent = socket.send(Message::Text(body.into())) => {
            sent.map_err(|_| error::unknown(ModelErrorKind::RequestOutcomeUnknown))?;
        }
    }
    Ok((socket, error_context))
}

pub(super) async fn frame(socket: &mut Socket) -> Result<Bytes, ModelErrorKind> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => return Ok(text.into()),
            Some(Ok(Message::Ping(_))) => socket
                .flush()
                .await
                .map_err(|_| ModelErrorKind::StreamInterrupted)?,
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None => return Err(ModelErrorKind::IncompleteResponse),
            Some(Ok(_)) => return Err(ModelErrorKind::InvalidProviderResponse),
            Some(Err(_)) => return Err(ModelErrorKind::StreamInterrupted),
        }
    }
}
