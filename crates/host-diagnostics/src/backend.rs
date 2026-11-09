use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

/// This snapshot covers the whole process pipeline, not one Host or Session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoggingStatus {
    pub status: LoggingState,
    /// Known rejected records. Omitted after a timeout: the pending tail's loss
    /// is unknown. Write failures are counted separately, not as record drops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_records: Option<u64>,
    /// Actual failed write_all or flush attempts observed by the worker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_failures: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoggingState {
    Ok,
    Degraded,
    Disabled,
}

#[derive(Default)]
struct Counters {
    dropped: AtomicU64,
    failed: AtomicU64,
    release_failed: AtomicBool,
    closing: AtomicBool,
    uncertain: AtomicBool,
}

/// Read-only process health. Cloning or dropping this handle never closes output.
/// It neither emits diagnostics nor depends on a working diagnostic destination.
#[derive(Clone)]
pub struct Health(Arc<Counters>);

impl Health {
    pub fn snapshot(&self) -> LoggingStatus {
        let failed = self.0.failed.load(Ordering::Relaxed);
        let dropped = self.0.dropped.load(Ordering::Relaxed);
        let uncertain = self.0.uncertain.load(Ordering::Relaxed);
        let status = if failed != 0
            || dropped != 0
            || uncertain
            || self.0.release_failed.load(Ordering::Relaxed)
        {
            LoggingState::Degraded
        } else if self.0.closing.load(Ordering::Relaxed) {
            LoggingState::Disabled
        } else {
            LoggingState::Ok
        };
        LoggingStatus {
            status,
            dropped_records: (!uncertain).then_some(dropped),
            write_failures: Some(failed),
        }
    }

    fn write_failed(&self) {
        self.0.failed.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct RecordSender {
    // This short lock serializes enqueue versus shutdown, never writer I/O.
    sender: Mutex<Option<Sender<Vec<u8>>>>,
    health: Health,
}

impl RecordSender {
    pub(crate) fn send(&self, record: Vec<u8>) {
        let sender = self.sender.lock().unwrap();
        if sender
            .as_ref()
            .is_none_or(|sender| sender.send(record).is_err())
        {
            self.dropped();
        }
    }

    pub(crate) fn dropped(&self) {
        self.health.0.dropped.fetch_add(1, Ordering::Relaxed);
    }

    fn close(&self) {
        self.sender.lock().unwrap().take();
        self.health.0.closing.store(true, Ordering::Relaxed);
    }
}

/// Result of bounded application shutdown, never a business-persistence claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownOutcome {
    /// All accepted records were processed, the final flush succeeded, and the
    /// output lease was released. This is not fsync or a durability guarantee.
    Flushed,
    /// The worker completed with an observed write/flush/release failure, or stopped
    /// before reporting completion. Partial output may exist.
    Failed,
    /// Completion was not observed before the deadline. The writer thread may
    /// still be blocked; process exit can lose its pending tail.
    TimedOut,
}

/// Unique application owner of the process writer. It is deliberately not Clone.
/// A Host receives only [`Health`]; it cannot close other Hosts' shared pipeline.
pub struct Guard {
    sender: Arc<RecordSender>,
    finished: Option<Receiver<()>>,
}

impl Guard {
    pub fn health(&self) -> Health {
        self.sender.health.clone()
    }

    /// Stop accepting records, drain, and flush within this total wait budget.
    /// Call only after business saves and cleanup. No I/O error is printed to an
    /// alternate destination, and a stuck OS write cannot block the caller past
    /// this wait. It can outlive this guard on the single detached writer thread.
    pub fn shutdown(mut self, timeout: Duration) -> ShutdownOutcome {
        self.finish(timeout)
    }

    fn finish(&mut self, timeout: Duration) -> ShutdownOutcome {
        self.sender.close();
        let Some(finished) = self.finished.take() else {
            return ShutdownOutcome::Failed;
        };
        match finished.recv_timeout(timeout) {
            Ok(())
                if self.sender.health.snapshot().write_failures == Some(0)
                    && !self.sender.health.0.release_failed.load(Ordering::Relaxed) =>
            {
                ShutdownOutcome::Flushed
            }
            Ok(()) => ShutdownOutcome::Failed,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.sender
                    .health
                    .0
                    .uncertain
                    .store(true, Ordering::Relaxed);
                ShutdownOutcome::TimedOut
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.sender
                    .health
                    .0
                    .uncertain
                    .store(true, Ordering::Relaxed);
                ShutdownOutcome::Failed
            }
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.finished.is_some() {
            let _ = self.finish(Duration::from_secs(1));
        }
    }
}

pub(crate) fn start<W: Write + Send + 'static>(
    mut writer: W,
    release: fn(&W) -> io::Result<()>,
) -> io::Result<(Arc<RecordSender>, Guard)> {
    let (send, receive) = mpsc::channel::<Vec<u8>>();
    let (done, finished) = mpsc::channel();
    let health = Health(Arc::new(Counters::default()));
    let sender = Arc::new(RecordSender {
        sender: Mutex::new(Some(send)),
        health: health.clone(),
    });
    // Adapted writer/guard separation from tracing-appender 0.2.5, not its guard:
    // that version prints to stdout on shutdown timeout. No backend-dependent
    // queue cap, rotation policy, raw error fallback, or per-owner worker here.
    thread::Builder::new()
        .name("minicore-diagnostics".into())
        .spawn(move || {
            for record in receive {
                if writer.write_all(&record).is_err() {
                    health.write_failed();
                }
            }
            if writer.flush().is_err() {
                health.write_failed();
            }
            // Only this worker releases after all output I/O has ended. Closing
            // a File alone can leave its flock held by a concurrent fork's
            // inherited open-file description until that child execs or exits.
            if release(&writer).is_err() {
                health.0.release_failed.store(true, Ordering::Relaxed);
            }
            drop(writer);
            let _ = done.send(());
        })?;
    let guard = Guard {
        sender: sender.clone(),
        finished: Some(finished),
    };
    Ok((sender, guard))
}

#[cfg(test)]
mod tests;
