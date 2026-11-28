//! Incremental SSE framing, based on the Agent adapter's CR/LF and multiline
//! handling (see PROVENANCE.md). No spawned reader or unbounded frame queue:
//! the consumer advances through a single network chunk and frame at a time.

use std::pin::Pin;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use minicore_host_runtime::model::ModelErrorKind;

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

pub(crate) struct Frames {
    bytes: ByteStream,
    chunk: Bytes,
    offset: usize,
    parser: Parser,
}

impl Frames {
    pub(crate) fn new(bytes: ByteStream) -> Self {
        Self {
            bytes,
            chunk: Bytes::new(),
            offset: 0,
            parser: Parser::new(),
        }
    }

    pub(crate) async fn frame(&mut self) -> Result<Bytes, ModelErrorKind> {
        loop {
            while self.offset < self.chunk.len() {
                let byte = self.chunk[self.offset];
                self.offset += 1;
                if let Some(frame) = self.parser.push(byte) {
                    return Ok(Bytes::from(frame));
                }
            }
            self.chunk = match self.bytes.next().await {
                Some(Ok(chunk)) => chunk,
                Some(Err(error)) if error.is_timeout() => return Err(ModelErrorKind::Timeout),
                Some(Err(_)) => return Err(ModelErrorKind::StreamInterrupted),
                None => return Err(ModelErrorKind::IncompleteResponse),
            };
            self.offset = 0;
        }
    }
}

#[derive(Default)]
struct Parser {
    line: Vec<u8>,
    data: Vec<u8>,
    has_data: bool,
    skip_lf: bool,
    first_line: bool,
}

impl Parser {
    fn new() -> Self {
        Self {
            first_line: true,
            ..Self::default()
        }
    }

    fn push(&mut self, byte: u8) -> Option<Vec<u8>> {
        if std::mem::take(&mut self.skip_lf) && byte == b'\n' {
            return None;
        }
        match byte {
            b'\r' => {
                self.skip_lf = true;
                self.end_line()
            }
            b'\n' => self.end_line(),
            _ => {
                self.line.push(byte);
                None
            }
        }
    }

    fn end_line(&mut self) -> Option<Vec<u8>> {
        let mut line = std::mem::take(&mut self.line);
        if std::mem::take(&mut self.first_line) && line.starts_with(&[0xef, 0xbb, 0xbf]) {
            line.drain(..3);
        }
        if line.is_empty() {
            if !std::mem::take(&mut self.has_data) {
                return None;
            }
            self.data.pop(); // exactly the newline added after the final data field
            return Some(std::mem::take(&mut self.data));
        }
        if line.first() == Some(&b':') {
            return None;
        }
        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(index) => (&line[..index], &line[index + 1..]),
            None => (line.as_slice(), b"".as_slice()),
        };
        if field == b"data" {
            self.data
                .extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
            self.data.push(b'\n');
            self.has_data = true;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fragmented_frames_keep_order_and_drop_unterminated_tail() {
        let bytes = b"\xef\xbb\xbf: hello\r\ndata: {\r\ndata: \"a\":1}\r\n\r\ndata: [DONE]\n\ndata: unfinished";
        for split in 0..=bytes.len() {
            let chunks =
                [&bytes[..split], &bytes[split..]].map(|chunk| Ok(Bytes::copy_from_slice(chunk)));
            let mut frames = Frames::new(Box::pin(futures_util::stream::iter(chunks)));
            assert_eq!(frames.frame().await.unwrap(), "{\n\"a\":1}");
            assert_eq!(frames.frame().await.unwrap(), "[DONE]");
            assert_eq!(
                frames.frame().await.unwrap_err(),
                ModelErrorKind::IncompleteResponse
            );
        }
    }
}
