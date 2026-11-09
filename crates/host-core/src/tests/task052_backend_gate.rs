//! Test-only gate inside JournalHistory's real disk mutex, before the real
//! SessionStore reader. No production coordination state is added.
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

pub(crate) struct TestReadGate {
    entered: Semaphore,
    released: Mutex<bool>,
    changed: Condvar,
}
impl TestReadGate {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: Semaphore::new(0),
            released: Mutex::new(false),
            changed: Condvar::new(),
        })
    }
    pub(crate) fn block(&self) {
        self.entered.add_permits(1);
        let released = self.released.lock().unwrap();
        let (_released, timeout) = self
            .changed
            .wait_timeout_while(released, Duration::from_secs(5), |released| !*released)
            .unwrap();
        assert!(!timeout.timed_out(), "test must release the disk-read gate");
    }
    pub(crate) async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
    }
    pub(crate) fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
