#![cfg(windows)]
//! Real NTFS access-control regression using local tools and actual OS ACLs.
//! The fixture has no live provider and does not impersonate another user.

mod support;

use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::Path,
};
use support::{fixture, invoke};
use windows_permissions::{
    LocalBox, SecurityDescriptor, Sid, Trustee,
    constants::{SeObjectType, SecurityInformation},
    wrappers,
};

const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const FILE_READ_DATA: u32 = 1;

fn security(path: &Path) -> LocalBox<SecurityDescriptor> {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    wrappers::GetSecurityInfo(
        &file,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Owner | SecurityInformation::Dacl,
    )
    .unwrap()
}

fn set_protected_dacl(path: &Path, sddl: &str) {
    let descriptor: LocalBox<SecurityDescriptor> = sddl.parse().unwrap();
    let mut file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    wrappers::SetSecurityInfo(
        &mut file,
        SeObjectType::SE_FILE_OBJECT,
        SecurityInformation::Dacl | SecurityInformation::ProtectedDacl,
        None,
        None,
        descriptor.dacl(),
        None,
    )
    .unwrap();
}

fn everyone_rights(path: &Path) -> u32 {
    let everyone: LocalBox<Sid> = "S-1-1-0".parse().unwrap();
    let trustee: Trustee = everyone.as_ref().into();
    security(path)
        .dacl()
        .unwrap()
        .effective_rights(&trustee)
        .unwrap()
        .bits()
}

fn protected(path: &Path) -> bool {
    let text = wrappers::ConvertSecurityDescriptorToStringSecurityDescriptor(
        &security(path),
        SecurityInformation::Dacl,
    )
    .unwrap()
    .into_string()
    .unwrap();
    text.strip_prefix("D:")
        .unwrap()
        .split('(')
        .next()
        .unwrap()
        .contains('P')
}

#[tokio::test]
async fn replacements_preserve_private_dacl_and_existing_inheritance_behavior() {
    let (root, tools, _) = fixture().await;
    let private = root.path().join("private.txt");
    fs::write(&private, "private before").unwrap();
    let owner = security(&private).owner().unwrap().to_string();
    let inherited_owner = format!("D:P(A;OICI;FA;;;{owner})");
    let public_parent = format!("{inherited_owner}(A;OICI;FR;;;WD)");
    set_protected_dacl(root.path(), &public_parent);
    set_protected_dacl(&private, &format!("D:P(A;;FA;;;{owner})"));
    assert_ne!(everyone_rights(root.path()) & FILE_READ_DATA, 0);
    let private_rights = everyone_rights(&private);
    assert_eq!(private_rights & FILE_READ_DATA, 0);
    assert!(protected(&private));

    let written = invoke(
        &tools,
        "write",
        json!({"path":"private.txt","content":"private written"}),
    )
    .await
    .unwrap();
    assert_eq!(written["status"], "applied", "{written}");
    assert_eq!(everyone_rights(&private), private_rights);
    assert!(protected(&private));
    let edited = invoke(
        &tools,
        "edit",
        json!({"path":"private.txt","old_text":"written","new_text":"edited"}),
    )
    .await
    .unwrap();
    assert_eq!(edited["status"], "applied", "{edited}");
    assert_eq!(fs::read_to_string(&private).unwrap(), "private edited");
    assert_eq!(everyone_rights(&private), private_rights);
    assert!(protected(&private));

    let patched = invoke(
        &tools,
        "apply_patch",
        json!({"path":"private.txt","patch":"*** Begin Patch\n*** Update File: private.txt\n@@\n-private edited\n+private patched\n*** End Patch\n"}),
    )
    .await
    .unwrap();
    assert_eq!(patched["status"], "applied", "{patched}");
    assert_eq!(fs::read_to_string(&private).unwrap(), "private patched");
    assert_eq!(everyone_rights(&private), private_rights);
    assert!(protected(&private));

    // A new file follows the parent policy. Replacing it must retain its
    // unprotected inheritance setting, not freeze the current effective ACL.
    let public = root.path().join("public.txt");
    let created = invoke(
        &tools,
        "write",
        json!({"path":"public.txt","content":"public before"}),
    )
    .await
    .unwrap();
    assert_eq!(created["status"], "applied", "{created}");
    let public_rights = everyone_rights(&public);
    assert_ne!(public_rights & FILE_READ_DATA, 0);
    assert!(!protected(&public));
    let edited = invoke(
        &tools,
        "edit",
        json!({"path":"public.txt","old_text":"before","new_text":"after"}),
    )
    .await
    .unwrap();
    assert_eq!(edited["status"], "applied", "{edited}");
    assert_eq!(everyone_rights(&public), public_rights);
    assert!(!protected(&public));

    let patched = invoke(
        &tools,
        "apply_patch",
        json!({"path":"public.txt","patch":"*** Begin Patch\n*** Update File: public.txt\n@@\n-public after\n+public patched\n*** End Patch\n"}),
    )
    .await
    .unwrap();
    assert_eq!(patched["status"], "applied", "{patched}");
    assert_eq!(fs::read_to_string(&public).unwrap(), "public patched");
    assert_eq!(everyone_rights(&public), public_rights);
    assert!(!protected(&public));

    // Actual OS propagation proves future inheritance still works after the
    // replacement. The separately protected file retains its original rights.
    set_protected_dacl(root.path(), &inherited_owner);
    assert_eq!(everyone_rights(&public) & FILE_READ_DATA, 0);
    assert!(!protected(&public));
    assert_eq!(everyone_rights(&private), private_rights);
    assert!(protected(&private));
    tools.finish().await;
}
