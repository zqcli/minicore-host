//! Initial examples are native configuration values and never contain secrets.
use crate::{
    HostConfig, LogOutput, LogSettings, OpenAiModelApi, OpenAiModelSettings,
    directory::{AgentFile, file_error},
    invalid,
};
use minicore_host_protocol::{Fault, Role, SessionDefaults, ToolBinding, WorkspaceRef};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Create missing example Host, Role and Agent files without replacing any
/// existing file. The initial workspace is explicit or the current directory;
/// subsequent loads never apply the caller's cwd to saved Session values.
pub fn create_config_directory(
    root: impl AsRef<Path>,
    workspace: Option<&Path>,
) -> Result<Vec<PathBuf>, Fault> {
    let root = root.as_ref();
    let workspace = match workspace {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()
            .map_err(|error| invalid(format!("cannot locate initial workspace: {error}")))?,
    };
    let workspace = workspace
        .canonicalize()
        .map_err(|error| file_error(&workspace, "resolve initial workspace", error))?;
    if !workspace.is_dir() {
        return Err(invalid(format!(
            "initial workspace is not a directory: {}",
            workspace.display()
        )));
    }
    let workspace = utf8_path(&workspace, "initial workspace")?;
    std::fs::create_dir_all(root)
        .map_err(|error| file_error(root, "create configuration directory", error))?;
    let root = root
        .canonicalize()
        .map_err(|error| file_error(root, "resolve configuration directory", error))?;
    let tools = ["read", "write", "edit", "apply_patch", "bash", "agent"]
        .map(|name| ToolBinding { name: name.into() })
        .to_vec();
    let model: minicore_host_protocol::ModelRef =
        "provider/chat".parse().expect("literal model reference");
    let host = HostConfig {
        name: "MiniCore Host".into(),
        models: vec![OpenAiModelSettings {
            model: model.clone(),
            api: OpenAiModelApi::ChatCompletions,
            endpoint: "https://provider.example.invalid/v1/chat/completions".into(),
            provider_model: "replace-with-provider-model".into(),
            api_key_env: "MINICORE_MODEL_KEY".into(),
            context_window: 128_000,
            supported_reasoning: vec![Default::default()],
            supports_tools: true,
            request_timeout_ms: Some(120_000),
        }],
        tools: tools.clone(),
        workspace_roots: vec![workspace.clone()],
        logging: LogSettings {
            output: LogOutput::File {
                path: utf8_path(&root.join("minicore-host.log"), "log file")?,
            },
            ..LogSettings::default()
        },
        ..HostConfig::default()
    };
    let role = Role {
        name: "Coding assistant".into(),
        description: "Works in the selected workspace".into(),
        instructions: "Inspect the code before changing it. Make focused changes, run relevant checks, and explain the result clearly.".into(),
    };
    let agent = AgentFile {
        key: "assistant".into(),
        name: "Assistant".into(),
        role: "coding".into(),
        instructions: String::new(),
        session_defaults: SessionDefaults {
            model,
            tools,
            workspace: Some(WorkspaceRef { path: workspace }),
            ..SessionDefaults::default()
        },
    };
    let mut created = Vec::new();
    for (path, text) in [
        (root.join("host.json"), json(&host)?),
        (root.join("roles/coding.json"), json(&role)?),
        (root.join("agents/assistant.json"), json(&agent)?),
    ] {
        if create_new(&path, &text)? {
            created.push(path);
        }
    }
    Ok(created)
}

fn utf8_path(path: &Path, label: &str) -> Result<String, Fault> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(format!("{label} must be representable as UTF-8")))
}

fn json(value: &impl serde::Serialize) -> Result<String, Fault> {
    serde_json::to_string_pretty(value)
        .map(|text| format!("{text}\n"))
        .map_err(|error| invalid(format!("cannot encode initial configuration: {error}")))
}

fn create_new(path: &Path, text: &str) -> Result<bool, Fault> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| file_error(parent, "create configuration directory", error))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(text.as_bytes())
                .map_err(|error| file_error(path, "write initial configuration", error))?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(file_error(path, "create initial configuration", error)),
    }
}
