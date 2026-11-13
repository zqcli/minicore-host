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

fn project_saved(id: &TurnId, records: &[HistoryRecord]) -> Result<TurnResultData, Fault> {
    let rows = records
        .iter()
        .map(HistoryRecordData::from_record)
        .collect::<Vec<_>>();
    let (history, partials, terminal) = project_records(id, &rows);
    Ok(TurnResultData {
        info: None,
        config: None,
        saved_terminal: Some(terminal.ok_or_else(missing_turn)?),
        history,
        partials,
    })
}

pub(super) fn project_records(
    id: &TurnId,
    rows: &[HistoryRecordData],
) -> (
    Vec<TurnHistoryItemData>,
    Vec<TurnPartialData>,
    Option<SavedTurnTerminal>,
) {
    let mut history = Vec::new();
    let mut partials = Vec::new();
    let mut terminal = None;
    for row in rows.iter().filter(|row| row.turn_id.as_ref() == Some(id)) {
        match &row.item {
            HistoryItemData::Partial { partial } => partials.push(TurnPartialData {
                position: row.position,
                recorded_at: row.recorded_at,
                response_index: row.response_index.expect("native partial response index"),
                partial: partial.clone(),
            }),
            HistoryItemData::TurnFinished { terminal: facts } => {
                terminal = Some(SavedTurnTerminal {
                    position: row.position,
                    recorded_at: row.recorded_at,
                    terminal: facts.clone(),
                })
            }
            _ => history.push(TurnHistoryItemData {
                position: row.position,
                recorded_at: row.recorded_at,
                response_index: row.response_index,
                item: row.item.clone(),
            }),
        }
    }
    (history, partials, terminal)
}
