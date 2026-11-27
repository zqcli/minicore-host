//! Adapt in-process fixture values to file declarations; production has no legacy file loader.
use minicore_host_core::{HostConfig, OpenAiModelApi};
use serde_json::{Map, Value, json};

pub fn declaration(config: &HostConfig) -> Value {
    let mut file = serde_json::to_value(config).unwrap();
    let mut providers = Map::new();
    let mut models = Vec::new();
    for (i, entry) in config.models.iter().enumerate() {
        let provider = format!("fixture-{i}");
        let suffix = match entry.api {
            OpenAiModelApi::ChatCompletions => "/chat/completions",
            OpenAiModelApi::Responses => "/responses",
        };
        let base = entry
            .endpoint
            .strip_suffix(suffix)
            .expect("standard fixture endpoint");
        let mut route = json!({
            "baseurl": base, "api": entry.api, "api_key": format!("${}", entry.api_key_env),
            "request_timeout_ms": entry.request_timeout_ms
        });
        if let Some(stream) = entry.stream {
            route["stream"] = json!(stream);
        }
        providers.insert(provider.clone(), route);
        let mut model = json!({
            "name": entry.model, "provider": provider, "model": entry.provider_model,
            "reasoning": entry.supported_reasoning, "context_window": entry.context_window,
            "supports_tools": entry.supports_tools,
        });
        if let Some(search) = entry.web_search {
            model["web_search"] = json!(search);
        }
        models.push(model);
    }
    file["providers"] = Value::Object(providers);
    file["models"] = Value::Array(models);
    file
}
