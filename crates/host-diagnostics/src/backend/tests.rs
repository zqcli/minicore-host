use super::*;
use crate::{EventData, EventSpec, Options, Scope, prepare_writer};
use std::{process::Command, time::Instant};
use tracing::Level;

#[cfg(target_os = "linux")]
mod file_lease;

const EVENT: EventSpec = EventSpec {
    level: Level::ERROR,
    scope: Scope::Host,
    component: "test.writer",
    event: "test.failure",
    message: "Controlled failure summary",
    attributes: &[],
};

struct FailingWriter {
    write_fails: bool,
    flush_fails: bool,
    released: Arc<AtomicBool>,
}
impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.write_fails {
            Err(io::Error::other(
                "SECRET raw write error with https://token@private.invalid",
            ))
        } else {
            Ok(bytes.len())
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.flush_fails {
            Err(io::Error::other(
                "SECRET raw flush error with Authorization body",
            ))
        } else {
            Ok(())
        }
    }
}

#[test]
fn ordinary_write_and_flush_failures_are_counted_separately_from_drops() {
    for (write_fails, flush_fails, expected) in
        [(true, false, 2), (false, true, 1), (true, true, 3)]
    {
        let released = Arc::new(AtomicBool::new(false));
        let (sender, guard) = start(
            FailingWriter {
                write_fails,
                flush_fails,
                released: released.clone(),
            },
            |writer| {
                writer.released.store(true, Ordering::Relaxed);
                Ok(())
            },
        )
        .unwrap();
        let health = guard.health();
        sender.send(b"one\n".to_vec());
        sender.send(b"two\n".to_vec());
        assert_eq!(
            guard.shutdown(Duration::from_secs(2)),
            ShutdownOutcome::Failed
        );
        assert!(released.load(Ordering::Relaxed));
        assert_eq!(
            health.snapshot(),
            LoggingStatus {
                status: LoggingState::Degraded,
                dropped_records: Some(0),
                write_failures: Some(expected),
            }
        );
    }
}

#[test]
fn output_release_failure_is_not_flushed_or_counted_as_a_write_or_drop() {
    let (_sender, guard) = start(io::sink(), |_| {
        Err(io::Error::other("SECRET raw output release error"))
    })
    .unwrap();
    let health = guard.health();
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Failed
    );
    assert_eq!(
        health.snapshot(),
        LoggingStatus {
            status: LoggingState::Degraded,
            dropped_records: Some(0),
            write_failures: Some(0),
        }
    );
}

struct BlockedWriter {
    entered: Sender<()>,
    release: Receiver<()>,
    flushed: Sender<()>,
    output_released: Sender<()>,
}
impl Write for BlockedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushed.send(()).unwrap();
        Ok(())
    }
}

#[test]
fn shutdown_timeout_is_bounded_and_does_not_claim_known_tail_loss_or_flush() {
    let (entered, observe) = mpsc::channel();
    let (release, unblock) = mpsc::channel();
    let (flushed, finish) = mpsc::channel();
    let (output_released, observe_release) = mpsc::channel();
    let (sender, guard) = start(
        BlockedWriter {
            entered,
            release: unblock,
            flushed,
            output_released,
        },
        |writer| {
            writer.output_released.send(()).unwrap();
            Ok(())
        },
    )
    .unwrap();
    let health = guard.health();
    sender.send(b"one\n".to_vec());
    observe.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    assert_eq!(
        guard.shutdown(Duration::from_millis(20)),
        ShutdownOutcome::TimedOut
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        health.snapshot(),
        LoggingStatus {
            status: LoggingState::Degraded,
            dropped_records: None,
            write_failures: Some(0),
        }
    );
    assert!(finish.try_recv().is_err());
    assert!(observe_release.try_recv().is_err());
    release.send(()).unwrap();
    finish.recv_timeout(Duration::from_secs(2)).unwrap();
    observe_release
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
}

#[test]
fn dropping_a_host_health_handle_cannot_close_the_shared_pipeline() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let (sender, guard) = start(file.reopen().unwrap(), |_| Ok(())).unwrap();
    let host_a = guard.health();
    let host_b = guard.health();
    drop(host_a);
    sender.send(b"another host still writes\n".to_vec());
    assert_eq!(host_b.snapshot().status, LoggingState::Ok);
    assert_eq!(
        guard.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Flushed
    );
    sender.send(b"after close\n".to_vec());
    assert_eq!(host_b.snapshot().dropped_records, Some(1));
    assert_eq!(
        std::fs::read_to_string(file.path()).unwrap(),
        "another host still writes\n"
    );
}

// The same test binary is an isolated child, but process::exit bypasses the Rust
// harness's own stdout progress text. There is no production probe executable.
#[test]
fn subprocess_writer_paths_keep_stdout_rpc_only_and_do_not_echo_raw_errors() {
    const MODE: &str = "MINICORE_DIAGNOSTICS_STDIO_PROBE";
    if let Ok(mode) = std::env::var(MODE) {
        let mut options = Options::new("stdio-test");
        options.level = Level::TRACE;
        let outcome = match mode.as_str() {
            "normal" => {
                let (dispatch, guard) = prepare_writer(options, io::stderr(), |_| Ok(())).unwrap();
                tracing::dispatcher::set_global_default(dispatch).unwrap();
                EVENT.emit(EventData::default);
                guard.shutdown(Duration::from_secs(2))
            }
            "error" => {
                let (dispatch, guard) = prepare_writer(
                    options,
                    FailingWriter {
                        write_fails: true,
                        flush_fails: true,
                        released: Arc::new(AtomicBool::new(false)),
                    },
                    |_| Ok(()),
                )
                .unwrap();
                tracing::dispatcher::set_global_default(dispatch).unwrap();
                EVENT.emit(EventData::default);
                guard.shutdown(Duration::from_secs(2))
            }
            "release_error" => {
                let (dispatch, guard) = prepare_writer(options, io::sink(), |_| {
                    Err(io::Error::other("SECRET raw output release error"))
                })
                .unwrap();
                tracing::dispatcher::set_global_default(dispatch).unwrap();
                EVENT.emit(EventData::default);
                guard.shutdown(Duration::from_secs(2))
            }
            "timeout" | "drop_timeout" => {
                let (entered, observe) = mpsc::channel();
                let (_release, unblock) = mpsc::channel();
                let (flushed, _finish) = mpsc::channel();
                let (output_released, _observe_release) = mpsc::channel();
                let (dispatch, guard) = prepare_writer(
                    options,
                    BlockedWriter {
                        entered,
                        release: unblock,
                        flushed,
                        output_released,
                    },
                    |_| Ok(()),
                )
                .unwrap();
                tracing::dispatcher::set_global_default(dispatch).unwrap();
                EVENT.emit(EventData::default);
                observe.recv_timeout(Duration::from_secs(2)).unwrap();
                if mode == "drop_timeout" {
                    drop(guard);
                } else {
                    assert_eq!(
                        guard.shutdown(Duration::from_millis(10)),
                        ShutdownOutcome::TimedOut
                    );
                }
                // Keep the release sender alive so only process exit ends this
                // deliberately stuck output operation, not a test writer panic.
                std::mem::forget(_release);
                ShutdownOutcome::TimedOut
            }
            _ => unreachable!(),
        };
        match mode.as_str() {
            "normal" => assert_eq!(outcome, ShutdownOutcome::Flushed),
            "error" | "release_error" => assert_eq!(outcome, ShutdownOutcome::Failed),
            _ => assert_eq!(outcome, ShutdownOutcome::TimedOut),
        }
        io::stdout()
            .write_all(b"{\"jsonrpc\":\"2.0\",\"result\":true,\"id\":1}\n")
            .unwrap();
        io::stdout().flush().unwrap();
        std::process::exit(0);
    }
    for mode in [
        "normal",
        "error",
        "release_error",
        "timeout",
        "drop_timeout",
    ] {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "backend::tests::subprocess_writer_paths_keep_stdout_rpc_only_and_do_not_echo_raw_errors", "--nocapture", "--quiet"])
            .env(MODE, mode)
            .output().unwrap();
        assert!(child.status.success(), "{mode}: {:?}", child.stderr);
        // libtest writes its startup banner before invoking the selected test.
        let stdout = String::from_utf8(child.stdout).unwrap();
        assert_eq!(
            stdout, "\nrunning 1 test\n{\"jsonrpc\":\"2.0\",\"result\":true,\"id\":1}\n",
            "{mode}"
        );
        if mode == "normal" {
            let line = String::from_utf8(child.stderr).unwrap();
            assert_eq!(line.lines().count(), 1);
            let record: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(record["event"], "test.failure");
        } else {
            assert!(child.stderr.is_empty(), "{mode}: {:?}", child.stderr);
        }
    }
}
