//! Per-Turn execution policy (`ask` / `auto`), with workspace-boundary checks
//! for built-in local tools and validated six-action agent classification.
//! There is no approval cache or second interaction owner.
use minicore_host_local_tools::{is_in_output_log_dir, resolve_policy_path};
use minicore_host_protocol::{ApprovalMode, Fault};
use minicore_host_runtime::tools::{
    ApprovalRequest, ApprovalRisk, ToolDecision, ToolPolicy, ToolPolicyError, ToolPolicyFuture,
    ToolPolicyRequest,
};
use minicore_host_tools::AgentAction;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

const INVALID_CALL_DENIAL: &str = "tool is not allowed by policy";

pub(crate) struct Policy {
    mode: ApprovalMode,
    // Registry source facts are copied once with the original Turn binding.
    builtin_read: bool,
    builtin_agent: bool,
    workspace_root: Option<PathBuf>,
}

enum PolicyDecision {
    Allow,
    RequireApproval { prompt: String, risk: ApprovalRisk },
}

impl Policy {
    pub(crate) fn new(
        mode: ApprovalMode,
        builtin_read: bool,
        builtin_agent: bool,
        workspace_root: Option<PathBuf>,
    ) -> Self {
        Self {
            mode,
            builtin_read,
            builtin_agent,
            workspace_root,
        }
    }

    fn decide_now(&self, request: &ToolPolicyRequest<'_>) -> Result<ToolDecision, ToolPolicyError> {
        if request.cancellation.is_cancelled() {
            return Err(ToolPolicyError::Cancelled);
        }
        if Instant::now() >= request.deadline {
            return Err(ToolPolicyError::Failed);
        }
        if request.invocation.tool_name() != request.spec.name() {
            return deny(INVALID_CALL_DENIAL);
        }
        let outcome = match self.classify(request) {
            Ok(outcome) => outcome,
            Err(error) => {
                return ToolDecision::invalid_invocation(format!(
                    "invalid agent arguments: {}",
                    error.message()
                ));
            }
        };
        match (self.mode, outcome) {
            (ApprovalMode::Auto, _) | (_, PolicyDecision::Allow) => Ok(ToolDecision::Allow),
            (ApprovalMode::Ask, PolicyDecision::RequireApproval { prompt, risk }) => {
                let approval =
                    ApprovalRequest::new(prompt, risk).map_err(|_| ToolPolicyError::Internal)?;
                ToolDecision::require_approval(approval).map_err(|_| ToolPolicyError::Internal)
            }
        }
    }

    fn classify(&self, request: &ToolPolicyRequest<'_>) -> Result<PolicyDecision, Fault> {
        let name = request.invocation.tool_name().as_str();
        let arguments = request.invocation.arguments();
        Ok(match name {
            "read" if self.builtin_read => {
                if let Some(outside) = self.outside_workspace_path(arguments, "path", None)
                    && !is_in_output_log_dir(&outside)
                {
                    PolicyDecision::RequireApproval {
                        prompt: outside_prompt("read", &outside),
                        risk: ApprovalRisk::Medium,
                    }
                } else {
                    PolicyDecision::Allow
                }
            }
            "write" | "edit" | "apply_patch" => {
                if let Some(outside) = self.outside_workspace_path(arguments, "path", None) {
                    PolicyDecision::RequireApproval {
                        prompt: outside_prompt(name, &outside),
                        risk: ApprovalRisk::High,
                    }
                } else {
                    PolicyDecision::RequireApproval {
                        prompt: standard_prompt(name),
                        risk: ApprovalRisk::Medium,
                    }
                }
            }
            "bash" => {
                if let Some(outside) = self.outside_workspace_path(arguments, "cwd", Some(".")) {
                    PolicyDecision::RequireApproval {
                        prompt: outside_prompt("bash", &outside),
                        risk: ApprovalRisk::High,
                    }
                } else {
                    PolicyDecision::RequireApproval {
                        prompt: standard_prompt("bash"),
                        risk: ApprovalRisk::Medium,
                    }
                }
            }
            // Use the same parser and validation as dispatch, not an unchecked
            // action string that might conceal a mutating or invalid branch.
            "agent" if self.builtin_agent => match AgentAction::parse(arguments.clone())? {
                AgentAction::Inspect(_) | AgentAction::Wait(_) => PolicyDecision::Allow,
                AgentAction::Create(_)
                | AgentAction::Send(_)
                | AgentAction::Update(_)
                | AgentAction::Control(_) => PolicyDecision::RequireApproval {
                    prompt: standard_prompt("agent"),
                    risk: ApprovalRisk::Medium,
                },
            },
            // Runtime resolves a selected registered tool before invoking policy.
            // External effects are unknown, so treat them conservatively as
            // mutating rather than silently trusting a read-only declaration.
            _ => PolicyDecision::RequireApproval {
                prompt: standard_prompt(name),
                risk: ApprovalRisk::Medium,
            },
        })
    }

    fn outside_workspace_path(
        &self,
        arguments: &serde_json::Value,
        field: &str,
        default: Option<&str>,
    ) -> Option<PathBuf> {
        let root = self.workspace_root.as_deref()?;
        let raw = arguments
            .get(field)
            .and_then(serde_json::Value::as_str)
            .or(default)?;
        let resolved = resolve_policy_path(root, raw).ok()?;
        (!resolved.in_workspace).then_some(resolved.path)
    }
}

fn standard_prompt(name: &str) -> String {
    format!("Allow tool `{name}` for this call?")
}

fn outside_prompt(name: &str, path: &Path) -> String {
    format!(
        "Allow tool `{name}` outside workspace ({})?",
        sanitize_display_path(path)
    )
}

fn sanitize_display_path(path: &Path) -> String {
    const MAX_CHARS: usize = 200;
    let raw = path.to_string_lossy();
    let cleaned: String = raw
        .chars()
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect();
    let count = cleaned.chars().count();
    if count <= MAX_CHARS {
        cleaned
    } else {
        let keep = MAX_CHARS.saturating_sub(1);
        let tail: String = cleaned.chars().skip(count - keep).collect();
        format!("…{tail}")
    }
}

impl ToolPolicy for Policy {
    fn decide<'a>(&'a self, request: ToolPolicyRequest<'a>) -> ToolPolicyFuture<'a> {
        Box::pin(std::future::ready(self.decide_now(&request)))
    }
}

fn deny(reason: &str) -> Result<ToolDecision, ToolPolicyError> {
    ToolDecision::deny(reason).map_err(|_| ToolPolicyError::Internal)
}

#[cfg(test)]
mod tests;
