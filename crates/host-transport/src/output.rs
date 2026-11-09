//! Client-owned content only. Sources and execution owners are never retained.
use minicore_host_core::{ActivityEvent, ModelTextSnapshot, ObservationEvent, ToolTextSnapshot};
use minicore_host_protocol::*;
use std::collections::BTreeMap;

/// One exact model/ToolCall output channel. Reuse it across overlapping
/// subscriptions and reconnect baselines. Missing source data remains missing:
/// this helper does not retry, resubscribe, or reconstruct an execution.
pub struct OutputBuffer {
    source: OutputSource,
    channel: String,
    text: String,
    pending: BTreeMap<u64, String>,
    required_end: u64,
    finished: bool,
}
impl OutputBuffer {
    pub fn new(source: OutputSource, channel: String) -> Self {
        Self {
            source,
            channel,
            text: String::new(),
            pending: BTreeMap::new(),
            required_end: 0,
            finished: false,
        }
    }
    pub fn source(&self) -> &OutputSource {
        &self.source
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    /// A finish fact alone does not make a missing tail complete.
    pub fn is_complete(&self) -> bool {
        self.finished && self.text.len() as u64 >= self.required_end
    }
    pub fn position(&mut self, position: &OutputPosition) -> bool {
        if self.source != position.source || self.channel != position.channel {
            return false;
        }
        self.required_end = self.required_end.max(position.end);
        true
    }
    /// Return the currently known fixed gap, never a growing-tail request.
    /// The caller may keep receiving events while the read is in flight.
    pub fn next_read(&self, max_bytes: u64) -> Option<TextReadRequest> {
        let start = self.text.len() as u64;
        let end = self
            .pending
            .keys()
            .next()
            .copied()
            .unwrap_or(self.required_end);
        (start < end && max_bytes > 0).then(|| TextReadRequest {
            channel: self.channel.clone(),
            start,
            end,
            max_bytes,
        })
    }
    pub fn event(&mut self, event: &ObservationEvent) -> Result<bool, CallError> {
        let (source, delta, completion) = match event {
            ObservationEvent::Activity(ActivityEvent::ModelOutput { source, data }) => (
                OutputSource::Model {
                    source: source.clone(),
                },
                Some(data),
                None,
            ),
            ObservationEvent::Activity(ActivityEvent::ToolOutput { source, data }) => (
                OutputSource::Toolcall {
                    source: source.clone(),
                },
                Some(data),
                None,
            ),
            ObservationEvent::Activity(ActivityEvent::ModelFinished { source, data }) => (
                OutputSource::Model {
                    source: source.clone(),
                },
                None,
                Some(&data.output_positions),
            ),
            ObservationEvent::Activity(ActivityEvent::ToolFinished { source, data }) => (
                OutputSource::Toolcall {
                    source: source.clone(),
                },
                None,
                Some(&data.output_positions),
            ),
            _ => return Ok(false),
        };
        if self.source != source {
            return Ok(false);
        }
        if let Some(delta) = delta {
            if delta.channel != self.channel {
                return Ok(false);
            }
            delta
                .validate()
                .map_err(|error| invalid(&error.to_string()))?;
            self.merge(delta.start, delta.end, &delta.text)?;
        }
        if let Some(positions) = completion {
            if let Some(position) = positions
                .iter()
                .find(|position| position.channel == self.channel)
            {
                self.required_end = self.required_end.max(position.end);
                self.finished = true;
            } else {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn model_read(&mut self, snapshot: &ModelTextSnapshot) -> Result<(), CallError> {
        self.read(
            &OutputSource::Model {
                source: snapshot.source.clone(),
            },
            &snapshot.data,
        )
    }
    pub fn tool_read(&mut self, snapshot: &ToolTextSnapshot) -> Result<(), CallError> {
        self.read(
            &OutputSource::Toolcall {
                source: snapshot.source.clone(),
            },
            &snapshot.data,
        )
    }
    fn read(&mut self, source: &OutputSource, data: &TextRangeData) -> Result<(), CallError> {
        if source != &self.source || data.channel != self.channel {
            return Err(invalid("read belongs to another output source or channel"));
        }
        data.validate()
            .map_err(|error| invalid(&error.to_string()))?;
        self.merge(data.start, data.end, &data.text)
    }
    fn merge(&mut self, start: u64, end: u64, text: &str) -> Result<(), CallError> {
        verify_overlap(0, &self.text, start, text)?;
        for (&offset, held) in &self.pending {
            verify_overlap(offset, held, start, text)?;
        }
        self.required_end = self.required_end.max(end);
        if start <= self.text.len() as u64 {
            append_tail(&mut self.text, start, text)?;
            while let Some((&offset, _)) = self.pending.first_key_value() {
                if offset > self.text.len() as u64 {
                    break;
                }
                let held = self.pending.remove(&offset).expect("first buffered range");
                append_tail(&mut self.text, offset, &held)?;
            }
        } else if self
            .pending
            .get(&start)
            .is_none_or(|held| held.len() < text.len())
        {
            self.pending.insert(start, text.to_owned());
        }
        Ok(())
    }
}
fn append_tail(held: &mut String, start: u64, text: &str) -> Result<(), CallError> {
    let overlap = usize::try_from(held.len() as u64 - start)
        .map_err(|_| invalid("output range exceeds this platform"))?;
    if overlap < text.len() {
        held.push_str(
            text.get(overlap..)
                .ok_or_else(|| invalid("overlap splits a UTF-8 character"))?,
        );
    }
    Ok(())
}
fn verify_overlap(a: u64, left: &str, b: u64, right: &str) -> Result<(), CallError> {
    let start = a.max(b);
    let end = (a + left.len() as u64).min(b + right.len() as u64);
    if start < end {
        let left = left.get((start - a) as usize..(end - a) as usize);
        let right = right.get((start - b) as usize..(end - b) as usize);
        if left.is_none() || right.is_none() || left != right {
            return Err(invalid("overlapping output bytes disagree"));
        }
    }
    Ok(())
}
fn invalid(message: &str) -> CallError {
    CallError::Protocol {
        message: message.into(),
    }
}
