//! Read one complete declaration set; publication belongs to Host owners.
use crate::{HostConfig, LogSettings, NetworkSettings, OpenAiModelSettings, invalid};
use minicore_host_protocol::{AgentConfig, Fault, Role, SessionDefaults, ToolBinding};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDefinition {
    pub key: String,
    pub name: String,
    pub config: AgentConfig,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedConfiguration {
    /// Canonical directory bound to this candidate's source.
    pub directory: PathBuf,
    pub host: HostConfig,
    pub agents: Vec<AgentDefinition>,
}

// Prompt paths exist only in file declarations; owners receive plain text.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostFile {
    name: String,
    #[serde(default, deserialize_with = "present_string")]
    instructions: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    instructions_file: Option<String>,
    models: Vec<OpenAiModelSettings>,
    tools: Vec<ToolBinding>,
    #[serde(default)]
    workspace_roots: Vec<String>,
    network: NetworkSettings,
    logging: LogSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentFile {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) role: String,
    #[serde(
        default,
        deserialize_with = "present_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) instructions: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) instructions_file: Option<String>,
    pub(crate) session_defaults: SessionDefaults,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleFile {
    name: String,
    description: String,
    #[serde(default, deserialize_with = "present_string")]
    instructions: Option<String>,
    #[serde(default, deserialize_with = "present_string")]
    instructions_file: Option<String>,
}

fn present_string<'de, D: Deserializer<'de>>(input: D) -> Result<Option<String>, D::Error> {
    String::deserialize(input).map(Some)
}

fn resolve_instructions(
    root: &Path,
    declaration: &Path,
    inline: Option<String>,
    file: Option<String>,
) -> Result<String, Fault> {
    let Some(file) = file else {
        return Ok(inline.unwrap_or_default());
    };
    if inline.is_some() {
        return Err(file_error(
            declaration,
            "load instructions",
            "use inline instructions or an instructions file, not both",
        ));
    }
    let relative = Path::new(&file);
    if relative.has_root()
        || file.contains(['\\', ':', '\0'])
        || relative.extension().and_then(|ext| ext.to_str()) != Some("md")
    {
        return Err(file_error(
            declaration,
            "load instructions",
            "use a relative .md path with / separators",
        ));
    }
    let path = declaration.with_file_name(relative);
    let resolved = path
        .canonicalize()
        .map_err(|error| file_error(&path, "resolve instructions file", error))?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(file_error(
            &path,
            "load instructions",
            "expected a regular file inside the configuration directory",
        ));
    }
    std::fs::read_to_string(&resolved)
        .map_err(|error| file_error(&path, "read instructions file", error))
}

/// Load Host, Agent and Role declarations. Frontend files are never read. Missing
/// Agent/Role directories mean empty declaration sets; host.json is required.
/// This performs filesystem I/O synchronously and must run outside owner locks.
pub fn load_config_directory(root: impl AsRef<Path>) -> Result<LoadedConfiguration, Fault> {
    let root = root.as_ref();
    let directory = root
        .canonicalize()
        .map_err(|error| file_error(root, "resolve configuration directory", error))?;
    if !directory.is_dir() {
        return Err(invalid(format!(
            "configuration root is not a directory: {}",
            directory.display()
        )));
    }
    let host_path = directory.join("host.json");
    let file: HostFile = read_json(&host_path)?;
    let host = HostConfig {
        name: file.name,
        instructions: resolve_instructions(
            &directory,
            &host_path,
            file.instructions,
            file.instructions_file,
        )?,
        models: file.models,
        tools: file.tools,
        workspace_roots: file.workspace_roots,
        network: file.network,
        logging: file.logging,
    };
    host.validate()
        .map_err(|error| file_error(&host_path, "validate configuration", error))?;

    let mut roles = BTreeMap::new();
    for path in json_files(&directory.join("roles"))? {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| invalid(format!("role filename is not UTF-8: {}", path.display())))?;
        validate_role_reference(stem)
            .map_err(|error| file_error(&path, "validate role filename", error))?;
        let file: RoleFile = read_json(&path)?;
        let role = Role {
            name: file.name,
            description: file.description,
            instructions: resolve_instructions(
                &directory,
                &path,
                file.instructions,
                file.instructions_file,
            )?,
        };
        role.validate()
            .map_err(|error| file_error(&path, "validate Role declaration", error))?;
        roles.insert(stem.to_owned(), role);
    }

    let mut keys = BTreeSet::new();
    let mut agents = Vec::new();
    for path in json_files(&directory.join("agents"))? {
        let file: AgentFile = read_json(&path)?;
        validate_label(&file.key, "Agent configuration key")
            .and_then(|()| validate_label(&file.name, "Agent name"))
            .and_then(|()| validate_role_reference(&file.role))
            .map_err(|error| file_error(&path, "validate Agent declaration", error))?;
        if !keys.insert(file.key.clone()) {
            return Err(invalid(format!(
                "duplicate Agent configuration key {:?} in {}",
                file.key,
                path.display()
            )));
        }
        let role = roles.get(&file.role).ok_or_else(|| {
            invalid(format!(
                "Agent declaration {} references missing Role {}",
                path.display(),
                directory
                    .join("roles")
                    .join(format!("{}.json", file.role))
                    .display()
            ))
        })?;
        let config = AgentConfig {
            role: role.clone(),
            instructions: resolve_instructions(
                &directory,
                &path,
                file.instructions,
                file.instructions_file,
            )?,
            session_defaults: file.session_defaults,
        };
        config
            .validate()
            .map_err(|error| file_error(&path, "validate Agent configuration", error))?;
        agents.push(AgentDefinition {
            key: file.key,
            name: file.name,
            config,
        });
    }
    // Source filenames are not identity. Deterministic key order also keeps
    // equivalent declaration sets stable after a source file rename.
    agents.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(LoadedConfiguration {
        directory,
        host,
        agents,
    })
}

fn validate_label(value: &str, label: &str) -> Result<(), Fault> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(invalid(format!(
            "{label} must be nonblank and contain no NUL"
        )));
    }
    Ok(())
}

fn validate_role_reference(value: &str) -> Result<(), Fault> {
    validate_label(value, "Role reference")?;
    let mut components = Path::new(value).components();
    if value.contains(['/', '\\'])
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(invalid("Role reference must be a single filename stem"));
    }
    Ok(())
}

fn json_files(directory: &Path) -> Result<Vec<PathBuf>, Fault> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(file_error(directory, "read configuration directory", error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry
            .map_err(|error| file_error(directory, "read configuration directory entry", error))?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Fault> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| file_error(path, "read configuration", error))?;
    serde_json::from_str(&text).map_err(|error| file_error(path, "parse JSON configuration", error))
}

pub(crate) fn file_error(path: &Path, action: &str, error: impl std::fmt::Display) -> Fault {
    invalid(format!("cannot {action} {}: {error}", path.display()))
}
