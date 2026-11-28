//! Public update acceptance followed by observable metadata completion.
use minicore_host_core::{CallerContext, Host, SessionInfo};
use minicore_host_protocol::*;

pub async fn commit_session_config(
    host: &Host,
    caller: &CallerContext,
    target: &SessionRef,
    expected_revision: u64,
    patch: &SessionConfigPatch,
) -> Result<SessionInfo, Fault> {
    host.session_update(
        caller,
        target,
        SessionUpdateRequest {
            operation_id: OperationId::new().unwrap(),
            expected_config_revision: expected_revision,
            changes: SessionUpdateChanges {
                config: patch.clone(),
                ..Default::default()
            },
        },
    )
    .await?;
    let mut states = host.session_states(caller, target)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = states.snapshot()?.data;
            if let Some(fault) = state.metadata_fault {
                return Err(fault);
            }
            if !state.metadata_saving {
                return host.session_info(caller, target);
            }
            states.changed().await?;
        }
    })
    .await
    .expect("accepted Session metadata did not settle")
}
