//! Workspace-scoped references identify roots, not the Session's latest page.
#![cfg(unix)]
mod compaction_common;
mod workspace_common;
mod wss_common;

use minicore_host_protocol::*;
use minicore_host_workspace::*;
use std::fs;
use workspace_common::*;

fn added(result: &DiffResult) -> String {
    result
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind == DiffLineKind::Added)
        .map(|line| line.text.as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn workspace_change_refs_reject_other_or_reconfigured_roots_but_accept_same_root() {
    let http = HttpFixture::start(vec![]).await;
    let host = host(&http, vec![]).await;
    let operator = host.connect().unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    repository(a.path());
    repository(b.path());
    fs::write(a.path().join("file"), "origin A\n").unwrap();
    fs::write(b.path().join("file"), "unrelated B\n").unwrap();
    let target_a = create(&host, &operator.caller(), Some(a.path()), vec![]).await;
    let target_b = create(&host, &operator.caller(), Some(b.path()), vec![]).await;
    let same_root = create(&host, &operator.caller(), Some(a.path()), vec![]).await;
    let peers = Clients::new(host.clone()).await;
    let mut references = Vec::new();
    for client in &peers.clients {
        let listed = client
            .session(target_a.clone())
            .changes_list(list(ChangeScope::Workspace), options())
            .await
            .unwrap();
        let reference = listed.data.records[0].change_ref.clone();
        // Both the original Session and another actual Session of this same
        // root can read a workspace reference. It is not a Session capability.
        for target in [&target_a, &same_root] {
            let actual = client
                .session(target.clone())
                .changes_diff(
                    diff(reference.clone(), DiffComparison::HeadToWorktree),
                    options(),
                )
                .await
                .unwrap();
            assert_eq!(actual.source.session_id, target.session_id);
            assert_eq!(actual.data.change_ref, reference);
            assert_eq!(added(&actual.data), "origin A\n");
            assert!(!actual.data.stale);
        }
        assert_eq!(
            fault(
                client
                    .session(target_b.clone())
                    .changes_diff(
                        diff(reference.clone(), DiffComparison::HeadToWorktree),
                        options()
                    )
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionChangeNotFound)
        );
        references.push(reference);
    }
    host.update_session(
        &operator.caller(),
        &target_a,
        0,
        &SessionConfigPatch {
            workspace: NullablePatch::Set(WorkspaceRef {
                path: b.path().to_string_lossy().into_owned(),
            }),
            ..Default::default()
        },
    )
    .unwrap();
    for (client, reference) in peers.clients.iter().zip(&references) {
        let session = client.session(target_a.clone());
        assert_eq!(
            fault(
                session
                    .changes_diff(
                        diff(reference.clone(), DiffComparison::HeadToWorktree),
                        options()
                    )
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionChangeNotFound)
        );
        // The current root remains queryable with its own newly listed source.
        let current = session
            .changes_list(list(ChangeScope::Workspace), options())
            .await
            .unwrap();
        let actual = session
            .changes_diff(
                diff(
                    current.data.records[0].change_ref.clone(),
                    DiffComparison::HeadToWorktree,
                ),
                options(),
            )
            .await
            .unwrap();
        assert_eq!(actual.config_revision, Some(1));
        assert_eq!(added(&actual.data), "unrelated B\n");
        let retained_root = client
            .session(same_root.clone())
            .changes_diff(
                diff(reference.clone(), DiffComparison::HeadToWorktree),
                options(),
            )
            .await
            .unwrap();
        assert_eq!(added(&retained_root.data), "origin A\n");
    }
    peers.finish().await;
}
