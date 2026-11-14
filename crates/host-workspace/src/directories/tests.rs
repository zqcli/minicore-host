use super::*;

fn request(path: &Path, limit: usize) -> DirectoryListRequest {
    DirectoryListRequest {
        path: display(path).unwrap(),
        hidden: false,
        cursor: None,
        limit,
    }
}
fn roots(path: &Path) -> WorkspaceDirectories {
    WorkspaceDirectories::prepare(&[display(path).unwrap()]).unwrap()
}

#[test]
fn explicit_roots_are_canonical_deduplicated_and_empty_means_no_access() {
    let temporary = tempfile::tempdir().unwrap();
    let empty = WorkspaceDirectories::prepare(&[]).unwrap();
    assert!(empty.roots().unwrap().roots.is_empty());
    assert!(matches!(
        empty.list(request(temporary.path(), 1)),
        Err(DirectoryError::OutsideRoots)
    ));
    let text = display(temporary.path()).unwrap();
    let directories = WorkspaceDirectories::prepare(&[text.clone(), text]).unwrap();
    let entries = directories.roots().unwrap().roots;
    assert_eq!(entries.len(), 1);
    assert_eq!(
        Path::new(&entries[0].path).canonicalize().unwrap(),
        temporary.path().canonicalize().unwrap()
    );
    assert!(matches!(
        WorkspaceDirectories::prepare(&["relative".into()]),
        Err(DirectoryError::InvalidPath)
    ));
    assert!(matches!(
        WorkspaceDirectories::prepare(&[display(&temporary.path().join("missing")).unwrap()]),
        Err(DirectoryError::NotFound)
    ));
    fs::write(temporary.path().join("file"), "file").unwrap();
    assert!(matches!(
        WorkspaceDirectories::prepare(&[display(&temporary.path().join("file")).unwrap()]),
        Err(DirectoryError::NotDirectory)
    ));
    let canonical = temporary.path().canonicalize().unwrap();
    let root = canonical.ancestors().last().unwrap();
    assert_eq!(
        roots(root).roots().unwrap().roots[0].name,
        display(root).unwrap()
    );
}

#[test]
fn directories_page_without_a_fixed_entry_quota_and_include_hidden_only_on_request() {
    let temporary = tempfile::tempdir().unwrap();
    for index in 0..1003 {
        fs::create_dir(temporary.path().join(format!("dir-{index:04}"))).unwrap();
    }
    fs::create_dir(temporary.path().join(".hidden")).unwrap();
    fs::write(temporary.path().join("file"), "excluded").unwrap();
    let directories = roots(temporary.path());
    let full = directories.list(request(temporary.path(), 2000)).unwrap();
    assert_eq!(full.entries.len(), 1003);
    assert!(full.next_cursor.is_none());
    assert!(full.parent.is_none());
    let mut query = request(temporary.path(), 137);
    let mut all = Vec::new();
    loop {
        let page = directories.list(query.clone()).unwrap();
        all.extend(page.entries);
        match page.next_cursor {
            None => break,
            Some(cursor) => query.cursor = Some(cursor),
        }
    }
    assert_eq!(all, full.entries);
    let mut query = request(temporary.path(), 2000);
    query.hidden = true;
    assert_eq!(directories.list(query).unwrap().entries[0].name, ".hidden");
    assert!(matches!(
        directories.list(request(temporary.path(), 0)),
        Err(DirectoryError::InvalidRange)
    ));
}

#[test]
fn cursors_bind_roots_directory_and_hidden_filter() {
    let temporary = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::create_dir(temporary.path().join(name)).unwrap();
    }
    let directories = roots(temporary.path());
    let mut query = request(temporary.path(), 1);
    query.cursor = directories.list(query.clone()).unwrap().next_cursor;
    let mut hidden = query.clone();
    hidden.hidden = true;
    assert!(matches!(
        directories.list(hidden),
        Err(DirectoryError::InvalidCursor)
    ));
    let mut other = query.clone();
    other.path = display(&temporary.path().join("a")).unwrap();
    assert!(matches!(
        directories.list(other),
        Err(DirectoryError::InvalidCursor)
    ));
    let wider = roots(temporary.path().parent().unwrap());
    assert!(matches!(
        wider.list(query.clone()),
        Err(DirectoryError::InvalidCursor)
    ));
    query.cursor.as_mut().unwrap().after_name = "../escape".into();
    assert!(matches!(
        directories.list(query),
        Err(DirectoryError::InvalidCursor)
    ));
}

#[test]
fn mkdir_has_real_single_child_effects_and_preserves_errors() {
    let temporary = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let directories = roots(temporary.path());
    let parent = display(temporary.path()).unwrap();
    let created = directories.mkdir(&parent, "子目录").unwrap();
    assert!(Path::new(&created).is_dir());
    assert!(matches!(
        directories.mkdir(&parent, "子目录"),
        Err(DirectoryError::AlreadyExists)
    ));
    assert!(matches!(
        directories.mkdir(&display(outside.path()).unwrap(), "outside"),
        Err(DirectoryError::OutsideRoots)
    ));
    assert!(!outside.path().join("outside").exists());
    for name in ["", ".", "..", "a/b", "bad\0name"] {
        assert!(
            matches!(
                directories.mkdir(&parent, name),
                Err(DirectoryError::InvalidPath)
            ),
            "{name:?}"
        );
    }
    assert!(matches!(
        directories.mkdir(
            &display(&temporary.path().join("missing")).unwrap(),
            "child"
        ),
        Err(DirectoryError::NotFound)
    ));
    fs::write(temporary.path().join("file"), "file").unwrap();
    assert!(matches!(
        directories.mkdir(&display(&temporary.path().join("file")).unwrap(), "child"),
        Err(DirectoryError::NotDirectory)
    ));
    let child = directories.list(request(Path::new(&created), 1)).unwrap();
    assert_eq!(
        child.parent.as_deref(),
        Some(directories.roots().unwrap().roots[0].path.as_str())
    );
    for path in ["relative".into(), format!("{parent}/../escape")] {
        assert!(matches!(
            directories.list(DirectoryListRequest {
                path,
                ..request(temporary.path(), 1)
            }),
            Err(DirectoryError::InvalidPath)
        ));
    }
}

#[cfg(unix)]
#[test]
fn symlinks_can_only_select_or_create_inside_an_allowed_root() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(temporary.path().join("inside")).unwrap();
    symlink(outside.path(), temporary.path().join("escape")).unwrap();
    symlink(
        temporary.path().join("inside"),
        temporary.path().join("link"),
    )
    .unwrap();
    symlink(
        temporary.path().join("missing"),
        temporary.path().join("broken"),
    )
    .unwrap();
    let directories = roots(temporary.path());
    assert_eq!(
        directories
            .list(request(temporary.path(), 100))
            .unwrap()
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["inside", "link"]
    );
    let escape = display(&temporary.path().join("escape")).unwrap();
    assert!(matches!(
        directories.list(DirectoryListRequest {
            path: escape.clone(),
            ..request(temporary.path(), 1)
        }),
        Err(DirectoryError::OutsideRoots)
    ));
    assert!(matches!(
        directories.mkdir(&escape, "child"),
        Err(DirectoryError::OutsideRoots)
    ));
    assert!(!outside.path().join("child").exists());
    let created = directories
        .mkdir(&display(&temporary.path().join("link")).unwrap(), "child")
        .unwrap();
    assert_eq!(
        Path::new(&created),
        temporary
            .path()
            .canonicalize()
            .unwrap()
            .join("inside/child")
    );
    // Prepared roots retain their canonical scope when a configured link changes.
    let configured = roots(&temporary.path().join("link"));
    fs::remove_file(temporary.path().join("link")).unwrap();
    symlink(outside.path(), temporary.path().join("link")).unwrap();
    assert!(matches!(
        configured.list(request(&temporary.path().join("link"), 1)),
        Err(DirectoryError::OutsideRoots)
    ));
}

#[cfg(unix)]
#[test]
fn unix_names_use_native_rules() {
    let temporary = tempfile::tempdir().unwrap();
    let directories = roots(temporary.path());
    for name in ["CON", "trailing.", "trailing ", "colon:name", "back\\slash"] {
        assert!(
            Path::new(
                &directories
                    .mkdir(&display(temporary.path()).unwrap(), name)
                    .unwrap()
            )
            .is_dir()
        );
    }
}

#[cfg(unix)]
#[test]
fn unix_non_utf8_paths_are_not_fabricated() {
    use std::os::unix::ffi::OsStringExt;
    let path = Path::new("/").join(std::ffi::OsString::from_vec(vec![0xff]));
    assert!(matches!(display(&path), Err(DirectoryError::NonUtf8Path)));
}

// The macOS CI filesystem rejects this name before Host can inspect it.
#[cfg(target_os = "linux")]
#[test]
fn linux_listing_rejects_non_utf8_directory_paths() {
    use std::os::unix::ffi::OsStringExt;
    let temporary = tempfile::tempdir().unwrap();
    let directories = roots(temporary.path());
    fs::create_dir(
        temporary
            .path()
            .join(std::ffi::OsString::from_vec(vec![0xff])),
    )
    .unwrap();
    assert!(matches!(
        directories.list(request(temporary.path(), 100)),
        Err(DirectoryError::NonUtf8Path)
    ));
}

#[cfg(windows)]
#[test]
fn windows_names_reject_device_aliases_and_non_child_paths() {
    for name in [
        "CON",
        "nul.txt",
        "COM1",
        "Lpt9.log",
        "a/b",
        "a\\b",
        "trailing.",
        "trailing ",
        "C:child",
        "bad?name",
    ] {
        assert!(
            matches!(valid_name(name), Err(DirectoryError::InvalidPath)),
            "{name:?}"
        );
    }
}

#[test]
fn windows_display_preserves_drives_and_unc_paths() {
    assert_eq!(strip_verbatim_prefix(r"\\?\C:\work\repo"), r"C:\work\repo");
    assert_eq!(
        strip_verbatim_prefix(r"\\?\UNC\server\share\repo"),
        r"\\server\share\repo"
    );
    assert_eq!(
        strip_verbatim_prefix(r"\\server\share\repo"),
        r"\\server\share\repo"
    );
    assert_eq!(strip_verbatim_prefix(r"\\?\C:\"), r"C:\");
    for path in [
        r"\\?\Volume{12345678-1234-1234-1234-123456789abc}\repo",
        r"\\?\C:\trailing.",
        r"\\?\C:\trailing \child",
        r"\\?\UNC\server\share\trailing ",
        r"\\?\C:\CON.txt",
        r"\\?\C:\COM¹",
        r"\\?\C:\parent\..\child",
    ] {
        assert_eq!(strip_verbatim_prefix(path), path);
    }
}

#[cfg(windows)]
#[test]
fn windows_existing_verbatim_names_roundtrip_through_roots_list_and_mkdir() {
    let temporary = tempfile::tempdir().unwrap();
    let verbatim = temporary.path().canonicalize().unwrap().join("trailing.");
    fs::create_dir(&verbatim).unwrap();
    fs::create_dir(temporary.path().join("zz")).unwrap();
    let directories = roots(temporary.path());
    let mut page = directories.list(request(temporary.path(), 1)).unwrap();
    let entry = page.entries.remove(0);
    let mut next = request(temporary.path(), 1);
    next.cursor = page.next_cursor;
    assert_eq!(directories.list(next).unwrap().entries[0].name, "zz");
    assert_eq!(entry.name, "trailing.");
    assert_eq!(
        Path::new(&entry.path).canonicalize().unwrap(),
        verbatim.canonicalize().unwrap()
    );
    let selected = roots(&verbatim).roots().unwrap().roots.remove(0);
    assert_eq!(selected.path, entry.path);
    let child = directories.mkdir(&entry.path, "child").unwrap();
    assert!(Path::new(&child).is_dir());
    assert_eq!(
        Path::new(&child).canonicalize().unwrap(),
        verbatim.join("child").canonicalize().unwrap()
    );
    fs::remove_dir_all(verbatim).unwrap();
}
