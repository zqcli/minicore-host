//! File declarations expand to the existing effective model catalog.
use crate::{
    InlineModelKeys, OpenAiModelApi, OpenAiModelSettings, OpenAiWebSearch, ResponsesStream, invalid,
};
use minicore_host_protocol::{Fault, ModelRef, ReasoningPreference};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderFile {
    pub(crate) baseurl: String,
    pub(crate) api: OpenAiModelApi,
    pub(crate) api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<ResponsesStream>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_timeout_ms: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelFile {
    pub(crate) name: ModelRef,
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) context_window: u64,
    pub(crate) reasoning: Vec<ReasoningPreference>,
    pub(crate) supports_tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) web_search: Option<OpenAiWebSearch>,
}

pub(crate) fn expand(
    providers: &BTreeMap<String, ProviderFile>,
    models: Vec<ModelFile>,
) -> Result<(Vec<OpenAiModelSettings>, InlineModelKeys), Fault> {
    let providers = providers
        .iter()
        .map(|(name, provider)| {
            if name.is_empty() || name.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
                return Err(invalid("invalid provider name"));
            }
            let (env, literal) = match provider.api_key.strip_prefix('$') {
                Some(name) if valid_env(name) => (name.to_owned(), None),
                Some(_) => {
                    return Err(invalid(format!(
                        "provider {name} has an invalid API key environment reference"
                    )));
                }
                None if !provider.api_key.is_empty()
                    && provider.api_key.bytes().all(|b| (0x21..=0x7e).contains(&b)) =>
                {
                    (
                        String::new(),
                        Some(Arc::<str>::from(provider.api_key.as_str())),
                    )
                }
                None => {
                    return Err(invalid(format!(
                        "provider {name} has an invalid API key literal"
                    )));
                }
            };
            if provider.stream.is_some() && provider.api != OpenAiModelApi::Responses {
                return Err(invalid(format!(
                    "provider {name}: stream requires the Responses API"
                )));
            }
            if provider.request_timeout_ms == Some(0) {
                return Err(invalid(format!(
                    "provider {name} has an invalid request timeout"
                )));
            }
            let endpoint = endpoint(&provider.baseurl, provider.api)
                .ok_or_else(|| invalid(format!("provider {name} has an invalid baseurl")))?;
            Ok((name.as_str(), (provider, endpoint, env, literal)))
        })
        .collect::<Result<BTreeMap<_, _>, Fault>>()?;

    let mut keys = InlineModelKeys::default();
    let models = models
        .into_iter()
        .map(|model| {
            let (provider, endpoint, env, literal) =
                providers.get(model.provider.as_str()).ok_or_else(|| {
                    invalid(format!(
                        "model {} references an unknown provider",
                        model.name
                    ))
                })?;
            if let Some(secret) = literal {
                keys.insert(model.name.clone(), Arc::clone(secret));
            }
            Ok(OpenAiModelSettings {
                model: model.name,
                api: provider.api,
                endpoint: endpoint.clone(),
                provider_model: model.model,
                api_key_env: env.clone(),
                context_window: model.context_window,
                supported_reasoning: model.reasoning,
                supports_tools: model.supports_tools,
                request_timeout_ms: provider.request_timeout_ms,
                stream: provider.stream,
                web_search: model.web_search,
            })
        })
        .collect::<Result<Vec<_>, Fault>>()?;
    Ok((models, keys))
}

fn valid_env(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b == b'_' || b.is_ascii_alphabetic())
        && bytes.all(|b| b == b'_' || b.is_ascii_alphanumeric())
}

fn endpoint(base: &str, api: OpenAiModelApi) -> Option<String> {
    let mut url = url::Url::parse(base).ok()?;
    let allowed = match url.scheme() {
        "https" => true,
        "http" => url.host().is_some_and(|host| match host {
            url::Host::Ipv4(ip) => ip.is_loopback(),
            url::Host::Ipv6(ip) => ip.is_loopback(),
            url::Host::Domain(_) => false,
        }),
        _ => false,
    };
    if !allowed
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut segments = url.path_segments_mut().ok()?;
    segments.pop_if_empty();
    match api {
        OpenAiModelApi::ChatCompletions => segments.extend(["chat", "completions"]),
        OpenAiModelApi::Responses => segments.push("responses"),
    };
    drop(segments);
    Some(url.into())
}
