use super::{base, support::*};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use minicore_host_runtime::HistoryItem;
use minicore_host_storage::testing::read_history_bytes;
use serde_json::{Value, json};
use std::sync::Arc;

async fn target(
    host: &Arc<Host>,
    caller: &CallerContext,
    persistent: bool,
    automatic: bool,
) -> SessionRef {
    let mut request = base::request(vec![], None);
    request.config.role.instructions = "TARGET".into();
    request.config.session_defaults.execution.compaction = CompactionSettings {
        enabled: automatic,
        trigger_percent: 50,
        target_percent: 20,
    };
    if persistent {
        request.mode = StorageMode::Persistent;
    }
    let receipt = host.create_agent_async(caller, request).await.unwrap();
    SessionRef {
        agent_id: receipt.agent_id.unwrap(),
        session_id: receipt.session_id.unwrap(),
    }
}

async fn peer_history(
    fixture: &mut Fixture,
    host: &Arc<Host>,
    caller: &CallerContext,
    target: &SessionRef,
) -> (SessionRef, MessageId, MessageId, String, Value) {
    let parent = create(host, caller, "SOURCE", false, true).await;
    let original = send(
        host,
        caller,
        target,
        MessageMode::FollowUp,
        &"user historical fact ".repeat(80),
        None,
    );
    let first = fixture.next().await;
    assert_fixed(&first.body, "TARGET");
    first.finish(answer(&"user answer ".repeat(150)));
    settled(host, caller, target, 1).await;
    send(
        host,
        caller,
        &parent,
        MessageMode::FollowUp,
        "send peer facts",
        None,
    );
    let command = fixture.next().await;
    assert_fixed(&command.body, "SOURCE");
    let peer_text = "peer historical fact ".repeat(100);
    command.finish(tool_answer("peer-to-summary", "agent", json!({
        "action": "send", "target": {"kind": "session", "agent_id": target.agent_id, "session_id": target.session_id},
        "text": peer_text, "in_reply_to": original,
    })));
    let mut peer_http = None;
    for _ in 0..2 {
        let request = fixture.next().await;
        if request.role() == "TARGET" {
            assert_fixed(&request.body, "TARGET");
            peer_http = Some(request.body.clone());
            request.finish(answer(&"retained last answer ".repeat(130)));
        } else {
            assert_eq!(request.role(), "SOURCE");
            request.finish(answer("peer facts sent"));
        }
    }
    settled(host, caller, target, 2).await;
    settled(host, caller, &parent, 1).await;
    let receipt = tool_result(host, caller, &parent, "peer-to-summary");
    let id: MessageId = serde_json::from_value(receipt["message_id"].clone()).unwrap();
    let peer_http = peer_http.unwrap();
    assert_input(
        &peer_http["messages"][4],
        &id,
        MessageSource::Agent {
            source: parent.clone(),
        },
        Some(&original),
        &peer_text,
    );
    (parent, original, id, peer_text, peer_http)
}

#[tokio::test]
async fn manual_utility_and_uncovered_inputs_keep_provenance_and_persistent_marker_bytes() {
    let mut fixture = Fixture::start().await;
    let root = tempfile::tempdir().unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut config = config(&fixture.endpoint);
    config.models[0].context_window = 4096;
    let b = builder.clone();
    let host = Host::open_configured(root.path(), config.clone(), move |c| async move {
        Ok(prepared(&b, c).await)
    })
    .await
    .unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = target(&host, &caller, true, false).await;
    let (parent, original, peer_id, peer_text, peer_http) =
        peer_history(&mut fixture, &host, &caller, &target).await;
    let recent = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "recent task",
        Some(peer_id.clone()),
    );
    fixture.next().await.finish(answer("recent answer"));
    settled(&host, &caller, &target, 3).await;
    let raw_before = host.history(&caller, &target).unwrap();
    let native_bytes = serde_json::to_vec(&raw_before).unwrap();
    let path = history_file(root.path());
    let stored_before = read_history_bytes(&path).unwrap();
    host.session_compact(
        &caller,
        &target,
        CompactRequest {
            operation_id: OperationId::new().unwrap(),
        },
    )
    .unwrap();
    let utility = fixture.next().await;
    let data = utility_data(&utility.body);
    let peer = utility_input(&data, &peer_id);
    assert_input(
        &peer,
        &peer_id,
        MessageSource::Agent {
            source: parent.clone(),
        },
        Some(&original),
        &peer_text,
    );
    assert_eq!(
        peer, peer_http["messages"][4],
        "utility reuses the same per-item model projection"
    );
    assert!(
        !data.to_string().contains(recent.as_str()),
        "the retained suffix is outside coverage"
    );
    utility.finish(answer("Fixture summary of prior conversation."));
    compacted(&host, &caller, &target).await;
    assert_eq!(
        serde_json::to_vec(&host.history(&caller, &target).unwrap()).unwrap(),
        native_bytes
    );
    let stored_after = read_history_bytes(&path).unwrap();
    assert_eq!(
        &stored_after[..stored_before.len()],
        stored_before.as_slice()
    );
    let marker: Value = serde_json::from_slice(&stored_after[stored_before.len()..]).unwrap();
    assert_eq!(marker["entry"]["kind"], "compaction");
    assert_eq!(marker["entry"]["data"]["covered_items"], 4);
    let marker_position = std::str::from_utf8(&stored_before).unwrap().lines().count() as u64;
    assert_eq!(marker["entry"]["data"]["source_position"], marker_position);
    assert_eq!(
        marker["entry"]["data"]["summary"],
        json!({"content": "Fixture summary of prior conversation."})
    );
    assert_eq!(
        marker["entry"]["data"].as_object().unwrap().len(),
        3,
        "no new durable marker fields"
    );
    let fresh = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "after compaction",
        Some(recent.clone()),
    );
    let normal = fixture.next().await;
    assert_fixed(&normal.body, "TARGET");
    assert_eq!(
        normal.body["messages"][2],
        json!({"role": "system", "content": "Conversation summary:\nFixture summary of prior conversation."})
    );
    assert_input(
        &normal.body["messages"][3],
        &recent,
        MessageSource::User,
        Some(&peer_id),
        "recent task",
    );
    assert_input(
        &normal.body["messages"][5],
        &fresh,
        MessageSource::User,
        Some(&recent),
        "after compaction",
    );
    assert!(!normal.body.to_string().contains(&peer_text));
    normal.finish(answer("after compaction reply"));
    settled(&host, &caller, &target, 4).await;
    let final_prefix = read_history_bytes(&path).unwrap();
    assert_eq!(&final_prefix[..stored_after.len()], stored_after.as_slice());
    stop(&host).await;
    drop(connection);
    drop(host);
    let b = builder.clone();
    let reopened = Host::open_configured(root.path(), config, move |c| async move {
        Ok(prepared(&b, c).await)
    })
    .await
    .unwrap();
    let connection = reopened.connect().unwrap();
    let caller = connection.caller();
    let restart = send(
        &reopened,
        &caller,
        &target,
        MessageMode::FollowUp,
        "after marker reload",
        Some(fresh.clone()),
    );
    let normal = fixture.next().await;
    assert_fixed(&normal.body, "TARGET");
    assert_input(
        &normal.body["messages"][3],
        &recent,
        MessageSource::User,
        Some(&peer_id),
        "recent task",
    );
    assert_input(
        &normal.body["messages"][5],
        &fresh,
        MessageSource::User,
        Some(&recent),
        "after compaction",
    );
    assert_input(
        &normal.body["messages"][7],
        &restart,
        MessageSource::User,
        Some(&fresh),
        "after marker reload",
    );
    assert_eq!(
        &read_history_bytes(&path).unwrap()[..final_prefix.len()],
        final_prefix.as_slice()
    );
    normal.finish(answer("marker reloaded"));
    settled(&reopened, &caller, &target, 1).await;
    let raw = reopened.history(&caller, &target).unwrap();
    assert_eq!(&raw[..raw_before.len()], raw_before.as_slice());
    assert_eq!(
        raw.iter()
            .filter(|item| matches!(item, HistoryItem::Summary(_)))
            .count(),
        0,
        "markers never rewrite native history"
    );
    stop(&reopened).await;
}

#[tokio::test]
async fn automatic_utility_uses_peer_projection_and_protects_actual_fresh_input() {
    let mut fixture = Fixture::start().await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut config = config(&fixture.endpoint);
    config.models[0].context_window = 4096;
    let host = Host::memory_configured(prepared(&builder, config).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = target(&host, &caller, false, true).await;
    let (parent, original, peer_id, peer_text, peer_http) =
        peer_history(&mut fixture, &host, &caller, &target).await;
    let before = host.history(&caller, &target).unwrap();
    let fresh = send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "automatic fresh input",
        Some(peer_id.clone()),
    );
    let utility = fixture.next().await;
    let data = utility_data(&utility.body);
    let peer = utility_input(&data, &peer_id);
    assert_input(
        &peer,
        &peer_id,
        MessageSource::Agent { source: parent },
        Some(&original),
        &peer_text,
    );
    assert_eq!(peer, peer_http["messages"][4]);
    assert!(!data.to_string().contains(fresh.as_str()));
    assert!(!data.to_string().contains("automatic fresh input"));
    let accepted = host.history(&caller, &target).unwrap();
    assert_eq!(accepted.len(), before.len() + 1);
    assert_eq!(&accepted[..before.len()], before.as_slice());
    assert!(
        matches!(&accepted[before.len()..], [HistoryItem::User(user)]
        if user.input.as_text() == "automatic fresh input" && user.kind == minicore_host_runtime::UserMessageKind::Prompt)
    );
    let fresh_record = host.message(&caller, &target, &fresh).unwrap();
    assert_eq!(fresh_record.input.message_id, fresh);
    assert_eq!(fresh_record.source, MessageSource::User);
    assert_eq!(fresh_record.input.mode, MessageMode::FollowUp);
    assert_eq!(fresh_record.input.in_reply_to, Some(peer_id.clone()));
    utility.finish(answer("Automatic fixture summary."));
    let normal = fixture.next().await;
    assert_fixed(&normal.body, "TARGET");
    assert_eq!(
        normal.body["messages"][2],
        json!({"role": "system", "content": "Conversation summary:\nAutomatic fixture summary."})
    );
    assert_input(
        normal.body["messages"].as_array().unwrap().last().unwrap(),
        &fresh,
        MessageSource::User,
        Some(&peer_id),
        "automatic fresh input",
    );
    assert_eq!(host.history(&caller, &target).unwrap(), accepted);
    normal.finish(answer("after automatic compaction"));
    settled(&host, &caller, &target, 3).await;
    compacted(&host, &caller, &target).await;
    let finished = host.history(&caller, &target).unwrap();
    assert_eq!(finished.len(), accepted.len() + 1);
    assert_eq!(&finished[..accepted.len()], accepted.as_slice());
    assert!(matches!(finished.last(), Some(HistoryItem::Assistant(_))));
    assert_eq!(
        finished
            .iter()
            .filter(|item| matches!(item, HistoryItem::User(user)
        if user.input.as_text() == "automatic fresh input"))
            .count(),
        1
    );
    stop(&host).await;
}
