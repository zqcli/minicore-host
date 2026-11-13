#![allow(dead_code)]
//! Real production owners, local HTTP/SSE fixture, OS pipes and verified TLS.
use super::compaction_common;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_workspace::*;
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};

pub use compaction_common::{HttpFixture, fault, options};
pub fn selection(names: &[&str]) -> Vec<ToolBinding> {
    names
        .iter()
        .map(|name| ToolBinding {
            name: (*name).into(),
        })
        .collect()
}
pub async fn candidate(http: &HttpFixture, tools: Vec<ToolBinding>) -> PreparedHostConfiguration {
    let config = HostConfig {
        framework_instructions: "Workspace query fixture".into(),
        models: vec![OpenAiModelSettings {
            api: minicore_host_core::OpenAiModelApi::ChatCompletions,
            model: "workspace/chat".parse().unwrap(),
            endpoint: http.endpoint.clone(),
            provider_model: "local-fixture".into(),
            api_key_env: "TASK044_MODEL_KEY".into(),
            context_window: 32_000,
            supported_reasoning: vec![ReasoningPreference::Auto],
            supports_tools: true,
            request_timeout_ms: None,
        }],
        tools,
        network: NetworkSettings {
            listen: None,
            access_key_env: Some("TASK044_HOST_KEY".into()),
        },
        ..Default::default()
    };
    let credentials = HostCredentials {
        model_keys: [(
            config.models[0].model.clone(),
            SecretApiKey::new("task044-fixture-only-key").unwrap(),
        )]
        .into_iter()
        .collect(),
        network_access_key: Some(compaction_common::KEY.into()),
    };
    HostResourceBuilder::builtins()
        .unwrap()
        .prepare_with_credentials(config, credentials)
        .await
        .unwrap()
}
pub async fn host(http: &HttpFixture, tools: Vec<ToolBinding>) -> Arc<Host> {
    Host::memory_configured(candidate(http, tools).await).unwrap()
}
pub fn request(root: Option<&Path>, tools: Vec<ToolBinding>) -> AgentCreate {
    let mut config = AgentConfig::default();
    config.session_defaults.model = "workspace/chat".parse().unwrap();
    config.session_defaults.execution.compaction.enabled = false;
    config.session_defaults.workspace = root.map(|path| WorkspaceRef {
        path: path.to_string_lossy().into_owned(),
    });
    config.session_defaults.tools = tools;
    AgentCreate {
        operation_id: OperationId::new().unwrap(),
        name: "workspace query owner".into(),
        mode: StorageMode::Memory,
        config,
        default_session: None,
        initial_message: None,
    }
}
pub async fn create(
    host: &Arc<Host>,
    caller: &CallerContext,
    root: Option<&Path>,
    tools: Vec<ToolBinding>,
) -> SessionRef {
    let receipt = host
        .create_agent_async(caller, request(root, tools))
        .await
        .unwrap();
    SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    }
}
pub use super::wss_common::Clients;
impl Clients {
    /// A deliberate disconnect may close the response pipe before an already
    /// dispatched query writes its late reply. Only that known pipe outcome is
    /// expected; ordinary completed-call teardown remains strict.
    pub async fn finish_disconnected(self) {
        for client in &self.clients {
            client.close();
        }
        if let Err(error) = self.pipe.await.unwrap() {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        }
    }
}
pub fn files() -> FilesRequest {
    FilesRequest {
        directory: String::new(),
        recursive: true,
        query: None,
        cursor: None,
        limit: 100,
    }
}
pub fn read(path: &str) -> ReadRequest {
    ReadRequest {
        path: path.into(),
        start: 0,
        end: None,
        max_bytes: 1024,
    }
}
pub fn list(scope: ChangeScope) -> ChangesListRequest {
    ChangesListRequest {
        scope,
        cursor: None,
        limit: 100,
    }
}
pub fn diff(change_ref: ChangeRef, comparison: DiffComparison) -> DiffRequest {
    DiffRequest {
        change_ref,
        comparison,
        context_lines: 3,
        max_bytes: 1024,
        cursor: None,
    }
}
pub fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "core.autocrlf=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
pub fn repository(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    git(root, &["config", "user.name", "Workspace fixture"]);
}
pub fn tool_reply(
    id: &str,
    name: &str,
    arguments: Value,
) -> (String, Option<Arc<tokio::sync::Semaphore>>) {
    (
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
        ),
        None,
    )
}
