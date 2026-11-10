//! Address-only Session calls; this context owns no Session or Runner state.
use super::HostClient;
use crate::{diagnostics::Correlation, operations::*};
use minicore_host_core::{
    InteractionAnswerData, MessageSnapshot, ModelTextSnapshot, SessionContextSnapshot,
    SessionHistorySnapshot, SessionInfo, SessionSnapshot, ToolCallSnapshot, ToolTextSnapshot,
    TurnDetailsSnapshot, TurnListSnapshot, TurnResultSnapshot, WorkspaceSnapshot,
};
use minicore_host_protocol::*;
use minicore_host_workspace::{
    ChangesListRequest, ChangesListResult, DiffRequest, DiffResult, FilesRequest, FilesResult,
    ReadRequest, ReadResult, SearchRequest, SearchResult, StatusResult,
};

/// An address-only convenience context. Multiple Sessions can share a client
/// and issue concurrent calls without any server-side current-Session state.
#[derive(Clone)]
pub struct SessionClient {
    pub(super) client: HostClient,
    pub(super) target: SessionRef,
}
macro_rules! session_control {
    ($($name:ident, $operation:ident;)+) => {$(
        pub async fn $name(&self, request: SessionControlRequest, options: CallOptions) -> Result<SessionControlReceipt, CallError> {
            self.client.call($operation(SessionOperation { target: self.target.clone(), operation_id: request.operation_id }), options,
                |operation| Correlation::session(&operation.0.target).operation(&operation.0.operation_id)).await
        }
    )+};
}
macro_rules! session_query {
    ($(#[$meta:meta])* $name:ident, $operation:ident => $output:ty) => {
        $(#[$meta])*
        pub async fn $name(&self, options: CallOptions) -> Result<$output, CallError> {
            self.client.call(
                $operation(SessionTarget { target: self.target.clone() }), options,
                |operation| Correlation::session(&operation.0.target),
            ).await
        }
    };
}
macro_rules! turn_query {
    ($name:ident, $operation:ident => $output:ty) => {
        pub async fn $name(
            &self,
            turn_id: TurnId,
            options: CallOptions,
        ) -> Result<$output, CallError> {
            self.client
                .call(
                    $operation(TurnTarget {
                        target: self.target.clone(),
                        turn_id,
                    }),
                    options,
                    |operation| {
                        Correlation::session(&operation.0.target).turn(&operation.0.turn_id)
                    },
                )
                .await
        }
    };
}
macro_rules! session_read {
    ($(#[$meta:meta])* $name:ident, $operation:ident, $request:ty => $output:ty) => {
        $(#[$meta])*
        pub async fn $name(&self, request: $request, options: CallOptions) -> Result<$output, CallError> {
            self.client.call(
                $operation(SessionRequest { target: self.target.clone(), request }), options,
                |operation| Correlation::session(&operation.0.target),
            ).await
        }
    };
}
impl SessionClient {
    pub fn target(&self) -> &SessionRef {
        &self.target
    }
    pub async fn send(
        &self,
        request: MessageInput,
        options: CallOptions,
    ) -> Result<MessageAccepted, CallError> {
        self.client
            .send(SendMessage::new(request, self.target.clone()), options)
            .await
    }
    session_query!(info, GetSession => SessionInfo);
    /// Changes commit as one candidate. A running Turn keeps its old binding.
    pub async fn update(
        &self,
        request: SessionUpdateRequest,
        options: CallOptions,
    ) -> Result<SessionUpdateReceipt, CallError> {
        self.client
            .call(
                UpdateSession(UpdateSessionParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    expected_config_revision: request.expected_config_revision,
                    changes: request.changes,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    session_query!(inputs, ListInputs => InputList);
    pub async fn remove_input(
        &self,
        request: InputRemoveRequest,
        options: CallOptions,
    ) -> Result<InputRemoveReceipt, CallError> {
        self.client
            .call(
                RemoveInput(RemoveInputParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    message_id: request.message_id,
                    expected_controller_revision: request.expected_controller_revision,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.operation_id)
                        .message(&request.message_id, None)
                },
            )
            .await
    }
    session_query!(
        /// Read only facts already held by the Session. Cold history stays unknown;
        /// this call does not load, prepare resources, or invoke a model.
        context, GetSessionContext => SessionContextSnapshot
    );
    session_query!(state, SessionState => SessionSnapshot);
    session_control! { pause, PauseSession; resume, ResumeSession; load, LoadSession; unload, UnloadSession; }
    /// Start manual compaction at the Session owner without creating a Turn or
    /// queuing a command. The receipt records acceptance, not completion. Read
    /// later progress from Session state; abandoning this wait does not cancel it.
    pub async fn compact(
        &self,
        request: CompactRequest,
        options: CallOptions,
    ) -> Result<CompactReceipt, CallError> {
        self.client
            .call(
                CompactSession(SessionOperation {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    /// Request cancellation of exactly the original manual operation. This
    /// does not clear either input queue, cancel a later operation, or promise
    /// rollback of an already committed history replacement.
    pub async fn cancel_compact(
        &self,
        request: CompactCancelRequest,
        options: CallOptions,
    ) -> Result<CompactCancelReceipt, CallError> {
        self.client
            .call(
                CancelCompact(CancelCompactParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    target_operation_id: request.target_operation_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.operation_id)
                        .target_operation(&request.target_operation_id)
                },
            )
            .await
    }
    pub async fn cancel(
        &self,
        request: SessionCancelRequest,
        options: CallOptions,
    ) -> Result<SessionCancelReceipt, CallError> {
        self.client
            .call(
                CancelSession(CancelSessionParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    keep_inbox: request.keep_inbox,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    pub async fn close(
        &self,
        request: SessionCloseRequest,
        options: CallOptions,
    ) -> Result<SessionCloseReceipt, CallError> {
        self.client
            .call(
                CloseSession(SessionOperation {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    /// Admission is separate from later close, worker settlement and file removal.
    pub async fn delete(
        &self,
        request: SessionDeleteRequest,
        options: CallOptions,
    ) -> Result<SessionDeleteReceipt, CallError> {
        self.client
            .call(
                DeleteSession(DeleteSessionParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    close: request.close,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).operation(&request.operation_id)
                },
            )
            .await
    }
    pub async fn message(
        &self,
        message_id: MessageId,
        options: CallOptions,
    ) -> Result<MessageSnapshot, CallError> {
        self.client
            .call(
                GetMessage(MessageTargetParams {
                    target: self.target.clone(),
                    message_id,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).message(&request.message_id, None)
                },
            )
            .await
    }
    session_query!(turns, ListTurns => TurnListSnapshot);
    turn_query!(turn, GetTurn => TurnDetailsSnapshot);
    turn_query!(wait_turn, WaitTurn => TurnResultSnapshot);
    pub async fn cancel_turn(
        &self,
        request: TurnCancelRequest,
        options: CallOptions,
    ) -> Result<TurnCancelReceipt, CallError> {
        self.client
            .call(
                CancelTurn(CancelTurnParams {
                    target: self.target.clone(),
                    operation_id: request.operation_id,
                    turn_id: request.turn_id,
                    keep_inbox: request.keep_inbox,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.operation_id)
                        .turn(&request.turn_id)
                },
            )
            .await
    }
    turn_query!(turn_result, GetTurnResult => TurnResultSnapshot);
    pub async fn answer_interaction(
        &self,
        request: InteractionAnswerRequest<InteractionAnswerData>,
        options: CallOptions,
    ) -> Result<InteractionAnswerReceipt, CallError> {
        self.client
            .call(
                AnswerInteraction(SessionRequest {
                    target: self.target.clone(),
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target)
                        .operation(&request.request.operation_id)
                        .turn(&request.request.turn_id)
                        .load(&request.request.load_id)
                },
            )
            .await
    }
    pub async fn turn_output(
        &self,
        turn_id: TurnId,
        response_index: u64,
        request: TextReadRequest,
        options: CallOptions,
    ) -> Result<ModelTextSnapshot, CallError> {
        self.client
            .call(
                ReadModelOutput(ModelReadParams {
                    target: self.target.clone(),
                    turn_id,
                    response_index,
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).turn(&request.turn_id)
                },
            )
            .await
    }
    pub async fn toolcall(
        &self,
        tool: ToolCallRef,
        options: CallOptions,
    ) -> Result<ToolCallSnapshot, CallError> {
        self.client
            .call(
                GetToolcall(ToolTargetParams {
                    target: self.target.clone(),
                    tool,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).tool(&request.tool)
                },
            )
            .await
    }
    pub async fn tool_output(
        &self,
        tool: ToolCallRef,
        request: TextReadRequest,
        options: CallOptions,
    ) -> Result<ToolTextSnapshot, CallError> {
        self.client
            .call(
                ReadToolOutput(ToolReadParams {
                    target: self.target.clone(),
                    tool,
                    request,
                }),
                options,
                |operation| {
                    let request = &operation.0;
                    Correlation::session(&request.target).tool(&request.tool)
                },
            )
            .await
    }
    session_read!(
        /// Read only the explicitly addressed Session's committed workspace.
        workspace_files, WorkspaceFiles, FilesRequest => WorkspaceSnapshot<FilesResult>
    );
    session_read!(workspace_read, WorkspaceRead, ReadRequest => WorkspaceSnapshot<ReadResult>);
    session_read!(workspace_search, WorkspaceSearch, SearchRequest => WorkspaceSnapshot<SearchResult>);
    session_read!(changes_list, ChangesList, ChangesListRequest => WorkspaceSnapshot<ChangesListResult>);
    session_read!(changes_diff, ChangesDiff, DiffRequest => WorkspaceSnapshot<DiffResult>);
    session_query!(workspace_status, WorkspaceStatus => WorkspaceSnapshot<StatusResult>);

    session_read!(history, ReadHistory, HistoryReadRequest => SessionHistorySnapshot);
}
