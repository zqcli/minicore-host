use super::apply::{HunkRange, PatchContent, PatchError, PatchLineKind, ResolvedHunk, add, sub};
use minicore_host_runtime::tools::ToolContext;

use crate::check;

pub(super) const NO_NEWLINE_AT_END: &str = "\\ No newline at end of file";

pub(super) fn parse_unified_patch<'a>(
    path: &str,
    patch: &'a str,
    context: &ToolContext,
) -> Result<Vec<ResolvedHunk<'a>>, PatchError> {
    check(context)?;
    let mut parser = PatchLines::new(patch);
    let mut first = parser.next().ok_or(PatchError::Invalid("patch is empty"))?;
    if first.starts_with("diff --git ") || first.starts_with("index ") {
        if first.starts_with("diff --git ") {
            first = parser.next().ok_or(PatchError::Invalid(
                "incomplete diff header: expected '--- ' after 'diff --git'",
            ))?;
        }
        if first.starts_with("index ") {
            first = parser.next().ok_or(PatchError::Invalid(
                "incomplete diff header: expected '--- ' after 'index'",
            ))?;
        }
        // Git metadata is informational; the ---/+++ paths still authorize
        // the single file this patch may change.
        if !first.starts_with("--- ") {
            return Err(PatchError::Invalid(
                "expected '--- <path>' header after diff metadata (binary patches are not supported)",
            ));
        }
    }
    let mut next_hunk = if strip_transport_ending(first).starts_with("--- ") {
        let original = parse_header_path(first, "--- ", context)?;
        let modified = parse_header_path(
            parser.next().ok_or(PatchError::Invalid(
                "missing '+++ <path>' header after '--- '",
            ))?,
            "+++ ",
            context,
        )?;
        validate_header_path(&original, path, "a/")?;
        validate_header_path(&modified, path, "b/")?;
        parser.next().ok_or(PatchError::Invalid(
            "patch has file headers but no '@@' hunks",
        ))?
    } else {
        first
    };

    let mut resolved = Vec::new();
    let mut previous_old_end = 0usize;
    let mut previous_new_end = 0usize;

    loop {
        check(context)?;
        let (old_range, new_range) = parse_hunk_header(strip_transport_ending(next_hunk))?;
        if old_range.index < previous_old_end || new_range.index < previous_new_end {
            return Err(PatchError::Invalid(
                "patch hunks overlap or are out of order",
            ));
        }
        previous_old_end = old_range.end;
        previous_new_end = new_range.end;

        let contents = parse_hunk_contents(&mut parser, old_range.count, new_range.count, context)?;
        resolved.push(ResolvedHunk {
            old: old_range,
            new: new_range,
            contents,
        });
        match parser.next() {
            Some(line) if strip_transport_ending(line).starts_with("@@ ") => {
                next_hunk = line;
            }
            Some(line) if strip_transport_ending(line).is_empty() => {
                parser.finish(context)?;
                break;
            }
            Some(_) => {
                return Err(PatchError::Invalid(
                    "unexpected trailing content or second file header; apply_patch accepts only a single-file patch",
                ));
            }
            None => break,
        }
    }
    Ok(resolved)
}

pub(super) struct PatchLines<'a> {
    lines: std::str::SplitInclusive<'a, char>,
    peeked: Option<&'a str>,
}

impl<'a> PatchLines<'a> {
    pub(super) fn new(patch: &'a str) -> Self {
        Self {
            lines: patch.split_inclusive('\n'),
            peeked: None,
        }
    }

    pub(super) fn peek(&mut self) -> Option<&'a str> {
        if self.peeked.is_none() {
            self.peeked = self.lines.next();
        }
        self.peeked
    }

    pub(super) fn finish(&mut self, context: &ToolContext) -> Result<(), PatchError> {
        while let Some(line) = self.next() {
            check(context)?;
            if !strip_transport_ending(line).is_empty() {
                return Err(PatchError::Invalid(
                    "unexpected trailing content after patch end",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn next(&mut self) -> Option<&'a str> {
        self.peeked.take().or_else(|| self.lines.next())
    }
}

pub(super) fn make_hunk_range(index: usize, count: usize) -> Result<HunkRange, PatchError> {
    let end = add(index, count)?;
    Ok(HunkRange { index, count, end })
}

fn parse_hunk_contents<'a>(
    parser: &mut PatchLines<'a>,
    expected_old: usize,
    expected_new: usize,
    context: &ToolContext,
) -> Result<Vec<PatchContent<'a>>, PatchError> {
    let mut contents = Vec::new();
    let mut old_count = 0usize;
    let mut new_count = 0usize;
    while old_count < expected_old || new_count < expected_new {
        check(context)?;
        let raw = parser.next().ok_or(PatchError::Invalid(
            "unified diff hunk ended early: line count does not match its '@@' header",
        ))?;
        let line = strip_transport_ending(raw);
        if line == NO_NEWLINE_AT_END {
            return Err(PatchError::Invalid(
                "unexpected '\\ No newline at end of file' without a preceding hunk line",
            ));
        }
        let mut content = parse_patch_content(line)?;
        let (old, new) = content.kind.delta();
        old_count = add(old_count, old)?;
        new_count = add(new_count, new)?;
        if old_count > expected_old || new_count > expected_new {
            return Err(PatchError::Invalid(
                "unified diff hunk exceeds line counts declared in its '@@' header",
            ));
        }
        consume_no_newline(parser, &mut content)?;
        contents.push(content);
    }
    if contents.is_empty() {
        return Err(PatchError::Invalid(
            "empty patch hunk: each '@@' hunk must include context or change lines",
        ));
    }
    Ok(contents)
}

pub(super) fn parse_patch_content(line: &str) -> Result<PatchContent<'_>, PatchError> {
    let (kind, text) = if line.is_empty() {
        (PatchLineKind::Context, "")
    } else if let Some(text) = line.strip_prefix(' ') {
        (PatchLineKind::Context, text)
    } else if let Some(text) = line.strip_prefix('-') {
        (PatchLineKind::Remove, text)
    } else if let Some(text) = line.strip_prefix('+') {
        (PatchLineKind::Add, text)
    } else {
        return Err(PatchError::Invalid(
            "invalid patch line prefix: every hunk line must start with ' ', '-', or '+'",
        ));
    };
    Ok(PatchContent {
        kind,
        text,
        old_no_newline: false,
        new_no_newline: false,
        context_new_ending: None,
    })
}

fn mark_no_newline(content: &mut PatchContent<'_>) -> Result<(), PatchError> {
    if content.old_no_newline || content.new_no_newline {
        return Err(PatchError::Invalid(
            "duplicate '\\ No newline at end of file' marker",
        ));
    }
    match content.kind {
        PatchLineKind::Context => {
            content.old_no_newline = true;
            content.new_no_newline = true;
        }
        PatchLineKind::Remove => content.old_no_newline = true,
        PatchLineKind::Add => content.new_no_newline = true,
    }
    Ok(())
}

pub(super) fn parse_hunk_header(header: &str) -> Result<(HunkRange, HunkRange), PatchError> {
    let header = header.strip_prefix("@@ ").ok_or(PatchError::Invalid(
        "invalid hunk header: expected '@@ -start[,count] +start[,count] @@'",
    ))?;
    let closing = header.find(" @@").ok_or(PatchError::Invalid(
        "invalid hunk header: missing closing ' @@'",
    ))?;
    let ranges = &header[..closing];
    let suffix = &header[closing + 3..];
    if suffix.contains('\0') || (!suffix.is_empty() && !suffix.starts_with(' ')) {
        return Err(PatchError::Invalid(
            "invalid hunk header suffix after closing '@@'",
        ));
    }
    let (old, new) = ranges.split_once(' ').ok_or(PatchError::Invalid(
        "invalid hunk header: expected '-old +new' ranges",
    ))?;
    if old.contains(' ') || new.contains(' ') {
        return Err(PatchError::Invalid(
            "invalid hunk header: unexpected space inside line ranges",
        ));
    }
    Ok((parse_hunk_range(old, '-')?, parse_hunk_range(new, '+')?))
}

fn parse_hunk_range(range: &str, prefix: char) -> Result<HunkRange, PatchError> {
    let range = range.strip_prefix(prefix).ok_or(PatchError::Invalid(
        "invalid hunk header range prefix (expected '-' then '+')",
    ))?;
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    if !is_decimal(start) || !is_decimal(count) {
        return Err(PatchError::Invalid(
            "invalid hunk header: line numbers and counts must be unsigned integers",
        ));
    }
    let start = start
        .parse::<usize>()
        .map_err(|_| PatchError::Invalid("patch hunk line number overflow"))?;
    let count = count
        .parse::<usize>()
        .map_err(|_| PatchError::Invalid("patch hunk line count overflow"))?;
    if start == 0 && count != 0 {
        return Err(PatchError::Invalid(
            "invalid hunk header: start line 0 is only allowed when line count is 0",
        ));
    }
    let index = if count == 0 { start } else { sub(start, 1)? };
    let end = add(index, count)?;
    add(start, count)?;
    Ok(HunkRange { index, count, end })
}

fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_header_path(
    line: &str,
    prefix: &str,
    context: &ToolContext,
) -> Result<String, PatchError> {
    let line = strip_transport_ending(line);
    let value = line.strip_prefix(prefix).ok_or(PatchError::Invalid(
        "expected matching '--- ' and '+++ ' file headers",
    ))?;
    let path = if value.starts_with('"') {
        decode_quoted_path(value, context)?
    } else {
        value
            .split_once('\t')
            .map_or(value, |(path, _)| path)
            .to_owned()
    };
    if path.is_empty() || path.contains('\0') {
        return Err(PatchError::Invalid("patch header path is empty or invalid"));
    }
    Ok(path)
}

fn decode_quoted_path(value: &str, context: &ToolContext) -> Result<String, PatchError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 1usize;
    let mut closed = false;
    while index < bytes.len() {
        check(context)?;
        match bytes[index] {
            b'"' => {
                index += 1;
                closed = true;
                break;
            }
            b'\\' => {
                index += 1;
                let escaped = *bytes.get(index).ok_or(PatchError::Invalid(
                    "invalid escape sequence in quoted patch header path",
                ))?;
                match escaped {
                    b'\\' | b'"' => {
                        decoded.push(escaped);
                        index += 1;
                    }
                    b'a' | b'b' | b't' | b'n' | b'v' | b'f' | b'r' => {
                        decoded.push(match escaped {
                            b'a' => 0x07,
                            b'b' => 0x08,
                            b't' => b'\t',
                            b'n' => b'\n',
                            b'v' => 0x0b,
                            b'f' => 0x0c,
                            b'r' => b'\r',
                            _ => unreachable!(),
                        });
                        index += 1;
                    }
                    b'0'..=b'7' => {
                        let mut value = 0u16;
                        let mut digits = 0usize;
                        while digits < 3
                            && index < bytes.len()
                            && matches!(bytes[index], b'0'..=b'7')
                        {
                            value = value
                                .checked_mul(8)
                                .and_then(|value| value.checked_add(u16::from(bytes[index] - b'0')))
                                .ok_or(PatchError::Invalid(
                                    "invalid octal escape in quoted patch header path",
                                ))?;
                            index += 1;
                            digits += 1;
                        }
                        decoded.push(u8::try_from(value).map_err(|_| {
                            PatchError::Invalid("invalid octal byte in quoted patch header path")
                        })?);
                    }
                    _ => {
                        return Err(PatchError::Invalid(
                            "unsupported escape sequence in quoted patch header path",
                        ));
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    if !closed || (index < bytes.len() && bytes[index] != b'\t') {
        return Err(PatchError::Invalid("unterminated quoted patch header path"));
    }
    let path = String::from_utf8(decoded)
        .map_err(|_| PatchError::Invalid("quoted patch header path is not valid UTF-8"))?;
    if path.contains('\0') {
        return Err(PatchError::Invalid("patch header path contains NUL"));
    }
    Ok(path)
}

pub(super) fn validate_header_path(
    header: &str,
    tool_path: &str,
    prefix: &str,
) -> Result<(), PatchError> {
    let candidate = if header == tool_path {
        header
    } else {
        header.strip_prefix(prefix).unwrap_or(header)
    };
    if candidate != tool_path || candidate.is_empty() || candidate.contains('\0') {
        return Err(PatchError::Invalid(
            "patch header path does not match the 'path' argument (Add/Delete/Move and multi-file patches are not supported)",
        ));
    }
    Ok(())
}

pub(super) fn strip_transport_ending(line: &str) -> &str {
    let Some(line) = line.strip_suffix('\n') else {
        return line;
    };
    line.strip_suffix('\r').unwrap_or(line)
}

pub(super) fn consume_no_newline(
    parser: &mut PatchLines<'_>,
    content: &mut PatchContent<'_>,
) -> Result<(), PatchError> {
    if parser
        .peek()
        .is_some_and(|line| strip_transport_ending(line) == NO_NEWLINE_AT_END)
    {
        parser.next();
        mark_no_newline(content)?;
        if parser
            .peek()
            .is_some_and(|line| strip_transport_ending(line) == NO_NEWLINE_AT_END)
        {
            return Err(PatchError::Invalid(
                "duplicate '\\ No newline at end of file' marker",
            ));
        }
    }
    Ok(())
}
