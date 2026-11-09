use minicore_host_workspace::*;
use std::{fs, path::Path, process::Command};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Workspace Test",
            "-c",
            "user.email=workspace@example.invalid",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", root.join("nonexistent-global-config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn repository() -> TempDir {
    let root = TempDir::new().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    root
}
fn list(limit: usize) -> ChangesListRequest {
    ChangesListRequest {
        scope: ChangeScope::Workspace,
        cursor: None,
        limit,
    }
}
fn diff(record: &ChangeRecord, comparison: DiffComparison) -> DiffRequest {
    DiffRequest {
        change_ref: record.change_ref.clone(),
        comparison,
        context_lines: 3,
        max_bytes: usize::MAX,
        cursor: None,
    }
}
fn text(result: &DiffResult, kind: DiffLineKind) -> String {
    result
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind == kind)
        .map(|line| line.text.as_str())
        .collect()
}

#[tokio::test]
async fn observes_actual_staged_unstaged_untracked_deleted_and_renamed_files() {
    let root = repository();
    for path in ["staged", "unstaged", "deleted", "old name", "both"] {
        fs::write(root.path().join(path), "original\n").unwrap();
    }
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture"]);
    fs::write(root.path().join("staged"), "index\n").unwrap();
    git(root.path(), &["add", "staged"]);
    fs::write(root.path().join("unstaged"), "worktree\n").unwrap();
    fs::remove_file(root.path().join("deleted")).unwrap();
    git(root.path(), &["mv", "old name", "new 中文"]);
    fs::write(root.path().join("new 中文"), "renamed worktree\n").unwrap();
    fs::write(root.path().join("both"), "index\n").unwrap();
    git(root.path(), &["add", "both"]);
    fs::write(root.path().join("both"), "working\n").unwrap();
    fs::write(root.path().join("untracked"), "new\n").unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let status = ws.status(QueryContext::default()).await.unwrap();
    assert!(status.repo_available && status.complete);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(!status.detached);
    assert_eq!(status.head_oid.as_ref().unwrap().len(), 40);
    let entry = |path: &str| {
        status
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap()
    };
    assert_eq!(entry("staged").index_status.as_deref(), Some("M"));
    assert_eq!(entry("unstaged").worktree_status.as_deref(), Some("M"));
    assert_eq!(entry("deleted").worktree_status.as_deref(), Some("D"));
    assert_eq!(entry("new 中文").kind, StatusEntryKind::Renamed);
    assert_eq!(entry("new 中文").original_path.as_deref(), Some("old name"));
    assert_eq!(entry("untracked").kind, StatusEntryKind::Untracked);
    let records = ws
        .changes_list(list(100), QueryContext::default())
        .await
        .unwrap()
        .records;
    assert_eq!(records.len(), 6);
    assert!(
        records
            .iter()
            .all(|record| record.origin == ChangeOrigin::WorkspaceUnknown
                && record.edit_outcome.is_none())
    );
    let record = |path: &str| records.iter().find(|record| record.path == path).unwrap();
    let staged = ws
        .changes_diff(
            diff(record("both"), DiffComparison::HeadToIndex),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&staged, DiffLineKind::Removed), "original\n");
    assert_eq!(text(&staged, DiffLineKind::Added), "index\n");
    let unstaged = ws
        .changes_diff(
            diff(record("both"), DiffComparison::IndexToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&unstaged, DiffLineKind::Removed), "index\n");
    assert_eq!(text(&unstaged, DiffLineKind::Added), "working\n");
    let renamed = ws
        .changes_diff(
            diff(record("new 中文"), DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&renamed, DiffLineKind::Removed), "original\n");
    assert_eq!(text(&renamed, DiffLineKind::Added), "renamed worktree\n");
    let deleted = ws
        .changes_diff(
            diff(record("deleted"), DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&deleted, DiffLineKind::Removed), "original\n");
    assert!(text(&deleted, DiffLineKind::Added).is_empty());
    let new = ws
        .changes_diff(
            diff(record("untracked"), DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&new, DiffLineKind::Added), "new\n");
    assert!(!new.stale && new.versions_refreshed);
}

#[tokio::test]
async fn duplicate_paths_in_git_status_can_be_paged_without_losing_records() {
    let root = repository();
    fs::write(root.path().join("file"), "base\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture"]);
    git(root.path(), &["rm", "--cached", "file"]);
    let ws = Workspace::open(root.path()).await.unwrap();
    let first = ws
        .changes_list(list(1), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(first.records.len(), 1);
    assert!(first.next_cursor.is_some());
    let mut next = list(1);
    next.cursor = first.next_cursor;
    let second = ws
        .changes_list(next, QueryContext::default())
        .await
        .unwrap();
    assert_eq!(second.records.len(), 1);
    assert!(second.next_cursor.is_none());
    assert_eq!(first.records[0].path, second.records[0].path);
    assert_ne!(first.records[0].change_ref, second.records[0].change_ref);
}

#[tokio::test]
async fn unborn_repository_staged_addition_and_empty_directory_are_distinct() {
    let plain = TempDir::new().unwrap();
    let ws = Workspace::open(plain.path()).await.unwrap();
    let status = ws.status(QueryContext::default()).await.unwrap();
    assert!(!status.repo_available);
    assert!(status.entries.is_empty());
    assert!(matches!(
        ws.changes_list(list(100), QueryContext::default()).await,
        Err(WorkspaceError::NotRepository)
    ));
    let root = repository();
    fs::write(root.path().join("new"), "new\n").unwrap();
    git(root.path(), &["add", "new"]);
    let ws = Workspace::open(root.path()).await.unwrap();
    let status = ws.status(QueryContext::default()).await.unwrap();
    assert!(status.head_oid.is_none());
    let records = ws
        .changes_list(list(100), QueryContext::default())
        .await
        .unwrap()
        .records;
    assert_eq!(records[0].kind, ChangeKind::Added);
    let result = ws
        .changes_diff(
            diff(&records[0], DiffComparison::HeadToIndex),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&result, DiffLineKind::Added), "new\n");
}

#[tokio::test]
async fn nested_workspace_does_not_report_sibling_changes_or_cross_boundary_renames() {
    let root = repository();
    fs::create_dir(root.path().join("selected")).unwrap();
    fs::write(root.path().join("outside"), "outside\n").unwrap();
    fs::write(root.path().join("selected/file"), "inside\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture"]);
    fs::write(root.path().join("outside"), "outside changed\n").unwrap();
    fs::write(root.path().join("selected/file"), "inside changed\n").unwrap();
    let ws = Workspace::open(root.path().join("selected")).await.unwrap();
    let status = ws.status(QueryContext::default()).await.unwrap();
    assert_eq!(status.entries.len(), 1);
    assert_eq!(status.entries[0].path, "file");
    let records = ws
        .changes_list(list(10), QueryContext::default())
        .await
        .unwrap()
        .records;
    let result = ws
        .changes_diff(
            diff(&records[0], DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(text(&result, DiffLineKind::Added), "inside changed\n");
    git(root.path(), &["mv", "outside", "selected/moved-in"]);
    // Git's scoped pathspec reports the incoming rename as an addition inside
    // the binding, without exposing the old path outside it.
    let scoped = ws.status(QueryContext::default()).await.unwrap();
    let moved = scoped
        .entries
        .iter()
        .find(|entry| entry.path == "moved-in")
        .unwrap();
    assert_eq!(moved.index_status.as_deref(), Some("A"));
    assert!(moved.original_path.is_none());
    let moved_record = ws
        .changes_list(list(10), QueryContext::default())
        .await
        .unwrap()
        .records
        .into_iter()
        .find(|record| record.path == "moved-in")
        .unwrap();
    let moved_diff = ws
        .changes_diff(
            diff(&moved_record, DiffComparison::HeadToIndex),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert!(text(&moved_diff, DiffLineKind::Removed).is_empty());
    assert_eq!(text(&moved_diff, DiffLineKind::Added), "outside\n");
}

#[tokio::test]
async fn diff_continuations_reject_changed_bytes_and_report_refreshed_git_observations() {
    let root = repository();
    fs::write(root.path().join("file"), "old\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture"]);
    fs::write(root.path().join("file"), "中🙂new\n").unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let record = ws
        .changes_list(list(10), QueryContext::default())
        .await
        .unwrap()
        .records
        .remove(0);
    let mut request = diff(&record, DiffComparison::HeadToWorktree);
    request.max_bytes = 4;
    let first = ws
        .changes_diff(request.clone(), QueryContext::default())
        .await
        .unwrap();
    assert!(first.next_cursor.is_some());
    request.cursor = first.next_cursor;
    fs::write(root.path().join("file"), "different\n").unwrap();
    assert!(matches!(
        ws.changes_diff(request, QueryContext::default()).await,
        Err(WorkspaceError::InvalidCursor)
    ));
    git(root.path(), &["add", "file"]);
    let refreshed = ws
        .changes_diff(
            diff(&record, DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert!(refreshed.stale);
    assert_eq!(text(&refreshed, DiffLineKind::Added), "different\n");
    git(root.path(), &["commit", "-m", "settled"]);
    assert!(matches!(
        ws.changes_diff(
            diff(&record, DiffComparison::HeadToWorktree),
            QueryContext::default()
        )
        .await,
        Err(WorkspaceError::ChangeNotFound)
    ));
}

#[tokio::test]
async fn binary_conflict_missing_and_corrupt_repository_are_factual() {
    let root = repository();
    fs::write(root.path().join("file"), "base\n").unwrap();
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "fixture"]);
    fs::write(root.path().join("binary"), [0, 0xff]).unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let records = ws
        .changes_list(list(20), QueryContext::default())
        .await
        .unwrap()
        .records;
    let result = ws
        .changes_diff(
            diff(&records[0], DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert!(result.binary);
    assert!(result.hunks.is_empty());
    git(root.path(), &["switch", "-c", "other"]);
    fs::write(root.path().join("file"), "other\n").unwrap();
    git(root.path(), &["add", "file"]);
    git(root.path(), &["commit", "-m", "other"]);
    git(root.path(), &["switch", "main"]);
    fs::write(root.path().join("file"), "main\n").unwrap();
    git(root.path(), &["add", "file"]);
    git(root.path(), &["commit", "-m", "main"]);
    let merged = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "merge",
            "other",
        ])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!merged.status.success());
    let records = ws
        .changes_list(list(20), QueryContext::default())
        .await
        .unwrap()
        .records;
    let conflict = records.iter().find(|record| record.path == "file").unwrap();
    assert_eq!(conflict.kind, ChangeKind::Conflict);
    assert!(!conflict.details_available);
    assert!(matches!(
        ws.changes_diff(
            diff(conflict, DiffComparison::HeadToWorktree),
            QueryContext::default()
        )
        .await,
        Err(WorkspaceError::UnsupportedChange)
    ));
    let broken = TempDir::new().unwrap();
    fs::write(broken.path().join(".git"), "invalid git directory\n").unwrap();
    let binding = Workspace::open(broken.path()).await.unwrap();
    assert!(matches!(
        binding.status(QueryContext::default()).await,
        Err(WorkspaceError::GitFailed { .. })
    ));
    let missing = TempDir::new().unwrap();
    let binding = Workspace::open(missing.path()).await.unwrap();
    missing.close().unwrap();
    assert!(matches!(
        binding.status(QueryContext::default()).await,
        Err(WorkspaceError::NotFound)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn git_symlink_diffs_read_link_text_not_outside_contents() {
    use std::os::unix::fs::symlink;
    let root = repository();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), "SECRET\n").unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let records = ws
        .changes_list(list(20), QueryContext::default())
        .await
        .unwrap()
        .records;
    let result = ws
        .changes_diff(
            diff(&records[0], DiffComparison::HeadToWorktree),
            QueryContext::default(),
        )
        .await
        .unwrap();
    assert!(!text(&result, DiffLineKind::Added).contains("SECRET"));
    assert_eq!(
        text(&result, DiffLineKind::Added),
        outside.path().join("secret").to_str().unwrap()
    );
}

#[tokio::test]
async fn git_paths_are_literal_and_detached_and_bare_states_are_real() {
    let root = repository();
    let paths = [":(glob)*", "--option", "tab\tand\nnewline", "中文"];
    for path in paths {
        fs::write(root.path().join(path), path).unwrap();
    }
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "-m", "literal paths"]);
    git(root.path(), &["switch", "--detach"]);
    for path in paths {
        fs::write(root.path().join(path), format!("changed {path}")).unwrap();
    }
    let ws = Workspace::open(root.path()).await.unwrap();
    let status = ws.status(QueryContext::default()).await.unwrap();
    assert!(status.detached && status.branch.is_none());
    let records = ws
        .changes_list(list(100), QueryContext::default())
        .await
        .unwrap()
        .records;
    assert_eq!(records.len(), paths.len());
    for record in records {
        let result = ws
            .changes_diff(
                diff(&record, DiffComparison::HeadToWorktree),
                QueryContext::default(),
            )
            .await
            .unwrap();
        let old_text: String = result
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.old_index.is_some())
            .map(|line| line.text.as_str())
            .collect();
        let new_text: String = result
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| line.new_index.is_some())
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(old_text, record.path);
        assert_eq!(new_text, format!("changed {}", record.path));
    }
    let bare = TempDir::new().unwrap();
    git(bare.path(), &["init", "--bare"]);
    let bare = Workspace::open(bare.path()).await.unwrap();
    assert!(
        !bare
            .status(QueryContext::default())
            .await
            .unwrap()
            .repo_available
    );
}
