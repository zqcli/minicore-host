//! Canonical query routing, real filesystem/Git sources, and all three adapters.
#![cfg(unix)]
mod compaction_common;
mod workspace_common;
mod wss_common;

use minicore_host_protocol::*;
use minicore_host_workspace::*;
use serde_json::json;
use std::fs;
use workspace_common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readonly_clients_read_the_explicit_session_and_utf8_pages_on_all_transports() {
    let mut http = HttpFixture::start(vec![]).await;
    let host = host(&http, vec![]).await;
    let operator = host.connect().unwrap();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    fs::write(first.path().join("a.txt"), "Aé中Z\nmatch é\nmatch 中\n").unwrap();
    fs::write(first.path().join("b.bin"), b"\0binary").unwrap();
    fs::write(second.path().join("different"), "selected second workspace").unwrap();
    let target = create(&host, &operator.caller(), Some(first.path()), vec![]).await;
    let other = create(&host, &operator.caller(), Some(second.path()), vec![]).await;
    let peers = Clients::new(host.clone()).await;
    for client in &peers.clients {
        assert!(!client.initial_info().can_operate);
        let session = client.session(target.clone());
        let mut request = files();
        request.limit = 1;
        let page = session
            .workspace_files(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(page.source.host_id, client.initial_info().host_id);
        assert_eq!(
            page.source.host_incarnation,
            client.initial_info().host_incarnation
        );
        assert_eq!(page.source.agent_id, target.agent_id);
        assert_eq!(page.source.session_id, target.session_id);
        assert_eq!(page.config_revision, Some(0));
        assert_eq!(page.data.entries[0].path, "a.txt");
        request.cursor = page.data.next_cursor;
        let next = session
            .workspace_files(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(next.data.entries[0].path, "b.bin");
        assert!(next.data.next_cursor.is_none());
        assert_eq!(
            fault(
                client
                    .session(other.clone())
                    .workspace_files(request, options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceCursorInvalid)
        );

        let mut request = read("a.txt");
        request.start = 1;
        request.end = Some(6);
        request.max_bytes = 3;
        let chunk = session
            .workspace_read(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(
            (
                chunk.data.start,
                chunk.data.end,
                chunk.data.text.as_str(),
                chunk.data.has_more
            ),
            (1, 3, "é", true)
        );
        request.start = chunk.data.end;
        let tail = session
            .workspace_read(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(
            (tail.data.end, tail.data.text.as_str(), tail.data.has_more),
            (6, "中", false)
        );
        request.start = 2;
        assert_eq!(
            fault(
                session
                    .workspace_read(request, options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceRangeInvalid)
        );
        assert_eq!(
            fault(
                session
                    .workspace_read(read("b.bin"), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceUnsupported)
        );
        let mut search = SearchRequest {
            path: "a.txt".into(),
            query: "match".into(),
            case_sensitive: true,
            max_matches: 1,
            cursor: None,
        };
        let result = session
            .workspace_search(search.clone(), options())
            .await
            .unwrap();
        assert_eq!(result.data.matches[0].line_number, 2);
        assert_eq!(
            result.data.matches[0].match_byte_ranges,
            [ByteRange { start: 0, end: 5 }]
        );
        search.cursor = result.data.next_cursor;
        assert_eq!(
            session
                .workspace_search(search, options())
                .await
                .unwrap()
                .data
                .matches[0]
                .line_number,
            3
        );
        assert!(
            !session
                .workspace_status(options())
                .await
                .unwrap()
                .data
                .repo_available
        );
        assert_eq!(
            client
                .session(other.clone())
                .workspace_read(read("different"), options())
                .await
                .unwrap()
                .data
                .text,
            "selected second workspace"
        );
    }
    http.no_request();
    peers.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_wrong_and_unsafe_targets_never_fall_back_to_an_active_session() {
    let http = HttpFixture::start(vec![]).await;
    let host = host(&http, vec![]).await;
    let operator = host.connect().unwrap();
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "not public").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    let target = create(&host, &operator.caller(), Some(root.path()), vec![]).await;
    let other = create(&host, &operator.caller(), None, vec![]).await;
    let foreign_host = workspace_common::host(&http, vec![]).await;
    let foreign_operator = foreign_host.connect().unwrap();
    let foreign = create(
        &foreign_host,
        &foreign_operator.caller(),
        Some(root.path()),
        vec![],
    )
    .await;
    let peers = Clients::new(host).await;
    for client in &peers.clients {
        let session = client.session(target.clone());
        for path in ["../secret", "/etc/passwd", "escape/secret"] {
            assert_eq!(
                fault(
                    session
                        .workspace_read(read(path), options())
                        .await
                        .unwrap_err()
                )
                .kind(),
                Some(FaultKind::SessionWorkspacePathInvalid)
            );
        }
        assert_eq!(
            fault(
                session
                    .workspace_read(read("missing"), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspacePathNotFound)
        );
        assert_eq!(
            fault(
                client
                    .session(other.clone())
                    .workspace_files(files(), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceUnavailable)
        );
        assert_eq!(
            fault(
                client
                    .session(SessionRef {
                        agent_id: target.agent_id.clone(),
                        session_id: other.session_id.clone()
                    })
                    .workspace_status(options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::AgentSessionNotFound)
        );
        assert_eq!(
            fault(
                client
                    .session(foreign.clone())
                    .workspace_status(options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::HostAgentNotFound)
        );
    }
    let mut raw = peers.tls.raw().await;
    raw.initialize(compaction_common::KEY).await;
    let mut bad_target = serde_json::to_value(&target).unwrap();
    bad_target["host_id"] = json!(foreign_operator.initial_info().host_id);
    let result = raw
        .request(
            1,
            "workspace.read",
            json!({"target":bad_target,"request":read("missing")}),
        )
        .await;
    assert_eq!(result["error"]["code"], -32602);
    let result = raw
        .request(
            2,
            "workspace.files",
            json!({"target":target,"request":files(),"internal":true}),
        )
        .await;
    assert_eq!(result["error"]["code"], -32602);
    peers.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn git_status_and_diffs_are_actual_workspace_facts_and_keep_live_cursor_semantics() {
    let http = HttpFixture::start(vec![]).await;
    let host = host(&http, vec![]).await;
    let operator = host.connect().unwrap();
    let root = tempfile::tempdir().unwrap();
    repository(root.path());
    fs::write(root.path().join("file"), "before\n").unwrap();
    git(root.path(), &["add", "file"]);
    git(root.path(), &["commit", "-qm", "fixture"]);
    fs::write(root.path().join("file"), "after é中\n").unwrap();
    fs::write(root.path().join("unrelated"), "external\n").unwrap();
    let target = create(&host, &operator.caller(), Some(root.path()), vec![]).await;
    let peers = Clients::new(host).await;
    for client in &peers.clients {
        let session = client.session(target.clone());
        let status = session.workspace_status(options()).await.unwrap();
        assert!(status.data.repo_available);
        assert_eq!(status.data.entries.len(), 2);
        assert!(
            session
                .changes_list(list(ChangeScope::Session), options())
                .await
                .unwrap()
                .data
                .records
                .is_empty()
        );
        let mut request = list(ChangeScope::Workspace);
        request.limit = 1;
        let changes = session
            .changes_list(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(
            changes.data.records[0].origin,
            ChangeOrigin::WorkspaceUnknown
        );
        assert_eq!(changes.data.records[0].path, "file");
        request.cursor = changes.data.next_cursor;
        assert_eq!(
            session
                .changes_list(request, options())
                .await
                .unwrap()
                .data
                .records[0]
                .path,
            "unrelated"
        );
        let mut request = diff(
            changes.data.records[0].change_ref.clone(),
            DiffComparison::HeadToWorktree,
        );
        request.max_bytes = 4;
        let first = session
            .changes_diff(request.clone(), options())
            .await
            .unwrap();
        assert_eq!(first.data.origin, ChangeOrigin::WorkspaceUnknown);
        assert!(first.data.next_cursor.is_some());
        request.cursor = first.data.next_cursor;
        assert!(
            !session
                .changes_diff(request.clone(), options())
                .await
                .unwrap()
                .data
                .hunks
                .is_empty()
        );
        fs::write(root.path().join("file"), "changed again\n").unwrap();
        assert_eq!(
            fault(session.changes_diff(request, options()).await.unwrap_err()).kind(),
            Some(FaultKind::SessionWorkspaceCursorInvalid)
        );
        fs::write(root.path().join("file"), "after é中\n").unwrap();
    }
    peers.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nonrepo_corrupt_repo_and_external_filter_have_distinct_real_outcomes() {
    let http = HttpFixture::start(vec![]).await;
    let host = host(&http, vec![]).await;
    let operator = host.connect().unwrap();
    let plain = tempfile::tempdir().unwrap();
    let broken = tempfile::tempdir().unwrap();
    fs::create_dir(broken.path().join(".git")).unwrap();
    fs::write(broken.path().join(".git/HEAD"), "damaged\n").unwrap();
    let filter = tempfile::tempdir().unwrap();
    repository(filter.path());
    fs::write(filter.path().join("file"), "old\n").unwrap();
    git(filter.path(), &["add", "file"]);
    git(filter.path(), &["commit", "-qm", "fixture"]);
    fs::write(filter.path().join("file"), "modified\n").unwrap();
    fs::write(
        filter.path().join(".gitattributes"),
        "file filter=observe\n",
    )
    .unwrap();
    git(
        filter.path(),
        &[
            "config",
            "filter.observe.clean",
            "touch filter-executed; cat",
        ],
    );
    let plain = create(&host, &operator.caller(), Some(plain.path()), vec![]).await;
    let broken = create(&host, &operator.caller(), Some(broken.path()), vec![]).await;
    let filtered = create(&host, &operator.caller(), Some(filter.path()), vec![]).await;
    let peers = Clients::new(host).await;
    for client in &peers.clients {
        let session = client.session(plain.clone());
        assert!(
            !session
                .workspace_status(options())
                .await
                .unwrap()
                .data
                .repo_available
        );
        assert_eq!(
            fault(
                session
                    .changes_list(list(ChangeScope::Workspace), options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceNotRepository)
        );
        assert_eq!(
            fault(
                client
                    .session(broken.clone())
                    .workspace_status(options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceReadFailed)
        );
        assert_eq!(
            fault(
                client
                    .session(filtered.clone())
                    .workspace_status(options())
                    .await
                    .unwrap_err()
            )
            .kind(),
            Some(FaultKind::SessionWorkspaceUnsupported)
        );
        assert!(!filter.path().join("filter-executed").exists());
    }
    peers.finish().await;
}
