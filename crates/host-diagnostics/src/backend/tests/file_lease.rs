//! Hold the same open-file description in a real child without timing a fork.
use crate::{
    EventData, EventSpec, InitError, Options, Output, Scope, ShutdownOutcome, prepare,
    prepare_file, prepare_writer,
};
use std::{
    fs::OpenOptions,
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

fn options(path: &Path) -> Options {
    Options {
        output: Output::File(path.to_owned()),
        ..Options::new("inherited-lease-test")
    }
}

/// Only timing is controlled: the worker owns an actual locked regular file,
/// and performs the real write, flush, and unlock after the test releases it.
struct GatedFileWriter {
    file: std::fs::File,
    entered: mpsc::Sender<()>,
    proceed: Option<mpsc::Receiver<()>>,
    released: mpsc::Sender<()>,
}

impl Write for GatedFileWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(proceed) = self.proceed.take() {
            self.entered.send(()).unwrap();
            proceed.recv().unwrap();
        }
        self.file.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[test]
fn bounded_shutdown_timeout_keeps_a_blocked_workers_actual_file_lease() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("blocked-output.jsonl");
    let alias = directory.path().join("blocked-alias.jsonl");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&path)
        .unwrap();
    std::fs::hard_link(&path, &alias).unwrap();
    file.try_lock().unwrap();
    let (entered, observe) = mpsc::channel();
    let (proceed, release) = mpsc::channel();
    let (released, finished) = mpsc::channel();
    let writer = GatedFileWriter {
        file,
        entered,
        proceed: Some(release),
        released,
    };
    let (dispatch, guard) = prepare_writer(options(&path), writer, |writer| {
        writer.file.unlock()?;
        writer.released.send(()).unwrap();
        Ok(())
    })
    .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        EventSpec {
            level: tracing::Level::INFO,
            scope: Scope::Host,
            component: "test.writer",
            event: "test.writer_blocked",
            message: "Controlled regular file writer fixture",
            attributes: &[],
        }
        .emit(EventData::default);
    });
    observe.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(dispatch);
    assert_eq!(
        guard.shutdown(Duration::from_millis(20)),
        ShutdownOutcome::TimedOut
    );
    held(&path);
    held(&alias);
    assert!(finished.try_recv().is_err());
    proceed.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(2)).unwrap();
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(record["event"], "test.writer_blocked");
    let (_dispatch, next_guard) = prepare(options(&alias)).unwrap();
    assert_eq!(
        next_guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
}

fn held(path: &Path) {
    assert!(matches!(
        prepare(options(path)),
        Err(InitError::LockOutput(std::io::ErrorKind::WouldBlock))
    ));
}

#[test]
fn completed_writer_releases_lease_while_inherited_descriptor_is_still_open() {
    const CHILD: &str = "MINICORE_INHERITED_DIAGNOSTIC_LEASE";
    if std::env::var_os(CHILD).is_some() {
        println!("inherited descriptor ready");
        std::io::stdout().flush().unwrap();
        std::io::stdin().read_exact(&mut [0]).unwrap();
        std::process::exit(0);
    }

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.jsonl");
    let alias = directory.path().join("alias.jsonl");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&path)
        .unwrap();
    std::fs::hard_link(&path, &alias).unwrap();
    // Passing a duplicate as stderr preserves the same open-file description
    // across exec. A concurrent fork temporarily inherits it in exactly the
    // same way; keeping it open makes that otherwise short window deterministic.
    // The child does not write to this descriptor or touch its advisory lock.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "backend::tests::file_lease::completed_writer_releases_lease_while_inherited_descriptor_is_still_open",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(file.try_clone().unwrap()))
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    while lines.next().unwrap().unwrap() != "inherited descriptor ready" {}

    // Exercise the same file acquisition and worker setup used by prepare.
    let (dispatch, guard) = prepare_file(options(&path), file).unwrap();
    held(&path);
    held(&alias);
    tracing::dispatcher::with_default(&dispatch, || {
        EventSpec {
            level: tracing::Level::INFO,
            scope: Scope::Host,
            component: "test.writer",
            event: "test.inherited_lease",
            message: "Controlled inherited descriptor fixture",
            attributes: &[],
        }
        .emit(EventData::default);
    });
    drop(dispatch);
    let outcome = guard.shutdown(Duration::from_secs(2));
    let reopened = prepare(options(&alias));
    let child_was_alive = child.try_wait().unwrap().is_none();
    // Reap the child before asserting the regression so the red run is clean.
    child.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(child_was_alive);
    assert_eq!(outcome, ShutdownOutcome::Flushed);
    let (_dispatch, next_guard) = reopened.unwrap();
    // Closing the inherited, already-unlocked description must not release the
    // new writer's independent lease.
    held(&path);
    assert_eq!(
        next_guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    let record: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(record["event"], "test.inherited_lease");
}
