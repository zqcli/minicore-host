//! Synthetic environment in a subprocess, with real Host resources, Runtime,
//! bash and loopback HTTP/SSE. No ambient credential values are read.

use minicore_host_storage::testing::read_history_bytes;
#[allow(dead_code)]
#[path = "support/production.rs"]
mod support;

use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use minicore_host_runtime::HistoryItem;
use serde_json::{Value, json};
use std::{process::Command, sync::Arc, time::Duration};
use support::*;
use tokio::sync::Semaphore;

const NAMES: [&str; 5] = [
    "TASK066_OLD_MODEL",
    "TASK066_OLD_NETWORK",
    "TASK066_NEW_MODEL",
    "TASK066_NEW_NETWORK",
    "TASK066_REMOVED_MODEL",
];

#[test]
fn credential_exclusions_follow_publication_without_rebinding_old_turns() {
    const CHILD: &str = "TASK066_RESOURCE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(publication_scenario());
        return;
    }
    let isolated_home = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "credential_exclusions_follow_publication_without_rebinding_old_turns",
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", isolated_home.path())
        .env("TMPDIR", isolated_home.path())
        .env("TEMP", isolated_home.path())
        .env("TMP", isolated_home.path())
        .env("LANG", "C.UTF-8")
        .env(CHILD, "1")
        .env("TASK066_ORDINARY", "ordinary bytes retained");
    #[cfg(unix)]
    child.env("PATH", "/bin:/usr/bin");
    #[cfg(windows)]
    for name in ["PATH", "SystemRoot", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(value) = std::env::var_os(name) {
            child.env(name, value);
        }
    }
    for name in NAMES {
        child.env(name, format!("synthetic-{name}-value"));
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "isolated resource regression failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn probe() -> Value {
    json!({"command": format!(
        "for name in {}; do if printenv \"$name\" > /dev/null; then printf '%s=present\\n' \"$name\"; else printf '%s=absent\\n' \"$name\"; fi; done; printf 'ordinary=%s\\n' \"$TASK066_ORDINARY\"",
        NAMES.join(" ")
    )})
}

fn output(history: &[HistoryItem], id: &str) -> String {
    let result = history
        .iter()
        .find_map(|item| match item {
            HistoryItem::ToolResult(result) if result.call_id.as_str() == id => Some(result),
            _ => None,
        })
        .expect("real native ToolResult");
    let result: Value = serde_json::from_str(result.output.content().as_str()).unwrap();
    assert_eq!(result["cleanup_confirmed"], true);
    result["stdout"]["output"]["data"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn expected(new_names_published: bool) -> String {
    let mut output = String::new();
    for (index, name) in NAMES.iter().enumerate() {
        let state = if index < 2 || index == 4 || new_names_published {
            "absent"
        } else {
            "present"
        };
        output.push_str(&format!("{name}={state}\n"));
    }
    output.push_str("ordinary=ordinary bytes retained\n");
    output
}

async fn publication_scenario() {
    let gate = Arc::new(Semaphore::new(0));
    let mut server = Server::start(vec![
        (tool_answer("before-reload", "bash", probe()), None),
        (
            tool_answer("after-reload", "bash", probe()),
            Some(gate.clone()),
        ),
        (answer("old Turn finished"), None),
        (tool_answer("next-turn", "bash", probe()), None),
        (answer("new Turn finished"), None),
        (tool_answer("after-removal", "bash", probe()), None),
        (answer("removed sources remain excluded"), None),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut original = config(&server.endpoint);
    original.models[0].api_key_env = NAMES[0].into();
    original.network.access_key_env = Some(NAMES[1].into());
    let mut unused = original.models[0].clone();
    unused.model = "custom/provider/unused".parse().unwrap();
    unused.api_key_env = NAMES[4].into();
    original.models.push(unused);
    let host = Host::open_configured(data.path(), original.clone(), |value| async {
        builder.prepare(value).await
    })
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let network = host
        .connect_network("synthetic-TASK066_OLD_NETWORK-value")
        .unwrap();
    let mut create_request = request(selection(&["bash"]), workspace(root.path()));
    create_request.mode = StorageMode::Persistent;
    let created = host
        .create_agent_async(&caller, create_request)
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    send(
        &host,
        &caller,
        &target,
        "probe before and after publication",
    );
    assert_eq!(server.request().await["model"], "provider-A");
    // The second model request proves the first bash completed and that this
    // old Turn is bound. Its next bash cannot start until the explicit gate.
    let second_request = server.request().await;
    assert_eq!(second_request["model"], "provider-A");
    assert_eq!(
        output(&host.history(&caller, &target).unwrap(), "before-reload"),
        expected(false)
    );
    let old_turn = host.turns(&caller, &target).unwrap().remove(0);
    assert_ne!(old_turn.state, TurnState::Finished);
    let mut next = original.clone();
    next.models[0].api_key_env = NAMES[2].into();
    next.models[0].provider_model = "provider-B".into();
    next.network.access_key_env = Some(NAMES[3].into());
    next.framework_instructions = "FRAMEWORK B".into();
    let prepared = builder.prepare(next.clone()).await.unwrap();
    assert_eq!(
        host.reload_configuration(&caller, 0, prepared)
            .await
            .unwrap(),
        HostReloadResult::Applied { config_revision: 1 }
    );
    assert!(host.info(&network.caller()).is_err());
    assert!(
        host.connect_network("synthetic-TASK066_NEW_NETWORK-value")
            .is_ok()
    );
    gate.add_permits(1);
    let third_request = server.request().await;
    assert_eq!(third_request["model"], "provider-A");
    assert!(third_request.to_string().contains("FRAMEWORK A"));
    assert!(!third_request.to_string().contains("FRAMEWORK B"));
    let finished = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(finished.id, old_turn.id);
    assert_eq!(finished.outcome, Some(TurnOutcome::Completed));
    assert_eq!(finished.persistence, PersistenceState::Persisted);
    assert_eq!(
        output(&host.history(&caller, &target).unwrap(), "after-reload"),
        expected(true)
    );
    send(
        &host,
        &caller,
        &target,
        "probe after removal of old source names",
    );
    let new_request = server.request().await;
    assert_eq!(new_request["model"], "provider-B");
    assert!(new_request.to_string().contains("FRAMEWORK B"));
    server.request().await;
    let finished = settled(&host, &caller, &target, 2).await;
    assert!(
        finished
            .iter()
            .all(|turn| turn.outcome == Some(TurnOutcome::Completed))
    );
    assert_eq!(
        output(&host.history(&caller, &target).unwrap(), "next-turn"),
        expected(true)
    );
    next.models.pop();
    next.network.access_key_env = None;
    assert_eq!(
        host.reload_configuration(&caller, 1, builder.prepare(next).await.unwrap())
            .await
            .unwrap(),
        HostReloadResult::Applied { config_revision: 2 }
    );
    assert!(
        host.connect_network("synthetic-TASK066_NEW_NETWORK-value")
            .is_err()
    );
    send(
        &host,
        &caller,
        &target,
        "probe after model and network source removal",
    );
    server.request().await;
    server.request().await;
    let finished = settled(&host, &caller, &target, 3).await;
    assert!(finished.iter().all(|turn| {
        turn.outcome == Some(TurnOutcome::Completed)
            && turn.persistence == PersistenceState::Persisted
    }));
    let history = host.history(&caller, &target).unwrap();
    assert_eq!(output(&history, "after-removal"), expected(true));
    let saved_results: Vec<_> = history
        .iter()
        .filter_map(|item| match item {
            HistoryItem::ToolResult(result) => {
                Some((result.call_id.as_str().to_owned(), result.output.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(saved_results.len(), 4);
    let journal = data
        .path()
        .join("agents")
        .join(target.agent_id.to_string())
        .join("sessions")
        .join(target.session_id.to_string())
        .join("history.jsonl");
    let bytes = read_history_bytes(&journal).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("ordinary bytes retained"));
    for name in NAMES {
        assert!(!text.contains(&format!("synthetic-{name}-value")));
    }
    host.shutdown(
        &caller,
        HostShutdownRequest {
            operation_id: OperationId::new().unwrap(),
            strategy: HostShutdownStrategy::Graceful,
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while host.info(&caller).unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reopened = Host::open_configured(data.path(), original, |value| async {
        builder.prepare(value).await
    })
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    let saved = reopened
        .history_read(
            &connection.caller(),
            &target,
            &HistoryReadRequest {
                cursor: None,
                limit: 100,
            },
        )
        .await
        .unwrap();
    for (id, output) in saved_results {
        assert!(saved.data.items.iter().any(|item| matches!(
            item,
            HistoryItemData::ToolResult { tool_call_id, result, .. }
                if tool_call_id.0 == id && result.output == output
        )));
    }
    assert_eq!(read_history_bytes(journal).unwrap(), bytes);
}
