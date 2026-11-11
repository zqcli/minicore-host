use super::*;
use crate::lease_tests::{InheritedDescriptor, inherit_next_lock, inherited};

#[test]
fn final_directory_owner_releases_root_with_unrelated_inherited_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("host");
    let root = Directory::root(&path).unwrap();
    let mut inherited = InheritedDescriptor::new(&root.file);
    let clone = root.clone();
    let child = root.child("owned", true).unwrap();
    drop(root);
    drop(clone);
    assert!(matches!(
        Directory::root(&path),
        Err(StorageError::RootInUse)
    ));
    child.atomic_write("result.json", b"{}").unwrap();
    assert!(matches!(
        Directory::root(&path),
        Err(StorageError::RootInUse)
    ));
    drop(child);
    let next = Directory::root(&path);
    assert!(inherited.is_alive());
    let _next = next.unwrap();
    drop(inherited);
    assert!(matches!(
        Directory::root(&path),
        Err(StorageError::RootInUse)
    ));
}

#[test]
fn failed_root_sync_releases_acquired_lease_with_inherited_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(temp.path()).unwrap().join("host");
    fail(&path);
    inherit_next_lock();
    let result = Directory::root(&path);
    let mut child = inherited();
    assert!(matches!(
        result,
        Err(StorageError::Io {
            operation: "sync data root",
            ..
        })
    ));
    let next = Directory::root(&path);
    assert!(child.is_alive());
    let _next = next.unwrap();
    drop(child);
    assert!(matches!(
        Directory::root(&path),
        Err(StorageError::RootInUse)
    ));
}
