//! Read actual retained output and ToolCall facts without changing execution.
use super::*;
use crate::observation::ToolCallData;

impl SessionController {
    pub(crate) fn output_read(
        &self,
        caller: &CallerContext,
        turn: &TurnId,
        response: Option<u64>,
        tool: Option<&ToolCallId>,
        request: &TextReadRequest,
    ) -> Result<TextRangeData, Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            request
                .validate()
                .map_err(|e| Fault::new(FaultKind::SessionOutputRangeInvalid, e.to_string()))?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let output = state.output.get(turn).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionTurnNotFound,
                    "Turn output is not retained",
                )
            })?;
            let channels = match (response, tool) {
                (Some(index), None) => output.models.get(&index),
                (None, Some(id)) => output.tools.get(id).map(|record| &record.output),
                _ => None,
            }
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionOutputNotFound,
                    "output source is not retained",
                )
            })?;
            let body = channels.get(&request.channel).ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionOutputNotFound,
                    "output channel is not retained",
                )
            })?;
            if body.non_utf8 && request.end > body.text().len() as u64 {
                return Err(Fault::new(
                    FaultKind::SessionOutputRangeInvalid,
                    "tool channel contains non-UTF-8 bytes; only its observed UTF-8 prefix is text-readable",
                ));
            }
            read_range(body.text(), request)
        })
    }
    pub(crate) fn tool_get(
        &self,
        caller: &CallerContext,
        turn: &TurnId,
        tool: &ToolCallId,
    ) -> Result<(ToolCallIdentity, ToolCallData), Fault> {
        self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            let info = state
                .active
                .as_ref()
                .filter(|a| &a.info.id == turn)
                .map(|a| &a.info)
                .or_else(|| state.completed.get(turn).map(|r| &r.info))
                .ok_or_else(|| {
                    Fault::new(FaultKind::SessionTurnNotFound, "Turn is not retained")
                })?;
            let record = state
                .output
                .get(turn)
                .and_then(|o| o.tools.get(tool))
                .ok_or_else(|| {
                    Fault::new(
                        FaultKind::SessionToolCallNotFound,
                        "ToolCall is not retained by this Turn",
                    )
                })?;
            Ok((
                ToolCallIdentity {
                    turn: self.turn_identity(info),
                    tool_call_id: tool.clone(),
                },
                record.data.clone(),
            ))
        })
    }
}

fn read_range(text: &str, request: &TextReadRequest) -> Result<TextRangeData, Fault> {
    let invalid = || {
        Fault::new(
            FaultKind::SessionOutputRangeInvalid,
            "read positions must be available UTF-8 boundaries",
        )
    };
    let start = usize::try_from(request.start).map_err(|_| invalid())?;
    let requested_end = usize::try_from(request.end).map_err(|_| invalid())?;
    if start > text.len()
        || !text.is_char_boundary(start)
        || (requested_end <= text.len() && !text.is_char_boundary(requested_end))
    {
        return Err(invalid());
    }
    let fixed_end = requested_end.min(text.len());
    let budget = usize::try_from(request.max_bytes).unwrap_or(usize::MAX);
    let mut end = start.saturating_add(budget).min(fixed_end);
    while end > start && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == start && start < fixed_end {
        end += text[start..fixed_end]
            .chars()
            .next()
            .expect("nonempty range")
            .len_utf8();
    }
    Ok(TextRangeData {
        channel: request.channel.clone(),
        start: start as u64,
        end: end as u64,
        text: text[start..end].to_owned(),
        has_more: end < fixed_end,
    })
}
