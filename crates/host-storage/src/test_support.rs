//! Real platform permission and link fixtures. These change the actual OS state.

#[cfg(unix)]
pub(crate) fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(windows)]
pub(crate) fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_permissions::{
        LocalBox, SecurityDescriptor,
        constants::{SeObjectType, SecurityInformation},
        wrappers,
    };
    let token = winsafe::HACCESSTOKEN::GetCurrentProcessToken();
    let information = token
        .GetTokenInformation(winsafe::co::TOKEN_INFORMATION_CLASS::User)
        .unwrap();
    let winsafe::TokenInfo::User(user) = information else {
        panic!("missing token user")
    };
    let sid = user.User.Sid().unwrap();
    let foreign = if mode & 0o022 != 0 {
        "(A;OICI;FA;;;WD)"
    } else {
        ""
    };
    let descriptor: LocalBox<SecurityDescriptor> =
        format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA){foreign}")
            .parse()
            .unwrap();
    let mut file = std::fs::OpenOptions::new()
        .access_mode(0x0006_0000) // READ_CONTROL | WRITE_DAC
        .custom_flags(0x0220_0000) // BACKUP_SEMANTICS | OPEN_REPARSE_POINT
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

#[cfg(unix)]
pub(crate) fn symlink(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
pub(crate) fn symlink(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}
