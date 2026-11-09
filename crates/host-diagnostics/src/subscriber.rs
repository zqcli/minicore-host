use crate::{
    Context, Options,
    backend::RecordSender,
    record::{EventPayload, FaultIdentity, LogValue, Scope, TARGET},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, sync::Arc};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id},
};
use tracing_subscriber::{Layer, layer::Context as LayerContext, registry::LookupSpan};

pub(crate) struct DiagnosticsLayer {
    options: Options,
    sender: Arc<RecordSender>,
}

impl DiagnosticsLayer {
    pub(crate) fn new(options: Options, sender: Arc<RecordSender>) -> Self {
        Self { options, sender }
    }
}

// Only the two typed helper fields are consumed. Unknown tracing fields,
// arbitrary Debug/Display implementations and error chains are never evaluated.
struct PayloadVisitor<T> {
    field: &'static str,
    value: Option<T>,
}
impl<T> PayloadVisitor<T> {
    fn new(field: &'static str) -> Self {
        Self { field, value: None }
    }
}
impl<T: for<'de> Deserialize<'de>> Visit for PayloadVisitor<T> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == self.field {
            self.value = serde_json::from_str(value).ok();
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn fmt::Debug) {}
}

#[derive(Serialize)]
struct LogRecord<'a> {
    schema_version: u8,
    timestamp: String,
    level: &'a str,
    scope: Scope,
    service: &'a str,
    component: &'a str,
    event: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_version: Option<&'a str>,
    #[serde(flatten)]
    context: Context,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    fault: Option<FaultIdentity>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    attributes: BTreeMap<String, LogValue>,
}

impl<S> Layer<S> for DiagnosticsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _: LayerContext<'_, S>) -> bool {
        metadata.target() == TARGET
            && metadata.module_path() == Some("minicore_host_diagnostics::record")
            && (metadata.is_span() || *metadata.level() <= self.options.level)
    }

    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: LayerContext<'_, S>) {
        let mut visitor = PayloadVisitor::<Context>::new("context");
        attributes.record(&mut visitor);
        if let (Some(identity), Some(span)) = (visitor.value, context.span(id)) {
            span.extensions_mut().insert(identity);
        }
    }

    fn on_event(&self, event: &Event<'_>, context: LayerContext<'_, S>) {
        let mut visitor = PayloadVisitor::<EventPayload>::new("record");
        event.record(&mut visitor);
        let Some(payload) = visitor.value else {
            return;
        };
        let identity = payload
            .context
            .or_else(|| {
                context
                    .event_scope(event)?
                    .find_map(|span| span.extensions().get::<Context>().cloned())
            })
            .unwrap_or_default();
        let Ok(timestamp) = OffsetDateTime::now_utc().format(&Rfc3339) else {
            self.sender.dropped();
            return;
        };
        let record = LogRecord {
            schema_version: 1,
            timestamp,
            level: event.metadata().level().as_str(),
            scope: payload.scope,
            service: self.options.service,
            component: &payload.component,
            event: &payload.event,
            message: &payload.message,
            service_version: self.options.service_version,
            context: identity,
            fault: payload.fault,
            attributes: payload.attributes,
        };
        match serde_json::to_vec(&record) {
            Ok(mut bytes) => {
                bytes.push(b'\n');
                self.sender.send(bytes);
            }
            Err(_) => self.sender.dropped(),
        }
    }
}
