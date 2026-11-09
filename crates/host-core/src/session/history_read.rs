use super::*;
use crate::{HistoryItemData, SessionHistorySnapshot};

impl HistoryItemData {
    pub(super) fn from_entry(entry: &HistoryEntry) -> Option<Self> {
        match entry {
            HistoryEntry::Input { user, .. } => Some(Self::User {
                text: user.input.as_text().to_owned(),
            }),
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
impl SessionState {
    pub(super) fn replace_history(&mut self, entries: &[HistoryEntry]) -> Result<(), Fault> {
        let mut raw = Vec::new();
        let mut projection: Option<minicore_host_storage::CompactionProjection> = None;
        for (position, entry) in entries.iter().enumerate() {
            if let HistoryEntry::Compaction {
                source_position,
                covered_items,
                summary,
            } = entry
            {
                let candidate = minicore_host_storage::CompactionProjection::new(
                    *source_position,
                    *covered_items,
                    summary.clone(),
                );
                candidate
                    .validate(
                        &raw,
                        position as u64,
                        projection.as_ref().map_or(0, |value| value.covered_items),
                    )
                    .map_err(|error| {
                        Fault::new(FaultKind::SessionHistoryLoadFailed, error.to_string())
                    })?;
                projection = Some(candidate);
            } else if let Some(item) = entry.runtime_item() {
                raw.push(item);
            }
        }
        self.history = raw;
        self.history_position = entries.len() as u64;
        self.projection = projection;
        self.readable_history = Some(
            entries
                .iter()
                .filter_map(HistoryItemData::from_entry)
                .collect(),
        );
        Ok(())
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
        let store = self
            .store
            .lock()
            .expect("Session store mutex poisoned")
            .clone()
            .ok_or_else(|| {
                Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "history source has not been loaded and is not readable",
                )
            })?;
        let history = crate::persistence::disk(FaultKind::SessionHistoryLoadFailed, move || {
            store.read_history()
        })
        .await?;
        let content = history
            .records
            .iter()
            .filter_map(|record| HistoryItemData::from_entry(&record.entry))
            .collect::<Vec<_>>();
        self.access.read(caller, || {
            self.ensure_visible()?;
            self.history_page(&content, start, request.limit)
        })
    }
    fn history_page(
        &self,
        history: &[HistoryItemData],
        start: usize,
        limit: u64,
    ) -> Result<SessionHistorySnapshot, Fault> {
        if start > history.len() {
            return Err(Fault::new(
                FaultKind::SessionHistoryCursorInvalid,
                "history cursor is beyond the retained content",
            ));
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

#[cfg(test)]
#[path = "tests/task052_history_read.rs"]
mod task052;
