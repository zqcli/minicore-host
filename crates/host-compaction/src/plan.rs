use crate::{CompactionError, source::exchange_ranges};
use minicore_host_protocol::CompactionSettings;
use minicore_host_runtime::{
    HistoryItem, SummaryHistory,
    model::{ModelDescriptor, ModelMessage, ModelRequest},
};
use std::{io, ops::Range, sync::Arc};

/// An immutable view with the Session owner's history version, not its general
/// controller revision. Queued messages are not consumed history.
#[derive(Clone, Debug)]
pub struct HistorySnapshot {
    pub revision: u64,
    pub items: Arc<[HistoryItem]>,
}

/// UTF-8 JSON bytes / 4 is a scheduling estimate, not a provider tokenizer,
/// reported usage, admission limit or permission to truncate any data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextEstimate {
    pub input_tokens: u64,
    pub context_window: u64,
    pub trigger_tokens: u64,
    pub target_tokens: u64,
    pub automatic_due: bool,
}

#[derive(Clone, Debug)]
pub struct CompactionPlan {
    pub(crate) source: HistorySnapshot,
    pub(crate) covered_end: usize,
    pub(crate) groups: Vec<Range<usize>>,
    pub(crate) descriptor: ModelDescriptor,
    pub(crate) original: ModelRequest,
    pub(crate) message_offset: usize,
    pub(crate) summary_target: u64,
    pub(crate) estimate: ContextEstimate,
}
impl CompactionPlan {
    pub fn source(&self) -> &HistorySnapshot {
        &self.source
    }
    pub fn covered(&self) -> Range<usize> {
        0..self.covered_end
    }
    pub fn estimate(&self) -> ContextEstimate {
        self.estimate
    }
    /// A candidate projection uses the exact fixed prefix and untouched tail
    /// of the caller's actual model request. Runtime validates the exchanges.
    pub(crate) fn project(
        &self,
        summary: &SummaryHistory,
    ) -> Result<ModelRequest, CompactionError> {
        let mut messages = self.original.messages()[..self.message_offset].to_vec();
        messages.push(summary_message(summary)?);
        messages
            .extend_from_slice(&self.original.messages()[self.message_offset + self.covered_end..]);
        ModelRequest::new(
            messages,
            self.original.tools().to_vec(),
            *self.original.limits(),
            self.original.reasoning(),
        )
        .map_err(|_| CompactionError::InvalidHistory)
    }
}

/// Generated content and exact source evidence. No persistence or cancellation
/// race has been settled by constructing this value.
#[derive(Clone, Debug)]
pub struct CompactionCandidate {
    pub(crate) source: HistorySnapshot,
    pub(crate) covered_end: usize,
    pub summary: SummaryHistory,
    pub request: ModelRequest,
    pub before_tokens: u64,
    pub after_tokens: u64,
    pub utility_usage: Vec<crate::UtilityUsage>,
}
impl CompactionCandidate {
    pub fn source(&self) -> &HistorySnapshot {
        &self.source
    }
    pub fn covered(&self) -> Range<usize> {
        0..self.covered_end
    }
    /// The owner must check its own original operation/configuration and
    /// cancellation fences too. This is only immutable history evidence.
    pub fn matches(&self, current: &HistorySnapshot) -> bool {
        self.source.revision == current.revision
            && (Arc::ptr_eq(&self.source.items, &current.items)
                || self.source.items == current.items)
    }
    /// Returns a new effective context. The source history is never modified.
    /// Persisting this projection is the caller's separate responsibility.
    pub fn history(&self) -> Vec<HistoryItem> {
        let mut result = vec![HistoryItem::Summary(self.summary.clone())];
        result.extend_from_slice(&self.source.items[self.covered_end..]);
        result
    }
}

/// Plan a manual or automatic prefix compaction. `request` must end with one
/// message for each `source.items` entry, in the same order, after any fixed
/// instructions. Incomplete old calls must already have the model-appropriate
/// projection; this module does not rewrite the normal prompt policy.
///
/// `preserve_from` protects that item and every following item. At an automatic
/// boundary this protects the newly appended inputs not yet handed to the
/// Session owner. It is not the public Turn's starting position: older accepted
/// inputs and completed exchanges within a long Turn may be summarized too.
/// An index inside an exchange rounds toward older history. For manual use,
/// pass the history length. At least the final exchange/item is always kept.
/// `None` means there is no eligible older prefix, not that history was changed.
///
/// The trigger and target are soft policy signals. Manual use is allowed below
/// the trigger or when automatic compaction is disabled. A large indivisible
/// item is never truncated or rejected by this estimate.
pub fn prepare(
    source: HistorySnapshot,
    preserve_from: usize,
    descriptor: &ModelDescriptor,
    settings: &CompactionSettings,
    request: ModelRequest,
) -> Result<Option<CompactionPlan>, CompactionError> {
    let estimate = estimate_context(descriptor, settings, &request)?;
    let length = source.items.len();
    if preserve_from > length || request.messages().len() < length {
        return Err(CompactionError::InvalidHistory);
    }
    let message_offset = request.messages().len() - length;
    let groups = exchange_ranges(&source.items)?;
    if groups.len() < 2 {
        return Ok(None);
    }
    let fixed = estimate_messages(&request.messages()[..message_offset])?
        .saturating_add(estimate_tools(&request)?);
    // Divide the desired reduced history between summary and recent context.
    // Neither share is a hard cap; complete groups and protected inputs win.
    let tail_target = estimate.target_tokens.saturating_sub(fixed) / 2;
    let mut kept = groups.len() - 1;
    while kept > 0 && groups[kept].start > preserve_from {
        kept -= 1;
    }
    let mut tail_tokens =
        estimate_messages(&request.messages()[message_offset + groups[kept].start..])?;
    while kept > 0 {
        let previous = &groups[kept - 1];
        let tokens = estimate_messages(
            &request.messages()[message_offset + previous.start..message_offset + previous.end],
        )?;
        if tail_tokens.saturating_add(tokens) > tail_target {
            break;
        }
        kept -= 1;
        tail_tokens = tail_tokens.saturating_add(tokens);
    }
    if kept == 0 {
        return Ok(None);
    }
    let covered_end = groups[kept].start;
    let summary_target = estimate
        .target_tokens
        .saturating_sub(fixed)
        .saturating_sub(tail_tokens);
    Ok(Some(CompactionPlan {
        source,
        covered_end,
        groups: groups[..kept].to_vec(),
        descriptor: descriptor.clone(),
        original: request,
        message_offset,
        summary_target,
        estimate,
    }))
}

/// Estimate the actual prepared request and derive the Session's soft trigger
/// and target from the current model/configured context. Works even when no
/// prefix can be compacted, so context queries need not manufacture a plan.
pub fn estimate_context(
    descriptor: &ModelDescriptor,
    settings: &CompactionSettings,
    request: &ModelRequest,
) -> Result<ContextEstimate, CompactionError> {
    settings
        .validate()
        .map_err(|_| CompactionError::InvalidConfiguration)?;
    descriptor
        .validate()
        .map_err(|_| CompactionError::InvalidConfiguration)?;
    if !descriptor.supports_reasoning(request.reasoning())
        || (!descriptor.supports_tools && !request.tools().is_empty())
    {
        return Err(CompactionError::InvalidConfiguration);
    }
    let context_window = request
        .limits()
        .context_window_tokens()
        .map_or(descriptor.context_window, |value| {
            descriptor.context_window.min(u64::from(value))
        });
    let percent = |value: u8| ((u128::from(context_window) * u128::from(value)) / 100) as u64;
    let input_tokens = estimate_request_tokens(request)?;
    Ok(ContextEstimate {
        input_tokens,
        context_window,
        trigger_tokens: percent(settings.trigger_percent),
        target_tokens: percent(settings.target_percent),
        automatic_due: settings.enabled && input_tokens >= percent(settings.trigger_percent),
    })
}

pub fn estimate_request_tokens(request: &ModelRequest) -> Result<u64, CompactionError> {
    Ok(serialized_request_bytes(request)?.div_ceil(4))
}

pub(crate) fn serialized_request_bytes(request: &ModelRequest) -> Result<u64, CompactionError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, request).map_err(|_| CompactionError::Serialization)?;
    Ok(counter.0)
}

/// Bytes added inside a JSON string, excluding its two enclosing quotes.
pub(crate) fn serialized_string_content_bytes(text: &str) -> Result<u64, CompactionError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, text).map_err(|_| CompactionError::Serialization)?;
    Ok(counter.0.saturating_sub(2))
}
fn estimate_messages(messages: &[ModelMessage]) -> Result<u64, CompactionError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, messages).map_err(|_| CompactionError::Serialization)?;
    Ok(counter.tokens())
}
fn estimate_tools(request: &ModelRequest) -> Result<u64, CompactionError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, request.tools())
        .map_err(|_| CompactionError::Serialization)?;
    Ok(counter.tokens())
}

#[derive(Default)]
struct ByteCounter(u64);
impl ByteCounter {
    fn tokens(&self) -> u64 {
        self.0.div_ceil(4)
    }
}
impl io::Write for ByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buffer.len() as u64);
        Ok(buffer.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn summary_message(summary: &SummaryHistory) -> Result<ModelMessage, CompactionError> {
    // Keep Runtime's standard SummaryHistory projection so a later reload
    // produces the same message as this candidate's normal model request.
    ModelMessage::system(format!("Conversation summary:\n{}", summary.content))
        .map_err(|_| CompactionError::InvalidResponse)
}
