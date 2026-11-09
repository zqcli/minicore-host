//! Incremental SSE framing, based on the Agent adapter's CR/LF and multiline
//! handling (see PROVENANCE.md). No spawned reader or unbounded frame queue:
//! the consumer advances through a single network chunk and frame at a time.

#[derive(Default)]
pub(crate) struct Parser {
    line: Vec<u8>,
    data: Vec<u8>,
    has_data: bool,
    skip_lf: bool,
    first_line: bool,
}

impl Parser {
    pub(crate) fn new() -> Self {
        Self {
            first_line: true,
            ..Self::default()
        }
    }

    pub(crate) fn push(&mut self, byte: u8) -> Option<Vec<u8>> {
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

    #[test]
    fn framing_handles_bom_comments_crlf_multiline_and_fragmentation() {
        let bytes = b"\xef\xbb\xbf: hello\r\ndata: {\r\ndata: \"a\":1}\r\n\r\ndata: [DONE]\n\n";
        for split in 0..=bytes.len() {
            let mut parser = Parser::new();
            let frames: Vec<_> = bytes[..split]
                .iter()
                .chain(&bytes[split..])
                .filter_map(|byte| parser.push(*byte))
                .collect();
            assert_eq!(frames, [b"{\n\"a\":1}".to_vec(), b"[DONE]".to_vec()]);
        }
    }

    #[test]
    fn an_unterminated_frame_is_never_dispatched() {
        let mut parser = Parser::new();
        assert!(
            b"data: {\"choices\":[]}"
                .iter()
                .filter_map(|byte| parser.push(*byte))
                .next()
                .is_none()
        );
    }
}
