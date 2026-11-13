use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{
    Model, ModelDescriptor, ModelLimits, ModelMessage, ModelRequest, ModelValueError,
    ReasoningPreference,
};
use crate::prompt::PromptProvider;
use crate::tools::{Tool, ToolPolicy, ToolSet, ToolSpec};
use crate::value::Text;

/// Errors constructing user input.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum UserInputError {
    #[error("user input is empty")]
    InvalidText,
}

/// Non-empty input text accepted by the Session.
///
/// Constructors require non-empty UTF-8 text without a byte limit.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserInput {
    Text(Text),
}

impl UserInput {
    /// Creates non-empty text user input.
    pub fn text(value: impl AsRef<str>) -> Result<Self, UserInputError> {
        let text = Text::new(value);
        if text.is_empty() {
            return Err(UserInputError::InvalidText);
        }
        Ok(Self::Text(text))
    }

    /// Returns the input text as a string slice.
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(text) => text.as_str(),
        }
    }
}

/// Operation budgets bound to one Turn by its Session.
#[derive(Clone, Debug)]
pub struct ExecutionOptions {
    pub deadline: Option<tokio::time::Instant>,
    pub prompt_timeout: Duration,
    pub model_timeout: Duration,
    pub policy_timeout: Duration,
    pub tool_timeout: Duration,
    pub model_retry_attempts: u8,
    pub model_retry_base_delay: Duration,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("invalid execution options")]
pub struct ExecutionOptionsError;

impl Default for ExecutionOptions {
    fn default() -> Self {
        Self {
            deadline: None,
            prompt_timeout: Duration::from_secs(30),
            model_timeout: Duration::from_secs(10 * 60),
            policy_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(30 * 60),
            model_retry_attempts: 2,
            model_retry_base_delay: Duration::from_millis(100),
        }
    }
}

impl ExecutionOptions {
    pub fn validate(&self) -> Result<(), ExecutionOptionsError> {
        if self.prompt_timeout.is_zero()
            || self.model_timeout.is_zero()
            || self.model_timeout > crate::model::MAX_MODEL_CALL_TIMEOUT
            || self.policy_timeout.is_zero()
            || self.tool_timeout.is_zero()
            || self.model_retry_attempts == 0
            || self.model_retry_attempts > 4
            || self.model_retry_base_delay.is_zero()
            || self.model_retry_base_delay > crate::model::MAX_MODEL_RETRY_DELAY
            || self
                .deadline
                .is_some_and(|deadline| deadline <= tokio::time::Instant::now())
        {
            return Err(ExecutionOptionsError);
        }
        Ok(())
    }
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ExecutionConfigError {
    #[error("model descriptor access panicked")]
    ModelDescriptorPanicked,
    #[error("model descriptor is invalid")]
    InvalidDescriptor,
    #[error("model does not support the requested reasoning preference")]
    UnsupportedReasoning,
    #[error("model does not support tools but tools are configured")]
    ToolsUnsupported,
}

/// Immutable execution resources bound by the Session to one Turn.
#[derive(Clone)]
pub struct ExecutionConfig {
    model: Arc<dyn Model>,
    descriptor: ModelDescriptor,
    reasoning: ReasoningPreference,
    tools: ToolSet,
    policy: Option<Arc<dyn ToolPolicy>>,
    prompt: Arc<dyn PromptProvider>,
}

impl ExecutionConfig {
    pub fn new(
        model: Arc<dyn Model>,
        reasoning: ReasoningPreference,
        tools: ToolSet,
        policy: Option<Arc<dyn ToolPolicy>>,
        prompt: Arc<dyn PromptProvider>,
    ) -> Result<Self, ExecutionConfigError> {
        let descriptor = match catch_unwind(AssertUnwindSafe(|| model.descriptor().clone())) {
            Ok(descriptor) => descriptor,
            Err(_) => return Err(ExecutionConfigError::ModelDescriptorPanicked),
        };
        descriptor
            .validate()
            .map_err(|_| ExecutionConfigError::InvalidDescriptor)?;
        if !descriptor.supports_reasoning(reasoning) {
            return Err(ExecutionConfigError::UnsupportedReasoning);
        }
        if !tools.specs().is_empty() && !descriptor.supports_tools {
            return Err(ExecutionConfigError::ToolsUnsupported);
        }
        Ok(Self {
            model,
            descriptor,
            reasoning,
            tools,
            policy,
            prompt,
        })
    }

    pub fn request(&self, messages: Vec<ModelMessage>) -> Result<ModelRequest, ModelValueError> {
        ModelRequest::with_validated_tools(
            messages,
            self.tools.shared_specs(),
            ModelLimits::default(),
            self.reasoning,
        )
    }

    /// Wraps the bound model while retaining its already validated descriptor
    /// and the rest of this Turn's frozen configuration.
    pub fn map_model(
        &self,
        map: impl FnOnce(Arc<dyn Model>, &ModelDescriptor) -> Arc<dyn Model>,
    ) -> Self {
        Self {
            model: map(Arc::clone(&self.model), &self.descriptor),
            ..self.clone()
        }
    }

    /// Wraps only selected tools, retaining their frozen declarations and all
    /// other validated execution resources.
    pub fn map_tools(&self, map: impl FnMut(Arc<dyn Tool>, &ToolSpec) -> Arc<dyn Tool>) -> Self {
        Self {
            tools: self.tools.map_tools(map),
            ..self.clone()
        }
    }

    pub fn model(&self) -> &Arc<dyn Model> {
        &self.model
    }

    pub fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    pub fn reasoning(&self) -> ReasoningPreference {
        self.reasoning
    }

    pub fn tools(&self) -> &ToolSet {
        &self.tools
    }

    pub fn policy(&self) -> Option<&Arc<dyn ToolPolicy>> {
        self.policy.as_ref()
    }

    pub fn prompt(&self) -> &Arc<dyn PromptProvider> {
        &self.prompt
    }
}
