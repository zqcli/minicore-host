//! Complete-message framing shared by stdio and WSS adapters.
use std::io;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::mpsc,
};

/// A complete transport message, independent of stdio/WSS framing. No buffer
/// cap is imposed here; each framing adapter enforces only its actual syntax.
pub(crate) trait FrameReader: Send {
    fn read(&mut self) -> impl std::future::Future<Output = io::Result<Option<Vec<u8>>>> + Send;
}
pub(crate) trait FrameWriter: Send {
    fn write(&mut self, bytes: Vec<u8>)
    -> impl std::future::Future<Output = io::Result<()>> + Send;
    fn close(&mut self) -> impl std::future::Future<Output = io::Result<()>> + Send;
}
pub(super) struct LineReader<R> {
    pub(super) reader: BufReader<R>,
    pub(super) frame: Vec<u8>,
}
impl<R: AsyncRead + Unpin + Send> FrameReader for LineReader<R> {
    async fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        // Keep partial bytes in self so cancellation of a select branch cannot
        // drop them or start an independent deadline.
        if self.reader.read_until(b'\n', &mut self.frame).await? == 0 && self.frame.is_empty() {
            return Ok(None);
        }
        if self.frame.last() != Some(&b'\n') {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unterminated JSON-RPC frame",
            ));
        }
        Ok(Some(std::mem::take(&mut self.frame)))
    }
}
pub(super) struct LineWriter<W>(pub(super) W);
impl<W: AsyncWrite + Unpin + Send> FrameWriter for LineWriter<W> {
    async fn write(&mut self, mut bytes: Vec<u8>) -> io::Result<()> {
        bytes.push(b'\n');
        self.0.write_all(&bytes).await?;
        self.0.flush().await
    }
    async fn close(&mut self) -> io::Result<()> {
        self.0.shutdown().await
    }
}

pub(super) async fn write_frames(
    mut writer: impl FrameWriter,
    mut frames: mpsc::UnboundedReceiver<Vec<u8>>,
) -> io::Result<()> {
    while let Some(frame) = frames.recv().await {
        writer.write(frame).await?;
    }
    writer.close().await
}
