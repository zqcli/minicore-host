//! Per-Turn execution policy. Reuses the three-mode policy from minicore-agent
//! f36a873, with Host's validated six-action agent tool in place of old names.
//! There is no approval cache or second interaction owner.
use minicore_host_protocol::ApprovalMode;
use minicore_host_tools::AgentAction;
use minicore_runtime::tools::{
    ApprovalRequest, ApprovalRisk, ToolDecision, ToolPolicy, ToolPolicyError, ToolPolicyFuture,
    ToolPolicyRequest,
};
use std::time::Instant;

const READ_ONLY_DENIAL: &str = "tool is not allowed in read-only mode";
const INVALID_CALL_DENIAL: &str = "tool is not allowed by policy";

pub(crate) struct Policy {
    mode: ApprovalMode,
    // Registry source facts are copied once with the original Turn binding.
    builtin_read: bool,
    builtin_agent: bool,
}

#[derive(Clone, Copy)]
enum ToolClass {
    ReadOnly,
    Mutating,
}

impl Policy {
    pub(crate) fn new(mode: ApprovalMode, builtin_read: bool, builtin_agent: bool) -> Self {
        Self {
            mode,
            builtin_read,
            builtin_agent,
        }
    }

    fn decide_now(&self, request: &ToolPolicyRequest) -> Result<ToolDecision, ToolPolicyError> {
        if request.cancellation.is_cancelled() {
            return Err(ToolPolicyError::Cancelled);
        }
        if Instant::now() >= request.deadline {
            return Err(ToolPolicyError::Failed);
        }
        if request.invocation.tool_name() != request.spec.name() {
            return deny(INVALID_CALL_DENIAL);
        }
        let Some(class) = self.classify(request) else {
            return deny(INVALID_CALL_DENIAL);
        };
        match (self.mode, class) {
            (ApprovalMode::Auto, _) | (_, ToolClass::ReadOnly) => Ok(ToolDecision::Allow),
            (ApprovalMode::ReadOnly, ToolClass::Mutating) => deny(READ_ONLY_DENIAL),
            (ApprovalMode::Ask, ToolClass::Mutating) => {
                let prompt = format!(
                    "Allow tool `{}` for this call?",
                    request.invocation.tool_name()
                );
                let approval = ApprovalRequest::new(prompt, ApprovalRisk::Medium)
                    .map_err(|_| ToolPolicyError::Internal)?;
                ToolDecision::require_approval(approval).map_err(|_| ToolPolicyError::Internal)
            }
        }
    }
    fn classify(&self, request: &ToolPolicyRequest) -> Option<ToolClass> {
        match request.invocation.tool_name().as_str() {
            "read" if self.builtin_read => Some(ToolClass::ReadOnly),
            "write" | "edit" | "bash" => Some(ToolClass::Mutating),
            // Use the same parser and validation as dispatch, not an unchecked
            // action string that might conceal a mutating or invalid branch.
            "agent" if self.builtin_agent => {
                match AgentAction::parse(request.invocation.arguments().clone()).ok()? {
                    AgentAction::Inspect(_) | AgentAction::Wait(_) => Some(ToolClass::ReadOnly),
                    AgentAction::Create(_)
                    | AgentAction::Send(_)
                    | AgentAction::Update(_)
                    | AgentAction::Control(_) => Some(ToolClass::Mutating),
                }
            }
            // Runtime resolves a selected registered tool before invoking policy.
            // External effects are unknown, so treat them conservatively as
            // mutating rather than silently trusting a read-only declaration.
            _ => Some(ToolClass::Mutating),
        }
    }
}

impl ToolPolicy for Policy {
    fn decide(&self, request: ToolPolicyRequest) -> ToolPolicyFuture<'_> {
        Box::pin(std::future::ready(self.decide_now(&request)))
    }
}

fn deny(reason: &str) -> Result<ToolDecision, ToolPolicyError> {
    ToolDecision::deny(reason).map_err(|_| ToolPolicyError::Internal)
}

#[cfg(test)]
mod tests;
