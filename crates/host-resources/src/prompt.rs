//! Frozen framework, Role and canonical Workspace context for model prompts.
//! History preparation continues through core's existing provenance adapter.
use minicore_host_workspace::Workspace;
use minicore_runtime::{
    Text,
    model::ModelMessage,
    prompt::{PromptFuture, PromptProvider, PromptRequest},
};
use std::sync::Arc;

/// Frozen model context. Workspace owns its canonical root and is never a
/// process cwd; the model receives it as explicit context. In a Turn, ordinary
/// filesystem tools share this binding and its mutation ordering capability.
pub(super) struct BoundPrompt {
    framework: Text,
    role: Text,
    workspace: Option<Arc<Workspace>>,
}
impl BoundPrompt {
    pub(super) fn new(framework: &str, role: &str, workspace: Option<Arc<Workspace>>) -> Self {
        Self {
            framework: Text::new(framework),
            role: Text::new(role),
            workspace,
        }
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
            if let Some(workspace) = &self.workspace {
                prefix.push(ModelMessage::System(Arc::from(format!(
                    "Workspace directory: {}",
                    workspace.root().display()
                ))));
            }
            prefix.append(&mut prompt.messages);
            prompt.messages = prefix;
            Ok(prompt)
        })
    }
}
