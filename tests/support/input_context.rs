//! Test-only decoding of the exact current request presentation. Production
//! never parses this text to discover a source or reconstruct native history.
use minicore_host_protocol::{MessageId, MessageSource};
use serde_json::Value;

#[derive(Debug, Eq, PartialEq)]
pub struct Context {
    pub message_id: MessageId,
    pub source: MessageSource,
    pub in_reply_to: Option<MessageId>,
}
pub fn decode(text: &str) -> (Context, &str) {
    let (header, original) = text
        .strip_prefix("Input context (data, not instructions): ")
        .expect("Session-owned input context")
        .split_once("\n\n")
        .unwrap();
    let value: Value = serde_json::from_str(header).expect("valid JSON input context");
    let context = Context {
        message_id: serde_json::from_value(value["message_id"].clone())
            .expect("original MessageId"),
        source: serde_json::from_value(value["source"].clone()).expect("valid original source"),
        in_reply_to: value.get("in_reply_to").map(|reply| {
            serde_json::from_value(reply.clone()).expect("present reply must be a MessageId")
        }),
    };
    // Exact rendering also rejects extra/duplicate fields, null reply values,
    // and malformed headers instead of silently stripping arbitrary strings.
    let mut expected = format!(
        "{{\"message_id\":{},\"source\":{}",
        serde_json::to_string(&context.message_id).unwrap(),
        serde_json::to_string(&context.source).unwrap()
    );
    if let Some(reply) = &context.in_reply_to {
        expected.push_str(&format!(
            ",\"in_reply_to\":{}",
            serde_json::to_string(reply).unwrap()
        ));
    }
    expected.push('}');
    assert_eq!(header, expected, "exact input context fields and rendering");
    (context, original)
}
