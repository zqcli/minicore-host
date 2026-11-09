//! Actual output file ownership, including a blocked OS writer after timeout.
use minicore_host_diagnostics::{
    EventData, EventSpec, InitError, Options, Output, Scope, ShutdownOutcome, prepare,
};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};
use tracing::Level;

fn options(path: &Path) -> Options {
    Options {
        output: Output::File(path.to_owned()),
        ..Options::new("file-lease-test")
    }
}
fn held(path: &Path) {
    let error = match prepare(options(path)) {
        Ok(_) => panic!("a second writer acquired the same file"),
        Err(error) => error,
    };
    assert_eq!(error, InitError::LockOutput(std::io::ErrorKind::WouldBlock));
    assert!(!error.to_string().contains(path.to_str().unwrap()));
}

#[test]
fn same_process_and_aliases_are_exclusive_until_actual_worker_completion() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.jsonl");
    let alias = directory.path().join("alias.jsonl");
    let (dispatch, guard) = prepare(options(&path)).unwrap();
    std::fs::hard_link(&path, &alias).unwrap();
    held(&path);
    held(&alias);
    #[cfg(unix)]
    {
        let symlink = directory.path().join("symbolic.jsonl");
        std::os::unix::fs::symlink(&path, &symlink).unwrap();
        held(&symlink);
    }
    drop(dispatch);
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let (_dispatch, guard) = prepare(options(&alias)).unwrap();
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
}

#[test]
fn output_file_lease_subprocess() {
    let Some(path) = std::env::var_os("MINICORE_DIAGNOSTIC_LEASE_PROBE") else {
        return;
    };
    let (_dispatch, guard) = prepare(options(Path::new(&path))).unwrap();
    println!("lease acquired");
    std::io::stdout().flush().unwrap();
    let mut input = [0];
    std::io::stdin().read_exact(&mut input).unwrap();
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
}

#[test]
fn another_process_cannot_open_an_existing_writer_and_can_open_after_exit() {
    use std::io::{BufRead, BufReader};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.jsonl");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "output_file_lease_subprocess", "--nocapture"])
        .env("MINICORE_DIAGNOSTIC_LEASE_PROBE", &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    loop {
        if lines.next().unwrap().unwrap() == "lease acquired" {
            break;
        }
    }
    held(&path);
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    let (_dispatch, guard) = prepare(options(&path)).unwrap();
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
}

#[cfg(unix)]
#[test]
fn bounded_shutdown_timeout_does_not_release_a_blocked_writers_real_file_lease() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("blocked-output");
    assert!(
        Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let (release, wait) = mpsc::channel();
    let reader_path = path.clone();
    let reader = std::thread::spawn(move || {
        let mut input = std::fs::File::open(reader_path).unwrap();
        wait.recv().unwrap();
        let mut data = Vec::new();
        input.read_to_end(&mut data).unwrap();
        data
    });
    let (dispatch, guard) = prepare(options(&path)).unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        EventSpec {
            level: Level::INFO,
            scope: Scope::Host,
            component: "test.writer",
            event: "test.writer_blocked",
            message: "Controlled blocked writer fixture",
            attributes: &["fixture_marker"],
        }
        .emit(|| EventData::default().attribute("fixture_marker", "x".repeat(1_048_576)));
    });
    assert_eq!(
        guard.shutdown(Duration::from_millis(20)),
        ShutdownOutcome::TimedOut
    );
    held(&path);
    release.send(()).unwrap();
    assert!(!reader.join().unwrap().is_empty());
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.try_lock().unwrap();
}
