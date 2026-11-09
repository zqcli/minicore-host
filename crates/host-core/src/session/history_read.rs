use super::*;
use crate::{HistoryItemData, SessionHistorySnapshot};

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
            state
                .history_loaded
                .then(|| self.history_page(&state.history, start, request.limit))
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
        .await?
        .runtime_history();
        self.access.read(caller, || {
            self.ensure_visible()?;
            self.history_page(&history, start, request.limit)
        })
    }
    fn history_page(
        &self,
        history: &[HistoryItem],
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
        let items = history[start..end]
            .iter()
            .map(HistoryItemData::from)
            .collect();
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
