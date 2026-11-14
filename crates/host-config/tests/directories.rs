use minicore_host_config::*;
use minicore_host_protocol::{FaultKind, SessionDefaults};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn write_json(path: &Path, value: &impl serde::Serialize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn agent(key: &str, role: &str) -> Value {
    json!({
        "key":key,"name":"Shared display name","role":role,
        "session_defaults":SessionDefaults::default()
    })
}

fn minimal(root: &Path) {
    write_json(&root.join("host.json"), &HostConfig::default());
    write_json(
        &root.join("roles/code.json"),
        &json!({"name":"Different display name","description":"Role description"}),
    );
}

#[test]
fn load_resolves_native_roles_stable_keys_and_ignores_frontend_files() {
    let root = tempfile::tempdir().unwrap();
    minimal(root.path());
    let mut first = agent("z-stable", "code");
    first["instructions"] = json!("Agent instructions");
    write_json(&root.path().join("agents/arbitrary-filename.json"), &first);
    write_json(
        &root.path().join("agents/z-file.json"),
        &agent("a-stable", "code"),
    );
    for relative in ["tui.json", "web.json", "agent-defaults.json", "config.toml"] {
        std::fs::write(root.path().join(relative), "not valid JSON").unwrap();
    }
    let loaded = load_config_directory(root.path()).unwrap();
    assert_eq!(loaded.directory, root.path().canonicalize().unwrap());
    assert_eq!(loaded.agents.len(), 2);
    assert_eq!(loaded.agents[0].key, "a-stable");
    assert_eq!(loaded.agents[1].key, "z-stable");
    assert_eq!(loaded.agents[0].name, loaded.agents[1].name);
    assert_eq!(loaded.agents[1].config.instructions, "Agent instructions");
    assert_eq!(loaded.agents[1].config.role.name, "Different display name");
    assert!(loaded.agents[1].config.role.instructions.is_empty());
    assert!(loaded.host.framework_instructions.is_empty());
    let native = serde_json::to_value(&loaded.agents[1].config).unwrap();
    assert!(native.get("key").is_none());
    assert!(native["role"].is_object());
    std::fs::rename(
        root.path().join("agents/arbitrary-filename.json"),
        root.path().join("agents/renamed.json"),
    )
    .unwrap();
    assert_eq!(load_config_directory(root.path()).unwrap(), loaded);
}

#[test]
fn required_host_and_missing_optional_directories_have_distinct_meaning() {
    let root = tempfile::tempdir().unwrap();
    let fault = load_config_directory(root.path()).unwrap_err();
    assert_eq!(fault.kind(), Some(FaultKind::HostConfigInvalid));
    assert!(fault.to_string().contains("host.json"));
    write_json(&root.path().join("host.json"), &HostConfig::default());
    let loaded = load_config_directory(root.path()).unwrap();
    assert!(loaded.agents.is_empty());
}

#[test]
fn missing_roles_duplicate_keys_and_path_references_are_errors() {
    let root = tempfile::tempdir().unwrap();
    minimal(root.path());
    let path = root.path().join("agents/a.json");
    for role in [
        "missing", "../code", "/code", "a/b", "a\\b", ".", "..", "C:\\code",
    ] {
        write_json(&path, &agent("stable", role));
        let fault = load_config_directory(root.path()).unwrap_err();
        assert_eq!(fault.kind(), Some(FaultKind::HostConfigInvalid));
        assert!(fault.to_string().contains("a.json"), "{fault}");
    }
    write_json(&path, &agent("stable", "code"));
    write_json(&root.path().join("agents/b.json"), &agent("stable", "code"));
    let fault = load_config_directory(root.path()).unwrap_err();
    assert!(
        fault
            .to_string()
            .contains("duplicate Agent configuration key")
    );
    for key in ["", " \t", "bad\0key"] {
        write_json(&path, &agent(key, "code"));
        assert!(load_config_directory(root.path()).is_err());
    }
}

#[test]
fn malformed_unknown_and_incomplete_native_json_never_fall_back() {
    let root = tempfile::tempdir().unwrap();
    minimal(root.path());
    let path = root.path().join("agents/a.json");
    write_json(&path, &agent("stable", "code"));
    for candidate in [
        {
            let mut value = agent("stable", "code");
            value["mode"] = json!("memory");
            value
        },
        {
            let mut value = agent("stable", "code");
            value["session_defaults"] = json!({"model":"default"});
            value
        },
        {
            let mut value = agent("stable", "code");
            value["instructions"] = Value::Null;
            value
        },
    ] {
        write_json(&path, &candidate);
        assert!(load_config_directory(root.path()).is_err());
    }
    std::fs::write(&path, "{broken").unwrap();
    let fault = load_config_directory(root.path()).unwrap_err();
    assert!(fault.to_string().contains("a.json"));
    assert!(fault.to_string().contains("parse JSON"));
    write_json(&path, &agent("stable", "code"));
    write_json(
        &root.path().join("roles/unused.json"),
        &json!({"name":"Unused","description":"","unknown":true}),
    );
    assert!(
        load_config_directory(root.path())
            .unwrap_err()
            .to_string()
            .contains("unused.json")
    );
    std::fs::remove_file(root.path().join("roles/unused.json")).unwrap();
    let mut host = serde_json::to_value(HostConfig::default()).unwrap();
    host["ui"] = json!({});
    write_json(&root.path().join("host.json"), &host);
    assert!(
        load_config_directory(root.path())
            .unwrap_err()
            .to_string()
            .contains("host.json")
    );
}

#[test]
fn path_selection_uses_executable_default_and_independent_explicit_paths() {
    let expected = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("config");
    let paths = resolve_config_paths(None, None).unwrap();
    assert_eq!(paths.config_dir, expected);
    assert_eq!(paths.data_root, expected.join("data"));
    let paths = resolve_config_paths(
        Some(Path::new("chosen-config")),
        Some(Path::new("chosen-data")),
    )
    .unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(paths.config_dir, cwd.join("chosen-config"));
    assert_eq!(paths.data_root, cwd.join("chosen-data"));
    assert!(resolve_config_paths(Some(Path::new("")), None).is_err());
    let root = tempfile::tempdir().unwrap();
    let paths = resolve_config_paths(Some(root.path()), None).unwrap();
    assert_eq!(paths.config_dir, root.path());
    assert_eq!(paths.data_root, root.path().join("data"));
}

#[test]
fn scaffold_is_native_explicit_and_does_not_replace_existing_files() {
    let root = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let created = create_config_directory(root.path(), Some(workspace.path())).unwrap();
    assert_eq!(created.len(), 3);
    assert!(created.iter().all(|path| path.is_absolute()));
    let loaded = load_config_directory(root.path()).unwrap();
    let workspace = workspace.path().canonicalize().unwrap();
    assert_eq!(loaded.agents.len(), 1);
    assert_eq!(loaded.agents[0].key, "assistant");
    assert_eq!(
        PathBuf::from(
            &loaded.agents[0]
                .config
                .session_defaults
                .workspace
                .as_ref()
                .unwrap()
                .path
        ),
        workspace
    );
    assert_eq!(
        loaded.host.workspace_roots,
        vec![workspace.to_str().unwrap()]
    );
    assert!(
        matches!(loaded.host.logging.output, LogOutput::File { ref path } if Path::new(path).is_absolute())
    );
    assert!(!root.path().join("tui.json").exists());
    assert!(!root.path().join("data").exists());
    std::fs::write(root.path().join("host.json"), "keep this user edit").unwrap();
    assert!(
        create_config_directory(root.path(), Some(&workspace))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("host.json")).unwrap(),
        "keep this user edit"
    );
}

#[test]
fn host_framework_omission_empty_and_null_remain_distinct() {
    let original = HostConfig::default();
    let mut wire = serde_json::to_value(&original).unwrap();
    assert_eq!(wire["framework_instructions"], "");
    assert_eq!(
        serde_json::from_value::<HostConfig>(wire.clone()).unwrap(),
        original
    );
    wire.as_object_mut()
        .unwrap()
        .remove("framework_instructions");
    assert_eq!(
        serde_json::from_value::<HostConfig>(wire.clone()).unwrap(),
        original
    );
    wire["framework_instructions"] = Value::Null;
    assert!(serde_json::from_value::<HostConfig>(wire).is_err());
}
