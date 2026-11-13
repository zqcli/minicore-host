//! Private agent-tool output views over facts already read from their owners.
//! These functions perform no authorization or queries and retain no state.
//! Explicit projections keep body/config selection out of JSON field mutation.
use super::encoded;
use crate::ToolSource;
use minicore_host_core as core;
use minicore_host_protocol::*;
use minicore_host_runtime::tools::{ToolOutput, ToolResultOutcome};
use serde::Serialize;
use serde_json::{Value, json};

pub(super) fn agent(
    source: &ToolSource,
    summary: &core::AgentSummary,
    config: Option<&AgentConfig>,
) -> Result<Value, Fault> {
    #[derive(Serialize)]
    struct Data<'a> {
        name: &'a str,
        mode: StorageMode,
        lifecycle: AgentLifecycle,
        config_revision: ConfigRevision,
        default_session_id: &'a Option<SessionId>,
        metadata_saving: bool,
        metadata_fault: &'a Option<Fault>,
        deletion: &'a Option<DataDeletion>,
        sessions: &'a [core::SessionSummary],
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<&'a AgentConfig>,
    }
    let data = encoded(Data {
        name: &summary.name,
        mode: summary.mode,
        lifecycle: summary.lifecycle,
        config_revision: summary.config_revision,
        default_session_id: &summary.default_session_id,
        metadata_saving: summary.metadata_saving,
        metadata_fault: &summary.metadata_fault,
        deletion: &summary.deletion,
        sessions: &summary.sessions,
        config,
    })?;
    Ok(json!({
        "host_id": source.host_id,
        "host_incarnation": source.host_incarnation,
        "agent_id": summary.id,
        "data": data,
    }))
}

pub(super) fn session(
    snapshot: core::SessionSnapshot,
    info: core::SessionInfo,
    include_config: bool,
    history: Option<HistoryPageData<core::HistoryRecordData>>,
) -> Result<Value, Fault> {
    #[derive(Serialize)]
    struct Data {
        #[serde(flatten)]
        state: core::SessionStateData,
        title: Option<String>,
        created_at: Timestamp,
        last_activity_at: Timestamp,
        workspace: Option<WorkspaceRef>,
        model: ModelRef,
        reasoning: ReasoningPreference,
        config_revision: ConfigRevision,
        agent_config_revision: ConfigRevision,
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<SessionConfig>,
        #[serde(skip_serializing_if = "Option::is_none")]
        history: Option<HistoryPageData<core::HistoryRecordData>>,
    }
    let data = encoded(Data {
        state: snapshot.data,
        title: info.title,
        created_at: info.created_at,
        last_activity_at: info.last_activity_at,
        workspace: info.config.workspace.clone(),
        model: info.config.model.clone(),
        reasoning: info.config.reasoning,
        config_revision: info.config_revision,
        agent_config_revision: info.agent_config_revision,
        config: include_config.then_some(info.config),
        history,
    })?;
    Ok(json!({
        "host_id": snapshot.host_id,
        "host_incarnation": snapshot.host_incarnation,
        "agent_id": snapshot.agent_id,
        "session_id": snapshot.session_id,
        "data": data,
    }))
}

pub(super) fn turn_result(result: &core::TurnResultSnapshot) -> Result<Value, Fault> {
    #[derive(Serialize)]
    struct Partial<'a> {
        position: u64,
        recorded_at: Timestamp,
        response_index: u64,
        #[serde(flatten)]
        partial: &'a core::ModelPartialData,
    }
    #[derive(Serialize)]
    struct Data<'a> {
        #[serde(flatten)]
        info: Option<&'a core::TurnStateData>,
        #[serde(skip_serializing_if = "Option::is_none")]
        load_id: Option<&'a LoadId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        saved_terminal: Option<&'a core::SavedTurnTerminal>,
        history: &'a [core::TurnHistoryItemData],
        partials: Vec<Partial<'a>>,
    }
    // The owner distinguishes retained metadata from saved-only terminal facts.
    // The tool omits bound config in both cases and keeps its flat partial view.
    encoded(Data {
        info: result.data.info.as_ref(),
        load_id: result.source.load_id.as_ref(),
        saved_terminal: result.data.saved_terminal.as_ref(),
        history: &result.data.history,
        partials: result
            .data
            .partials
            .iter()
            .map(|value| Partial {
                position: value.position,
                recorded_at: value.recorded_at,
                response_index: value.response_index,
                partial: &value.partial,
            })
            .collect(),
    })
}

pub(super) fn current_turn(
    info: &core::TurnInfo,
    output: Option<&core::ModelTextSnapshot>,
) -> Result<Value, Fault> {
    #[derive(Serialize)]
    struct Range<'a> {
        output: &'a TextRangeData,
        response_index: u64,
    }
    #[derive(Serialize)]
    struct Data<'a> {
        load_id: &'a LoadId,
        state: TurnState,
        config_revision: ConfigRevision,
        outcome: Option<TurnOutcome>,
        persistence: PersistenceState,
        requests: u64,
        tool_rounds: u64,
        phase: &'a Option<TurnPhase>,
        model_retry: &'a Option<ModelRetryProgress>,
        model_attempts: &'a [u8],
        fault: &'a Option<Fault>,
        persistence_fault: &'a Option<Fault>,
        cleanup_faults: &'a [Fault],
        #[serde(flatten)]
        range: Option<Range<'a>>,
    }
    // TurnInfo has no runner revision. Do not manufacture one to adapt this
    // lightweight/current-range path into the full result's TurnStateData.
    encoded(Data {
        load_id: &info.load_id,
        state: info.state,
        config_revision: info.config_revision,
        outcome: info.outcome,
        persistence: info.persistence,
        requests: info.requests,
        tool_rounds: info.tool_rounds,
        phase: &info.phase,
        model_retry: &info.model_retry,
        model_attempts: &info.model_attempts,
        fault: &info.fault,
        persistence_fault: &info.persistence_fault,
        cleanup_faults: &info.cleanup_faults,
        range: output.map(|value| Range {
            output: &value.data,
            response_index: value.source.response_index,
        }),
    })
}

pub(super) fn toolcall(
    snapshot: &core::ToolCallSnapshot,
    include_output: bool,
    output: Option<&core::ToolTextSnapshot>,
) -> Result<Value, Fault> {
    #[derive(Serialize)]
    struct ResultData<'a> {
        outcome: ToolResultOutcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<&'a ToolOutput>,
    }
    #[derive(Serialize)]
    struct Data<'a> {
        #[serde(flatten)]
        state: &'a core::ToolCallStateData,
        #[serde(skip_serializing_if = "Option::is_none")]
        arguments: Option<&'a Value>,
        result: Option<ResultData<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<&'a TextRangeData>,
    }
    #[derive(Serialize)]
    struct View<'a> {
        #[serde(flatten)]
        source: &'a ToolCallIdentity,
        data: Data<'a>,
    }
    encoded(View {
        source: &snapshot.source,
        data: Data {
            state: &snapshot.data.state,
            arguments: include_output.then_some(&snapshot.data.arguments),
            result: snapshot.data.result.as_ref().map(|result| ResultData {
                outcome: result.outcome,
                output: (include_output && output.is_none()).then_some(&result.output),
            }),
            output: output.map(|value| &value.data),
        },
    })
}

pub(super) fn message(
    source: &ToolSource,
    target: &SessionRef,
    message_id: &MessageId,
    record: &core::MessageRecord,
    include_output: bool,
) -> Result<Value, Fault> {
    #[derive(Serialize)]
    struct Input<'a> {
        mode: MessageMode,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        in_reply_to: Option<&'a MessageId>,
    }
    #[derive(Serialize)]
    struct Data<'a> {
        received_at: Timestamp,
        history_position: Option<u64>,
        input: Input<'a>,
        source: &'a MessageSource,
        queue: InputQueue,
        order: u64,
        binding: &'a core::MessageBinding,
    }
    let data = encoded(Data {
        received_at: record.received_at,
        history_position: record.history_position,
        input: Input {
            mode: record.input.mode,
            text: include_output.then_some(record.input.text.as_str()),
            in_reply_to: record.input.in_reply_to.as_ref(),
        },
        source: &record.source,
        queue: record.queue,
        order: record.order,
        binding: &record.binding,
    })?;
    Ok(json!({
        "host_id": source.host_id,
        "host_incarnation": source.host_incarnation,
        "agent_id": target.agent_id,
        "session_id": target.session_id,
        "message_id": message_id,
        "data": data,
    }))
}
