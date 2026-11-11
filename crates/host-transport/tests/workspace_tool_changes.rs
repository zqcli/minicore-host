//! Original tool observations, never Git heuristics or intended-write fixtures.
mod compaction_common;
mod os_pipe;
mod workspace_common;
mod wss_common;

use compaction_common::{input, reply, settled};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_provider::SecretApiKey;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_transport::HostClient;
use minicore_host_workspace::*;
use serde_json::json;
use std::{fs, sync::Arc, time::Duration};
use tokio::sync::Semaphore;
use workspace_common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_edits_remain_bound_to_original_tool_while_workspace_config_changes() {
    let gate = Arc::new(Semaphore::new(0));
    let mut http = HttpFixture::start(vec![
        tool_reply(
            "original-write",
            "write",
            json!({"path":"file","content":"é中 first\n"}),
        ),
        tool_reply(
            "original-edit",
            "edit",
            json!({"path":"file","old_text":"first","new_text":"second"}),
        ),
        compaction_common::gated("finished original Turn", &gate),
    ])
    .await;
    let host = host(&http, selection(&["write", "edit"])).await;
    let operator = host.connect().unwrap();
    let original = tempfile::tempdir().unwrap();
    repository(original.path());
    let current = tempfile::tempdir().unwrap();
    repository(current.path());
    fs::write(current.path().join("file"), "current workspace only\n").unwrap();
    let mut creation = request(Some(original.path()), selection(&["write", "edit"]));
    // This fixture verifies actual edit ownership, not interactive approval.
    creation.config.session_defaults.execution.approval = ApprovalMode::Auto;
    let receipt = host
        .create_agent_async(&operator.caller(), creation)
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    let other = create(&host, &operator.caller(), Some(current.path()), vec![]).await;
    let observer = HostClient::in_process(host.clone())
        .unwrap()
        .session(target.clone());
    host.send(
        &operator.caller(),
        SendMessage::new(
            input("make actual edits", MessageMode::FollowUp),
            target.clone(),
        ),
    )
    .unwrap();
    for _ in 0..3 {
        http.request().await;
    }
    let turn = observer
        .state(options())
        .await
        .unwrap()
        .data
        .active_turn_id
        .unwrap();
    assert_eq!(
        fs::read_to_string(original.path().join("file")).unwrap(),
        "é中 second\n"
    );
    fs::write(original.path().join("unrelated"), "external edit\n").unwrap();
    let updated = host
        .update_session_async(
            &operator.caller(),
            &target,
            0,
            &SessionConfigPatch {
                workspace: NullablePatch::Set(WorkspaceRef {
                    path: current.path().to_string_lossy().into_owned(),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.config_revision, 1);
    let peers = Clients::new(host.clone(), compaction_common::KEY, options()).await;
    for client in &peers.clients {
        let session = client.session(target.clone());
        let content = session
            .workspace_read(read("file"), options())
            .await
            .unwrap();
        assert_eq!(content.config_revision, Some(1));
        assert_eq!(content.data.text, "current workspace only\n");
        let git = session
            .changes_list(list(ChangeScope::Workspace), options())
            .await
            .unwrap();
        assert_eq!(git.config_revision, Some(1));
        assert_eq!(git.data.records.len(), 1);
        assert_eq!(git.data.records[0].origin, ChangeOrigin::WorkspaceUnknown);
        let mut request = list(ChangeScope::Turn {
            turn_id: turn.clone(),
        });
        request.limit = 1;
        let first = session
            .changes_list(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(first.config_revision, None);
        assert_eq!(first.data.records.len(), 1);
        assert_eq!(
            first.data.records[0].edit_outcome,
            Some(EditOutcome::Applied)
        );
        assert_eq!(first.data.records[0].origin, ChangeOrigin::Tool);
        request.cursor = first.data.next_cursor;
        let second = session.changes_list(request, options()).await.unwrap();
        assert_eq!(second.data.records.len(), 1);
        assert!(second.data.next_cursor.is_none());
        assert_ne!(
            first.data.records[0].change_ref,
            second.data.records[0].change_ref
        );
        let request = diff(
            second.data.records[0].change_ref.clone(),
            DiffComparison::ToolBeforeAfter,
        );
        let actual = session
            .changes_diff(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(actual.config_revision, None);
        assert_eq!(actual.source.session_id, target.session_id);
        let lines = actual
            .data
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .collect::<Vec<_>>();
        assert!(
            lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Removed && line.text == "é中 first\n")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.kind == DiffLineKind::Added && line.text == "é中 second\n")
        );
        assert!(!actual.data.stale);
        assert!(actual.data.complete);
        assert_eq!(
            fault(
                client
                    .session(other.clone())
                    .changes_diff(request, options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionChangeNotFound)
        );
        assert_eq!(
            session
                .changes_list(list(ChangeScope::Session), options())
                .await
                .unwrap()
                .data
                .records
                .len(),
            2
        );
        assert_eq!(
            fault(
                session
                    .changes_list(
                        list(ChangeScope::Turn {
                            turn_id: TurnId::new().unwrap()
                        }),
                        options()
                    )
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionTurnNotFound)
        );
    }
    gate.add_permits(1);
    settled(&observer, 1).await;
    host.unload(&operator.caller(), &target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while observer.state(options()).await.unwrap().data.residency != Residency::Unloaded {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        observer
            .changes_list(list(ChangeScope::Session), options())
            .await
            .unwrap()
            .data
            .records
            .len(),
        2
    );
    host.session_close(
        &operator.caller(),
        &target,
        SessionCloseRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match observer
                .changes_list(list(ChangeScope::Session), options())
                .await
            {
                Err(error) => {
                    assert!(matches!(
                        fault(error).kind(),
                        Some(FaultKind::AgentSessionNotFound | FaultKind::HostAgentNotFound)
                    ));
                    break;
                }
                Ok(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .unwrap();
    peers.finish().await;
}

async fn persistent(data: &std::path::Path, http: &HttpFixture) -> Arc<Host> {
    let prepared = candidate(http, selection(&["write"])).await;
    let config = prepared.config().clone();
    Host::open_configured(data, config, |config| async {
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
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reopened_sessions_expose_missing_edit_coverage_but_current_turn_edits_are_queryable() {
    let mut http = HttpFixture::start(vec![
        tool_reply(
            "old-write",
            "write",
            json!({"path":"file", "content":"before restart"}),
        ),
        reply("first done"),
        tool_reply(
            "new-write",
            "write",
            json!({"path":"file", "content":"after restart"}),
        ),
        reply("second done"),
    ])
    .await;
    let data = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let host = persistent(data.path(), &http).await;
    let first = HostClient::in_process(host.clone()).unwrap();
    let mut create = request(Some(root.path()), selection(&["write"]));
    // Preserve unattended execution across the persistent edit-source reopen.
    create.config.session_defaults.execution.approval = ApprovalMode::Auto;
    create.mode = StorageMode::Persistent;
    let receipt = first.create_agent(create, options()).await.unwrap();
    let target = SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    };
    let session = first.session(target.clone());
    session
        .send(input("first incarnation", MessageMode::FollowUp), options())
        .await
        .unwrap();
    http.request().await;
    http.request().await;
    settled(&session, 1).await;
    let old_turn = session.turns(options()).await.unwrap().data[0]
        .turn_id
        .clone();
    let old_ref = session
        .changes_list(list(ChangeScope::Session), options())
        .await
        .unwrap()
        .data
        .records[0]
        .change_ref
        .clone();
    first
        .shutdown(
            HostShutdownRequest {
                operation_id: OperationId::new().unwrap(),
                strategy: HostShutdownStrategy::Graceful,
            },
            options(),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while first.info(options()).await.unwrap().lifecycle != HostLifecycle::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reopened = persistent(data.path(), &http).await;
    let operator = HostClient::in_process(reopened.clone()).unwrap();
    assert_eq!(
        first.initial_info().host_id,
        operator.initial_info().host_id
    );
    assert_ne!(
        first.initial_info().host_incarnation,
        operator.initial_info().host_incarnation
    );
    let peers = Clients::new(reopened, compaction_common::KEY, options()).await;
    for client in &peers.clients {
        let session = client.session(target.clone());
        assert_eq!(
            fault(
                session
                    .changes_list(list(ChangeScope::Session), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionChangeSourceUnavailable)
        );
        assert_eq!(
            fault(
                session
                    .changes_list(
                        list(ChangeScope::Turn {
                            turn_id: old_turn.clone()
                        }),
                        options()
                    )
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionTurnNotFound)
        );
        assert_eq!(
            fault(
                session
                    .changes_diff(
                        diff(old_ref.clone(), DiffComparison::ToolBeforeAfter),
                        options()
                    )
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionChangeNotFound)
        );
        assert_eq!(
            session
                .workspace_read(read("file"), options())
                .await
                .unwrap()
                .data
                .text,
            "before restart"
        );
    }
    let current = operator.session(target.clone());
    current
        .send(input("new incarnation", MessageMode::FollowUp), options())
        .await
        .unwrap();
    http.request().await;
    http.request().await;
    settled(&current, 1).await;
    let turn = current.turns(options()).await.unwrap().data[0]
        .turn_id
        .clone();
    for client in &peers.clients {
        let session = client.session(target.clone());
        assert_eq!(
            fault(
                session
                    .changes_list(list(ChangeScope::Session), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionChangeSourceUnavailable)
        );
        let records = session
            .changes_list(
                list(ChangeScope::Turn {
                    turn_id: turn.clone(),
                }),
                options(),
            )
            .await
            .unwrap()
            .data
            .records;
        assert_eq!(records.len(), 1);
        let actual = session
            .changes_diff(
                diff(
                    records[0].change_ref.clone(),
                    DiffComparison::ToolBeforeAfter,
                ),
                options(),
            )
            .await
            .unwrap();
        assert!(
            actual
                .data
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .any(|line| line.kind == DiffLineKind::Removed && line.text == "before restart")
        );
        assert!(
            actual
                .data
                .hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .any(|line| line.kind == DiffLineKind::Added && line.text == "after restart")
        );
    }
    peers.finish().await;
}
