#![allow(dead_code)]
//! Native byte pipes keep endpoint closure and backpressure under OS control.

#[cfg(unix)]
pub type Sender = tokio::net::unix::pipe::Sender;
#[cfg(unix)]
pub type Receiver = tokio::net::unix::pipe::Receiver;
#[cfg(windows)]
pub type Sender = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(windows)]
pub type Receiver = tokio::net::windows::named_pipe::NamedPipeServer;

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

#[cfg(unix)]
pub fn close_reader(receiver: Receiver) -> std::io::Result<()> {
    drop(receiver);
    Ok(())
}

#[cfg(windows)]
pub fn close_reader(receiver: Receiver) -> std::io::Result<()> {
    // Mio may retain a Windows handle until cancelled overlapped reads finish.
    // Disconnect first so success proves the peer can no longer write, without
    // depending on the later IOCP completion that releases the final handle.
    let result = receiver.disconnect();
    drop(receiver);
    result
}

pub fn assert_reader_closed_error(error: &std::io::Error) {
    #[cfg(unix)]
    assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    // A disconnected Windows client reports one of these native pipe states:
    // ERROR_BROKEN_PIPE, ERROR_NO_DATA, or ERROR_PIPE_NOT_CONNECTED. Rust 1.90
    // maps the first two to BrokenPipe but leaves 233 uncategorized.
    #[cfg(windows)]
    assert!(
        matches!(error.raw_os_error(), Some(109 | 232 | 233)),
        "expected a closed native pipe, got {error:?}"
    );
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
    let receiver = ServerOptions::new()
        .first_pipe_instance(true)
        .access_inbound(true)
        .access_outbound(false)
        .reject_remote_clients(true)
        .create(&name)?;
    let sender = ClientOptions::new().read(false).write(true).open(&name)?;
    receiver.connect().await?;
    Ok((sender, receiver))
}
