//! Query waits have no authority over the separately owned real Runtime Turn.
#![cfg(unix)]
mod compaction_common;
mod workspace_common;
mod wss_common;

use compaction_common::{input, settled};
use minicore_host_protocol::*;
use minicore_host_transport::HostClient;
use minicore_host_workspace::*;
use std::{
    fs,
    sync::{Arc, mpsc},
    time::Instant,
};
use tokio::sync::Semaphore;
use workspace_common::*;

struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

#[test]
fn query_timeout_disconnect_and_context_cancellation_leave_real_turn_running() {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let finish = Arc::new(Semaphore::new(0));
            let mut http = HttpFixture::start(vec![compaction_common::gated(
                "actual completed Turn",
                &finish,
            )])
            .await;
            let host = host(&http, vec![]).await;
            let operator = host.connect().unwrap();
            let caller = operator.caller();
            let old = tempfile::tempdir().unwrap();
            let new = tempfile::tempdir().unwrap();
            fs::write(old.path().join("file"), "original query source").unwrap();
            fs::write(new.path().join("file"), "new committed source").unwrap();
            let target = create(&host, &caller, Some(old.path()), vec![]).await;
            host.send(
                &caller,
                SendMessage::new(
                    input("continue until released", MessageMode::FollowUp),
                    target.clone(),
                ),
            )
            .unwrap();
            http.request().await;
            let observer = HostClient::in_process(host.clone())
                .unwrap()
                .session(target.clone());
            let turn = observer
                .state(options())
                .await
                .unwrap()
                .data
                .active_turn_id
                .unwrap();
            let peers = Clients::new(host.clone()).await;

            let cancelled = QueryContext::default();
            cancelled.cancellation.cancel();
            assert_eq!(
                host.workspace_read(&caller, &target, read("file"), cancelled)
                    .await
                    .unwrap_err()
                    .kind(),
                Some(FaultKind::SessionWorkspaceQueryCancelled)
            );
            let deadline = QueryContext {
                deadline: Some(Instant::now()),
                ..Default::default()
            };
            assert_eq!(
                host.workspace_status(&caller, &target, deadline)
                    .await
                    .unwrap_err()
                    .kind(),
                Some(FaultKind::SessionWorkspaceQueryDeadline)
            );

            // Occupying the sole blocking worker makes binding wait deterministically
            // without a fake backend, a slow external filter, or large-file timing.
            let (release, wait) = mpsc::channel();
            let release = Release(Some(release));
            let (started, ready) = tokio::sync::oneshot::channel();
            let blocking = tokio::task::spawn_blocking(move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
            });
            ready.await.unwrap();
            let mut captured = Box::pin(host.workspace_read(
                &caller,
                &target,
                read("file"),
                QueryContext::default(),
            ));
            assert!(futures_util::poll!(captured.as_mut()).is_pending());
            // This synchronous owner write completes while binding is blocked: no
            // Host/Agent/Session/authority lock is held across the query's I/O.
            host.update_session(
                &caller,
                &target,
                0,
                &SessionConfigPatch {
                    workspace: NullablePatch::Set(WorkspaceRef {
                        path: new.path().to_string_lossy().into_owned(),
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
            for client in &peers.clients {
                let error = client
                    .session(target.clone())
                    .workspace_read(
                        read("file"),
                        CallOptions {
                            wait_timeout_ms: Some(10),
                        },
                    )
                    .await
                    .unwrap_err();
                assert!(matches!(error, CallError::WaitTimeout { .. }));
                assert_eq!(
                    client.info(options()).await.unwrap().host_id,
                    operator.initial_info().host_id
                );
                assert_eq!(
                    client
                        .session(target.clone())
                        .state(options())
                        .await
                        .unwrap()
                        .data
                        .active_turn_id
                        .as_ref(),
                    Some(&turn)
                );
            }
            for client in &peers.clients {
                client.close();
            }
            drop(release);
            blocking.await.unwrap();
            let actual = captured.await.unwrap();
            assert_eq!(actual.config_revision, Some(0));
            assert_eq!(actual.data.text, "original query source");
            assert_eq!(
                observer
                    .workspace_read(read("file"), options())
                    .await
                    .unwrap()
                    .data
                    .text,
                "new committed source"
            );
            assert_eq!(
                observer.state(options()).await.unwrap().data.active_turn_id,
                Some(turn)
            );
            finish.add_permits(1);
            settled(&observer, 1).await;
            peers.finish_disconnected().await;
        });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_binding_and_invalid_edit_continuations_are_faults_not_empty_pages() {
    let http = HttpFixture::start(vec![]).await;
    let host = host(&http, vec![]).await;
    let operator = host.connect().unwrap();
    let root = tempfile::tempdir().unwrap();
    let target = create(&host, &operator.caller(), Some(root.path()), vec![]).await;
    let peers = Clients::new(host.clone()).await;
    let missing_root = root.path().join("removed");
    fs::create_dir(&missing_root).unwrap();
    host.update_session(
        &operator.caller(),
        &target,
        0,
        &SessionConfigPatch {
            workspace: NullablePatch::Set(WorkspaceRef {
                path: missing_root.to_string_lossy().into_owned(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    fs::remove_dir(&missing_root).unwrap();
    for client in &peers.clients {
        let session = client.session(target.clone());
        assert_eq!(
            fault(
                session
                    .workspace_files(files(), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspacePathNotFound)
        );
        let mut empty = list(ChangeScope::Session);
        empty.limit = 0;
        assert_eq!(
            fault(session.changes_list(empty, options()).await.unwrap_err()).kind(),
            Some(FaultKind::SessionWorkspaceRangeInvalid)
        );
        let mut invalid = list(ChangeScope::Session);
        invalid.cursor = Some(ChangesCursor::Tool {
            session_id: target.session_id.clone(),
            turn_id: None,
            after_index: 0,
        });
        assert_eq!(
            fault(session.changes_list(invalid, options()).await.unwrap_err()).kind(),
            Some(FaultKind::SessionWorkspaceCursorInvalid)
        );
        // An empty retained journal is truly empty and does not need the now
        // missing current workspace to manufacture historic observations.
        assert!(
            session
                .changes_list(list(ChangeScope::Session), options())
                .await
                .unwrap()
                .data
                .records
                .is_empty()
        );
    }
    peers.finish().await;
}
