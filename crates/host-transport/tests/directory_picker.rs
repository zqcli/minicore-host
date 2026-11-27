//! Real configured Host owners and filesystem queries through all adapters.
mod common;
mod os_pipe;
mod wss_common;

use minicore_host_core::{Host, HostConfig, NetworkSettings, PreparedHostConfiguration};
use minicore_host_protocol::*;
use minicore_host_resources::{HostCredentials, HostResourceBuilder};
use minicore_host_transport::HostClient;
use serde_json::json;
use std::{path::Path, sync::Arc};
use wss_common::{Clients, FIRST};

fn options() -> CallOptions {
    CallOptions {
        wait_timeout_ms: Some(5_000),
    }
}
async fn host(root: &Path) -> Arc<Host> {
    let candidate = HostResourceBuilder::builtins()
        .unwrap()
        .prepare_with_credentials(
            HostConfig {
                workspace_roots: vec![root.to_str().unwrap().into()],
                network: NetworkSettings {
                    listen: None,
                    access_key_env: Some("DIRECTORY_PICKER_TEST_KEY".into()),
                },
                ..Default::default()
            },
            HostCredentials {
                model_keys: Default::default(),
                network_access_key: Some(FIRST.into()),
            },
        )
        .await
        .unwrap();
    Host::memory_configured(candidate).unwrap()
}
fn mkdir(parent: &str, name: &str) -> DirectoryMkdirRequest {
    DirectoryMkdirRequest {
        operation_id: OperationId::new().unwrap(),
        parent: parent.into(),
        name: name.into(),
    }
}
fn list(path: &str) -> DirectoryListRequest {
    DirectoryListRequest {
        path: path.into(),
        hidden: false,
        include_files: false,
        workspace_session: None,
        cursor: None,
        limit: 1,
    }
}

#[tokio::test]
async fn shared_typed_directory_operations_return_real_created_and_failed_receipts() {
    let root = tempfile::tempdir().unwrap();
    let host = host(root.path()).await;
    let operator = HostClient::in_process(host.clone()).unwrap();
    let observer = HostClient::in_process(host).unwrap();
    let roots = observer.workspace_roots(options()).await.unwrap();
    let path = &roots.roots[0].path;
    let receipt = operator
        .workspace_mkdir(mkdir(path, "new"), options())
        .await
        .unwrap();
    assert!(
        matches!(&receipt.result, DirectoryMkdirResult::Created { path } if Path::new(path).is_dir())
    );
    assert_eq!(
        observer
            .receipt(receipt.operation_id.clone(), options())
            .await
            .unwrap(),
        Receipt::WorkspaceMkdir(receipt)
    );
    let failed = operator
        .workspace_mkdir(mkdir(path, "new"), options())
        .await
        .unwrap();
    assert!(
        matches!(failed.result, DirectoryMkdirResult::Failed { error } if error.kind() == Some(FaultKind::HostWorkspaceAlreadyExists))
    );
    assert!(
        matches!(observer.workspace_mkdir(mkdir(path, "denied"), options()).await, Err(CallError::Business(error)) if error.kind() == Some(FaultKind::HostAccessReadOnly))
    );
    assert_eq!(
        observer
            .workspace_list(list(path), options())
            .await
            .unwrap()
            .entries[0]
            .name,
        "new"
    );
    assert!(!root.path().join("denied").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wss_directory_picker_authenticates_and_all_transports_share_scoped_pagination() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let host = host(root.path()).await;
    let server = wss_common::tls_server(host.clone()).await;
    let mut unauthorized = server.raw().await;
    let response = unauthorized.request(1, "workspace.roots", json!({})).await;
    assert!(response.get("error").is_some(), "{response}");
    assert!(response.get("result").is_none());
    let mut unauthorized_write = server.raw().await;
    let response = unauthorized_write
        .request(
            2,
            "workspace.mkdir",
            serde_json::to_value(mkdir(root.path().to_str().unwrap(), "unauthenticated")).unwrap(),
        )
        .await;
    assert!(response.get("error").is_some(), "{response}");
    assert!(!root.path().join("unauthenticated").exists());
    assert!(server.client("wrong-fixture-key").await.is_err());
    let operator = server.client(FIRST).await.unwrap();
    assert!(operator.initial_info().can_operate);
    let roots = operator.workspace_roots(options()).await.unwrap();
    let path = &roots.roots[0].path;
    for name in ["a", "b"] {
        let receipt = operator
            .workspace_mkdir(mkdir(path, name), options())
            .await
            .unwrap();
        assert!(matches!(
            receipt.result,
            DirectoryMkdirResult::Created { .. }
        ));
        assert!(root.path().join(name).is_dir());
    }
    let peers = Clients::new(host, FIRST, options()).await;
    for client in &peers.clients {
        assert!(!client.initial_info().can_operate);
        assert_eq!(client.workspace_roots(options()).await.unwrap(), roots);
        let first = client.workspace_list(list(path), options()).await.unwrap();
        assert_eq!(first.entries[0].name, "a");
        let second = client
            .workspace_list(
                DirectoryListRequest {
                    cursor: first.next_cursor,
                    ..list(path)
                },
                options(),
            )
            .await
            .unwrap();
        assert_eq!(second.entries[0].name, "b");
        assert!(second.next_cursor.is_none());
        assert!(
            matches!(client.workspace_mkdir(mkdir(path, "denied"), options()).await, Err(CallError::Business(error)) if error.kind() == Some(FaultKind::HostAccessReadOnly))
        );
        assert!(
            matches!(client.workspace_list(list(outside.path().to_str().unwrap()), options()).await, Err(CallError::Business(error)) if error.kind() == Some(FaultKind::HostWorkspaceOutsideRoots))
        );
    }
    peers.finish().await;
    operator.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_picker_uses_session_workspace_and_configured_roots_across_transports() {
    let workspace = tempfile::tempdir().unwrap();
    let extra = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let folder = workspace.path().join("folder");
    let file = workspace.path().join("screen shot.png");
    let extra_file = extra.path().join("extra.txt");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(&file, [0xff, 0x00]).unwrap();
    std::fs::write(&extra_file, "reference only").unwrap();

    for allow_extra in [false, true] {
        // Only resource validation is a fixture; no model request is made.
        let (_, resources) = common::fixture();
        let host = Host::memory_configured(
            PreparedHostConfiguration::new(
                HostConfig {
                    workspace_roots: if allow_extra {
                        vec![extra.path().to_str().unwrap().into()]
                    } else {
                        vec![]
                    },
                    network: NetworkSettings {
                        listen: None,
                        access_key_env: Some("DIRECTORY_PICKER_TEST_KEY".into()),
                    },
                    ..Default::default()
                },
                resources.clone(),
                vec![],
                Some(FIRST.into()),
            )
            .unwrap(),
        )
        .unwrap();
        let operator = HostClient::in_process(host.clone()).unwrap();
        let mut create = common::create_request();
        create.config.session_defaults.workspace = Some(WorkspaceRef {
            path: workspace.path().to_str().unwrap().into(),
        });
        let created = operator.create_agent(create, options()).await.unwrap();
        let target = SessionRef {
            agent_id: created.agent_id.unwrap(),
            session_id: created.session_id.unwrap(),
        };
        let browse = |path: &Path| DirectoryListRequest {
            include_files: true,
            workspace_session: Some(target.clone()),
            ..list(path.to_str().unwrap())
        };
        let peers = Clients::new(host, FIRST, options()).await;
        for client in &peers.clients {
            let roots = client.workspace_roots(options()).await.unwrap();
            assert_eq!(roots.roots.len(), usize::from(allow_extra));
            let first = client
                .workspace_list(browse(workspace.path()), options())
                .await
                .unwrap();
            assert!(!first.is_file);
            assert!(first.parent.is_none());
            assert_eq!(first.entries[0].name, "folder");
            assert!(!first.entries[0].is_file);
            let second = client
                .workspace_list(
                    DirectoryListRequest {
                        cursor: first.next_cursor,
                        ..browse(workspace.path())
                    },
                    options(),
                )
                .await
                .unwrap();
            assert_eq!(second.entries[0].name, "screen shot.png");
            assert!(second.entries[0].is_file);
            assert!(second.next_cursor.is_none());
            let child = client
                .workspace_list(browse(&folder), options())
                .await
                .unwrap();
            assert_eq!(child.parent, Some(first.path));
            let selected = client
                .workspace_list(browse(&file), options())
                .await
                .unwrap();
            assert!(selected.is_file);
            assert_eq!(selected.path, second.entries[0].path);
            assert!(selected.entries.is_empty());
            assert!(selected.parent.is_none());
            assert!(selected.next_cursor.is_none());
            // Session scope is read-local, opt-in and not added to the Host's roots.
            let default = DirectoryListRequest {
                include_files: false,
                ..browse(workspace.path())
            };
            for denied in [default, browse(outside.path())] {
                assert!(matches!(
                    client.workspace_list(denied, options()).await,
                    Err(CallError::Business(fault)) if fault.kind() == Some(FaultKind::HostWorkspaceOutsideRoots)
                ));
            }
            let additional = client.workspace_list(browse(&extra_file), options()).await;
            if allow_extra {
                assert!(additional.unwrap().is_file);
            } else {
                assert!(matches!(
                    additional,
                    Err(CallError::Business(fault)) if fault.kind() == Some(FaultKind::HostWorkspaceOutsideRoots)
                ));
            }
            assert_eq!(client.workspace_roots(options()).await.unwrap(), roots);
        }
        assert_eq!(
            resources
                .model
                .calls
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        peers.finish().await;
        operator.close();
    }
}
