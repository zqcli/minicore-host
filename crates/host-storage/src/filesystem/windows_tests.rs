use super::*;

#[test]
fn failed_root_creation_retries_every_directory_down_from_the_os_profile() {
    let profile = winsafe::SHGetKnownFolderPath(
        &winsafe::co::KNOWNFOLDERID::Profile,
        winsafe::co::KF::DEFAULT,
        None,
    )
    .unwrap();
    let profile = fs::canonicalize(profile).unwrap();
    let temp = tempfile::tempdir_in(&profile).unwrap();
    let parent = fs::canonicalize(temp.path()).unwrap().join("new-parent");
    let path = parent.join("host");
    SYNC_FAILURE.with(|failure| *failure.borrow_mut() = Some(parent.clone()));
    assert!(matches!(
        Directory::root(&path),
        Err(StorageError::Io {
            operation: "sync data root ancestor",
            ..
        })
    ));
    assert!(path.exists());
    SYNC_TRACE.with(|trace| trace.borrow_mut().clear());
    let root = Directory::root(&path).unwrap();
    SYNC_TRACE.with(|trace| {
        let trace = trace.borrow();
        assert!(trace.contains(&parent));
        assert!(trace.contains(&root.path));
        assert!(trace.contains(&profile));
        assert!(!trace.contains(&profile.parent().unwrap().to_owned()));
    });
}

#[test]
fn root_lease_and_descendants_reject_directory_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("host");
    let root = Directory::root(&path).unwrap();
    let child = root.child("owned", true).unwrap();
    let alias = temp.path().join("alias");
    crate::test_support::symlink(&path, &alias).unwrap();
    assert!(matches!(
        Directory::root(&alias),
        Err(StorageError::RootInUse)
    ));
    for directory in [&root, &child] {
        let error = fs::rename(&directory.path, directory.path.with_extension("detached"))
            .expect_err("live handle must reject directory replacement");
        assert!(matches!(error.raw_os_error(), Some(5 | 32)));
        directory.verify_attached().unwrap();
    }
    drop(root);
    assert!(matches!(
        Directory::root(&path),
        Err(StorageError::RootInUse)
    ));
    drop(child);
    let _reopened = Directory::root(&path).unwrap();
}

#[test]
fn validated_empty_child_can_be_explicitly_deleted_after_its_guard_drops() {
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    let child = root.child("owned", true).unwrap();
    assert!(root.remove_empty_child("owned").is_err());
    assert!(child.path.exists());
    drop(child);
    root.remove_empty_child("owned").unwrap();
    assert!(!root.path.join("owned").exists());
}

#[test]
fn windows_alias_components_are_rejected_before_open() {
    for name in [
        "..",
        "a\\b",
        "a:b",
        "a.",
        "a ",
        "CON",
        "nul.json",
        "COM1",
        "LPT².log",
    ] {
        assert!(
            matches!(component(name), Err(StorageError::UnsafePath { .. })),
            "{name}"
        );
    }
    component("history.jsonl").unwrap();
}

#[test]
fn actual_file_permissions_and_links_are_checked_on_the_handle() {
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    root.atomic_write("object.json", b"original").unwrap();
    let path = root.path.join("object.json");
    fs::hard_link(&path, temp.path().join("foreign-link")).unwrap();
    assert!(matches!(
        root.read_metadata("object.json"),
        Err(StorageError::UnsafePath { .. })
    ));
    fs::remove_file(temp.path().join("foreign-link")).unwrap();
    crate::test_support::set_mode(&path, 0o666);
    assert!(matches!(
        root.read_metadata("object.json"),
        Err(StorageError::UnsafePath { .. })
    ));
    crate::test_support::set_mode(&path, 0o600);
    assert_eq!(root.read_metadata("object.json").unwrap(), b"original");
}

#[test]
fn published_replacement_and_directory_flush_preserve_exact_identity() {
    let temp = tempfile::tempdir().unwrap();
    let root = Directory::root(&temp.path().join("host")).unwrap();
    root.atomic_write("object.json", b"first").unwrap();
    let first = root.open_file("object.json", false, false).unwrap();
    root.atomic_write("object.json", b"second").unwrap();
    assert!(matches!(
        root.verify_file_identity("object.json", &first),
        Err(StorageError::UncertainWrite)
    ));
    assert_eq!(root.read_metadata("object.json").unwrap(), b"second");
}
