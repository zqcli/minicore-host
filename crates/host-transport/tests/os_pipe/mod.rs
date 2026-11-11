#![allow(dead_code)]
//! Native byte pipes keep endpoint closure and backpressure under OS control.

#[cfg(unix)]
pub type Sender = tokio::net::unix::pipe::Sender;
#[cfg(unix)]
pub type Receiver = tokio::net::unix::pipe::Receiver;
#[cfg(windows)]
pub type Sender = tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(windows)]
pub type Receiver = tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(unix)]
pub async fn pipe() -> std::io::Result<(Sender, Receiver)> {
    tokio::net::unix::pipe::pipe()
}

#[cfg(unix)]
pub fn duplicate_writer(sender: &Sender) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::AsFd;
    sender.as_fd().try_clone_to_owned()
}

#[cfg(windows)]
pub fn duplicate_writer(sender: &Sender) -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::AsHandle;
    sender.as_handle().try_clone_to_owned()
}

#[cfg(windows)]
pub async fn pipe() -> std::io::Result<(Sender, Receiver)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    static NEXT_PIPE: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        r"\\.\pipe\minicore-host-test-{}-{}",
        std::process::id(),
        NEXT_PIPE.fetch_add(1, Ordering::Relaxed),
    );
    let sender = ServerOptions::new()
        .first_pipe_instance(true)
        .access_inbound(false)
        .access_outbound(true)
        .reject_remote_clients(true)
        .create(&name)?;
    let receiver = ClientOptions::new().read(true).write(false).open(&name)?;
    sender.connect().await?;
    Ok((sender, receiver))
}
