use super::*;
use crate::{HistoryItemData, SessionHistorySnapshot};

impl HistoryItemData {
    pub(in crate::session) fn from_input(
        user: &minicore_host_runtime::UserHistory,
        message_id: &MessageId,
        source: &MessageSource,
        mode: MessageMode,
        in_reply_to: Option<&MessageId>,
    ) -> Self {
        Self::User {
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
                user,
                message_id,
                source,
                mode,
                in_reply_to,
                ..
            } => Some(Self::from_input(
                user,
                message_id,
                source,
                *mode,
                in_reply_to.as_ref(),
            )),
            HistoryEntry::Runtime { item, .. } => Some(item.into()),
            HistoryEntry::Partial { partial, .. } => Some(Self::Partial {
                partial: partial.into(),
            }),
            HistoryEntry::Summary { summary } | HistoryEntry::Compaction { summary, .. } => {
                Some(Self::Summary {
                    text: summary.content.as_str().to_owned(),
                })
            }
            HistoryEntry::TurnFinished { .. } => None,
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
        let start = self.history_cursor(request.cursor.as_deref())?;
        let retained = self.access.read(caller, || {
            self.ensure_visible()?;
            let state = self.state.lock().expect("Session mutex poisoned");
            // Unload retains accepted content, including unconfirmed saves.
            // Readability is independent of Runtime residency and persistence.
            state
                .readable_history
                .as_ref()
                .map(|history| self.history_page(history, start, request.limit))
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
                .map(|entries| self.cold_history_page(&entries, start, request.limit))
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
                    return self.history_page(history, start, request.limit).map(Some);
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
        history: &[HistoryEntry],
        start: usize,
        limit: u64,
    ) -> Result<SessionHistorySnapshot, Fault> {
        // The backend has already validated every record, including the suffix
        // outside this page. Count public positions without projecting bodies.
        let mut entries = history
            .iter()
            .filter(|entry| !matches!(entry, HistoryEntry::TurnFinished { .. }));
        if entries.by_ref().take(start).count() < start {
            return Err(history_cursor_beyond_tail());
        }
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let items = entries
            .by_ref()
            .take(limit)
            .map(|entry| {
                HistoryItemData::from_entry(entry).expect("content entry has public history data")
            })
            .collect::<Vec<_>>();
        let end = start.saturating_add(items.len());
        let next_cursor = entries
            .next()
            .is_some()
            .then(|| format!("v1.{}.{}.{}", self.incarnation, self.id, end));
        Ok(SessionHistorySnapshot {
            source: self.identity(),
            data: HistoryPageData { items, next_cursor },
        })
    }
    fn history_page(
        &self,
        history: &[HistoryItemData],
        start: usize,
        limit: u64,
    ) -> Result<SessionHistorySnapshot, Fault> {
        if start > history.len() {
            return Err(history_cursor_beyond_tail());
        }
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let end = start.saturating_add(limit).min(history.len());
        let items = history[start..end].to_vec();
        let next_cursor =
            (end < history.len()).then(|| format!("v1.{}.{}.{}", self.incarnation, self.id, end));
        Ok(SessionHistorySnapshot {
            source: self.identity(),
            data: HistoryPageData { items, next_cursor },
        })
    }
    fn history_cursor(&self, cursor: Option<&str>) -> Result<usize, Fault> {
        let Some(cursor) = cursor else {
            return Ok(0);
        };
        let invalid = || {
            Fault::new(
                FaultKind::SessionHistoryCursorInvalid,
                "history cursor does not identify this Session and Host run",
            )
        };
        let mut parts = cursor.split('.');
        if parts.next() != Some("v1")
            || parts.next() != Some(self.incarnation.as_str())
            || parts.next() != Some(self.id.as_str())
        {
            return Err(invalid());
        }
        let index = parts
            .next()
            .ok_or_else(invalid)?
            .parse::<usize>()
            .map_err(|_| invalid())?;
        if parts.next().is_some() {
            return Err(invalid());
        }
        Ok(index)
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
