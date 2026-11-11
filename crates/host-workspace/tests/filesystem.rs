use minicore_host_protocol::WorkspaceRef;
use minicore_host_workspace::*;
use std::{
    fs,
    time::{Duration, Instant},
};
use tempfile::TempDir;

fn files(directory: &str, recursive: bool, limit: usize) -> FilesRequest {
    FilesRequest {
        directory: directory.into(),
        recursive,
        query: None,
        cursor: None,
        limit,
    }
}
fn read(path: &str, start: u64, end: Option<u64>, max_bytes: usize) -> ReadRequest {
    ReadRequest {
        path: path.into(),
        start,
        end,
        max_bytes,
    }
}
fn search(path: &str, query: &str, max_matches: usize) -> SearchRequest {
    SearchRequest {
        path: path.into(),
        query: query.into(),
        case_sensitive: true,
        max_matches,
        cursor: None,
    }
}

#[tokio::test]
async fn binds_only_explicit_existing_directory_without_chdir() {
    let cwd = std::env::current_dir().unwrap();
    assert!(matches!(
        Workspace::bind(None).await,
        Err(WorkspaceError::NotConfigured)
    ));
    assert!(matches!(
        Workspace::open("relative").await,
        Err(WorkspaceError::InvalidRoot)
    ));
    let root = TempDir::new().unwrap();
    assert!(matches!(
        Workspace::open(root.path().join("missing")).await,
        Err(WorkspaceError::NotFound)
    ));
    fs::write(root.path().join("file"), "text").unwrap();
    assert!(matches!(
        Workspace::open(root.path().join("file")).await,
        Err(WorkspaceError::NotDirectory)
    ));
    let binding = Workspace::bind(Some(&WorkspaceRef {
        path: root.path().to_str().unwrap().into(),
    }))
    .await
    .unwrap();
    assert_eq!(binding.root(), root.path().canonicalize().unwrap());
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}

#[tokio::test]
async fn files_are_sorted_scoped_ignore_aware_and_page_without_limits() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::write(root.path().join(".git/secret"), "private").unwrap();
    fs::write(root.path().join(".gitignore"), "ignored\n").unwrap();
    for path in ["src/中文.rs", "src/a.rs", "z.txt", "ignored", ".hidden"] {
        fs::write(root.path().join(path), "text").unwrap();
    }
    let ws = Workspace::open(root.path()).await.unwrap();
    let first = ws
        .files(files("src", true, 1), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(first.entries[0].path, "src/a.rs");
    assert!(first.truncated);
    let mut next = files("src", true, 2);
    next.cursor = first.next_cursor.clone();
    let last = ws.files(next, QueryContext::default()).await.unwrap();
    assert_eq!(last.entries[0].path, "src/中文.rs");
    assert!(!last.truncated);
    assert!(last.next_cursor.is_none());
    let mut wrong = files("", true, 2);
    wrong.cursor = first.next_cursor;
    assert!(matches!(
        ws.files(wrong, QueryContext::default()).await,
        Err(WorkspaceError::InvalidCursor)
    ));
    let all = ws
        .files(files("", true, usize::MAX), QueryContext::default())
        .await
        .unwrap();
    assert!(all.entries.iter().any(|entry| entry.path == ".hidden"));
    assert!(
        all.entries
            .iter()
            .all(|entry| !entry.path.starts_with(".git/")
                && entry.path != ".git"
                && entry.path != "ignored")
    );
    let top = ws
        .files(files("", false, 2000), QueryContext::default())
        .await
        .unwrap();
    assert!(top.entries.iter().all(|entry| !entry.path.contains('/')));
    assert!(
        top.entries
            .iter()
            .any(|entry| entry.kind == FileKind::Directory)
    );
    let mut filtered = files("", true, 2000);
    filtered.query = Some("中文".into());
    assert_eq!(
        ws.files(filtered, QueryContext::default())
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[tokio::test]
async fn read_uses_actual_byte_range_and_character_boundaries() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("text"), "a中🙂z\r\nlast").unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let first = ws
        .read(read("text", 1, Some(8), 5), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(
        (first.start, first.end, first.text.as_str(), first.has_more),
        (1, 4, "中", true)
    );
    let second = ws
        .read(read("text", first.end, Some(8), 4), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(second.text, "🙂");
    assert!(!second.has_more);
    let tail = ws
        .read(read("text", 8, None, usize::MAX), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(tail.text, "z\r\nlast");
    let empty = ws
        .read(read("text", tail.end, None, 0), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(empty.start, empty.end);
    assert!(!empty.has_more);
    for request in [
        read("text", 2, None, 10),
        read("text", 0, Some(3), 10),
        read("text", 1, None, 2),
        read("text", 999, None, 10),
        read("text", 0, None, 0),
    ] {
        assert!(matches!(
            ws.read(request, QueryContext::default()).await,
            Err(WorkspaceError::InvalidRange)
        ));
    }
}

#[tokio::test]
async fn common_read_errors_are_not_empty_success() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("dir")).unwrap();
    fs::write(root.path().join("binary"), [0, 0xff, 0x80]).unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    assert!(matches!(
        ws.read(read("missing", 0, None, 100), QueryContext::default())
            .await,
        Err(WorkspaceError::NotFound)
    ));
    assert!(matches!(
        ws.read(read("dir", 0, None, 100), QueryContext::default())
            .await,
        Err(WorkspaceError::NotFile)
    ));
    assert!(matches!(
        ws.read(read("binary", 0, None, 100), QueryContext::default())
            .await,
        Err(WorkspaceError::Binary)
    ));
    assert!(matches!(
        ws.read(read("../outside", 0, None, 100), QueryContext::default())
            .await,
        Err(WorkspaceError::InvalidPath)
    ));
    assert!(matches!(
        ws.files(files("binary", false, 20), QueryContext::default())
            .await,
        Err(WorkspaceError::NotDirectory)
    ));
}

#[tokio::test]
async fn literal_search_has_real_unicode_ranges_and_explicit_continuation() {
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("src/a"),
        "中🙂 need.le NEED.LE\r\nsecond need.le\n",
    )
    .unwrap();
    fs::write(root.path().join("src/b"), "another need.le\n").unwrap();
    fs::write(root.path().join("z-binary"), b"need.le\0").unwrap();
    fs::write(root.path().join("unrelated"), "needXle\n").unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let mut request = search("src", "need.le", 1);
    request.case_sensitive = false;
    let first = ws
        .search(request.clone(), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(first.matches.len(), 1);
    let line = &first.matches[0];
    assert_eq!(line.line_text, "中🙂 need.le NEED.LE");
    assert_eq!(line.line_number, 1);
    assert_eq!(
        &line.line_text[line.match_byte_ranges[0].start..line.match_byte_ranges[0].end],
        "need.le"
    );
    assert_eq!(line.match_byte_ranges.len(), 2);
    request.cursor = first.next_cursor.clone();
    let second = ws
        .search(request.clone(), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(second.matches[0].line_number, 2);
    request.cursor = second.next_cursor;
    let last = ws.search(request, QueryContext::default()).await.unwrap();
    assert_eq!(last.matches[0].path, "src/b");
    assert!(last.next_cursor.is_none());
    let all = ws
        .search(search("", "need.le", 9000), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(all.matches.len(), 3);
    assert_eq!(all.skipped_binary_files, ["z-binary"]);
    let mut wrong = search("src", "other", 1);
    wrong.cursor = first.next_cursor;
    assert!(matches!(
        ws.search(wrong, QueryContext::default()).await,
        Err(WorkspaceError::InvalidCursor)
    ));
    let single = ws
        .search(search("src/a", "second", 1), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(single.matches[0].line_number, 2);
}

#[tokio::test]
async fn reads_are_not_subject_to_previous_file_or_result_quotas() {
    let root = TempDir::new().unwrap();
    let text = "x".repeat(2 * 1024 * 1024);
    fs::write(root.path().join("large"), &text).unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let result = ws
        .read(read("large", 0, None, text.len()), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(result.text, text);
    assert!(!result.has_more);
}

#[tokio::test]
async fn cancellation_and_caller_deadline_are_explicit() {
    let root = TempDir::new().unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    let context = QueryContext::default();
    context.cancellation.cancel();
    assert!(matches!(
        ws.files(files("", true, 1), context.clone()).await,
        Err(WorkspaceError::Cancelled)
    ));
    assert!(matches!(
        ws.status(context).await,
        Err(WorkspaceError::Cancelled)
    ));
    let context = QueryContext {
        deadline: Some(Instant::now() - Duration::from_millis(1)),
        ..QueryContext::default()
    };
    assert!(matches!(
        ws.search(search("", "x", 1), context).await,
        Err(WorkspaceError::Deadline)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_cannot_escape_and_directory_links_are_not_walked() {
    use std::os::unix::fs::symlink;
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    fs::write(root.path().join("local"), "local").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    symlink(root.path().join("local"), root.path().join("inside")).unwrap();
    symlink(root.path(), root.path().join("cycle")).unwrap();
    let ws = Workspace::open(root.path()).await.unwrap();
    assert!(matches!(
        ws.read(read("escape/secret", 0, None, 20), QueryContext::default())
            .await,
        Err(WorkspaceError::OutsideWorkspace)
    ));
    assert!(matches!(
        ws.capture("escape/missing".into(), QueryContext::default())
            .await,
        Err(WorkspaceError::OutsideWorkspace)
    ));
    assert!(matches!(
        ws.files(files("escape", true, 20), QueryContext::default())
            .await,
        Err(WorkspaceError::OutsideWorkspace)
    ));
    assert_eq!(
        ws.read(read("inside", 0, None, 20), QueryContext::default())
            .await
            .unwrap()
            .text,
        "local"
    );
    let entries = ws
        .files(files("", true, 20), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(entries.entries.len(), 4);
    assert_eq!(
        entries
            .entries
            .iter()
            .filter(|entry| entry.kind == FileKind::Symlink)
            .count(),
        3
    );
    let found = ws
        .search(search("", "secret", 20), QueryContext::default())
        .await
        .unwrap();
    assert!(found.matches.is_empty());
}

#[cfg(windows)]
#[tokio::test]
async fn junctions_cannot_escape_and_are_not_recursively_walked() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("secret"), "secret").unwrap();
    fs::write(root.path().join("local"), "local").unwrap();
    for (name, destination) in [("escape", outside.path()), ("cycle", root.path())] {
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(root.path().join(name))
            .arg(destination)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    let ws = Workspace::open(root.path()).await.unwrap();
    assert!(matches!(
        ws.read(read("escape/secret", 0, None, 20), QueryContext::default())
            .await,
        Err(WorkspaceError::OutsideWorkspace)
    ));
    assert!(matches!(
        ws.capture("escape/missing".into(), QueryContext::default())
            .await,
        Err(WorkspaceError::OutsideWorkspace)
    ));
    assert!(matches!(
        ws.files(files("escape", true, 20), QueryContext::default())
            .await,
        Err(WorkspaceError::OutsideWorkspace)
    ));
    let entries = ws
        .files(files("", true, 20), QueryContext::default())
        .await
        .unwrap();
    assert_eq!(entries.entries.len(), 3);
    assert_eq!(
        entries
            .entries
            .iter()
            .filter(|entry| entry.kind == FileKind::Symlink)
            .count(),
        2
    );
    let found = ws
        .search(search("", "secret", 20), QueryContext::default())
        .await
        .unwrap();
    assert!(found.matches.is_empty());
    for name in ["escape", "cycle"] {
        fs::remove_dir(root.path().join(name)).unwrap();
    }
}
