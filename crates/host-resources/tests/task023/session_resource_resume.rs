//! Production resource revalidation and real filesystem binding; HTTP/SSE
//! responses remain the parent module's explicitly local provider fixture.
use super::*;

fn control() -> SessionControlRequest {
    SessionControlRequest {
        operation_id: OperationId::new().unwrap(),
    }
}

async fn setup(
    mode: StorageMode,
    builder: &HostResourceBuilder,
    initial: &HostConfig,
    directory: &std::path::Path,
    workspace: Option<WorkspaceRef>,
) -> (Arc<Host>, ConnectionHandle, SessionRef) {
    let host = if mode == StorageMode::Persistent {
        Host::open_configured(directory, initial.clone(), |config| async {
            Ok(prepare(builder, config, "old-key").await)
        })
        .await
        .unwrap()
    } else {
        Host::memory_configured(prepare(builder, initial.clone(), "old-key").await).unwrap()
    };
    let connection = host.connect().unwrap();
    let mut creation = request(vec![], workspace);
    creation.mode = mode;
    let created = host
        .create_agent_async(&connection.caller(), creation)
        .await
        .unwrap();
    let target = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    (host, connection, target)
}

async fn restored_model_resumes(mode: StorageMode) {
    let mut server = Server::start(vec![(answer("restored"), None)]).await;
    let directory = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config(&server.endpoint);
    let (host, connection, target) = setup(mode, &builder, &initial, directory.path(), None).await;
    let caller = connection.caller();
    let mut removed = initial.clone();
    removed.models.clear();
    host.reload_configuration(&caller, 0, prepare(&builder, removed, "old-key").await)
        .await
        .unwrap();
    let id = send(&host, &caller, &target, "accepted once");
    let state = blocked(&host, &caller, &target).await;
    assert_eq!(
        state.data.block_reason.unwrap().fault.unwrap().kind(),
        Some(FaultKind::SessionResourceMissing)
    );
    assert_eq!(state.data.next_turn[0].message_id, id);
    host.reload_configuration(&caller, 1, prepare(&builder, initial, "old-key").await)
        .await
        .unwrap();
    // Host publication itself does not mutate or restart this Session.
    assert!(
        host.session_state(&caller, &target)
            .unwrap()
            .data
            .block_reason
            .is_some()
    );
    // Also cover the persistent explicit history-load route after a failed
    // preparation. It must revalidate history before consuming the same queue.
    if mode == StorageMode::Persistent {
        let mut states = host.session_states(&caller, &target).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while states.snapshot().unwrap().data.metadata_saving {
                states.changed().await.unwrap();
            }
        })
        .await
        .expect("accepted input activity metadata save completed");
        host.session_unload(&caller, &target, control()).unwrap();
    }
    host.session_resume(&caller, &target, control()).unwrap();
    let request = server.request().await;
    settled(&host, &caller, &target, 1).await;
    assert_eq!(
        host.session_info(&caller, &target).unwrap().config_revision,
        0
    );
    assert_single_input(&request, &id, "accepted once");
    assert!(matches!(
        host.message(&caller, &target, &id).unwrap().binding,
        MessageBinding::Bound { .. }
    ));
}

async fn corrected_workspace_continues(mode: StorageMode) {
    let mut server = Server::start(vec![(answer("rebound"), None)]).await;
    let directory = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let initial = config(&server.endpoint);
    let (host, connection, target) = setup(
        mode,
        &builder,
        &initial,
        &directory.path().join("host"),
        Some(WorkspaceRef {
            path: directory.path().join("missing").to_str().unwrap().into(),
        }),
    )
    .await;
    let caller = connection.caller();
    let id = send(&host, &caller, &target, "keep workspace input");
    let state = blocked(&host, &caller, &target).await;
    assert_eq!(
        state.data.block_reason.unwrap().fault.unwrap().kind(),
        Some(FaultKind::SessionConfigInvalid)
    );
    assert_eq!(state.data.next_turn[0].message_id, id);
    assert!(host.turns(&caller, &target).unwrap().is_empty());
    // Input acceptance owns an activity checkpoint independently of preparation.
    let mut states = host.session_states(&caller, &target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while states.snapshot().unwrap().data.metadata_saving {
            states.changed().await.unwrap();
        }
    })
    .await
    .expect("accepted input activity metadata save completed");
    host.update_session_async(
        &caller,
        &target,
        0,
        &SessionConfigPatch {
            workspace: NullablePatch::Set(WorkspaceRef {
                path: directory.path().to_str().unwrap().into(),
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let request = server.request().await;
    settled(&host, &caller, &target, 1).await;
    assert_eq!(
        host.message(&caller, &target, &id)
            .unwrap()
            .input
            .message_id,
        id
    );
    assert_single_input(&request, &id, "keep workspace input");
    let workspace_context = format!(
        "Workspace directory: {}",
        std::fs::canonicalize(directory.path()).unwrap().display()
    );
    assert!(
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| { m["role"] == "system" && m["content"] == workspace_context })
    );
}

#[tokio::test]
async fn restored_model_resumes_memory() {
    restored_model_resumes(StorageMode::Memory).await;
}
#[tokio::test]
async fn restored_model_resumes_persistent() {
    restored_model_resumes(StorageMode::Persistent).await;
}
#[tokio::test]
async fn corrected_workspace_continues_memory() {
    corrected_workspace_continues(StorageMode::Memory).await;
}
#[tokio::test]
async fn corrected_workspace_continues_persistent() {
    corrected_workspace_continues(StorageMode::Persistent).await;
}
