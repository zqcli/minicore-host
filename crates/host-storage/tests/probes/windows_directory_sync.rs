//! Standalone native Windows filesystem probe; it is not a storage-contract test.
//!
//! Compile this file directly with rustc so the probe can run before the Windows
//! storage backend exists. It uses only safe standard-library calls, prints each
//! attempted access mask and its OS result, and never turns a failed sync into
//! success. A successful call observes the API result; it is not a power-loss test.
//!
//! Relevant contracts:
//! https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
//! https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers
//! https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/0de7dc40-9627-437e-a4df-c4696cdc3d02

#![forbid(unsafe_code)]

#[cfg(windows)]
mod windows {
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Write};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_LIST_DIRECTORY: u32 = 0x0001;
    const FILE_ADD_FILE: u32 = 0x0002;
    const FILE_ADD_SUBDIRECTORY: u32 = 0x0004;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    const SYNCHRONIZE: u32 = 0x0010_0000;

    #[derive(Clone, Copy)]
    enum Access {
        Read,
        ReadWrite,
        Write,
        Mask(u32),
    }

    const MODES: &[(&str, Access)] = &[
        ("read", Access::Read),
        ("read_write", Access::ReadWrite),
        ("write", Access::Write),
        ("write_attributes", Access::Mask(FILE_WRITE_ATTRIBUTES)),
        (
            "attributes_sync",
            Access::Mask(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE),
        ),
        (
            "directory_write_sync",
            Access::Mask(
                FILE_LIST_DIRECTORY
                    | FILE_ADD_FILE
                    | FILE_ADD_SUBDIRECTORY
                    | FILE_READ_ATTRIBUTES
                    | FILE_WRITE_ATTRIBUTES
                    | SYNCHRONIZE,
            ),
        ),
    ];

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                eprintln!("cleanup path={:?} {}", self.0, describe(&error));
            }
        }
    }

    fn describe(error: &io::Error) -> String {
        format!(
            "error kind={:?} os_code={:?} message={:?}",
            error.kind(),
            error.raw_os_error(),
            error.to_string()
        )
    }

    fn open_directory(path: &Path, access: Access) -> io::Result<File> {
        let mut options = OpenOptions::new();
        match access {
            Access::Read => {
                options.read(true);
            }
            Access::ReadWrite => {
                options.read(true).write(true);
            }
            Access::Write => {
                options.write(true);
            }
            Access::Mask(mask) => {
                options.access_mode(mask);
            }
        }
        options
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    fn probe_directory(label: &str, path: &Path) -> usize {
        let mut successes = 0;
        for &(mode, access) in MODES {
            let file = match open_directory(path, access) {
                Ok(file) => file,
                Err(error) => {
                    println!(
                        "phase={label} path={path:?} mode={mode} open={} sync=not_attempted",
                        describe(&error)
                    );
                    continue;
                }
            };
            match file.sync_all() {
                Ok(()) => {
                    successes += 1;
                    println!("phase={label} path={path:?} mode={mode} open=ok sync=ok");
                }
                Err(error) => println!(
                    "phase={label} path={path:?} mode={mode} open=ok sync={}",
                    describe(&error)
                ),
            }
        }
        successes
    }

    pub fn run() -> io::Result<()> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "minicore-windows-directory-sync-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root)?;
        let _scratch = Scratch(root.clone());
        let root = fs::canonicalize(root)?;
        let child = root.join("new-directory");
        fs::create_dir(&child)?;

        println!("probe=windows_directory_sync root={root:?}");
        let after_create = probe_directory("after_directory_create", &root);
        let new_directory = probe_directory("new_directory", &child);

        let pending = child.join("pending.json");
        let committed = child.join("committed.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(b"{\"probe\":true}\n")?;
        file.sync_all()?;
        println!("phase=file_content_sync path={pending:?} sync=ok");
        drop(file);
        fs::rename(&pending, &committed)?;
        let after_rename = probe_directory("after_file_rename", &child);
        fs::remove_file(&committed)?;
        let after_unlink = probe_directory("after_file_unlink", &child);
        fs::remove_dir(&child)?;
        let after_remove = probe_directory("after_directory_remove", &root);

        // The current Unix storage backend syncs every canonical ancestor. Report
        // whether Windows grants the same access to already-existing ancestors.
        for ancestor in root.ancestors().skip(1) {
            probe_directory("existing_ancestor", ancestor);
        }

        if [
            after_create,
            new_directory,
            after_rename,
            after_unlink,
            after_remove,
        ]
        .contains(&0)
        {
            Err(io::Error::other(
                "no directory access mode completed sync_all in at least one mutation phase",
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
fn main() -> std::io::Result<()> {
    windows::run()
}

#[cfg(not(windows))]
fn main() {
    println!("windows_directory_sync requires native Windows; no probe was run");
}
