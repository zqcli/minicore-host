use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::wire::{IncompleteDetails, Item, ProviderUsage};

pub(super) const FORMAT: &str = "minicore-host.openai-responses.v1";
pub(super) const PARTIAL_FORMAT: &str = "minicore-host.openai-responses.partial.v1";

/// Only native metadata and references into Runtime's canonical text are kept.
/// There is no second cumulative assistant text or function-argument buffer.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Replay {
    pub model: String,
    pub response_id: Option<String>,
    pub status: Option<String>,
    pub text: Checksum,
    pub reasoning: Checksum,
    pub items: Vec<ReplayItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProviderUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_tool_header: Option<Item>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ReplayItem {
    Message {
        id: String,
        phase: Option<String>,
        content: Vec<ReplayText>,
    },
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
    },
    Reasoning {
        item: Item,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReplayText {
    pub kind: String,
    pub ranges: Vec<TextRange>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TextRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checksum {
    pub bytes: usize,
    pub sha256: [u8; 32],
}

#[derive(Clone, Default)]
pub(super) struct Fingerprint {
    hash: Sha256,
    pub bytes: usize,
}

impl Fingerprint {
    pub fn push(&mut self, value: &str) -> Result<(), ()> {
        self.bytes = self.bytes.checked_add(value.len()).ok_or(())?;
        self.hash.update(value.as_bytes());
        Ok(())
    }

    pub fn checksum(&self) -> Checksum {
        Checksum {
            bytes: self.bytes,
            sha256: self.hash.clone().finalize().into(),
        }
    }

    pub fn matches(&self, value: &str) -> bool {
        self.bytes == value.len() && self.checksum().sha256 == Sha256::digest(value.as_bytes())[..]
    }
}

/// Counts/hashes precisely the normalization performed by Runtime's assembler,
/// including a CR/LF pair split across deltas. It never stores the text.
#[derive(Default)]
pub(super) struct Normalized {
    pub fingerprint: Fingerprint,
    ended_with_cr: bool,
}

impl Normalized {
    pub fn push(&mut self, raw: &str) -> Result<TextRange, ()> {
        let start = self.fingerprint.bytes;
        if raw.is_empty() {
            return Ok(TextRange { start, end: start });
        }
        let value = if self.ended_with_cr {
            raw.strip_prefix('\n').unwrap_or(raw)
        } else {
            raw
        };
        self.ended_with_cr = raw.ends_with('\r');
        let mut chars = value.char_indices().peekable();
        let mut unchanged = 0;
        while let Some((index, ch)) = chars.next() {
            if !ch.is_control() || matches!(ch, '\n' | '\t') {
                continue;
            }
            self.fingerprint.push(&value[unchanged..index])?;
            unchanged = index + ch.len_utf8();
            if ch == '\r' {
                if let Some((next, '\n')) = chars.peek().copied() {
                    chars.next();
                    unchanged = next + 1;
                }
                self.fingerprint.push("\n")?;
            } else {
                self.fingerprint.push(" ")?;
            }
        }
        // Ordinary UTF-8 deltas reach SHA-256 in one update; only actual
        // normalization boundaries split a run. No cumulative copy is made.
        self.fingerprint.push(&value[unchanged..])?;
        Ok(TextRange {
            start,
            end: self.fingerprint.bytes,
        })
    }
}

pub(super) fn append_range(ranges: &mut Vec<TextRange>, range: TextRange) {
    if range.start == range.end {
        return;
    }
    if let Some(previous) = ranges.last_mut()
        && previous.end == range.start
    {
        previous.end = range.end;
    } else {
        ranges.push(range);
    }
}
