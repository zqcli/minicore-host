use std::io;

#[cfg(unix)]
pub struct Signals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}
#[cfg(unix)]
impl Signals {
    pub fn new() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: signal(SignalKind::interrupt())?,
            terminate: signal(SignalKind::terminate())?,
        })
    }
    pub async fn recv(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {},
            _ = self.terminate.recv() => {},
        }
    }
}

#[cfg(not(unix))]
pub struct Signals;
#[cfg(not(unix))]
impl Signals {
    pub fn new() -> io::Result<Self> {
        Ok(Self)
    }
    pub async fn recv(&mut self) {
        let _ = tokio::signal::ctrl_c().await;
    }
}
