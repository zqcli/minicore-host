use super::*;
use serde_json::{Value, json};

fn settings(api: &str) -> Value {
    json!({
        "model": "fixture/model",
        "api": api,
        "endpoint": "https://example.invalid/custom-endpoint",
        "provider_model": "private-provider-model",
        "api_key_env": "PRIVATE_FIXTURE_KEY_SOURCE",
        "context_window": 128_000,
        "supported_reasoning": ["auto"],
        "supports_tools": true,
        "request_timeout_ms": 20_000
    })
}

#[test]
fn protocol_is_required_and_never_guessed_from_an_endpoint() {
    for (name, api) in [
        ("chat_completions", OpenAiModelApi::ChatCompletions),
        ("responses", OpenAiModelApi::Responses),
    ] {
        let value = settings(name);
        let parsed: OpenAiModelSettings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed.api, api);
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }
    let mut missing = settings("responses");
    missing.as_object_mut().unwrap().remove("api");
    missing["endpoint"] = json!("https://example.invalid/v1/responses");
    assert!(serde_json::from_value::<OpenAiModelSettings>(missing).is_err());
    assert!(serde_json::from_value::<OpenAiModelSettings>(settings("auto")).is_err());
}

#[test]
fn discovery_retains_protocol_without_private_connection_configuration() {
    let model: OpenAiModelSettings = serde_json::from_value(settings("responses")).unwrap();
    let description = serde_json::to_value(ModelDescription::from(&model)).unwrap();
    assert_eq!(description["api"], "responses");
    assert_eq!(description["model"], "fixture/model");
    for name in [
        "endpoint",
        "provider_model",
        "api_key_env",
        "request_timeout_ms",
    ] {
        assert!(description.get(name).is_none());
    }
    let debug = format!("{model:?}");
    assert!(debug.contains("Responses"));
    assert!(!debug.contains("example.invalid"));
    assert!(!debug.contains("PRIVATE_FIXTURE_KEY_SOURCE"));
    assert!(!debug.contains("private-provider-model"));
}
