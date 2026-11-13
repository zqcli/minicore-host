use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;

use minicore_host_runtime::error::{DiagnosticCategory, DiagnosticCode, DiagnosticSummary};
use minicore_host_runtime::model::{
    AssistantPart, Model, ModelCallContext, ModelDescriptor, ModelError, ModelErrorKind,
    ModelFinishReason, ModelLimits, ModelMessage, ModelRequest, ModelStartFuture,
    ReasoningPreference, Usage,
};
use minicore_host_runtime::prompt::{
    PreparedPrompt, PromptError, PromptFuture, PromptProvider, PromptRequest,
};
use minicore_host_runtime::tools::{
    Tool, ToolContext, ToolExecutionOutcome, ToolFuture, ToolInvocation, ToolName, ToolOutput,
    ToolResultOutcome, ToolSet, ToolSpec,
};
use minicore_host_runtime::value::Text;
use minicore_host_runtime::{
    AssistantHistory, ExecutionConfig, ExecutionConfigError, ExecutionOptions,
    ExecutionOptionsError, HistoryItem, HistoryView, LoopId, SummaryHistory, ToolCallId,
    ToolResultHistory, UserHistory, UserInput, UserMessageKind,
};

struct FakeModel {
    descriptor: ModelDescriptor,
}

impl Model for FakeModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn start<'a>(
        &'a self,
        _request: ModelRequest,
        _context: ModelCallContext,
    ) -> ModelStartFuture<'a> {
        Box::pin(async {
            Err(ModelError::unknown(
                ModelErrorKind::Internal,
                DiagnosticSummary::new(
                    DiagnosticCode::Internal,
                    DiagnosticCategory::Internal,
                    Text::new("fake model start is not exercised here"),
                    false,
                )
                .expect("valid diagnostic text"),
            ))
        })
    }
}

struct FakeTool {
    spec: ToolSpec,
}

impl Tool for FakeTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn execute<'a>(&'a self, _invocation: ToolInvocation, _context: ToolContext) -> ToolFuture<'a> {
        Box::pin(async {
            Ok(ToolExecutionOutcome::Completed(
                ToolOutput::new("done").unwrap(),
            ))
        })
    }
}

struct CapturingPrompt {
    system: Option<Text>,
}

impl PromptProvider for CapturingPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(async move {
            let mut messages = Vec::new();
            if let Some(system) = &self.system {
                messages.push(ModelMessage::system(system.as_str()).unwrap());
            }
            for item in request.history.iter() {
                match item {
                    HistoryItem::User(user) => {
                        messages.push(ModelMessage::user(user.input.as_text()).unwrap())
                    }
                    HistoryItem::Assistant(assistant) => {
                        messages.push(ModelMessage::assistant(assistant.content.clone()).unwrap())
                    }
                    HistoryItem::ToolResult(result) => messages.push(
                        ModelMessage::tool_with_outcome(
                            result.call_id.clone(),
                            result.output.clone(),
                            result.outcome,
                        )
                        .unwrap(),
                    ),
                    HistoryItem::Summary(summary) => messages.push(
                        ModelMessage::system(format!("summary: {}", summary.content.as_str()))
                            .unwrap(),
                    ),
                }
            }
            if messages.is_empty() {
                return Err(PromptError::EmptyPrompt);
            }
            Ok(PreparedPrompt { messages })
        })
    }
}

fn text_model(reasoning: &[ReasoningPreference], supports_tools: bool) -> Arc<dyn Model> {
    let descriptor = ModelDescriptor::new(
        "fake/text-model".parse().unwrap(),
        8192,
        reasoning.iter().copied().collect(),
        supports_tools,
    )
    .unwrap();
    Arc::new(FakeModel { descriptor })
}

fn fake_tools() -> ToolSet {
    let spec = ToolSpec::new(
        "echo".parse().unwrap(),
        "echoes its input back",
        json!({"type": "object"}),
    )
    .unwrap();
    let mut builder = ToolSet::builder();
    builder.register(FakeTool { spec });
    builder.build().unwrap()
}

fn sample_items(loop_id: LoopId) -> (HistoryItem, HistoryItem, HistoryItem, HistoryItem) {
    let user = HistoryItem::User(UserHistory {
        loop_id,
        kind: UserMessageKind::Prompt,
        input: UserInput::text("Fix the parser").unwrap(),
    });
    let assistant = HistoryItem::Assistant(AssistantHistory {
        provider_replay: None,
        loop_id,
        request_index: 0,
        model: "fake/text-model".parse().unwrap(),
        reasoning: ReasoningPreference::High,
        content: vec![AssistantPart::Text("done".into())],
        finish_reason: ModelFinishReason::Stop,
        usage: Usage::new(1, 2, 0),
    });
    let tool_result = HistoryItem::ToolResult(ToolResultHistory {
        loop_id,
        request_index: 0,
        call_id: ToolCallId::new("call_1").unwrap(),
        tool_name: "echo".parse().unwrap(),
        outcome: ToolResultOutcome::Success,
        output: ToolOutput::new("ok").unwrap(),
    });
    let summary = HistoryItem::Summary(SummaryHistory {
        content: Text::new("earlier context"),
    });
    (user, assistant, tool_result, summary)
}

#[test]
fn loop_id_is_canonical_random_prefixed_hex() {
    let id = LoopId::new().expect("the test entropy source is available");
    let encoded = id.to_string();
    assert_eq!(encoded.len(), "lup_".len() + 32);
    assert!(encoded.starts_with("lup_"));
    assert!(
        encoded["lup_".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
    assert_eq!(LoopId::from_str(&encoded).unwrap(), id);
    assert_ne!(id.to_string(), LoopId::new().unwrap().to_string());

    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, format!("\"{encoded}\""));
    assert_eq!(serde_json::from_str::<LoopId>(&json).unwrap(), id);

    let zero = "lup_00000000000000000000000000000000";
    assert!(
        LoopId::from_str(zero).is_err(),
        "zero payload must be rejected"
    );
}

#[test]
fn history_items_serialize_with_typed_tags_and_round_trip() {
    let loop_id = LoopId::new().unwrap();
    let (user, assistant, tool_result, summary) = sample_items(loop_id);

    assert_eq!(
        serde_json::to_value(&user).unwrap(),
        json!({
            "type": "user",
            "data": {
                "loop_id": loop_id.to_string(),
                "kind": "prompt",
                "input": { "text": "Fix the parser" },
            }
        })
    );
    assert_eq!(
        serde_json::to_value(&assistant).unwrap()["type"],
        json!("assistant")
    );
    assert_eq!(
        serde_json::to_value(&tool_result).unwrap()["type"],
        json!("tool_result")
    );
    assert_eq!(
        serde_json::to_value(&summary).unwrap()["type"],
        json!("summary")
    );

    for item in [&user, &assistant, &tool_result, &summary] {
        let encoded = serde_json::to_string(item).unwrap();
        assert_eq!(
            &serde_json::from_str::<HistoryItem>(&encoded).expect("history item must deserialize"),
            item,
            "round trip mismatch for {encoded}"
        );
    }
}

#[test]
fn history_view_projects_base_and_appended_in_order() {
    let loop_id = LoopId::new().unwrap();
    let (user, assistant, _, _) = sample_items(loop_id);
    let base = vec![user.clone()];
    let appended = vec![assistant.clone()];

    let view = HistoryView::new(&base, &appended);
    assert_eq!(view.base(), &base[..]);
    assert_eq!(view.appended(), &appended[..]);
    assert_eq!(view.len(), 2);
    assert!(!view.is_empty());

    let forward: Vec<_> = view.iter().collect();
    assert_eq!(forward, vec![&base[0], &appended[0]]);

    let mut reversed = view.iter().rev();
    assert_eq!(reversed.next(), Some(&appended[0]));
    assert_eq!(reversed.next(), Some(&base[0]));

    let empty = HistoryView::new(&[][..], &[][..]);
    assert_eq!(empty.len(), 0);
    assert!(empty.is_empty());
}

#[test]
fn execution_values_have_no_application_size_limits() {
    let options = ExecutionOptions::default();
    assert!(options.validate().is_ok());
    let debug = format!("{options:?}");
    for removed in ["limits", "max_tool_rounds", "event_capacity"] {
        assert!(!debug.contains(removed));
    }
    let input = UserInput::text("x".repeat(256 * 1024 + 1)).unwrap();
    assert_eq!(input.as_text().len(), 256 * 1024 + 1);
}

#[test]
fn execution_options_reject_invalid_timeouts_and_retry_budgets() {
    let options = ExecutionOptions::default();
    assert!(options.validate().is_ok());

    let mut invalid = options.clone();
    invalid.model_retry_attempts = 0;
    assert_eq!(invalid.validate(), Err(ExecutionOptionsError));

    let mut invalid = options.clone();
    invalid.tool_timeout = Duration::ZERO;
    assert_eq!(invalid.validate(), Err(ExecutionOptionsError));

    let mut invalid = options.clone();
    invalid.deadline = Some(
        tokio::time::Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap(),
    );
    assert_eq!(invalid.validate(), Err(ExecutionOptionsError));

    let mut boundary = options.clone();
    boundary.model_timeout = Duration::from_secs(24 * 60 * 60);
    assert!(boundary.validate().is_ok());

    let mut invalid = options.clone();
    invalid.model_timeout = Duration::from_secs(24 * 60 * 60 + 1);
    assert_eq!(invalid.validate(), Err(ExecutionOptionsError));

    let mut boundary = options.clone();
    boundary.model_retry_base_delay = Duration::from_secs(30);
    assert!(boundary.validate().is_ok());

    let mut invalid = options.clone();
    invalid.model_retry_base_delay = Duration::from_secs(31);
    assert_eq!(invalid.validate(), Err(ExecutionOptionsError));
}

#[test]
fn extended_reasoning_preferences_have_exact_wire_names_and_order() {
    let values = [
        (ReasoningPreference::XHigh, "xhigh"),
        (ReasoningPreference::Max, "max"),
        (ReasoningPreference::Ultra, "ultra"),
    ];
    for (preference, wire) in values {
        let encoded = serde_json::to_string(&preference).unwrap();
        assert_eq!(encoded, format!("\"{wire}\""));
        assert_eq!(
            serde_json::from_str::<ReasoningPreference>(&encoded).unwrap(),
            preference
        );
    }

    let ordered = [
        ReasoningPreference::Auto,
        ReasoningPreference::Disabled,
        ReasoningPreference::Low,
        ReasoningPreference::Medium,
        ReasoningPreference::High,
        ReasoningPreference::XHigh,
        ReasoningPreference::Max,
        ReasoningPreference::Ultra,
    ];
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn extended_reasoning_preferences_flow_through_runtime_boundaries() {
    let preferences = [
        ReasoningPreference::XHigh,
        ReasoningPreference::Max,
        ReasoningPreference::Ultra,
    ];
    let model = text_model(&preferences, false);
    let loop_id = LoopId::new().unwrap();

    for preference in preferences {
        assert!(model.descriptor().supported_reasoning.contains(&preference));
        let config = ExecutionConfig::new(
            Arc::clone(&model),
            preference,
            ToolSet::default(),
            None,
            Arc::new(CapturingPrompt { system: None }),
        )
        .unwrap();
        assert_eq!(config.reasoning(), preference);

        let request = ModelRequest::new(
            vec![ModelMessage::user("request").unwrap()],
            Vec::new(),
            ModelLimits::default(),
            config.reasoning(),
        )
        .unwrap();
        assert_eq!(request.reasoning(), preference);
        let request_wire = serde_json::to_value(&request).unwrap();
        assert_eq!(
            request_wire["reasoning"],
            serde_json::to_value(preference).unwrap()
        );
        assert_eq!(
            serde_json::from_value::<ModelRequest>(request_wire).unwrap(),
            request
        );

        let history = HistoryItem::Assistant(AssistantHistory {
            provider_replay: None,
            loop_id,
            request_index: 0,
            model: "fake/text-model".parse().unwrap(),
            reasoning: preference,
            content: vec![AssistantPart::Text("done".into())],
            finish_reason: ModelFinishReason::Stop,
            usage: Usage::new(1, 2, 0),
        });
        let history_wire = serde_json::to_value(&history).unwrap();
        assert_eq!(
            history_wire["data"]["reasoning"],
            serde_json::to_value(preference).unwrap()
        );
        assert_eq!(
            serde_json::from_value::<HistoryItem>(history_wire).unwrap(),
            history
        );
    }
}

#[test]
fn execution_config_accepts_a_text_only_model() {
    let model = text_model(&[ReasoningPreference::Auto], false);
    let config = ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        ToolSet::default(),
        None,
        Arc::new(CapturingPrompt { system: None }),
    )
    .expect("text model with no tools must be a valid snapshot");

    assert_eq!(config.reasoning(), ReasoningPreference::Auto);
    assert_eq!(config.descriptor().model_ref.as_str(), "fake/text-model");
    assert!(config.policy().is_none());
    assert!(config.tools().specs_for(&Default::default()).is_empty());
    // Getter borrows hold across all snapshot parts without copying.
    let prompt = Arc::clone(config.prompt());
    assert!(Arc::ptr_eq(config.prompt(), &prompt));
}

#[test]
fn execution_config_accepts_tool_capable_model_with_tools() {
    let model = text_model(
        &[ReasoningPreference::Auto, ReasoningPreference::High],
        true,
    );
    let config = ExecutionConfig::new(
        model,
        ReasoningPreference::High,
        fake_tools(),
        None,
        Arc::new(CapturingPrompt { system: None }),
    )
    .expect("tool-capable model with tools must be valid");
    assert!(
        config
            .tools()
            .get(&"echo".parse::<ToolName>().unwrap())
            .is_some()
    );
}

#[test]
fn execution_wrappers_reuse_frozen_resources_without_reentering_the_model() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct OnceDescriptor {
        model: Arc<dyn Model>,
        calls: AtomicUsize,
    }
    impl Model for OnceDescriptor {
        fn descriptor(&self) -> &ModelDescriptor {
            assert_eq!(self.calls.fetch_add(1, Ordering::SeqCst), 0);
            self.model.descriptor()
        }
        fn start<'a>(&'a self, _: ModelRequest, _: ModelCallContext) -> ModelStartFuture<'a> {
            panic!("configuration mapping does not execute the model")
        }
    }

    let model = Arc::new(OnceDescriptor {
        model: text_model(&[ReasoningPreference::High], true),
        calls: AtomicUsize::new(0),
    });
    let config = ExecutionConfig::new(
        model.clone(),
        ReasoningPreference::High,
        fake_tools(),
        None,
        Arc::new(CapturingPrompt { system: None }),
    )
    .unwrap();
    let mapped = config
        .map_tools(|tool, spec| {
            assert_eq!(spec, &config.tools().specs()[0]);
            tool
        })
        .map_model(|inner, descriptor| {
            assert!(Arc::ptr_eq(&inner, config.model()));
            assert_eq!(descriptor, config.descriptor());
            inner
        });
    assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    assert_eq!(mapped.descriptor(), config.descriptor());
    assert_eq!(mapped.reasoning(), config.reasoning());
    assert!(Arc::ptr_eq(mapped.prompt(), config.prompt()));
    assert!(std::ptr::eq(mapped.tools().specs(), config.tools().specs()));
    let request = mapped
        .request(vec![ModelMessage::user("frozen config").unwrap()])
        .unwrap();
    assert!(std::ptr::eq(request.tools(), config.tools().specs()));
}

#[test]
fn execution_config_rejects_untrusted_reasoning() {
    let model = text_model(&[ReasoningPreference::Auto], false);
    let error = ExecutionConfig::new(
        model,
        ReasoningPreference::High,
        ToolSet::default(),
        None,
        Arc::new(CapturingPrompt { system: None }),
    )
    .err()
    .unwrap();
    assert_eq!(error, ExecutionConfigError::UnsupportedReasoning);
}

#[test]
fn execution_config_rejects_tools_without_model_support() {
    let model = text_model(&[ReasoningPreference::Auto], false);
    let error = ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        fake_tools(),
        None,
        Arc::new(CapturingPrompt { system: None }),
    )
    .err()
    .unwrap();
    assert_eq!(error, ExecutionConfigError::ToolsUnsupported);
}

#[test]
fn execution_config_rejects_an_invalid_descriptor() {
    let descriptor = ModelDescriptor {
        model_ref: "fake/broken".parse().unwrap(),
        context_window: 0,
        supported_reasoning: BTreeSet::from([ReasoningPreference::Auto]),
        supports_tools: false,
    };
    let model: Arc<dyn Model> = Arc::new(FakeModel { descriptor });
    let error = ExecutionConfig::new(
        model,
        ReasoningPreference::Auto,
        ToolSet::default(),
        None,
        Arc::new(CapturingPrompt { system: None }),
    )
    .err()
    .unwrap();
    assert_eq!(error, ExecutionConfigError::InvalidDescriptor);
}

#[tokio::test]
async fn prompt_provider_receives_the_full_history_view() {
    let loop_id = LoopId::new().unwrap();
    let (user, assistant, _, summary) = sample_items(loop_id);
    let base = vec![user, summary];
    let appended = vec![assistant];
    let descriptor = ModelDescriptor::new(
        "fake/text-model".parse().unwrap(),
        8192,
        BTreeSet::from([ReasoningPreference::Auto]),
        false,
    )
    .unwrap();

    let provider = CapturingPrompt {
        system: Some(Text::new("be concise")),
    };
    let request = PromptRequest {
        loop_id,
        request_index: 0,
        history: HistoryView::new(&base, &appended),
        model: &descriptor,
        reasoning: ReasoningPreference::Auto,
        tools: &[],
        cancellation: CancellationToken::new(),
        deadline: tokio::time::Instant::now() + Duration::from_secs(30),
    };

    let prepared = provider
        .prepare(request)
        .await
        .expect("prepare must succeed");
    assert_eq!(prepared.messages.len(), 4);
    assert!(matches!(
        &prepared.messages[1],
        ModelMessage::User(text) if text.as_ref() == "Fix the parser"
    ));
    // base = [user, summary], appended = [assistant]; the summary lands between.
    assert!(
        matches!(&prepared.messages[2], ModelMessage::System(text) if text.starts_with("summary:"))
    );
    assert!(matches!(&prepared.messages[3], ModelMessage::Assistant(_)));
}

#[tokio::test]
async fn prompt_provider_rejects_an_empty_prompt() {
    let loop_id = LoopId::new().unwrap();
    let descriptor = ModelDescriptor::new(
        "fake/text-model".parse().unwrap(),
        8192,
        BTreeSet::from([ReasoningPreference::Auto]),
        false,
    )
    .unwrap();
    let provider = CapturingPrompt { system: None };
    let request = PromptRequest {
        loop_id,
        request_index: 0,
        history: HistoryView::new(&[][..], &[][..]),
        model: &descriptor,
        reasoning: ReasoningPreference::Auto,
        tools: &[],
        cancellation: CancellationToken::new(),
        deadline: tokio::time::Instant::now() + Duration::from_secs(30),
    };
    let result = provider.prepare(request).await;
    assert!(matches!(result, Err(PromptError::EmptyPrompt)));
}
