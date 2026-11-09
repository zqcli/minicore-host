//! Actual temporary storage and blocking-pool gates, not simulated save success.
mod common;
use common::*;
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_transport::*;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

async fn persistent() -> (
    tempfile::TempDir,
    Arc<Host>,
    HostClient,
    AgentRef,
    SessionClient,
    Arc<FixtureResources>,
) {
    let directory = tempfile::tempdir().unwrap();
    let (_, resources) = fixture();
    let host = Host::open_persistent(
        directory.path(),
        HostSettings {
            name: "public controls".into(),
        },
        resources.clone(),
    )
    .await
    .unwrap();
    let client = HostClient::in_process(host.clone()).unwrap();
    let mut create = create_request();
    create.mode = StorageMode::Persistent;
    let receipt = client.create_agent(create, options(5000)).await.unwrap();
    let agent = AgentRef {
        host_id: client.initial_info().host_id.clone(),
        agent_id: receipt.agent_id.unwrap(),
    };
    let session = client.session(SessionRef {
        agent_id: agent.agent_id.clone(),
        session_id: receipt.session_id.unwrap(),
    });
    session.pause(control(), options(2000)).await.unwrap();
    (directory, host, client, agent, session, resources)
}
fn agent_update() -> AgentUpdateRequest {
    AgentUpdateRequest {
        operation_id: OperationId::new().unwrap(),
        expected_config_revision: 0,
        changes: AgentUpdateChanges {
            name: Some("saved Agent".into()),
            config: AgentConfigPatch {
                role: Some(Role {
                    name: "saved default".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        },
    }
}
fn session_update() -> SessionUpdateRequest {
    SessionUpdateRequest {
        operation_id: OperationId::new().unwrap(),
        expected_config_revision: 0,
        changes: SessionUpdateChanges {
            title: NullablePatch::Set("saved title".into()),
            config: SessionConfigPatch {
                role: Some(Role {
                    name: "saved role".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        },
    }
}
fn paths(
    directory: &std::path::Path,
    agent: &AgentRef,
    session: &SessionClient,
) -> (PathBuf, PathBuf) {
    let root = directory.join("agents").join(agent.agent_id.as_str());
    (
        root.join("agent.json"),
        root.join("sessions")
            .join(session.target().session_id.as_str())
            .join("session.json"),
    )
}
fn read(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
fn queue_disk_gate() -> (
    std::sync::mpsc::Sender<()>,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (release, held) = std::sync::mpsc::channel::<()>();
    let (entered, entering) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        entered.send(()).unwrap();
        held.recv_timeout(Duration::from_secs(15)).unwrap();
    });
    (release, task, entering)
}
async fn gate_disk() -> (std::sync::mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let (release, task, entering) = queue_disk_gate();
    entering.await.unwrap();
    (release, task)
}
fn arm_validation(resources: &FixtureResources) -> Arc<ValidationGate> {
    let gate = ValidationGate::new();
    *resources.gate.lock().unwrap() = Some((
        resources.validate_calls.load(Ordering::SeqCst) + 1,
        gate.clone(),
    ));
    gate
}
async fn settled(client: &HostClient, agent: &AgentRef, session: &SessionClient) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !client
                .agent(agent.clone(), options(2000))
                .await
                .unwrap()
                .info
                .metadata_saving
                && !session
                    .state(options(2000))
                    .await
                    .unwrap()
                    .data
                    .metadata_saving
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap()
}
#[test]
fn accepted_updates_outlive_dropped_callers_and_keep_original_receipts() {
    runtime().block_on(async {
        let (directory, host, client, agent, session, resources) = persistent().await;
        let (agent_path, session_path) = paths(directory.path(), &agent, &session);
        let original_agent = read(&agent_path);
        let original_session = read(&session_path);
        // Both pure validations are queued ahead of the disk blocker. The
        // single blocking worker then holds real saves behind that blocker.
        let validation = arm_validation(&resources);
        let mut agent_call =
            Box::pin(client.update_agent(agent.clone(), agent_update(), options(5000)));
        assert!(futures_util::poll!(agent_call.as_mut()).is_pending());
        validation.entered.acquire().await.unwrap().forget();
        let mut session_call = Box::pin(session.update(session_update(), options(5000)));
        assert!(futures_util::poll!(session_call.as_mut()).is_pending());
        let (release, blocked, entering) = queue_disk_gate();
        validation.release();
        entering.await.unwrap();
        let agent_receipt = agent_call.await.unwrap();
        let session_receipt = session_call.await.unwrap();
        assert_eq!(
            agent_receipt.result,
            MetadataUpdateResult::Saving { config_revision: 0 }
        );
        assert_eq!(
            session_receipt.result,
            MetadataUpdateResult::Saving { config_revision: 0 }
        );
        assert_eq!(
            client
                .agent(agent.clone(), options(2000))
                .await
                .unwrap()
                .info
                .config_revision,
            0
        );
        assert_eq!(
            session.info(options(2000)).await.unwrap().config_revision,
            0
        );
        assert_eq!(read(&agent_path), original_agent);
        assert_eq!(read(&session_path), original_session);
        client.close();
        drop(session);
        drop(client);
        let next = HostClient::in_process(host).unwrap();
        let session = next.session(SessionRef {
            agent_id: agent.agent_id.clone(),
            session_id: session_receipt.session_id.clone(),
        });
        release.send(()).unwrap();
        blocked.await.unwrap();
        settled(&next, &agent, &session).await;
        assert_eq!(
            next.agent(agent.clone(), options(2000))
                .await
                .unwrap()
                .info
                .name,
            "saved Agent"
        );
        assert_eq!(
            session.info(options(2000)).await.unwrap().title.as_deref(),
            Some("saved title")
        );
        assert_eq!(read(&agent_path)["data"]["config_revision"], 1);
        assert_eq!(read(&session_path)["data"]["config_revision"], 1);
        assert_eq!(
            next.receipt(agent_receipt.operation_id.clone(), options(2000))
                .await
                .unwrap(),
            Receipt::AgentUpdate(agent_receipt)
        );
        assert_eq!(
            next.receipt(session_receipt.operation_id.clone(), options(2000))
                .await
                .unwrap(),
            Receipt::SessionUpdate(session_receipt)
        );
        next.close();
    });
}
#[test]
fn failed_metadata_saves_keep_committed_config_and_report_fault_without_rewriting_receipt() {
    runtime().block_on(async {
        let (directory, _host, client, agent, session, _resources) = persistent().await;
        let (agent_path, session_path) = paths(directory.path(), &agent, &session);
        let original_agent = client
            .agent(agent.clone(), options(2000))
            .await
            .unwrap()
            .info;
        let original_session = session.info(options(2000)).await.unwrap();
        for path in [&agent_path, &session_path] {
            std::fs::rename(path, path.with_extension("original")).unwrap();
            std::fs::create_dir(path).unwrap();
        }
        let agent_receipt = client
            .update_agent(agent.clone(), agent_update(), options(2000))
            .await
            .unwrap();
        let session_receipt = session
            .update(session_update(), options(2000))
            .await
            .unwrap();
        settled(&client, &agent, &session).await;
        let after = client
            .agent(agent.clone(), options(2000))
            .await
            .unwrap()
            .info;
        assert_eq!(after.config, original_agent.config);
        assert_eq!(after.name, original_agent.name);
        assert_eq!(after.config_revision, 0);
        assert!(after.metadata_fault.is_some());
        let after = session.info(options(2000)).await.unwrap();
        assert_eq!(after.config, original_session.config);
        assert_eq!(after.title, original_session.title);
        assert_eq!(after.config_revision, 0);
        assert!(
            session
                .state(options(2000))
                .await
                .unwrap()
                .data
                .metadata_fault
                .is_some()
        );
        assert_eq!(
            client
                .receipt(agent_receipt.operation_id.clone(), options(2000))
                .await
                .unwrap(),
            Receipt::AgentUpdate(agent_receipt)
        );
        assert_eq!(
            client
                .receipt(session_receipt.operation_id.clone(), options(2000))
                .await
                .unwrap(),
            Receipt::SessionUpdate(session_receipt)
        );
        for path in [&agent_path, &session_path] {
            std::fs::remove_dir(path).unwrap();
            std::fs::rename(path.with_extension("original"), path).unwrap();
        }
        client.close();
    });
}
#[test]
fn default_save_preserves_old_pointer_until_commit_and_close_clears_exact_target() {
    runtime().block_on(async {
        let (directory,host,client,agent,old,_resources)=persistent().await;
        let new=client.create_session(SessionCreate{operation_id:OperationId::new().unwrap(),target:agent.clone(),mode:None,overrides:None,initial_message:None},options(5000)).await.unwrap();
        let new=client.session(SessionRef{agent_id:agent.agent_id.clone(),session_id:new.session_id.unwrap()});
        let (agent_path,_)=paths(directory.path(),&agent,&old);
        let (release,blocked)=gate_disk().await;
        let receipt=client.set_default_session(agent.clone(),AgentSetDefaultSessionRequest{operation_id:OperationId::new().unwrap(),session_id:Some(new.target().session_id.clone())},options(2000)).await.unwrap();
        assert_eq!(receipt.result,AgentSetDefaultSessionResult::Saving{default_session_id:Some(old.target().session_id.clone()),requested_session_id:Some(new.target().session_id.clone())});
        assert_eq!(client.agent(agent.clone(),options(2000)).await.unwrap().info.default_session_id,Some(old.target().session_id.clone()));
        let closed=new.close(SessionCloseRequest{operation_id:OperationId::new().unwrap()},options(2000)).await.unwrap();
        assert_eq!(closed.result.lifecycle,SessionCloseLifecycle::Closing);
        client.close();
        let next=HostClient::in_process(host).unwrap(); let new=next.session(new.target().clone());
        release.send(()).unwrap(); blocked.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),async { loop { if new.state(options(2000)).await.unwrap().data.lifecycle==SessionLifecycle::Closed {break;} tokio::task::yield_now().await; } }).await.unwrap();
        assert_eq!(next.agent(agent.clone(),options(2000)).await.unwrap().info.default_session_id,None);
        assert_eq!(read(&agent_path)["data"]["default_session_id"],Value::Null);
        assert_eq!(next.receipt(receipt.operation_id.clone(),options(2000)).await.unwrap(),Receipt::AgentSetDefaultSession(receipt));
        let unavailable=next.set_default_session(agent.clone(),AgentSetDefaultSessionRequest{operation_id:OperationId::new().unwrap(),session_id:Some(new.target().session_id.clone())},options(2000)).await.unwrap_err();
        assert!(matches!(unavailable,CallError::Business(f) if f.kind()==Some(FaultKind::AgentDefaultSessionUnavailable)));
        // A memory target remains a valid runtime default, but its ID is never saved.
        let memory=next.create_session(SessionCreate{operation_id:OperationId::new().unwrap(),target:agent.clone(),mode:Some(StorageMode::Memory),overrides:None,initial_message:None},options(5000)).await.unwrap();
        let memory_id=memory.session_id.unwrap();
        next.set_default_session(agent.clone(),AgentSetDefaultSessionRequest{operation_id:OperationId::new().unwrap(),session_id:Some(memory_id.clone())},options(2000)).await.unwrap();
        settled(&next,&agent,&new).await;
        assert_eq!(next.agent(agent.clone(),options(2000)).await.unwrap().info.default_session_id,Some(memory_id));
        assert_eq!(read(&agent_path)["data"]["default_session_id"],Value::Null);
        next.close();
    });
}

#[test]
fn lost_stdio_waiter_does_not_cancel_an_already_accepted_save() {
    runtime().block_on(async {
        let (directory,host,client,agent,session,resources)=persistent().await;
        let (_,session_path)=paths(directory.path(),&agent,&session);
        let target=session.target().clone(); client.close(); drop(session); drop(client);
        let (mut peer,server)=raw(host.clone());peer.initialize().await;
        let reader=HostClient::in_process(host.clone()).unwrap();let observer=reader.session(target.clone());
        let validation=arm_validation(&resources);
        let request=session_update();let operation=request.operation_id.clone();
        peer.write(serde_json::json!({"jsonrpc":"2.0","id":"discarded","method":"session.update","params":{
            "target":target,"operation_id":request.operation_id,"expected_config_revision":request.expected_config_revision,"changes":request.changes
        }})).await;
        validation.entered.acquire().await.unwrap().forget();
        let (release,blocked,entering)=queue_disk_gate();
        validation.release();entering.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5),async {loop {if observer.state(options(2000)).await.unwrap().data.metadata_saving{break;}tokio::task::yield_now().await;}}).await.unwrap();
        let original=reader.receipt(operation.clone(),options(2000)).await.unwrap();
        assert!(matches!(original,Receipt::SessionUpdate(SessionUpdateReceipt{result:MetadataUpdateResult::Saving{..},..})));
        drop(peer); // No response is read and no retry is made.
        release.send(()).unwrap();blocked.await.unwrap();settled(&reader,&agent,&observer).await;
        assert_eq!(read(&session_path)["data"]["config_revision"],1);
        assert_eq!(reader.receipt(operation,options(2000)).await.unwrap(),original);
        reader.close();server.abort();
    });
}
