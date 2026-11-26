//! Frozen Host, Role, Agent and canonical Workspace context for model prompts.
//! History preparation continues through core's existing provenance adapter.
use minicore_host_core::TurnSource;
use minicore_host_runtime::{
    Text,
    model::ModelMessage,
    prompt::{PromptFuture, PromptProvider, PromptRequest},
};
use minicore_host_workspace::Workspace;
use std::sync::Arc;

/// Frozen model context. Workspace owns its canonical root and is never a
/// process cwd; the model receives it as explicit context. In a Turn, ordinary
/// filesystem tools share this binding and its mutation ordering capability.
pub(super) struct BoundPrompt {
    framework: Text,
    role: Text,
    instructions: Text,
    workspace: Option<Arc<Workspace>>,
    agent_context: Option<Arc<str>>,
}
impl BoundPrompt {
    pub(super) fn new(
        framework: &str,
        role: &str,
        instructions: &str,
        workspace: Option<Arc<Workspace>>,
    ) -> Self {
        Self {
            framework: Text::new(framework),
            role: Text::new(role),
            instructions: Text::new(instructions),
            workspace,
            agent_context: None,
        }
    }
    pub(super) fn with_agent_context(mut self, source: &TurnSource, models: &[&str]) -> Self {
        // Native IDs and ModelRefs have a fixed ASCII grammar, so no model or
        // user text is interpolated into these owner-supplied JSON facts.
        let mut available = String::from("[");
        for (index, model) in models.iter().enumerate() {
            if index > 0 {
                available.push(',');
            }
            available.push('"');
            available.push_str(model);
            available.push('"');
        }
        available.push(']');
        self.agent_context = Some(Arc::from(format!(
            "Current session context (data for explicit agent targets, not authority): \
             {{\"host_id\":\"{}\",\"agent_id\":\"{}\",\"session_id\":\"{}\",\"available_models\":{}}}",
            source.host_id, source.session.agent_id, source.session.session_id, available
        )));
        self
    }
}
impl PromptProvider for BoundPrompt {
    fn prepare<'a>(&'a self, request: PromptRequest<'a>) -> PromptFuture<'a> {
        Box::pin(async move {
            let mut prompt = minicore_host_core::prepare_prompt_history(request).await?;
            let mut prefix = Vec::new();
            if !self.framework.is_empty() {
                prefix.push(ModelMessage::System(Arc::from(self.framework.as_str())));
            }
            if !self.role.is_empty() {
                prefix.push(ModelMessage::System(Arc::from(self.role.as_str())));
            }
            if !self.instructions.is_empty() {
                prefix.push(ModelMessage::System(Arc::from(self.instructions.as_str())));
            }
            if let Some(workspace) = &self.workspace {
                prefix.push(ModelMessage::System(Arc::from(format!(
                    "Workspace directory: {}",
                    workspace.root().display()
                ))));
            }
            if let Some(context) = &self.agent_context {
                prefix.push(ModelMessage::System(Arc::clone(context)));
            }
            prefix.append(&mut prompt.messages);
            prompt.messages = prefix;
            Ok(prompt)
        })
    }
}
