use std::{collections::VecDeque, pin::Pin};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use minicore_host_runtime::{
    ToolCallId,
    model::{ModelError, ModelErrorKind, ModelEvent, ModelFinishReason, ProviderReplay, Usage},
    tools::ToolName,
};
use serde_json::json;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    replay::{
        self, Fingerprint, Normalized, Replay, ReplayItem, ReplayText, TextRange, append_range,
    },
    wire::{self, Content, Event, Item, ProviderUsage, Response},
};
use crate::{error, sse::Parser};

type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

pub(super) struct State {
    bytes: ByteStream,
    status: u16,
    retry_at: Option<std::time::Instant>,
    chunk: Bytes,
    offset: usize,
    parser: Parser,
    pending: VecDeque<ModelEvent>,
    pending_error: Option<ModelError>,
    cancellation: CancellationToken,
    deadline: Instant,
    done: bool,
    seen: bool,
    sequence: Option<u64>,
    response_id: Option<String>,
    terminal_status: Option<String>,
    incomplete_details: Option<wire::IncompleteDetails>,
    usage: Option<ProviderUsage>,
    runtime_usage: Option<Usage>,
    model: String,
    items: Vec<Output>,
    incomplete_tool_header: Option<Item>,
    text: Normalized,
    reasoning: Normalized,
    refusal: bool,
}

struct Output {
    id: String,
    done: bool,
    body: Body,
}

enum Body {
    Message {
        phase: Option<String>,
        content: Vec<Part>,
    },
    Reasoning {
        summary: Vec<Part>,
        content: Vec<Part>,
        native: Option<Item>,
    },
    Tool {
        call_id: ToolCallId,
        name: ToolName,
        arguments: Fingerprint,
        arguments_done: bool,
    },
}

struct Part {
    kind: String,
    raw: Fingerprint,
    ranges: Vec<TextRange>,
    text_done: bool,
    done: bool,
}

impl Part {
    fn new(kind: String) -> Self {
        Self {
            kind,
            raw: Fingerprint::default(),
            ranges: Vec::new(),
            text_done: false,
            done: false,
        }
    }
}

impl State {
    pub(super) fn new(
        response: reqwest::Response,
        cancellation: CancellationToken,
        deadline: Instant,
        model: String,
    ) -> Self {
        let status = response.status().as_u16();
        let retry_at = error::retry_after(response.headers());
        Self {
            bytes: Box::pin(response.bytes_stream()),
            status,
            retry_at,
            chunk: Bytes::new(),
            offset: 0,
            parser: Parser::new(),
            pending: VecDeque::new(),
            pending_error: None,
            cancellation,
            deadline,
            done: false,
            seen: false,
            sequence: None,
            response_id: None,
            terminal_status: None,
            incomplete_details: None,
            usage: None,
            runtime_usage: None,
            model,
            items: Vec::new(),
            incomplete_tool_header: None,
            text: Normalized::default(),
            reasoning: Normalized::default(),
            refusal: false,
        }
    }

    fn malformed(&self) -> ModelError {
        error::stream(ModelErrorKind::InvalidProviderResponse, self.seen)
    }

    fn handle(&mut self, frame: &[u8]) -> Result<(), ModelError> {
        if frame == b"[DONE]" {
            return Err(error::stream(ModelErrorKind::IncompleteResponse, self.seen));
        }
        let frame = std::str::from_utf8(frame).map_err(|_| self.malformed())?;
        let event = match serde_json::from_str::<Event>(frame) {
            Ok(event) => event,
            Err(_) => {
                // Rejections take precedence over malformed normal fields. A
                // separate typed envelope still rejects repeated known fields.
                let envelope: wire::ErrorEnvelope =
                    serde_json::from_str(frame).map_err(|_| self.malformed())?;
                let nested = if envelope.error.is_none()
                    && envelope.kind.as_deref() == Some("response.failed")
                {
                    let response = serde_json::from_str::<wire::FailedEnvelope>(frame)
                        .map_err(|_| self.malformed())?
                        .response;
                    if let Some(response) = response {
                        self.seen |= response
                            .output
                            .as_ref()
                            .is_some_and(|output| !output.is_empty());
                        response.error
                    } else {
                        None
                    }
                } else {
                    None
                };
                let value = envelope.error.or(nested).or_else(|| {
                    (envelope.kind.as_deref() == Some("error"))
                        .then(|| json!({"code":envelope.code}))
                });
                return Err(match value {
                    Some(value) => error::provider(&value, self.seen, self.status, self.retry_at),
                    None => self.malformed(),
                });
            }
        };
        if let Some(value) = event.error.as_ref() {
            return Err(error::provider(
                value,
                self.seen,
                self.status,
                self.retry_at,
            ));
        }
        if event.kind == "error" {
            return Err(error::provider(
                &json!({"code":event.code}),
                self.seen,
                self.status,
                self.retry_at,
            ));
        }
        if event.kind == "response.failed" {
            let response = event.response.ok_or_else(|| self.malformed())?;
            let value = response.error.as_ref().ok_or_else(|| self.malformed())?;
            // A first-frame structured rejection may still prove NotStarted;
            // response.created or any actual output forbids that inference.
            let seen = self.seen || !response.output.is_empty();
            let failure = error::provider(value, seen, self.status, self.retry_at);
            self.terminal_metadata(&response, "failed")
                .map_err(|_| self.malformed())?;
            self.seen = seen;
            return Err(failure);
        }
        let sequence = event.sequence_number.ok_or_else(|| self.malformed())?;
        if self.sequence.is_some_and(|previous| sequence <= previous) {
            return Err(self.malformed());
        }
        self.sequence = Some(sequence);
        self.seen = true;
        let result = match event.kind.as_str() {
            "response.created" => self.created(event.response.ok_or(())),
            "response.in_progress" => self.in_progress(event.response.ok_or(())),
            "response.output_item.added" => {
                self.add_item(event.output_index.ok_or(()), event.item.ok_or(()))
            }
            "response.output_item.done" => {
                self.finish_item(event.output_index.ok_or(()), event.item.ok_or(()))
            }
            "response.content_part.added" => self.add_part(&event, false),
            "response.reasoning_summary_part.added" => self.add_part(&event, true),
            "response.content_part.done" => self.finish_part(&event, false),
            "response.reasoning_summary_part.done" => self.finish_part(&event, true),
            "response.output_text.delta" => self.text_delta(&event, false, "output_text"),
            "response.refusal.delta" => self.text_delta(&event, false, "refusal"),
            "response.reasoning_text.delta" => self.text_delta(&event, false, "reasoning_text"),
            "response.reasoning_summary_text.delta" => {
                self.text_delta(&event, true, "summary_text")
            }
            "response.output_text.done" => self.text_done(&event, false, "output_text"),
            "response.refusal.done" => self.text_done(&event, false, "refusal"),
            "response.reasoning_text.done" => self.text_done(&event, false, "reasoning_text"),
            "response.reasoning_summary_text.done" => self.text_done(&event, true, "summary_text"),
            "response.function_call_arguments.delta" => self.arguments_delta(&event),
            "response.function_call_arguments.done" => self.arguments_done(&event),
            "response.completed" => self.complete(event.response.ok_or(())),
            "response.incomplete" => {
                let response = event.response.ok_or_else(|| self.malformed())?;
                self.terminal_metadata(&response, "incomplete")
                    .map_err(|_| self.malformed())?;
                return Err(error::stream(ModelErrorKind::IncompleteResponse, self.seen));
            }
            _ => Err(()),
        };
        result.map_err(|_| self.malformed())
    }

    fn response_identity(&self, response: &Response) -> Result<(), ()> {
        if !wire::valid_id(&response.id)
            || self
                .response_id
                .as_ref()
                .is_some_and(|id| id != &response.id)
        {
            return Err(());
        }
        Ok(())
    }

    fn created(&mut self, response: Result<Response, ()>) -> Result<(), ()> {
        let response = response?;
        self.response_identity(&response)?;
        if self.response_id.is_some()
            || !self.items.is_empty()
            || response.status != "in_progress"
            || !response.output.is_empty()
            || response.error.is_some()
            || response.usage.is_some()
            || response.incomplete_details.is_some()
        {
            return Err(());
        }
        self.response_id = Some(response.id);
        Ok(())
    }

    fn in_progress(&self, response: Result<Response, ()>) -> Result<(), ()> {
        let response = response?;
        self.response_identity(&response)?;
        if self.response_id.is_none()
            || response.status != "in_progress"
            || !response.output.is_empty()
            || response.error.is_some()
            || response.usage.is_some()
            || response.incomplete_details.is_some()
        {
            return Err(());
        }
        Ok(())
    }

    fn add_item(&mut self, index: Result<usize, ()>, item: Result<Item, ()>) -> Result<(), ()> {
        let index = index?;
        let item = item?;
        if item.kind == "function_call" {
            self.incomplete_tool_header = Some(item.clone());
        }
        if self.response_id.is_none()
            || index != self.items.len()
            || !wire::valid_id(&item.id)
            || self.items.iter().any(|output| output.id == item.id)
            || item
                .status
                .as_deref()
                .is_some_and(|status| status != "in_progress")
        {
            return Err(());
        }
        let body = match item.kind.as_str() {
            "message" => {
                if item.role.as_deref() != Some("assistant")
                    || !wire::valid_phase(item.phase.as_deref())
                    || item
                        .content
                        .as_ref()
                        .is_none_or(|content| !content.is_empty())
                    || item.summary.is_some()
                    || item.encrypted_content.is_some()
                    || item.call_id.is_some()
                    || item.name.is_some()
                    || item.arguments.is_some()
                {
                    return Err(());
                }
                Body::Message {
                    phase: item.phase,
                    content: Vec::new(),
                }
            }
            "reasoning" => {
                if item.role.is_some()
                    || item.phase.is_some()
                    || item.call_id.is_some()
                    || item.name.is_some()
                    || item.arguments.is_some()
                    || item
                        .summary
                        .as_ref()
                        .is_none_or(|summary| !summary.is_empty())
                    || item
                        .content
                        .as_ref()
                        .is_some_and(|content| !content.is_empty())
                {
                    return Err(());
                }
                // Added encrypted_content can be provisional. Only the actual
                // completed item is eligible for native history replay.
                Body::Reasoning {
                    summary: Vec::new(),
                    content: Vec::new(),
                    native: None,
                }
            }
            "function_call" => {
                if item.role.is_some()
                    || item.phase.is_some()
                    || item.content.is_some()
                    || item.summary.is_some()
                    || item.encrypted_content.is_some()
                {
                    return Err(());
                }
                let call_id =
                    ToolCallId::new(item.call_id.as_deref().ok_or(())?).map_err(|_| ())?;
                let name: ToolName = item.name.as_deref().ok_or(())?.parse().map_err(|_| ())?;
                if self.items.iter().any(|output| matches!(&output.body, Body::Tool { call_id: previous, .. } if previous == &call_id)) { return Err(()); }
                let initial = item.arguments.as_deref().ok_or(())?;
                let mut arguments = Fingerprint::default();
                arguments.push(initial)?;
                self.pending.push_back(ModelEvent::ToolCallStart {
                    tool_call_id: call_id.clone(),
                    tool_name: name.clone(),
                });
                if !initial.is_empty() {
                    self.pending
                        .push_back(ModelEvent::tool_call_arguments_delta(
                            call_id.clone(),
                            initial,
                        ));
                }
                Body::Tool {
                    call_id,
                    name,
                    arguments,
                    arguments_done: false,
                }
            }
            _ => return Err(()),
        };
        self.incomplete_tool_header = None;
        self.items.push(Output {
            id: item.id,
            done: false,
            body,
        });
        Ok(())
    }

    fn reference(&self, event: &Event) -> Result<usize, ()> {
        let index = event.output_index.ok_or(())?;
        let output = self.items.get(index).ok_or(())?;
        if output.done || event.item_id.as_deref() != Some(output.id.as_str()) {
            return Err(());
        }
        Ok(index)
    }

    fn parts(&mut self, index: usize, summary: bool) -> Result<&mut Vec<Part>, ()> {
        match &mut self.items.get_mut(index).ok_or(())?.body {
            Body::Message { content, .. } if !summary => Ok(content),
            Body::Reasoning { summary: parts, .. } if summary => Ok(parts),
            Body::Reasoning { content, .. } => Ok(content),
            _ => Err(()),
        }
    }

    fn part_index(event: &Event, summary: bool) -> Result<usize, ()> {
        if summary {
            if event.content_index.is_some() {
                return Err(());
            }
            event.summary_index.ok_or(())
        } else {
            if event.summary_index.is_some() {
                return Err(());
            }
            event.content_index.ok_or(())
        }
    }

    fn add_part(&mut self, event: &Event, summary: bool) -> Result<(), ()> {
        let index = self.reference(event)?;
        let part_index = Self::part_index(event, summary)?;
        let part = event.part.as_ref().ok_or(())?;
        let text = part.text()?;
        let expected = match (&self.items[index].body, summary) {
            (Body::Message { .. }, false)
                if matches!(part.kind.as_str(), "output_text" | "refusal") =>
            {
                part.kind.as_str()
            }
            (Body::Reasoning { .. }, false) => "reasoning_text",
            (Body::Reasoning { .. }, true) => "summary_text",
            _ => return Err(()),
        };
        if part.kind != expected {
            return Err(());
        }
        let parts = self.parts(index, summary)?;
        if part_index != parts.len() || parts.last().is_some_and(|part| !part.done) {
            return Err(());
        }
        parts.push(Part::new(part.kind.clone()));
        self.append_text(index, part_index, summary, text)
    }

    fn append_text(
        &mut self,
        index: usize,
        part_index: usize,
        summary: bool,
        value: &str,
    ) -> Result<(), ()> {
        let reasoning = matches!(self.items[index].body, Body::Reasoning { .. });
        let part = self.parts(index, summary)?.get_mut(part_index).ok_or(())?;
        if part.done || part.text_done {
            return Err(());
        }
        part.raw.push(value)?;
        let refusal = part.kind == "refusal";
        let range = if reasoning {
            self.reasoning.push(value)?
        } else {
            self.text.push(value)?
        };
        if !reasoning {
            append_range(&mut self.parts(index, summary)?[part_index].ranges, range);
        }
        self.refusal |= refusal && !value.is_empty();
        if !value.is_empty() {
            self.pending.push_back(if reasoning {
                ModelEvent::reasoning_delta(value)
            } else {
                ModelEvent::text_delta(value)
            });
        }
        Ok(())
    }

    fn text_delta(&mut self, event: &Event, summary: bool, kind: &str) -> Result<(), ()> {
        let index = self.reference(event)?;
        let part_index = Self::part_index(event, summary)?;
        if self.parts(index, summary)?.get(part_index).ok_or(())?.kind != kind {
            return Err(());
        }
        self.append_text(
            index,
            part_index,
            summary,
            event.delta.as_deref().ok_or(())?,
        )
    }

    fn text_done(&mut self, event: &Event, summary: bool, kind: &str) -> Result<(), ()> {
        let index = self.reference(event)?;
        let part_index = Self::part_index(event, summary)?;
        let value = if kind == "refusal" {
            event.refusal.as_deref()
        } else {
            event.text.as_deref()
        }
        .ok_or(())?;
        let part = self.parts(index, summary)?.get_mut(part_index).ok_or(())?;
        if part.done || part.text_done || part.kind != kind || !part.raw.matches(value) {
            return Err(());
        }
        part.text_done = true;
        Ok(())
    }

    fn finish_part(&mut self, event: &Event, summary: bool) -> Result<(), ()> {
        let index = self.reference(event)?;
        let part_index = Self::part_index(event, summary)?;
        let snapshot = event.part.as_ref().ok_or(())?;
        let part = self.parts(index, summary)?.get_mut(part_index).ok_or(())?;
        if part.done
            || !part.text_done
            || part.kind != snapshot.kind
            || !part.raw.matches(snapshot.text()?)
        {
            return Err(());
        }
        part.done = true;
        Ok(())
    }

    fn arguments_delta(&mut self, event: &Event) -> Result<(), ()> {
        let index = self.reference(event)?;
        let value = event.delta.as_deref().ok_or(())?;
        let Body::Tool {
            call_id,
            arguments,
            arguments_done,
            ..
        } = &mut self.items[index].body
        else {
            return Err(());
        };
        if *arguments_done {
            return Err(());
        }
        arguments.push(value)?;
        self.pending
            .push_back(ModelEvent::tool_call_arguments_delta(
                call_id.clone(),
                value,
            ));
        Ok(())
    }

    fn arguments_done(&mut self, event: &Event) -> Result<(), ()> {
        let index = self.reference(event)?;
        let Body::Tool {
            arguments,
            arguments_done,
            ..
        } = &mut self.items[index].body
        else {
            return Err(());
        };
        if *arguments_done || !arguments.matches(event.arguments.as_deref().ok_or(())?) {
            return Err(());
        }
        *arguments_done = true;
        Ok(())
    }

    fn check_parts(parts: &[Part], snapshots: &[Content]) -> Result<(), ()> {
        if parts.len() != snapshots.len() {
            return Err(());
        }
        for (part, snapshot) in parts.iter().zip(snapshots) {
            if !part.done || part.kind != snapshot.kind || !part.raw.matches(snapshot.text()?) {
                return Err(());
            }
        }
        Ok(())
    }

    fn check_item(&self, index: usize, item: &Item, terminal: bool) -> Result<(), ()> {
        let output = self.items.get(index).ok_or(())?;
        if output.id != item.id || output.done != terminal {
            return Err(());
        }
        match &output.body {
            Body::Message { phase, content } => {
                if content.is_empty()
                    || item.kind != "message"
                    || item.role.as_deref() != Some("assistant")
                    || item.status.as_deref() != Some("completed")
                    || !wire::valid_phase(item.phase.as_deref())
                    || (phase.is_some() && phase != &item.phase)
                    || (terminal && phase != &item.phase)
                    || item.summary.is_some()
                    || item.encrypted_content.is_some()
                    || item.call_id.is_some()
                    || item.name.is_some()
                    || item.arguments.is_some()
                {
                    return Err(());
                }
                Self::check_parts(content, item.content.as_deref().ok_or(())?)
            }
            Body::Tool {
                call_id,
                name,
                arguments,
                arguments_done,
            } => {
                if item.kind != "function_call"
                    || item.status.as_deref() != Some("completed")
                    || item.call_id.as_deref() != Some(call_id.as_str())
                    || item.name.as_deref() != Some(name.as_str())
                    || !arguments_done
                    || !arguments.matches(item.arguments.as_deref().ok_or(())?)
                    || item.role.is_some()
                    || item.phase.is_some()
                    || item.content.is_some()
                    || item.summary.is_some()
                    || item.encrypted_content.is_some()
                {
                    return Err(());
                }
                Ok(())
            }
            Body::Reasoning {
                summary,
                content,
                native,
            } => {
                wire::validate_reasoning(item)?;
                Self::check_parts(summary, item.summary.as_deref().ok_or(())?)?;
                Self::check_parts(content, item.content.as_deref().unwrap_or_default())?;
                if terminal && native.as_ref() != Some(item) {
                    return Err(());
                }
                Ok(())
            }
        }
    }

    fn finish_item(&mut self, index: Result<usize, ()>, item: Result<Item, ()>) -> Result<(), ()> {
        let index = index?;
        let item = item?;
        // A completed reasoning item can carry a real summary without delta
        // events. Deliver that actual data once, then retain the native item.
        if matches!(
            self.items.get(index).map(|o| &o.body),
            Some(Body::Reasoning { .. })
        ) {
            wire::validate_reasoning(&item)?;
            if self.items[index].done || self.items[index].id != item.id {
                return Err(());
            }
            for (summary, snapshots) in [
                (true, item.summary.as_deref().unwrap_or_default()),
                (false, item.content.as_deref().unwrap_or_default()),
            ] {
                if self.parts(index, summary)?.is_empty() && !snapshots.is_empty() {
                    for (part_index, snapshot) in snapshots.iter().enumerate() {
                        self.parts(index, summary)?
                            .push(Part::new(snapshot.kind.clone()));
                        self.append_text(index, part_index, summary, snapshot.text()?)?;
                        let part = &mut self.parts(index, summary)?[part_index];
                        part.text_done = true;
                        part.done = true;
                    }
                }
            }
        }
        self.check_item(index, &item, false)?;
        let output = &mut self.items[index];
        match &mut output.body {
            Body::Message { phase, .. } => *phase = item.phase,
            Body::Reasoning { native, .. } => *native = Some(item),
            Body::Tool { .. } => {}
        }
        output.done = true;
        Ok(())
    }

    fn terminal_metadata(&mut self, response: &Response, expected: &str) -> Result<(), ()> {
        self.response_identity(response)?;
        if self.terminal_status.is_some() || response.status != expected {
            return Err(());
        }
        self.response_id = Some(response.id.clone());
        self.terminal_status = Some(response.status.clone());
        self.incomplete_details = response.incomplete_details.clone();
        self.usage = response.usage.clone();
        self.runtime_usage = self
            .usage
            .as_ref()
            .map(ProviderUsage::runtime)
            .transpose()?;
        Ok(())
    }

    fn complete(&mut self, response: Result<Response, ()>) -> Result<(), ()> {
        let response = response?;
        if self.response_id.is_none()
            || self.items.is_empty()
            || response.output.len() != self.items.len()
            || response.error.is_some()
            || response.incomplete_details.is_some()
        {
            return Err(());
        }
        self.terminal_metadata(&response, "completed")?;
        for (index, item) in response.output.iter().enumerate() {
            self.check_item(index, item, true)?;
        }
        let tool_ids: Vec<_> = self
            .items
            .iter()
            .filter_map(|item| match &item.body {
                Body::Tool { call_id, .. } => Some(call_id.clone()),
                _ => None,
            })
            .collect();
        if self.refusal && !tool_ids.is_empty() {
            return Err(());
        }
        let opaque_reasoning = self.items.iter().any(|item| matches!(&item.body, Body::Reasoning { native: Some(native), .. } if native.encrypted_content.is_some()));
        if self.text.fingerprint.bytes == 0
            && self.reasoning.fingerprint.bytes == 0
            && tool_ids.is_empty()
            && !opaque_reasoning
        {
            return Err(());
        }
        let reason = if !tool_ids.is_empty() {
            ModelFinishReason::ToolCalls
        } else if self.refusal {
            ModelFinishReason::Refused
        } else {
            ModelFinishReason::Stop
        };
        // Metadata precedes every ToolCallEnd. Runtime alone parses arguments
        // and decides whether this protocol-complete attempt is a response.
        let replay = self.take_replay(replay::FORMAT)?;
        self.pending
            .push_back(ModelEvent::ProviderReplay { replay });
        if let Some(usage) = self.runtime_usage.take() {
            self.pending.push_back(ModelEvent::Usage { usage });
        }
        for tool_call_id in tool_ids {
            self.pending
                .push_back(ModelEvent::ToolCallEnd { tool_call_id });
        }
        self.pending.push_back(ModelEvent::Finish { reason });
        self.done = true;
        Ok(())
    }

    fn take_replay(&mut self, format: &str) -> Result<ProviderReplay, ()> {
        let items = std::mem::take(&mut self.items)
            .into_iter()
            .filter_map(|output| match output.body {
                Body::Message { phase, content } => Some(ReplayItem::Message {
                    id: output.id,
                    phase,
                    content: content
                        .into_iter()
                        .map(|part| ReplayText {
                            kind: part.kind,
                            ranges: part.ranges,
                        })
                        .collect(),
                }),
                Body::Tool { call_id, name, .. } => Some(ReplayItem::FunctionCall {
                    id: output.id,
                    call_id: call_id.as_str().to_owned(),
                    name: name.as_str().to_owned(),
                }),
                Body::Reasoning {
                    native: Some(item), ..
                } => Some(ReplayItem::Reasoning { item }),
                Body::Reasoning { native: None, .. } => None,
            })
            .collect();
        let replay = Replay {
            model: self.model.clone(),
            response_id: self.response_id.take(),
            status: self.terminal_status.take(),
            text: self.text.fingerprint.checksum(),
            reasoning: self.reasoning.fingerprint.checksum(),
            items,
            usage: self.usage.take(),
            incomplete_details: self.incomplete_details.take(),
            incomplete_tool_header: self.incomplete_tool_header.take(),
        };
        ProviderReplay::new(format, serde_json::to_value(replay).map_err(|_| ())?).map_err(|_| ())
    }

    fn fail(&mut self, failure: ModelError) -> Result<ModelEvent, ModelError> {
        self.pending.clear();
        self.done = true;
        if (self.seen || self.usage.is_some() || self.incomplete_tool_header.is_some())
            && let Ok(replay) = self.take_replay(replay::PARTIAL_FORMAT)
        {
            if let Some(usage) = self.runtime_usage.take() {
                self.pending.push_back(ModelEvent::Usage { usage });
            }
            self.pending_error = Some(failure);
            return Ok(ModelEvent::ProviderReplay { replay });
        }
        Err(failure)
    }
}

pub(super) async fn next(mut state: State) -> Option<(Result<ModelEvent, ModelError>, State)> {
    loop {
        if let Some(event) = state.pending.pop_front() {
            return Some((Ok(event), state));
        }
        if let Some(error) = state.pending_error.take() {
            return Some((Err(error), state));
        }
        if state.done {
            return None;
        }
        let failure = if state.cancellation.is_cancelled() {
            Some(error::stream(ModelErrorKind::Cancelled, state.seen))
        } else if Instant::now() >= state.deadline {
            Some(error::stream(ModelErrorKind::Timeout, state.seen))
        } else {
            None
        };
        if let Some(error) = failure {
            let result = state.fail(error);
            return Some((result, state));
        }
        if state.offset < state.chunk.len() {
            let mut frame = None;
            while state.offset < state.chunk.len() {
                let byte = state.chunk[state.offset];
                state.offset += 1;
                if let Some(value) = state.parser.push(byte) {
                    frame = Some(value);
                    break;
                }
            }
            if let Some(frame) = frame
                && let Err(error) = state.handle(&frame)
            {
                let result = state.fail(error);
                return Some((result, state));
            }
            continue;
        }
        let next = tokio::select! {
            biased;
            _ = state.cancellation.cancelled() => Err(error::stream(ModelErrorKind::Cancelled, state.seen)),
            _ = tokio::time::sleep_until(state.deadline) => Err(error::stream(ModelErrorKind::Timeout, state.seen)),
            chunk = state.bytes.next() => match chunk {
                Some(Ok(chunk)) => Ok(chunk),
                Some(Err(failure)) if failure.is_timeout() => Err(error::stream(ModelErrorKind::Timeout, state.seen)),
                Some(Err(_)) => Err(error::stream(ModelErrorKind::StreamInterrupted, state.seen)),
                None => Err(error::stream(ModelErrorKind::IncompleteResponse, state.seen)),
            }
        };
        match next {
            Ok(chunk) => {
                state.chunk = chunk;
                state.offset = 0;
            }
            Err(error) => {
                let result = state.fail(error);
                return Some((result, state));
            }
        }
    }
}
