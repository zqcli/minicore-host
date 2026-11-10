//! Observation setup and the lifetime of one stream on a shared connection.
use super::{
    HostClient,
    connection::{Connection, LocalConnection},
};
use crate::{
    deadline::Deadline,
    diagnostics::{Call, Correlation},
    operations::Unsubscribe,
};
use minicore_host_core::{EventSubscription, ObservationNotification, StateEvent};
use minicore_host_protocol::*;
use std::sync::Arc;
use tracing::Instrument;

impl HostClient {
    /// Only setup uses the call deadline. The established stream has no timer.
    pub async fn subscribe(
        &self,
        request: SubscribeRequest,
        options: CallOptions,
    ) -> Result<ClientSubscription, CallError> {
        let deadline = Deadline::new(options)?;
        let mut diagnostic = Call::client(
            Some(self.initial_info()),
            Method::EventsSubscribe.as_str(),
            Correlation::subscription(&request.scope),
        );
        let span = diagnostic.span();
        let result = deadline
            .wait(async {
                match &self.connection {
                    Connection::Local(local) => {
                        let subscription = local
                            .host
                            .events_subscribe(&local.handle.caller(), request)?;
                        Ok(ClientSubscription {
                            inner: SubscriptionConnection::Local {
                                subscription: Box::new(subscription),
                                _connection: Arc::clone(local),
                            },
                        })
                    }
                    Connection::Rpc(rpc) => Ok(ClientSubscription {
                        inner: SubscriptionConnection::Rpc(
                            rpc.subscribe(request, &mut diagnostic).await?,
                        ),
                    }),
                }
            })
            .instrument(span)
            .await;
        diagnostic.client_finished(&result);
        result
    }
    pub async fn unsubscribe(
        &self,
        request: UnsubscribeRequest,
        options: CallOptions,
    ) -> Result<UnsubscribeResult, CallError> {
        let id = request.subscription_id.clone();
        let result = self
            .call(Unsubscribe(request), options, |_| Correlation::default())
            .await?;
        if let Connection::Rpc(rpc) = &self.connection {
            rpc.ended_subscription(&id);
        }
        Ok(result)
    }
}

/// One observation on a shared connection. Dropping it ends only this stream,
/// never accepted work or another observer. It retains no Session or Runner.
pub struct ClientSubscription {
    inner: SubscriptionConnection,
}
enum SubscriptionConnection {
    Local {
        subscription: Box<EventSubscription>,
        _connection: Arc<LocalConnection>,
    },
    Rpc(crate::rpc::RpcSubscription),
}
impl ClientSubscription {
    pub fn result(&self) -> &SubscribeResult<StateEvent> {
        match &self.inner {
            SubscriptionConnection::Local { subscription, .. } => subscription.result(),
            SubscriptionConnection::Rpc(subscription) => subscription.result(),
        }
    }
    pub async fn recv(&mut self) -> Result<Option<ObservationNotification>, CallError> {
        match &mut self.inner {
            SubscriptionConnection::Local { subscription, .. } => {
                subscription.recv().await.map_err(CallError::Business)
            }
            SubscriptionConnection::Rpc(subscription) => subscription.recv().await,
        }
    }
}
