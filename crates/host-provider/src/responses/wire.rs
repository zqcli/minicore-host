//! Concrete consumed wire fields. Unknown extensions are skipped by serde;
//! duplicate recognized fields and mistyped consumed data remain errors.

use minicore_host_runtime::model::Usage;
use serde::{Deserialize, Serialize, de::IgnoredAny};
use serde_json::Value;

#[derive(Deserialize)]
pub(super) struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    pub sequence_number: Option<u64>,
    pub response: Option<Response>,
    pub output_index: Option<usize>,
    pub item_id: Option<String>,
    pub content_index: Option<usize>,
    pub summary_index: Option<usize>,
    pub item: Option<Item>,
    pub part: Option<Content>,
    pub delta: Option<String>,
    pub text: Option<String>,
    pub refusal: Option<String>,
    pub arguments: Option<String>,
    pub error: Option<Value>,
    pub code: Option<String>,
    // Free text is never put in diagnostics, but its consumed type is checked.
    #[serde(rename = "message")]
    pub _message: Option<String>,
    #[serde(rename = "param")]
    pub _param: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ErrorEnvelope {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub error: Option<Value>,
    pub code: Option<String>,
    #[serde(rename = "response")]
    pub _response: Option<IgnoredAny>,
    #[serde(rename = "sequence_number")]
    pub _sequence_number: Option<IgnoredAny>,
    #[serde(rename = "output_index")]
    pub _output_index: Option<IgnoredAny>,
    #[serde(rename = "item_id")]
    pub _item_id: Option<IgnoredAny>,
    #[serde(rename = "content_index")]
    pub _content_index: Option<IgnoredAny>,
    #[serde(rename = "summary_index")]
    pub _summary_index: Option<IgnoredAny>,
    #[serde(rename = "item")]
    pub _item: Option<IgnoredAny>,
    #[serde(rename = "part")]
    pub _part: Option<IgnoredAny>,
    #[serde(rename = "delta")]
    pub _delta: Option<IgnoredAny>,
    #[serde(rename = "text")]
    pub _text: Option<IgnoredAny>,
    #[serde(rename = "refusal")]
    pub _refusal: Option<IgnoredAny>,
    #[serde(rename = "arguments")]
    pub _arguments: Option<IgnoredAny>,
    #[serde(rename = "message")]
    pub _message: Option<IgnoredAny>,
    #[serde(rename = "param")]
    pub _param: Option<IgnoredAny>,
}

#[derive(Deserialize)]
pub(super) struct FailedEnvelope {
    pub response: Option<ErrorResponse>,
}

#[derive(Deserialize)]
pub(super) struct ErrorResponse {
    pub error: Option<Value>,
    #[serde(rename = "id")]
    pub _id: Option<IgnoredAny>,
    #[serde(rename = "status")]
    pub _status: Option<IgnoredAny>,
    // Keep only whether actual output exists, even when an unsupported normal
    // item forced the error-envelope path. Such evidence forbids a retry.
    pub output: Option<Vec<IgnoredAny>>,
    #[serde(rename = "usage")]
    pub _usage: Option<IgnoredAny>,
    #[serde(rename = "incomplete_details")]
    pub _incomplete_details: Option<IgnoredAny>,
}

#[derive(Deserialize)]
pub(super) struct Response {
    pub id: String,
    pub status: String,
    pub output: Vec<Item>,
    pub usage: Option<ProviderUsage>,
    pub error: Option<Value>,
    pub incomplete_details: Option<IncompleteDetails>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct IncompleteDetails {
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Item {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<Content>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Vec<Content>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct Content {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<String>,
}

impl Content {
    pub fn text(&self) -> Result<&str, ()> {
        match self.kind.as_str() {
            "output_text" | "reasoning_text" | "summary_text" if self.refusal.is_none() => {
                self.text.as_deref().ok_or(())
            }
            "refusal" if self.text.is_none() => self.refusal.as_deref().ok_or(()),
            _ => Err(()),
        }
    }
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

pub(super) fn valid_phase(value: Option<&str>) -> bool {
    matches!(value, None | Some("commentary" | "final_answer"))
}

pub(super) fn validate_reasoning(item: &Item) -> Result<(), ()> {
    if item.kind != "reasoning"
        || !valid_id(&item.id)
        || item.status.as_deref().is_some_and(|s| s != "completed")
        || item.role.is_some()
        || item.phase.is_some()
        || item.call_id.is_some()
        || item.name.is_some()
        || item.arguments.is_some()
        || item
            .encrypted_content
            .as_deref()
            .is_some_and(|s| s.is_empty())
    {
        return Err(());
    }
    for (parts, kind) in [
        (&item.summary, "summary_text"),
        (&item.content, "reasoning_text"),
    ] {
        if let Some(parts) = parts {
            for part in parts {
                if part.kind != kind {
                    return Err(());
                }
                part.text()?;
            }
        } else if kind == "summary_text" {
            return Err(());
        }
    }
    Ok(())
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct ProviderUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputDetails>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct InputDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct OutputDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

impl ProviderUsage {
    pub fn runtime(&self) -> Result<Usage, ()> {
        let cached = self
            .input_tokens_details
            .as_ref()
            .and_then(|v| v.cached_tokens);
        let written = self
            .input_tokens_details
            .as_ref()
            .and_then(|v| v.cache_write_tokens);
        let reasoning = self
            .output_tokens_details
            .as_ref()
            .and_then(|v| v.reasoning_tokens);
        let input = match (self.input_tokens, cached) {
            (Some(total), Some(cached)) => Some(
                total
                    .checked_sub(cached)
                    .and_then(|v| v.checked_sub(written.unwrap_or(0)))
                    .ok_or(())?,
            ),
            _ => None,
        };
        // Even when another category is unknown, a known subset cannot exceed its total.
        if let (Some(total), Some(written)) = (self.input_tokens, written) {
            total.checked_sub(written).ok_or(())?;
        }
        let output = match (self.output_tokens, reasoning) {
            (Some(total), Some(reasoning)) => Some(total.checked_sub(reasoning).ok_or(())?),
            _ => None,
        };
        if let (Some(input), Some(output), Some(total)) =
            (self.input_tokens, self.output_tokens, self.total_tokens)
            && input.checked_add(output) != Some(total)
        {
            return Err(());
        }
        Ok(Usage::from_optional(input, output, reasoning)
            .with_cache_read_tokens(cached)
            .with_cache_write_tokens(written)
            .with_provider_total_tokens(self.total_tokens))
    }
}
