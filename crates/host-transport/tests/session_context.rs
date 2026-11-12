//! Canonical context reads through all three adapters, with actual owner facts.
mod common;
mod os_pipe;
mod wss_common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

async fn exercise(client: HostClient, reader: HostClient, resources: Arc<FixtureResources>) {
    let session = create(&client).await;
    let readonly = reader.session(session.target().clone());
    session.pause(control(), options(2000)).await.unwrap();
    let validations = resources.validate_calls.load(Ordering::SeqCst);
    let empty = session.context(options(2000)).await.unwrap();
    assert_eq!(empty.source.session_id, session.target().session_id);
    assert_eq!(empty.data.history.as_ref().unwrap().raw_items, 0);
    assert_eq!(
        empty
            .data
            .history
            .as_ref()
            .unwrap()
            .latest_response
            .as_ref()
            .map(|u| u.raw_item_index),
        None
    );
    session
        .send(input("queued, not context history"), options(2000))
        .await
        .unwrap();
    let queued = session.context(options(2000)).await.unwrap();
    assert_eq!(queued.data.queued_inputs.next_turn, 1);
    assert_eq!(queued.data.queued_inputs.next_step, 0);
    assert_eq!(queued.data.history.as_ref().unwrap().raw_items, 0);
    assert!(queued.data.active_turn.is_none());
    assert_eq!(
        value(&queued),
        value(readonly.context(options(2000)).await.unwrap())
    );
    assert_eq!(resources.validate_calls.load(Ordering::SeqCst), validations);
    assert_eq!(resources.model.calls.load(Ordering::SeqCst), 0);

    session.resume(control(), options(2000)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), resources.model.started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let turn = running(&session).await;
    let mut execution = ExecutionSettings::default();
    execution.compaction.enabled = false;
    session
        .update(
            SessionUpdateRequest {
                operation_id: OperationId::new().unwrap(),
                expected_config_revision: 0,
                changes: SessionUpdateChanges {
                    title: NullablePatch::Unchanged,
                    config: SessionConfigPatch {
                        execution: Some(execution),
                        ..Default::default()
                    },
                },
            },
            options(2000),
        )
        .await
        .unwrap();
    let active = readonly.context(options(2000)).await.unwrap();
    assert_eq!(active.data.config_revision, 1);
    assert!(!active.data.compaction_settings.enabled);
    let binding = active.data.active_turn.unwrap();
    assert_eq!(binding.turn_id, turn);
    assert_eq!(binding.config_revision, 0);
    assert!(binding.compaction_settings.enabled);
    assert_eq!(binding.model.model.as_str(), "fixture/model");
    assert_eq!(binding.model.context_window, 128000);
    assert_eq!(resources.model.calls.load(Ordering::SeqCst), 1);
    resources.model.finish.add_permits(1);
    session.wait_turn(turn, options(3000)).await.unwrap();
    let ended = readonly.context(options(2000)).await.unwrap();
    assert!(ended.data.active_turn.is_none());
    let history = ended.data.history.unwrap();
    assert_eq!(history.raw_items, 2);
    assert_eq!(history.effective_items, 2);
    let response = history.latest_response.unwrap();
    assert_eq!(response.model.as_str(), "fixture/model");
    assert_eq!(response.raw_item_index, 1);
    assert_eq!(
        response.usage,
        minicore_host_runtime::model::Usage::default()
    );
    assert!(!ended.data.history_save.pending);
    assert_eq!(resources.model.calls.load(Ordering::SeqCst), 1);
    client.close();
    reader.close();
}
#[tokio::test]
async fn typed_context_reads_preserve_real_queue_history_and_bound_resources() {
    let (host, resources) = fixture();
    let client = HostClient::in_process(host.clone()).unwrap();
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader, resources).await;
}
#[tokio::test]
async fn os_pipe_context_reads_preserve_same_owner_facts() {
    let (host, resources) = fixture();
    let (client, server) = stdio_pipes(host.clone()).await;
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader, resources).await;
    server.abort();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tls_wss_context_reads_preserve_same_owner_facts() {
    let (host, resources) = fixture();
    let server = wss_common::Server::start(host.clone()).await;
    let client = server.client(wss_common::FIRST).await.unwrap();
    let reader = HostClient::in_process(host).unwrap();
    exercise(client, reader, resources).await;
}
#[tokio::test]
async fn cold_persistent_context_stays_null_until_explicit_owner_load_over_stdio() {
    let directory = tempfile::tempdir().unwrap();
    let (_, resources) = fixture();
    let host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "context transport".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let (client, server) = stdio_pipes(host).await;
    let mut request = create_request();
    request.mode = StorageMode::Persistent;
    let created = client.create_agent(request, options(5000)).await.unwrap();
    let session = client.session(SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    });
    let before = session.context(options(2000)).await.unwrap();
    assert!(!before.data.history_loaded);
    assert!(before.data.history.is_none());
    assert_eq!(value(&before)["data"]["history"], serde_json::Value::Null);
    assert_eq!(resources.model.calls.load(Ordering::SeqCst), 0);
    session.load(control(), options(2000)).await.unwrap();
    let loaded = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let context = session.context(options(2000)).await.unwrap();
            if context.data.history_loaded {
                break context;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(loaded.data.history.unwrap().raw_items, 0);
    assert_eq!(resources.model.calls.load(Ordering::SeqCst), 0);
    client.close();
    server.abort();
}
