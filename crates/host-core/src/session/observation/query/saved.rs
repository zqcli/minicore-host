//! Read-only projection of actual saved facts. No execution/result/output cache
//! is populated, and no old inputs, tools, waits or Runtime work are restored.
use super::*;
use crate::SavedTurnTerminal;

impl SessionController {
    pub(crate) async fn turn_result_snapshot(
        &self,
        caller: &CallerContext,
        id: &TurnId,
    ) -> Result<TurnResultSnapshot, Fault> {
        if let Some(result) = self.retained_result_snapshot(caller, id)? {
            return Ok(result);
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
        let saved = tokio::select! {
            result = AssertUnwindSafe(async { self.backend.read_saved_turn(id).await }).catch_unwind() => {
                result.unwrap_or_else(|_| Err(Fault::new(
                    FaultKind::SessionHistoryLoadFailed,
                    "saved Turn reader failed unexpectedly",
                )))
            }
            _ = revoked => return Err(Fault::new(
                FaultKind::HostAccessInvalid,
                "saved Turn reader connection is no longer valid",
            )),
        };
        let record = if let Err(fault) = &saved
            && fault.kind() == Some(FaultKind::SessionHistoryLoadFailed)
        {
            let state = self.state.lock().expect("Session mutex poisoned");
            Some(diagnostics::history_failed(
                diagnostic::session_context(&self.identity()),
                fault,
                "saved_turn_read",
                state.controller_revision,
            ))
        } else {
            None
        };
        diagnostic::emit(record);
        // Recheck revocation, visibility and current accepted ownership after
        // disk I/O. New retained results/finalization outrank an opening baseline
        // or read failure; a revoked/reclaimed owner cannot publish stale data.
        if let Some(result) = self.retained_result_snapshot(caller, id)? {
            return Ok(result);
        }
        let data = project_saved(id, &saved?)?;
        // Projection does not hold any admission guard. Validate again before
        // exposing a possibly large result, including races during projection.
        if let Some(result) = self.retained_result_snapshot(caller, id)? {
            return Ok(result);
        }
        Ok(TurnResultSnapshot {
            source: TurnIdentity {
                session: self.identity(),
                turn_id: id.clone(),
                load_id: None,
            },
            data,
        })
    }
}

fn project_saved(id: &TurnId, entries: &[HistoryEntry]) -> Result<TurnResultData, Fault> {
    let mut history = Vec::new();
    let mut partials = Vec::new();
    let mut terminal = None;
    for entry in entries.iter().filter(|entry| entry.turn_id() == Some(id)) {
        match entry {
            HistoryEntry::Input { user, .. } => history.push(TurnHistoryItemData {
                response_index: None,
                item: HistoryItemData::User {
                    text: user.input.as_text().to_owned(),
                },
            }),
            HistoryEntry::Runtime {
                response_index,
                item,
                ..
            } => history.push(TurnHistoryItemData {
                // Only complete model/tool records belong to a response; no
                // inferred index for an explicitly stored User/Summary item.
                response_index: matches!(
                    item,
                    HistoryItem::Assistant(_) | HistoryItem::ToolResult(_)
                )
                .then_some(u64::from(*response_index)),
                item: item.into(),
            }),
            HistoryEntry::Partial {
                response_index,
                partial,
                ..
            } => partials.push(TurnPartialData {
                response_index: u64::from(*response_index),
                partial: partial.into(),
            }),
            HistoryEntry::TurnFinished {
                outcome, reason, ..
            } => {
                terminal = Some(SavedTurnTerminal {
                    outcome: *outcome,
                    reason: reason.clone(),
                });
            }
            // Global summary/compaction markers have no TurnId. Do not guess
            // ownership based on their position or restore compacted context.
            HistoryEntry::Summary { .. } | HistoryEntry::Compaction { .. } => {}
        }
    }
    let terminal = terminal.ok_or_else(missing_turn)?;
    Ok(TurnResultData {
        info: None,
        config: None,
        saved_terminal: Some(terminal),
        history,
        partials,
    })
}
