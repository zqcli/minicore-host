use super::*;
use crate::{HistoryItemData, HistoryRecordData, SessionHistorySnapshot, TurnTerminalData};
use minicore_host_storage::HistoryRecord;

impl HistoryItemData {
    pub(in crate::session) fn from_input(
        user: &minicore_host_runtime::UserHistory,
        received_at: Option<Timestamp>,
        message_id: &MessageId,
        source: &MessageSource,
        mode: MessageMode,
        in_reply_to: Option<&MessageId>,
    ) -> Self {
        Self::User {
            received_at,
            text: user.input.as_text().to_owned(),
            message_id: Some(message_id.clone()),
            source: Some(source.clone()),
            mode: Some(mode),
            in_reply_to: in_reply_to.cloned(),
        }
    }
    pub(in crate::session) fn from_entry(entry: &HistoryEntry) -> Option<Self> {
        match entry {
            HistoryEntry::Input {
                received_at,
                user,
                message_id,
                source,
                mode,
                in_reply_to,
                ..
            } => Some(Self::from_input(
                user,
                Some(*received_at),
                message_id,
                source,
                *mode,
                in_reply_to.as_ref(),
            )),
            HistoryEntry::Runtime { item, .. } => Some(item.into()),
            HistoryEntry::Partial { partial, .. } => Some(Self::Partial {
                partial: partial.into(),
            }),
            HistoryEntry::Summary { summary } => Some(Self::Summary {
                text: summary.content.as_str().to_owned(),
            }),
            HistoryEntry::Compaction {
                source_position,
                covered_items,
                summary,
            } => Some(Self::Compaction {
                text: summary.content.as_str().to_owned(),
                source_position: *source_position,
                covered_items: *covered_items,
            }),
            HistoryEntry::TurnFinished {
                finished_at,
                requests,
                tool_rounds,
                model_attempts,
                outcome,
                reason,
                ..
            } => Some(Self::TurnFinished {
                terminal: TurnTerminalData {
                    finished_at: *finished_at,
                    requests: *requests,
                    tool_rounds: *tool_rounds,
                    model_attempts: model_attempts.clone(),
                    outcome: *outcome,
                    reason: reason.clone(),
                },
            }),
        }
    }
}

impl SessionController {
    /// A content read does not create a Runner, consume input or reconcile saves.
    pub(crate) async fn history_read(
        &self,
        caller: &CallerContext,
        request: &HistoryReadRequest,
    ) -> Result<SessionHistorySnapshot, Fault> {
        request
            .validate()
            .map_err(|e| Fault::new(FaultKind::SessionHistoryCursorInvalid, e.to_string()))?;
        let window = self.history_cursor(request.cursor.as_deref(), request.direction)?;
        let retained = self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            // Unload retains accepted content, including unconfirmed saves.
            // Readability is independent of Runtime residency and persistence.
            state
                .readable_history
                .as_ref()
                .map(|history| self.history_page(history, window, request))
                .transpose()
        })?;
        if let Some(page) = retained {
            return Ok(page);
        }
        let revoked = async {
            if let Some(mut validity) = caller.read_revocation() {
                if *validity.borrow() {
                    let _ = validity.changed().await;
                }
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::pin!(revoked);
        let cold = tokio::select! {
            result = AssertUnwindSafe(async { self.backend.read_cold().await }).catch_unwind() => {
                result.unwrap_or_else(|_| Err(Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "history reader failed unexpectedly",
                )))
            }
            _ = &mut revoked => return Err(history_reader_revoked()),
        };
        // Only this actual backend attempt reports its failure. A query that
        // receives Ok(None) and waits for an existing loader must not repeat
        // that loader's primary ERROR when it later reads the retained Fault.
        let record = if let Err(fault) = &cold
            && fault.kind() == Some(FaultKind::SessionHistoryLoadFailed)
        {
            let state = self.state.lock().expect("Session mutex poisoned");
            Some(diagnostics::history_failed(
                diagnostic::session_context(&self.identity()),
                fault,
                "cold_read",
                state.controller_revision,
            ))
        } else {
            None
        };
        diagnostic::emit(record);
        let mut content = cold.and_then(|entries| {
            entries
                .map(|entries| self.cold_history_page(&entries, window, request))
                .transpose()
        });
        loop {
            // Register before checking publication, including when load completed
            // while the disk read was returning. No notification can be lost.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let page = self.access.read(caller, || {
                self.ensure_visible()?;
                let state = self.state.lock().expect("Session mutex poisoned");
                if let Some(history) = &state.readable_history {
                    // The owner may already have accepted newer, unconfirmed
                    // records. Never replace them with a cold opening snapshot.
                    return self.history_page(history, window, request).map(Some);
                }
                Self::available(&state)?;
                match &mut content {
                    Ok(page @ Some(_)) => Ok(page.take()),
                    Ok(None) if state.preparing => Ok(None),
                    Ok(None) => Err(state.block_reason.clone().unwrap_or_else(|| {
                        Fault::new(
                            FaultKind::SessionHistoryLoadFailed,
                            "the history owner has no readable retained content",
                        )
                    })),
                    Err(fault) => Err(fault.clone()),
                }
            })?;
            if let Some(page) = page {
                return Ok(page);
            }
            tokio::select! {
                _ = changed => {}
                _ = &mut revoked => return Err(history_reader_revoked()),
            }
        }
    }
    fn cold_history_page(
        &self,
        history: &[HistoryRecord],
        window: Option<HistoryWindow>,
        request: &HistoryReadRequest,
    ) -> Result<SessionHistorySnapshot, Fault> {
        // The backend validates the whole file. Projection touches only this page.
        let (start, end, upper_bound, next_cursor) =
            self.history_range(history.len(), window, request)?;
        let items = history[start..end]
            .iter()
            .map(HistoryRecordData::from_record)
            .collect();
        Ok(SessionHistorySnapshot {
            source: self.identity(),
            data: HistoryPageData {
                items,
                next_cursor,
                upper_bound,
            },
        })
    }
    fn history_page(
        &self,
        history: &[HistoryRecordData],
        window: Option<HistoryWindow>,
        request: &HistoryReadRequest,
    ) -> Result<SessionHistorySnapshot, Fault> {
        let (start, end, upper_bound, next_cursor) =
            self.history_range(history.len(), window, request)?;
        // Native positions are contiguous, including terminal and compaction facts.
        let items = history[start..end].to_vec();
        Ok(SessionHistorySnapshot {
            source: self.identity(),
            data: HistoryPageData {
                items,
                next_cursor,
                upper_bound,
            },
        })
    }
    fn history_range(
        &self,
        length: usize,
        window: Option<HistoryWindow>,
        request: &HistoryReadRequest,
    ) -> Result<(usize, usize, u64, Option<String>), Fault> {
        let upper = window.map_or(length, |window| window.upper);
        let boundary = window.map_or_else(
            || match request.direction {
                HistoryDirection::Forward => 0,
                HistoryDirection::Backward => upper,
            },
            |window| window.boundary,
        );
        if boundary > upper || upper > length {
            return Err(history_cursor_beyond_tail());
        }
        let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
        let (start, end, next) = match request.direction {
            HistoryDirection::Forward => {
                let end = boundary.saturating_add(limit).min(upper);
                (boundary, end, (end < upper).then_some(end))
            }
            HistoryDirection::Backward => {
                let start = boundary.saturating_sub(limit);
                (start, boundary, (start > 0).then_some(start))
            }
        };
        let direction = match request.direction {
            HistoryDirection::Forward => "f",
            HistoryDirection::Backward => "b",
        };
        let cursor = next.map(|next| {
            format!(
                "v2.{}.{}.{direction}.{next}.{upper}",
                self.incarnation, self.id
            )
        });
        Ok((start, end, upper as u64, cursor))
    }
    fn history_cursor(
        &self,
        cursor: Option<&str>,
        direction: HistoryDirection,
    ) -> Result<Option<HistoryWindow>, Fault> {
        let Some(cursor) = cursor else {
            return Ok(None);
        };
        let invalid = || {
            Fault::new(
                FaultKind::SessionHistoryCursorInvalid,
                "history cursor does not identify this Session, Host run, direction and fixed window",
            )
        };
        let mut parts = cursor.split('.');
        let expected_direction = match direction {
            HistoryDirection::Forward => "f",
            HistoryDirection::Backward => "b",
        };
        if parts.next() != Some("v2")
            || parts.next() != Some(self.incarnation.as_str())
            || parts.next() != Some(self.id.as_str())
            || parts.next() != Some(expected_direction)
        {
            return Err(invalid());
        }
        let boundary = parts
            .next()
            .ok_or_else(invalid)?
            .parse::<usize>()
            .map_err(|_| invalid())?;
        let upper = parts
            .next()
            .ok_or_else(invalid)?
            .parse::<usize>()
            .map_err(|_| invalid())?;
        if parts.next().is_some() || boundary > upper {
            return Err(invalid());
        }
        Ok(Some(HistoryWindow { boundary, upper }))
    }
}

#[derive(Clone, Copy)]
struct HistoryWindow {
    boundary: usize,
    upper: usize,
}

impl HistoryRecordData {
    pub(in crate::session) fn from_record(record: &HistoryRecord) -> Self {
        let response_index = match &record.entry {
            HistoryEntry::Runtime {
                response_index,
                item: HistoryItem::Assistant(_) | HistoryItem::ToolResult(_),
                ..
            }
            | HistoryEntry::Partial { response_index, .. } => Some(u64::from(*response_index)),
            _ => None,
        };
        Self {
            position: record.position,
            recorded_at: record.recorded_at,
            turn_id: record.entry.turn_id().cloned(),
            response_index,
            item: HistoryItemData::from_entry(&record.entry)
                .expect("every native record has a public projection"),
        }
    }
}

fn history_cursor_beyond_tail() -> Fault {
    Fault::new(
        FaultKind::SessionHistoryCursorInvalid,
        "history cursor is beyond the retained content",
    )
}

fn history_reader_revoked() -> Fault {
    Fault::new(
        FaultKind::HostAccessInvalid,
        "history reader connection is no longer valid",
    )
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
